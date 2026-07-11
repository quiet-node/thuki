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

use crate::config::defaults::{BM25_B, BM25_K1, CHUNK_TARGET_WORDS, RANK_MAX_CHUNKS_PER_PAGE};
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
/// score zero contributes nothing.
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
}
