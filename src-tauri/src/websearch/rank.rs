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
    BM25_B, BM25_K1, CHUNK_CJK_MAX_CHARS, CHUNK_CJK_SENTENCE_TERMINATORS, CHUNK_CJK_TARGET_CHARS,
    CHUNK_TARGET_WORDS, CHUNK_UNSPACED_RATIO_MIN, QUOTE_STAT_SCORE_NUDGE, RANK_MAX_CHUNKS_PER_PAGE,
    STATISTIC_MIN_DIGIT_RUN,
};
use crate::websearch::credibility::{classify_domain, DomainClass};
use crate::websearch::domain_of;
use crate::websearch::fetch::FetchedPage;
use crate::websearch::script::{is_bigram_script, unspaced_ratio};

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
///
/// Text dominated by an unspaced script (more than [`CHUNK_UNSPACED_RATIO_MIN`]
/// of its non-whitespace characters) has no word delimiters, so the word path
/// would return a handful of enormous units, often the whole page as one. Such
/// text takes the character-based path instead (see [`chunk_chars`]).
pub(crate) fn chunk_text(text: &str, target_words: usize) -> Vec<String> {
    if unspaced_ratio(text) > CHUNK_UNSPACED_RATIO_MIN {
        return chunk_chars(text, CHUNK_CJK_TARGET_CHARS, CHUNK_CJK_MAX_CHARS);
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    words
        .chunks(target_words.max(1))
        .map(|c| c.join(" "))
        .collect()
}

/// Chunks unspaced-script text on characters: split into sentences, hard-split
/// any sentence wider than `max_chars`, then greedily pack the pieces up to
/// `target_chars`. The hard split guarantees forward progress, so no chunk can
/// exceed `max_chars` however the page is punctuated (Thai prose, which carries
/// no sentence terminator at all, relies on it). Blank pieces are dropped, so
/// whitespace-only text yields no chunks.
fn chunk_chars(text: &str, target_chars: usize, max_chars: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut acc = String::new();
    let mut acc_chars = 0usize;
    for piece in split_sentences(text)
        .iter()
        .flat_map(|s| hard_split(s, max_chars))
    {
        let piece_chars = piece.chars().count();
        if acc_chars > 0 && acc_chars + piece_chars > target_chars {
            out.push(std::mem::take(&mut acc));
            acc_chars = 0;
        }
        acc.push_str(&piece);
        acc_chars += piece_chars;
    }
    if !acc.is_empty() {
        out.push(acc);
    }
    out
}

/// Splits `text` after every [`CHUNK_CJK_SENTENCE_TERMINATORS`] character,
/// keeping the terminator with the sentence it ends. A trailing run with no
/// terminator is its own sentence; a blank one is dropped.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if CHUNK_CJK_SENTENCE_TERMINATORS.contains(&ch) {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// Splits `sentence` into pieces of at most `max_chars` characters, trimming
/// each and dropping any that is blank. Splits on character boundaries, never
/// byte indices, so multi-byte text can never be cut mid-codepoint.
fn hard_split(sentence: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = sentence.chars().collect();
    chars
        .chunks(max_chars.max(1))
        .map(|w| w.iter().collect::<String>().trim().to_string())
        .filter(|p| !p.is_empty())
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

/// Lowercased terms of `text`, the shared tokenisation for both query and
/// chunks. Two kinds of run, split at every script boundary and at every
/// non-alphanumeric character:
///
/// - A run of alphanumeric characters outside the bigram scripts emits one
///   token, exactly as splitting on non-alphanumeric characters always did.
/// - A run of bigram-script characters (Han, Kana, Hangul, Thai, Lao, Khmer,
///   Myanmar) emits overlapping character bigrams, Lucene's shipped `cjk_bigram`
///   semantics. Rust's `char::is_alphanumeric` is true for those scripts, so
///   without this a whole punctuation-delimited clause became one mega-token
///   that no query term could ever match, and every chunk scored zero.
fn tokenize(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut bigram_run: Vec<char> = Vec::new();
    for ch in text.chars() {
        if is_bigram_script(ch) {
            push_word(&mut out, &mut word);
            bigram_run.push(ch);
        } else if ch.is_alphanumeric() {
            push_bigrams(&mut out, &mut bigram_run);
            word.push(ch);
        } else {
            push_word(&mut out, &mut word);
            push_bigrams(&mut out, &mut bigram_run);
        }
    }
    push_word(&mut out, &mut word);
    push_bigrams(&mut out, &mut bigram_run);
    out
}

/// Moves a non-bigram-script alphanumeric run into `out` as one lowercased
/// token, then clears it. A no-op for an empty run. Lowercasing the whole run
/// at once (not character by character) keeps multi-character lowerings, e.g.
/// `İ`, byte-identical to the previous `str::to_lowercase` tokenisation.
fn push_word(out: &mut Vec<String>, word: &mut String) {
    if !word.is_empty() {
        out.push(word.to_lowercase());
        word.clear();
    }
}

/// Moves a bigram-script run into `out` as its overlapping character bigrams
/// (`ABCD` yields `AB`, `BC`, `CD`), then clears it. A single-character run
/// emits that character as a unigram, so a one-character word is not lost. A
/// no-op for an empty run.
fn push_bigrams(out: &mut Vec<String>, run: &mut Vec<char>) {
    if run.is_empty() {
        return;
    }
    if run.len() == 1 {
        out.push(run[0].to_lowercase().collect());
    } else {
        for pair in run.windows(2) {
            out.push(pair.iter().collect::<String>().to_lowercase());
        }
    }
    run.clear();
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

    // ── chunk_text: unspaced-script (character) path ──────────────────────────

    /// A Chinese paragraph: no whitespace, so the word path returned the whole
    /// page as one oversized unit. The character path splits it on sentences.
    const ZH_PAGE: &str = "北京是中华人民共和国的首都，位于华北平原北部。北京的常住人口约为2184万人。故宫是明清两代的皇家宫殿，位于北京中轴线的中心。北京市总面积为16410平方公里，下辖16个区。北京也是中国的政治、文化和国际交往中心。";

    /// A Japanese paragraph, same shape as [`ZH_PAGE`].
    const JA_PAGE: &str = "東京は日本の首都であり、人口はおよそ1400万人です。東京都は関東地方に位置し、政治と経済の中心地として知られています。皇居は千代田区にあり、多くの観光客が訪れます。東京の面積は約2194平方キロメートルで、23の特別区から構成されています。";

    /// A Thai paragraph. Thai carries no sentence terminator at all, so it
    /// exercises the hard-split path exclusively.
    const TH_PAGE: &str = "กรุงเทพมหานครเป็นเมืองหลวงของประเทศไทย มีประชากรประมาณ 10 ล้านคน กรุงเทพมหานครตั้งอยู่บริเวณปากแม่น้ำเจ้าพระยา และเป็นศูนย์กลางทางเศรษฐกิจของประเทศ";

    #[test]
    fn chunk_text_takes_char_path_for_unspaced_script() {
        // Two sentences well inside the target pack into one chunk; a page of
        // sentences past the target splits into several. Either way every chunk
        // stays within the hard window, which the word path could not promise.
        let long_zh = ZH_PAGE.repeat(8);
        let chunks = chunk_text(&long_zh, CHUNK_TARGET_WORDS);
        assert!(chunks.len() > 1, "expected several chunks, got {chunks:?}");
        for c in &chunks {
            assert!(c.chars().count() <= CHUNK_CJK_MAX_CHARS);
        }
        // Nothing is dropped: the concatenation is the source text back again.
        assert_eq!(chunks.concat(), long_zh);
    }

    #[test]
    fn chunk_text_char_path_packs_short_page_into_one_chunk() {
        let chunks = chunk_text(JA_PAGE, CHUNK_TARGET_WORDS);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].chars().count() <= CHUNK_CJK_TARGET_CHARS);
    }

    #[test]
    fn chunk_text_char_path_hard_splits_a_terminatorless_page() {
        // Thai has no sentence terminator, so the whole page is one "sentence"
        // and only the hard window bounds it.
        let long_th = TH_PAGE.repeat(20);
        let chunks = chunk_text(&long_th, CHUNK_TARGET_WORDS);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.chars().count() <= CHUNK_CJK_MAX_CHARS);
        }
    }

    #[test]
    fn chunk_text_keeps_word_path_for_korean() {
        // Korean is bigram-tokenized but whitespace-delimited, so it must stay
        // on the word path: 4 words at a target of 2 gives 2 chunks.
        let chunks = chunk_text("한국 서울 인구 수", 2);
        assert_eq!(chunks, vec!["한국 서울", "인구 수"]);
    }

    #[test]
    fn chunk_text_keeps_word_path_below_the_ratio() {
        // Mostly Latin text carrying a couple of Han characters stays on the
        // word path (ratio under the threshold).
        let chunks = chunk_text("the chinese word for capital is 首都 here", 4);
        assert_eq!(chunks, vec!["the chinese word for", "capital is 首都 here"]);
    }

    #[test]
    fn hard_split_drops_blank_pieces_and_cuts_on_char_boundaries() {
        assert!(hard_split("   ", 4).is_empty());
        assert_eq!(hard_split("中文中文中", 2), vec!["中文", "中文", "中"]);
    }

    #[test]
    fn split_sentences_keeps_terminators_and_drops_a_blank_tail() {
        assert_eq!(
            split_sentences("一。二！三"),
            vec!["一。", "二！", "三"],
            "terminator stays with its sentence"
        );
        assert_eq!(split_sentences("一。  "), vec!["一。"]);
    }

    // ── tokenize ──────────────────────────────────────────────────────────────

    /// The tokenisation that shipped before bigram support: split on any
    /// non-alphanumeric character, lowercase each run. Kept here, and only
    /// here, as the reference the new tokenizer must reproduce exactly on text
    /// with no bigram-script character.
    fn old_tokenize(text: &str) -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(str::to_lowercase)
            .collect()
    }

    #[test]
    fn tokenize_matches_the_old_tokenizer_on_non_bigram_text() {
        let cases = [
            "Rust's BM25, v2!",
            "The treaty was signed in Paris in 1919.",
            // Vietnamese diacritics are alphanumeric letters and must survive
            // whole, not be split apart or stripped.
            "Hà Nội có dân số khoảng 8,4 triệu người, đường phố rất đông.",
            "2026-07-14T00:00:00Z",
            "...,,,;!?",
            "",
            "   ",
        ];
        for text in cases {
            assert_eq!(tokenize(text), old_tokenize(text), "diverged on {text:?}");
        }
    }

    #[test]
    fn tokenize_lowercases_and_splits_nonalnum() {
        assert_eq!(
            tokenize("Rust's BM25, v2!"),
            vec!["rust", "s", "bm25", "v2"]
        );
    }

    #[test]
    fn tokenize_keeps_vietnamese_diacritics_whole() {
        assert_eq!(
            tokenize("Đường phố Hà Nội"),
            vec!["đường", "phố", "hà", "nội"]
        );
    }

    #[test]
    fn tokenize_emits_overlapping_bigrams_for_cjk() {
        assert_eq!(tokenize("北京人口"), vec!["北京", "京人", "人口"]);
        // Punctuation ends the run, so no bigram straddles a sentence break.
        assert_eq!(tokenize("北京。人口"), vec!["北京", "人口"]);
    }

    #[test]
    fn tokenize_single_bigram_char_run_is_a_unigram() {
        assert_eq!(tokenize("单"), vec!["单"]);
        assert_eq!(tokenize("2184万人"), vec!["2184", "万人"]);
    }

    #[test]
    fn tokenize_splits_at_a_script_boundary() {
        assert_eq!(tokenize("iPhone15手机"), vec!["iphone15", "手机"]);
        assert_eq!(tokenize("手机iPhone15"), vec!["手机", "iphone15"]);
    }

    #[test]
    fn tokenize_bigrams_korean_and_thai() {
        assert_eq!(tokenize("한국어"), vec!["한국", "국어"]);
        assert_eq!(tokenize("ประชากร").len(), "ประชากร".chars().count() - 1);
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

    #[test]
    fn bm25_ranks_the_chunk_that_answers_a_cjk_question() {
        // Before bigram tokenisation both chunks scored exactly 0.0 (each was
        // one mega-token that no query term could equal), so the ranking was
        // arbitrary. The answering chunk must now win outright.
        let chunks = vec![
            "北京的常住人口约为2184万人。".to_string(),
            "香蕉是一种黄色的水果，富含钾元素。".to_string(),
        ];
        let scores = bm25_scores("北京 人口", &chunks, BM25_K1, BM25_B);
        assert!(scores[0] > 0.0);
        assert_eq!(scores[1], 0.0);
    }

    #[test]
    fn bm25_ranks_the_chunk_that_answers_a_japanese_question() {
        let chunks = vec![
            "東京の人口はおよそ1400万人です。".to_string(),
            "バナナは黄色い果物です。".to_string(),
        ];
        let scores = bm25_scores("東京 人口", &chunks, BM25_K1, BM25_B);
        assert!(scores[0] > scores[1]);
        assert_eq!(scores[1], 0.0);
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
