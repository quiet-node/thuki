//! Extractive filter: chunk fetched pages and rank the chunks against the
//! standalone question so only the passages that actually answer it reach the
//! writer's context budget.
//!
//! Ranking sits behind the [`Scorer`] trait, which takes the query and a page's
//! chunks and returns a per-chunk relevance score. v1 ships one implementation,
//! [`Bm25Scorer`] (Okapi BM25 over the chunk set as its own corpus); a dense
//! embedding scorer joins it later via rank fusion without changing this
//! module's shape — the trait is the seam. Everything here is pure CPU over its
//! inputs, so it is unit-tested directly with no model or network.

use crate::config::defaults::{
    BM25_B, BM25_K1, CHUNK_TARGET_WORDS, QUOTE_STAT_SCORE_NUDGE, RANK_MAX_CHUNKS_PER_PAGE,
    STATISTIC_MIN_DIGIT_RUN,
};
use crate::websearch::credibility::{classify_domain, DomainClass};
use crate::websearch::domain_of;
use crate::websearch::fetch::FetchedPage;

/// A page chunk that survived ranking, carrying the provenance the citation
/// stage needs (source URL and title) plus its relevance score.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredChunk {
    pub url: String,
    pub title: String,
    pub text: String,
    pub score: f64,
}

/// Scores each chunk's relevance to `query`. Higher is more relevant; a score
/// of `0.0` means no relevance (e.g. no shared terms) and the chunk is dropped
/// by [`select_chunks`]. The corpus statistics a scorer needs are derived from
/// the `chunks` slice itself.
pub trait Scorer: Send + Sync {
    fn score(&self, query: &str, chunks: &[String]) -> Vec<f64>;
}

/// Okapi BM25 scorer over the supplied chunks as the corpus.
pub struct Bm25Scorer;

impl Scorer for Bm25Scorer {
    fn score(&self, query: &str, chunks: &[String]) -> Vec<f64> {
        bm25_scores(query, chunks, BM25_K1, BM25_B)
    }
}

/// Splits `text` into chunks of about `target_words` whitespace-separated
/// words, preserving order. Empty or whitespace-only text yields no chunks.
pub(crate) fn chunk_text(text: &str, target_words: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    words
        .chunks(target_words.max(1))
        .map(|c| c.join(" "))
        .collect()
}

/// True when `text` carries a quoted phrase or an inline statistic: the two
/// content shapes GEO (arXiv:2311.09735) found LLM answer synthesis
/// preferentially cites. Used to decide whether [`QUOTE_STAT_SCORE_NUDGE`]
/// applies to a chunk from a boosted domain. Pure char scan, no regex crate,
/// bounded `O(len(text))`.
fn has_quote_or_statistic(text: &str) -> bool {
    has_quoted_phrase(text) || has_inline_statistic(text)
}

/// True when `text` contains a paired quotation mark: two or more straight
/// double quotes, or a curly opening/closing pair. A single stray mark (an
/// apostrophe-like straight quote, or an unmatched curly quote from a
/// truncated extraction) does not count, since it is not evidence of an
/// actual quoted phrase.
fn has_quoted_phrase(text: &str) -> bool {
    text.matches('"').count() >= 2 || (text.contains('\u{201c}') && text.contains('\u{201d}'))
}

/// True when `text` contains an inline statistic: a `%`-suffixed figure
/// (counted regardless of digit run length) or a run of at least
/// [`STATISTIC_MIN_DIGIT_RUN`] consecutive ASCII digits (a count, year, or
/// other reported figure).
fn has_inline_statistic(text: &str) -> bool {
    text.contains('%') || has_digit_run(text, STATISTIC_MIN_DIGIT_RUN)
}

/// True when `text` contains a run of at least `min_run` consecutive ASCII
/// digits. Split out of [`has_inline_statistic`] so the run-length threshold
/// is independently testable.
fn has_digit_run(text: &str, min_run: usize) -> bool {
    let mut run = 0usize;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= min_run {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Lowercased alphanumeric terms of `text`, splitting on any non-alphanumeric
/// character. The shared tokenisation for both query and chunks.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Okapi BM25 score of every chunk against `query`, treating the `chunks` slice
/// as the document corpus for term-frequency and inverse-document-frequency
/// statistics. A chunk sharing no query term scores `0.0`. Returns one score
/// per chunk, in input order.
pub(crate) fn bm25_scores(query: &str, chunks: &[String], k1: f64, b: f64) -> Vec<f64> {
    let n = chunks.len();
    if n == 0 {
        return Vec::new();
    }
    let chunk_terms: Vec<Vec<String>> = chunks.iter().map(|c| tokenize(c)).collect();
    let lengths: Vec<usize> = chunk_terms.iter().map(Vec::len).collect();
    let total_len: usize = lengths.iter().sum();
    if total_len == 0 {
        return vec![0.0; n];
    }
    let avgdl = total_len as f64 / n as f64;

    // Unique query terms and their document frequency across the chunks.
    let mut query_terms: Vec<String> = tokenize(query);
    query_terms.sort();
    query_terms.dedup();
    let df: std::collections::HashMap<&str, usize> = query_terms
        .iter()
        .map(|term| {
            let count = chunk_terms
                .iter()
                .filter(|terms| terms.iter().any(|t| t == term))
                .count();
            (term.as_str(), count)
        })
        .collect();

    (0..n)
        .map(|i| {
            let len = lengths[i] as f64;
            query_terms
                .iter()
                .map(|term| {
                    let tf = chunk_terms[i].iter().filter(|t| *t == term).count() as f64;
                    if tf == 0.0 {
                        return 0.0;
                    }
                    let dfi = df[term.as_str()] as f64;
                    let idf = (1.0 + (n as f64 - dfi + 0.5) / (dfi + 0.5)).ln();
                    let denom = tf + k1 * (1.0 - b + b * len / avgdl);
                    idf * (tf * (k1 + 1.0)) / denom
                })
                .sum()
        })
        .collect()
}

/// Chunks each page, scores its chunks against `query` through the injected
/// [`Scorer`], keeps the top [`RANK_MAX_CHUNKS_PER_PAGE`] chunks scoring above
/// zero per page, and returns them all best-first. A page whose chunks all
/// score zero contributes nothing. A chunk from a credibility-boosted
/// reference-grade domain that also carries a quote or inline statistic gets
/// [`QUOTE_STAT_SCORE_NUDGE`] added to its score before ranking (see
/// [`has_quote_or_statistic`]'s rustdoc for why).
pub fn select_chunks(pages: &[FetchedPage], query: &str, scorer: &dyn Scorer) -> Vec<ScoredChunk> {
    select_chunks_sized(pages, query, scorer, CHUNK_TARGET_WORDS)
}

/// [`select_chunks`] with an explicit chunk-word target, so the per-page cap
/// and page-drop logic can be exercised without a multi-hundred-word fixture.
fn select_chunks_sized(
    pages: &[FetchedPage],
    query: &str,
    scorer: &dyn Scorer,
    target_words: usize,
) -> Vec<ScoredChunk> {
    let mut out: Vec<ScoredChunk> = Vec::new();
    for page in pages {
        let chunks = chunk_text(&page.text, target_words);
        if chunks.is_empty() {
            continue;
        }
        let scores = scorer.score(query, &chunks);
        // Nudge is domain-level (one classification per page, not per chunk)
        // and only ever applied to a chunk that already scored above zero, so
        // it tips an already-relevant boosted-domain chunk higher and can
        // never resurrect an irrelevant one.
        let is_boosted_domain = classify_domain(&domain_of(&page.url)) == DomainClass::Boost;
        let scores: Vec<f64> = scores
            .into_iter()
            .zip(chunks.iter())
            .map(|(score, text)| {
                if is_boosted_domain && score > 0.0 && has_quote_or_statistic(text) {
                    score + QUOTE_STAT_SCORE_NUDGE
                } else {
                    score
                }
            })
            .collect();
        let mut kept: Vec<ScoredChunk> = chunks
            .into_iter()
            .zip(scores)
            .filter(|(_, score)| *score > 0.0)
            .map(|(text, score)| ScoredChunk {
                url: page.url.clone(),
                title: page.title.clone(),
                text,
                score,
            })
            .collect();
        sort_by_score_desc(&mut kept);
        kept.truncate(RANK_MAX_CHUNKS_PER_PAGE);
        out.extend(kept);
    }
    sort_by_score_desc(&mut out);
    out
}

/// Re-establishes global best-first (descending score) order over an owned
/// chunk set and returns it. Used by the engine-tier requery merge to fuse
/// round-one and round-two chunks into one relevance ranking, so a stronger
/// round-two chunk can outrank a weaker round-one one rather than the two
/// rounds' separately-sorted chunks staying in append order. Ties keep their
/// relative order (a stable sort), so within-round order is preserved on
/// equal scores.
pub(crate) fn rerank_by_score(mut chunks: Vec<ScoredChunk>) -> Vec<ScoredChunk> {
    sort_by_score_desc(&mut chunks);
    chunks
}

/// Sorts scored chunks by descending score. Ties keep their relative order
/// (stable), and any non-comparable score is treated as equal.
fn sort_by_score_desc(chunks: &mut [ScoredChunk]) {
    chunks.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(url: &str, text: &str) -> FetchedPage {
        FetchedPage {
            url: url.into(),
            title: "T".into(),
            text: text.into(),
            published: None,
        }
    }

    /// A scorer returning a scripted score per chunk (by index), so the
    /// selection logic is tested independently of BM25 math.
    struct FakeScorer(Vec<f64>);
    impl Scorer for FakeScorer {
        fn score(&self, _query: &str, chunks: &[String]) -> Vec<f64> {
            self.0.iter().take(chunks.len()).copied().collect()
        }
    }

    // ── chunk_text ────────────────────────────────────────────────────────────

    #[test]
    fn chunk_text_splits_on_word_target() {
        let text = (0..800)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let chunks = chunk_text(&text, 350);
        assert_eq!(chunks.len(), 3); // 350 + 350 + 100
        assert_eq!(chunks[0].split_whitespace().count(), 350);
        assert_eq!(chunks[2].split_whitespace().count(), 100);
    }

    #[test]
    fn chunk_text_single_chunk_when_short() {
        assert_eq!(chunk_text("a b c", 350), vec!["a b c"]);
    }

    #[test]
    fn chunk_text_empty_on_blank() {
        assert!(chunk_text("   \n\t ", 350).is_empty());
    }

    // ── tokenize ──────────────────────────────────────────────────────────────

    #[test]
    fn tokenize_lowercases_and_splits_nonalnum() {
        assert_eq!(
            tokenize("Rust's BM25, v2!"),
            vec!["rust", "s", "bm25", "v2"]
        );
    }

    // ── has_quote_or_statistic ────────────────────────────────────────────────

    #[test]
    fn has_quoted_phrase_needs_a_pair() {
        assert!(has_quoted_phrase(r#"she said "hello there" to him"#));
        assert!(has_quoted_phrase("curly \u{201c}quoted\u{201d} phrase"));
        assert!(!has_quoted_phrase("no quotes here"));
        // A single stray straight quote (e.g. a truncated extraction) is not a
        // quoted phrase.
        assert!(!has_quoted_phrase(r#"it was 6' tall"#));
    }

    #[test]
    fn has_inline_statistic_matches_percent_and_digit_run() {
        assert!(has_inline_statistic("inflation rose 4%"));
        assert!(has_inline_statistic("population is 2024 residents"));
        assert!(!has_inline_statistic("no numbers at all"));
        // Two digits is below the run threshold and carries no `%`.
        assert!(!has_inline_statistic("chapter 12 begins"));
    }

    #[test]
    fn has_digit_run_resets_on_non_digit() {
        assert!(!has_digit_run("1 2 3", 3)); // never 3 consecutive digits
        assert!(has_digit_run("123", 3));
        assert!(has_digit_run("a123b", 3));
    }

    #[test]
    fn has_quote_or_statistic_is_true_if_either_matches() {
        assert!(has_quote_or_statistic(r#""a direct quote""#));
        assert!(has_quote_or_statistic("42% of respondents"));
        assert!(!has_quote_or_statistic("plain unremarkable prose"));
    }

    // ── bm25_scores ───────────────────────────────────────────────────────────

    #[test]
    fn bm25_empty_corpus_is_empty() {
        assert!(bm25_scores("q", &[], 1.5, 0.75).is_empty());
    }

    #[test]
    fn bm25_all_empty_chunks_score_zero() {
        let chunks = vec![String::new(), "   ".into()];
        assert_eq!(bm25_scores("query", &chunks, 1.5, 0.75), vec![0.0, 0.0]);
    }

    #[test]
    fn bm25_scores_matching_chunk_above_nonmatching() {
        let chunks = vec![
            "the capital of france is paris".into(),
            "bananas are a yellow fruit".into(),
        ];
        let scores = bm25_scores("capital france paris", &chunks, 1.5, 0.75);
        assert!(scores[0] > 0.0);
        assert_eq!(scores[1], 0.0);
    }

    #[test]
    fn bm25_rarer_term_weighs_more() {
        // "paris" appears in one chunk (rare), "the" in both (common). The chunk
        // matching the rare term should outscore the one matching only common.
        let chunks = vec!["paris is the capital".into(), "the the the the the".into()];
        let scores = bm25_scores("paris the", &chunks, 1.5, 0.75);
        assert!(scores[0] > scores[1]);
    }

    // ── select_chunks ─────────────────────────────────────────────────────────

    #[test]
    fn select_drops_zero_score_pages() {
        let pages = vec![page("https://a/", "aaa"), page("https://b/", "bbb")];
        // page a's chunk scores 0.9, page b's chunk scores 0.0 (dropped).
        struct PerPage;
        impl Scorer for PerPage {
            fn score(&self, _q: &str, chunks: &[String]) -> Vec<f64> {
                chunks
                    .iter()
                    .map(|c| if c.contains("aaa") { 0.9 } else { 0.0 })
                    .collect()
            }
        }
        let out = select_chunks(&pages, "q", &PerPage);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://a/");
    }

    #[test]
    fn select_caps_chunks_per_page_and_sorts_desc() {
        // One page splitting into 5 single-word chunks, all scoring > 0.
        let big = page("https://a/", "one two three four five");
        let scorer = FakeScorer(vec![5.0, 4.0, 3.0, 2.0, 1.0]);
        let out = select_chunks_sized(&[big], "q", &scorer, 1);
        assert_eq!(out.len(), RANK_MAX_CHUNKS_PER_PAGE);
        assert_eq!(out[0].score, 5.0); // best-first
        assert_eq!(out[2].score, 3.0);
    }

    #[test]
    fn select_skips_pages_with_no_chunks() {
        let pages = vec![page("https://a/", "   "), page("https://b/", "real text")];
        let scorer = FakeScorer(vec![1.0]);
        let out = select_chunks(&pages, "q", &scorer);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://b/");
    }

    #[test]
    fn select_end_to_end_with_bm25() {
        let pages = vec![
            page("https://match/", "the treaty was signed in paris in 1919"),
            page("https://nomatch/", "cats sleep most of the day"),
        ];
        let out = select_chunks(&pages, "treaty paris 1919", &Bm25Scorer);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://match/");
    }

    // ── quote/statistic score nudge ──────────────────────────────────────────

    #[test]
    fn nudge_applies_only_to_boosted_domain_quote_or_stat_chunk() {
        // Same base score (1.0) and same statistic-bearing text on both pages;
        // only the wikipedia.org page is on the credibility-boost list, so
        // only its chunk should carry the nudge.
        let pages = vec![
            page(
                "https://en.wikipedia.org/wiki/Test",
                "population is 2024 residents",
            ),
            page("https://example.com/page", "population is 2024 residents"),
        ];
        let scorer = FakeScorer(vec![1.0]);
        let out = select_chunks_sized(&pages, "q", &scorer, 100);
        let boosted = out.iter().find(|c| c.url.contains("wikipedia")).unwrap();
        let neutral = out.iter().find(|c| c.url.contains("example")).unwrap();
        assert_eq!(boosted.score, 1.0 + QUOTE_STAT_SCORE_NUDGE);
        assert_eq!(neutral.score, 1.0);
    }

    #[test]
    fn nudge_not_applied_without_quote_or_statistic() {
        // Boosted domain, but the chunk carries no quote or statistic: score
        // is left untouched.
        let pages = vec![page(
            "https://en.wikipedia.org/wiki/Test",
            "a plain unremarkable sentence",
        )];
        let scorer = FakeScorer(vec![1.0]);
        let out = select_chunks_sized(&pages, "q", &scorer, 100);
        assert_eq!(out[0].score, 1.0);
    }

    #[test]
    fn nudge_never_resurrects_a_zero_score_chunk() {
        // Boosted domain, quote-bearing text, but the underlying relevance
        // score is zero (no query term matched): the nudge must not lift it
        // above zero and admit an irrelevant chunk.
        let pages = vec![page(
            "https://en.wikipedia.org/wiki/Test",
            r#""a direct quote" with 2024 in it"#,
        )];
        let scorer = FakeScorer(vec![0.0]);
        let out = select_chunks_sized(&pages, "q", &scorer, 100);
        assert!(out.is_empty());
    }
}
