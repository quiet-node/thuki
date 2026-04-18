//! BM25F + Reciprocal Rank Fusion reranker for SearXNG results.
//!
//! The retrieved SearXNG set (≤ [`super::searxng::MAX_RESULTS`]) is reranked
//! before being exposed to the synthesis prompt. Two signals are combined:
//!
//! 1. **BM25F** — field-weighted Okapi BM25 over `title` and `content` fields,
//!    using the retrieved set itself as the corpus. Captures query-document
//!    lexical relevance.
//! 2. **SearXNG engine order** — the authority/popularity signal the upstream
//!    metasearch engine already provides.
//!
//! The two ranked lists are fused with **Reciprocal Rank Fusion** (Cormack,
//! Clarke, Buettcher, SIGIR 2009) at the canonical `k = 60` constant used by
//! Elasticsearch, OpenSearch, Vespa, and Weaviate hybrid search.
//!
//! The module is pure Rust: no I/O, no allocations beyond the result vector
//! and per-query scratch buffers, no `unsafe`, and no external crates. All
//! inputs are bounded by [`MAX_QUERY_TOKENS`] and the retrieved-set size so
//! the scorer has fixed O(N · T) time and O(N + T) space per call, where
//! `N ≤ MAX_RESULTS` and `T ≤ MAX_QUERY_TOKENS`.

use super::types::SearxResult;

/// Upper bound on query tokens retained after tokenisation. Queries reaching
/// the reranker are already LLM-optimised and short; this cap is a defence-in-
/// depth bound against pathologically long inputs and keeps scoring O(T · N)
/// with `T` trivially small in practice.
const MAX_QUERY_TOKENS: usize = 128;

/// BM25 term-frequency saturation. 1.2 is the Robertson-Zaragoza recommended
/// default and the Lucene/Elasticsearch out-of-the-box constant.
const BM25_K1: f64 = 1.2;

/// BM25 length-normalisation strength. 0.75 is the Robertson-Zaragoza
/// recommended default used by Lucene, Elasticsearch, OpenSearch, and Vespa.
const BM25_B: f64 = 0.75;

/// BM25F field weight for titles. Titles are higher-signal than body snippets
/// on web search results (engines already optimise them for topicality), so we
/// weight them 2× the body contribution — a standard field-weighting choice
/// used in Lucene BM25FSimilarity defaults.
const TITLE_WEIGHT: f64 = 2.0;

/// BM25F field weight for body snippets.
const CONTENT_WEIGHT: f64 = 1.0;

/// Reciprocal Rank Fusion constant. `k = 60` is the value validated by
/// Cormack, Clarke, and Buettcher (SIGIR 2009) and adopted as the default in
/// Elasticsearch, OpenSearch, Vespa, and Weaviate hybrid search.
const RRF_K: f64 = 60.0;

/// Computes a BM25F score per document in `results` for the given tokenised
/// query. Uses the retrieved set as the corpus; document frequency is the
/// count of in-set documents containing the term, and average field length is
/// taken over the retrieved set.
///
/// Scores are 0.0 when no query token matches the document in any field, and
/// strictly positive otherwise. IDF uses the Lucene-style `ln(1 + (N - df +
/// 0.5) / (df + 0.5))` smoothing which is always non-negative.
///
/// Time complexity: `O(N · T)` where `N = results.len()` and `T =
/// query_tokens.len()`; both are bounded (`N ≤ MAX_RESULTS`, `T ≤
/// MAX_QUERY_TOKENS`).
fn bm25f_scores(query_tokens: &[String], results: &[SearxResult]) -> Vec<f64> {
    if results.is_empty() {
        return Vec::new();
    }
    if query_tokens.is_empty() {
        return vec![0.0; results.len()];
    }

    // Per-document tokenised fields (kept in parallel vectors to avoid storing
    // extra owned strings beyond what's needed for scoring).
    let titles: Vec<Vec<String>> = results.iter().map(|r| tokenize(&r.title)).collect();
    let bodies: Vec<Vec<String>> = results.iter().map(|r| tokenize(&r.content)).collect();

    let avg_title_len = average_len(&titles);
    let avg_body_len = average_len(&bodies);

    let n = results.len() as f64;

    let mut scores = vec![0.0_f64; results.len()];
    for term in query_tokens {
        let df = titles
            .iter()
            .zip(bodies.iter())
            .filter(|(t, b)| t.contains(term) || b.contains(term))
            .count() as f64;
        if df == 0.0 {
            continue;
        }
        let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();

        for i in 0..results.len() {
            let occ_title = titles[i].iter().filter(|t| *t == term).count() as f64;
            let occ_body = bodies[i].iter().filter(|t| *t == term).count() as f64;
            if occ_title == 0.0 && occ_body == 0.0 {
                continue;
            }
            let norm_title = field_norm(titles[i].len() as f64, avg_title_len);
            let norm_body = field_norm(bodies[i].len() as f64, avg_body_len);
            let pseudo_tf =
                TITLE_WEIGHT * (occ_title / norm_title) + CONTENT_WEIGHT * (occ_body / norm_body);
            scores[i] += idf * (pseudo_tf / (pseudo_tf + BM25_K1));
        }
    }
    scores
}

/// Average token count across a slice of tokenised documents. Returns `1.0`
/// when every document tokenises to zero tokens (avoids division by zero while
/// keeping the length-normalisation term at the identity `1.0`).
///
/// Caller (`bm25f_scores`) guarantees `docs.len() >= 1`.
fn average_len(docs: &[Vec<String>]) -> f64 {
    let total: usize = docs.iter().map(Vec::len).sum();
    if total == 0 {
        return 1.0;
    }
    total as f64 / docs.len() as f64
}

/// Converts `scores` into 1-based ranks using stable descending order:
/// equal scores share the same rank and subsequent distinct scores skip ranks
/// (standard competition ranking, "1224" scheme). The returned vector is in
/// input order, i.e. `ranks[i]` is the rank of the `i`-th score.
///
/// Generic over the score type so the same helper ranks BM25F floats and any
/// other ordered signal we may wire in later.
fn rank_positions<T: PartialOrd + Copy>(scores: &[T]) -> Vec<usize> {
    let mut indexed: Vec<(usize, T)> = scores.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let mut ranks = vec![0_usize; scores.len()];
    let mut current_rank = 1_usize;
    let mut i = 0_usize;
    while i < indexed.len() {
        let mut j = i + 1;
        while j < indexed.len()
            && indexed[j]
                .1
                .partial_cmp(&indexed[i].1)
                .unwrap_or(std::cmp::Ordering::Equal)
                == std::cmp::Ordering::Equal
        {
            j += 1;
        }
        for entry in &indexed[i..j] {
            ranks[entry.0] = current_rank;
        }
        current_rank += j - i;
        i = j;
    }
    ranks
}

/// Maps BM25F scores to RRF ranks. Docs with a zero score (no query term
/// matched any field) are returned as `None`, i.e. **off-list** from BM25F's
/// perspective: RRF must not award them a reciprocal contribution from the
/// lexical signal, or a completely non-matching doc sitting at a high engine
/// rank would be dragged to the top when fused against a tied lexical rank.
fn bm25f_ranks(scores: &[f64]) -> Vec<Option<usize>> {
    let dense = rank_positions(scores);
    scores
        .iter()
        .zip(dense)
        .map(|(s, rank)| if *s > 0.0 { Some(rank) } else { None })
        .collect()
}

/// Reciprocal Rank Fusion: combines two or more ranked lists into a single
/// fused score per document. Higher is better.
///
/// Formula (Cormack, Clarke, Buettcher 2009):
/// ```text
/// rrf(d) = Σ_i 1 / (k + rank_i(d))     for each list i where d is ranked
/// ```
/// with `k = 60`. A `None` entry means the document is not ranked by that
/// list and contributes nothing from that source — standard RRF treatment of
/// sparse/off-list signals. All input lists must have identical length (the
/// fixed universe of documents being fused).
fn rrf_fuse(rank_lists: &[Vec<Option<usize>>]) -> Vec<f64> {
    let n = rank_lists.first().map(Vec::len).unwrap_or(0);
    let mut fused = vec![0.0_f64; n];
    for list in rank_lists {
        debug_assert_eq!(list.len(), n);
        for (i, rank) in list.iter().enumerate() {
            if let Some(r) = rank {
                fused[i] += 1.0 / (RRF_K + *r as f64);
            }
        }
    }
    fused
}

/// BM25 length-normalisation denominator for a single field occurrence
/// contribution. `avg_len` is guaranteed ≥ 1.0 by [`average_len`] and
/// `len` ≥ 0, so the result is bounded below by `1 - BM25_B = 0.25`.
fn field_norm(len: f64, avg_len: f64) -> f64 {
    1.0 - BM25_B + BM25_B * (len / avg_len)
}

/// Splits `text` into lowercase alphanumeric tokens.
///
/// The tokenizer is deliberately minimal: Unicode-aware via
/// [`char::is_alphanumeric`], locale-free lowercase, no stemming, no
/// stopword list. Stemming and language-specific stopwording would require
/// per-locale tables and offer marginal gains on short web snippets reranked
/// against an already keyword-optimised query. Output length is capped at
/// [`MAX_QUERY_TOKENS`] to bound scoring cost.
fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.extend(ch.to_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
            if tokens.len() == MAX_QUERY_TOKENS {
                return tokens;
            }
        }
    }
    if !current.is_empty() && tokens.len() < MAX_QUERY_TOKENS {
        tokens.push(current);
    }
    tokens
}

/// Reranks `results` by fusing BM25F field-weighted lexical scores with the
/// upstream SearXNG engine order via Reciprocal Rank Fusion.
///
/// Returns the input ordering unchanged when `query` tokenises to zero terms
/// or when `results` contains fewer than two entries (nothing to reorder). On
/// ties (e.g. queries with no lexical match anywhere in the set) the original
/// order is preserved via stable sort, which keeps the engine signal intact.
pub fn rerank(query: &str, results: Vec<SearxResult>) -> Vec<SearxResult> {
    if results.len() < 2 {
        return results;
    }
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return results;
    }

    let bm25f = bm25f_scores(&query_tokens, &results);
    let bm25f_ranks_list = bm25f_ranks(&bm25f);

    // Engine-order ranks: the input is already 1..=N in engine order and every
    // doc is ranked by the engine, so no None entries here.
    let engine_ranks: Vec<Option<usize>> = (1..=results.len()).map(Some).collect();

    let fused = rrf_fuse(&[bm25f_ranks_list, engine_ranks]);

    // Stable sort by fused score descending, preserving original order on ties.
    let mut indices: Vec<usize> = (0..results.len()).collect();
    indices.sort_by(|&a, &b| {
        fused[b]
            .partial_cmp(&fused[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    indices.into_iter().map(|i| results[i].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_lowercases_and_splits_on_non_alphanumeric() {
        assert_eq!(
            tokenize("Hello, WORLD! 42 rust-lang"),
            vec!["hello", "world", "42", "rust", "lang"]
        );
    }

    #[test]
    fn tokenize_preserves_unicode_letters() {
        assert_eq!(
            tokenize("Café München 東京"),
            vec!["café", "münchen", "東京"]
        );
    }

    #[test]
    fn tokenize_returns_empty_for_blank_and_punctuation_only() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("   \t\n").is_empty());
        assert!(tokenize("!?.,;:").is_empty());
    }

    #[test]
    fn tokenize_caps_output_at_max_query_tokens() {
        let huge: String = (0..MAX_QUERY_TOKENS + 50)
            .map(|i| format!("w{i} "))
            .collect();
        assert_eq!(tokenize(&huge).len(), MAX_QUERY_TOKENS);
    }

    #[test]
    fn bm25f_scores_are_zero_when_no_query_terms_match() {
        let results = vec![make_result("a", "hello world"), make_result("b", "foo bar")];
        let scores = bm25f_scores(&["missing".to_string()], &results);
        assert_eq!(scores, vec![0.0, 0.0]);
    }

    #[test]
    fn bm25f_title_match_outscores_content_only_match() {
        let results = vec![
            make_result("rust async runtime", "unrelated filler text"),
            make_result("unrelated filler text", "rust async runtime"),
        ];
        let scores = bm25f_scores(&["rust".to_string(), "async".to_string()], &results);
        assert!(
            scores[0] > scores[1],
            "title match should score above content-only match: {scores:?}"
        );
    }

    #[test]
    fn bm25f_rewards_rare_terms_over_common_ones() {
        // "rust" appears in every doc (common), "scheduler" appears only in d1.
        let results = vec![
            make_result("rust tutorial", "rust tutorial body"),
            make_result("rust scheduler", "rust scheduler internals"),
            make_result("rust pattern", "rust pattern body"),
        ];
        let scores = bm25f_scores(&["rust".to_string(), "scheduler".to_string()], &results);
        assert!(
            scores[1] > scores[0] && scores[1] > scores[2],
            "rare-term doc should rank highest: {scores:?}"
        );
    }

    #[test]
    fn bm25f_empty_inputs_produce_empty_or_zero_scores() {
        let results: Vec<SearxResult> = vec![];
        assert!(bm25f_scores(&["q".to_string()], &results).is_empty());

        let results = vec![make_result("a", "b")];
        assert_eq!(bm25f_scores(&[], &results), vec![0.0]);
    }

    #[test]
    fn bm25f_saturates_with_repeated_terms() {
        // Document with term repeated 10× should score higher than once, but
        // the incremental gain from 10→20 is smaller than from 1→2 — the
        // defining saturation property of BM25.
        let one = vec![make_result("rust", &"filler ".repeat(20))];
        let two = vec![make_result("rust rust", &"filler ".repeat(20))];
        let ten = vec![make_result(&"rust ".repeat(10), &"filler ".repeat(20))];
        let twenty = vec![make_result(&"rust ".repeat(20), &"filler ".repeat(20))];

        let q = vec!["rust".to_string()];
        let s1 = bm25f_scores(&q, &one)[0];
        let s2 = bm25f_scores(&q, &two)[0];
        let s10 = bm25f_scores(&q, &ten)[0];
        let s20 = bm25f_scores(&q, &twenty)[0];

        let delta_early = s2 - s1;
        let delta_late = s20 - s10;
        assert!(s2 > s1);
        assert!(s20 > s10);
        assert!(
            delta_late < delta_early,
            "TF saturation violated: Δ(20→10)={delta_late} should be < Δ(2→1)={delta_early}"
        );
    }

    #[test]
    fn bm25f_handles_docs_with_only_punctuation_fields() {
        // Docs whose title and content tokenise to zero tokens exercise the
        // `total == 0` branch in `average_len` without crashing.
        let results = vec![make_result("!!!", "???"), make_result(".", ",")];
        let scores = bm25f_scores(&["q".to_string()], &results);
        assert_eq!(scores, vec![0.0, 0.0]);
    }

    #[test]
    fn rank_positions_ties_share_rank_and_later_items_skip() {
        // Scores sorted descending: 5, 5, 3, 1 → ranks 1, 1, 3, 4.
        let scores = vec![1.0_f64, 5.0, 3.0, 5.0];
        let ranks = rank_positions(&scores);
        assert_eq!(ranks, vec![4, 1, 3, 1]);
    }

    #[test]
    fn rank_positions_empty_input_returns_empty() {
        assert!(rank_positions::<f64>(&[]).is_empty());
    }

    #[test]
    fn rrf_fuse_averages_two_rank_lists_at_k_60() {
        // Doc 0 ranks 1 + 2, doc 1 ranks 2 + 1.
        let r1 = vec![Some(1), Some(2)];
        let r2 = vec![Some(2), Some(1)];
        let fused = rrf_fuse(&[r1, r2]);
        let expected = 1.0 / (RRF_K + 1.0) + 1.0 / (RRF_K + 2.0);
        assert!((fused[0] - expected).abs() < 1e-12);
        assert!((fused[1] - expected).abs() < 1e-12);
    }

    #[test]
    fn rrf_fuse_top_ranked_across_both_lists_wins() {
        // Doc 0 top in both; doc 1 second in both; doc 2 third in both.
        let r = vec![Some(1), Some(2), Some(3)];
        let fused = rrf_fuse(&[r.clone(), r]);
        assert!(fused[0] > fused[1]);
        assert!(fused[1] > fused[2]);
    }

    #[test]
    fn rrf_fuse_skips_none_entries_so_off_list_docs_get_no_contribution() {
        // Doc 0 on-list in both; doc 1 only in the second list.
        let lexical = vec![Some(1), None];
        let engine = vec![Some(2), Some(1)];
        let fused = rrf_fuse(&[lexical, engine]);
        let expected_0 = 1.0 / (RRF_K + 1.0) + 1.0 / (RRF_K + 2.0);
        let expected_1 = 1.0 / (RRF_K + 1.0);
        assert!((fused[0] - expected_0).abs() < 1e-12);
        assert!((fused[1] - expected_1).abs() < 1e-12);
        assert!(fused[0] > fused[1]);
    }

    #[test]
    fn bm25f_ranks_marks_zero_scores_as_off_list() {
        let ranks = bm25f_ranks(&[0.0, 5.0, 0.0, 3.0]);
        assert_eq!(ranks, vec![None, Some(1), None, Some(2)]);
    }

    #[test]
    fn rerank_empty_results_returns_empty() {
        let out = rerank("anything", vec![]);
        assert!(out.is_empty());
    }

    #[test]
    fn rerank_single_result_is_unchanged() {
        let input = vec![make_result("only", "one")];
        let out = rerank("query", input.clone());
        assert_eq!(out, input);
    }

    #[test]
    fn rerank_blank_query_preserves_input_order() {
        let input = vec![
            make_result("alpha", "first"),
            make_result("beta", "second"),
            make_result("gamma", "third"),
        ];
        let out = rerank("   ", input.clone());
        assert_eq!(out, input);
    }

    #[test]
    fn rerank_promotes_strong_lexical_match_over_engine_first() {
        // Engine-order puts the irrelevant doc first; BM25F should pull the
        // lexically-matching doc to the top via fusion.
        let input = vec![
            make_result("totally unrelated header", "nothing to see here filler"),
            make_result(
                "rust async runtime design",
                "a walkthrough of the rust async runtime internals",
            ),
            make_result("another filler doc", "more filler body text"),
        ];
        let out = rerank("rust async runtime", input.clone());
        assert_eq!(out[0].url, input[1].url);
    }

    #[test]
    fn rerank_is_stable_when_all_scores_tie() {
        // No query-doc matches → BM25F scores all zero → RRF tie-broken only
        // by original order. The input order must be preserved.
        let input = vec![
            make_result("a", "x"),
            make_result("b", "y"),
            make_result("c", "z"),
        ];
        let out = rerank("zzzz_no_match", input.clone());
        assert_eq!(out, input);
    }

    fn make_result(title: &str, content: &str) -> SearxResult {
        SearxResult {
            title: title.to_string(),
            url: format!("https://example.com/{}", title.replace(' ', "-")),
            content: content.to_string(),
        }
    }

    #[test]
    fn tokenize_respects_cap_when_input_ends_mid_token() {
        // Build an input whose final token would be the (MAX+1)th.
        let mut s: String = (0..MAX_QUERY_TOKENS).map(|i| format!("w{i} ")).collect();
        s.push_str("overflow");
        assert_eq!(tokenize(&s).len(), MAX_QUERY_TOKENS);
    }
}
