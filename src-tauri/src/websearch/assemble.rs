//! Context assembly: turn the ranked, best-first chunks into a small set of
//! numbered source blocks that fit a hard token budget.
//!
//! Chunks from the same page are grouped into one numbered source (the `[n]`
//! the writer cites), sources are emitted best-first (their strongest chunk
//! decides order, which the ranker already sorted), and blocks are admitted
//! greedily until the budget — `min(CONTEXT_BUDGET_CTX_PERCENT% of num_ctx,
//! CONTEXT_MAX_TOKENS)` — is reached. If even the single strongest source
//! overflows the budget it is truncated to fit, so there is always at least one
//! block to cite. Pure over its inputs; the delimiter-wrapping and prompt
//! framing happen later in the writer stage.

use crate::config::defaults::{CHARS_PER_TOKEN, CONTEXT_BUDGET_CTX_PERCENT, CONTEXT_MAX_TOKENS};
use crate::websearch::rank::ScoredChunk;

/// One numbered source in the assembled context: the `[n]` a citation refers
/// to, its origin, and the (possibly multi-chunk, possibly truncated) text.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SourceBlock {
    /// 1-based citation index.
    pub index: usize,
    pub url: String,
    pub title: String,
    pub text: String,
}

/// The token budget for retrieved sources: the smaller of a fixed ceiling and a
/// fraction of the context window, so retrieval scales down on small windows
/// and is capped on large ones.
pub(crate) fn budget_tokens(num_ctx: u32) -> usize {
    ((num_ctx as usize) * CONTEXT_BUDGET_CTX_PERCENT / 100).min(CONTEXT_MAX_TOKENS)
}

/// Rough token count of `text`, rounding up so the assembled context stays
/// under the real token budget rather than over it. Crate-visible because the
/// orchestrator's escalation merge re-budgets with the same accounting this
/// module uses, so the two stages can never disagree about a block's cost.
pub(crate) fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN)
}

/// Truncates `text` to at most `max_tokens` worth of characters, on a character
/// boundary. Used only for the single strongest source when it alone exceeds
/// the whole budget.
fn truncate_to_tokens(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens * CHARS_PER_TOKEN;
    text.chars().take(max_chars).collect()
}

/// Groups best-first chunks into numbered source blocks under the `num_ctx`
/// budget. Chunks are grouped by URL (first appearance sets the order, which is
/// best-first since the ranker sorted the input); whole sources are admitted
/// until the budget is reached; the first source is truncated if it alone
/// overflows. Returns 1-based indexed blocks in citation order.
pub fn assemble_context(chunks: &[ScoredChunk], num_ctx: u32) -> Vec<SourceBlock> {
    // Group by URL, preserving first-seen (best-first) order.
    let mut order: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut position: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for chunk in chunks {
        match position.get(&chunk.url) {
            Some(&i) => order[i].2.push(chunk.text.clone()),
            None => {
                position.insert(chunk.url.clone(), order.len());
                order.push((
                    chunk.url.clone(),
                    chunk.title.clone(),
                    vec![chunk.text.clone()],
                ));
            }
        }
    }

    let budget = budget_tokens(num_ctx);
    let mut used = 0usize;
    let mut blocks: Vec<SourceBlock> = Vec::new();
    for (url, title, texts) in order {
        let text = texts.join("\n");
        let cost = estimate_tokens(&text);
        if used + cost <= budget {
            used += cost;
            blocks.push(SourceBlock {
                index: blocks.len() + 1,
                url,
                title,
                text,
            });
        } else if blocks.is_empty() {
            // The strongest source alone overflows: truncate it to fit so the
            // writer always has at least one block to cite.
            blocks.push(SourceBlock {
                index: 1,
                url,
                title,
                text: truncate_to_tokens(&text, budget),
            });
            break;
        } else {
            break;
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(url: &str, text: &str) -> ScoredChunk {
        ScoredChunk {
            url: url.into(),
            title: "T".into(),
            text: text.into(),
            score: 1.0,
        }
    }

    // ── budget_tokens ─────────────────────────────────────────────────────────

    #[test]
    fn budget_scales_with_small_ctx() {
        assert_eq!(budget_tokens(8192), 8192 * 40 / 100); // 3276
    }

    #[test]
    fn budget_capped_on_large_ctx() {
        assert_eq!(budget_tokens(16384), CONTEXT_MAX_TOKENS);
        assert_eq!(budget_tokens(1_000_000), CONTEXT_MAX_TOKENS);
    }

    // ── estimate_tokens / truncate_to_tokens ──────────────────────────────────

    #[test]
    fn estimate_rounds_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abc"), 1); // ceil(3/4)
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn truncate_keeps_short_and_cuts_long() {
        assert_eq!(truncate_to_tokens("abc", 10), "abc");
        assert_eq!(truncate_to_tokens("abcdefghij", 1), "abcd"); // 1 token = 4 chars
    }

    // ── assemble_context ──────────────────────────────────────────────────────

    #[test]
    fn assemble_empty_input_is_empty() {
        assert!(assemble_context(&[], 16384).is_empty());
    }

    #[test]
    fn assemble_groups_same_url_and_numbers_sources() {
        let chunks = vec![
            chunk("https://a/", "first passage"),
            chunk("https://a/", "second passage"),
            chunk("https://b/", "other source"),
        ];
        let blocks = assemble_context(&chunks, 16384);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].index, 1);
        assert_eq!(blocks[0].url, "https://a/");
        assert_eq!(blocks[0].text, "first passage\nsecond passage");
        assert_eq!(blocks[1].index, 2);
        assert_eq!(blocks[1].url, "https://b/");
    }

    #[test]
    fn assemble_stops_admitting_whole_blocks_at_budget() {
        // num_ctx 2048 -> budget 819 tokens. Two ~600-token sources: the first
        // fits, the second would overflow and is dropped whole.
        let big = "x ".repeat(1200); // ~2400 chars -> ~600 tokens
        let chunks = vec![chunk("https://a/", &big), chunk("https://b/", &big)];
        let blocks = assemble_context(&chunks, 2048);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].url, "https://a/");
    }

    #[test]
    fn assemble_truncates_first_source_when_it_alone_overflows() {
        // A single source far larger than the budget is truncated, never dropped.
        let huge = "y".repeat(100_000);
        let budget = budget_tokens(2048); // 819 tokens
        let blocks = assemble_context(&[chunk("https://a/", &huge)], 2048);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text.chars().count(), budget * CHARS_PER_TOKEN);
    }
}
