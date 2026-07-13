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

use crate::config::defaults::{
    CHARS_PER_TOKEN, CONTEXT_BUDGET_CTX_PERCENT, CONTEXT_MAX_TOKENS, UNLISTED_DOMAIN_CHUNK_CAP,
};
use crate::websearch::credibility::{classify_domain, DomainClass};
use crate::websearch::domain_of;
use crate::websearch::rank::ScoredChunk;
use std::collections::HashMap;

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

/// Caps how many chunks a domain absent from the credibility list ("unlisted")
/// may contribute, but only once a credibility-boosted reference-grade domain
/// has already contributed at least one chunk to the candidate set. Realistic-
/// RAG research (arXiv:2505.15561) found that distracting passages admitted
/// into the top-K context, not their rank position, are what drive an LLM to
/// cite junk over a reference source sitting right beside it: this keeps a
/// thin unlisted aggregator's chunks from crowding out the reference once one
/// is present, while an unlisted domain still contributes up to
/// [`UNLISTED_DOMAIN_CHUNK_CAP`] chunks (retrieved information is never
/// discarded wholesale). A candidate set with no boosted domain is returned
/// unchanged, since there is no reference chunk yet to protect. Input order is
/// preserved; only chunks past the cap for an over-threshold unlisted domain
/// are dropped, so the ordering [`assemble_context`] groups by stays
/// best-first.
fn cap_unlisted_domain_chunks(chunks: &[ScoredChunk]) -> Vec<ScoredChunk> {
    let has_boosted_reference = chunks
        .iter()
        .any(|c| classify_domain(&domain_of(&c.url)) == DomainClass::Boost);
    if !has_boosted_reference {
        return chunks.to_vec();
    }
    let mut kept_per_domain: HashMap<String, usize> = HashMap::new();
    chunks
        .iter()
        .filter(|c| {
            let domain = domain_of(&c.url);
            if classify_domain(&domain) != DomainClass::Neutral {
                // The cap targets only domains absent from the credibility
                // list; a classified domain (boost, penalize, or drop) is
                // unaffected here, since credibility already governs it
                // upstream in fusion.
                return true;
            }
            let count = kept_per_domain.entry(domain).or_insert(0);
            *count += 1;
            *count <= UNLISTED_DOMAIN_CHUNK_CAP
        })
        .cloned()
        .collect()
}

/// Groups best-first chunks into numbered source blocks under the `num_ctx`
/// budget. Chunks are grouped by URL (first appearance sets the order, which is
/// best-first since the ranker sorted the input); whole sources are admitted
/// until the budget is reached; the first source is truncated if it alone
/// overflows. Returns 1-based indexed blocks in citation order.
///
/// Before grouping, [`cap_unlisted_domain_chunks`] trims chunks from
/// credibility-unlisted domains once a boosted reference domain is present in
/// the candidate set (see its rustdoc for the distractor-crowding rationale).
pub fn assemble_context(chunks: &[ScoredChunk], num_ctx: u32) -> Vec<SourceBlock> {
    let chunks = cap_unlisted_domain_chunks(chunks);
    // Group by URL, preserving first-seen (best-first) order.
    let mut order: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut position: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for chunk in &chunks {
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

    // ── cap_unlisted_domain_chunks ────────────────────────────────────────────

    #[test]
    fn cap_inactive_without_a_boosted_domain() {
        // No boosted (credibility-listed) domain anywhere in the set: the cap
        // never fires and every chunk from the thin domain survives.
        let chunks: Vec<ScoredChunk> = (0..5)
            .map(|i| chunk(&format!("https://aggregator.example/{i}"), "text"))
            .collect();
        let out = cap_unlisted_domain_chunks(&chunks);
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn cap_active_and_at_boundary_when_boosted_domain_present() {
        // A boosted reference chunk plus five chunks from an unlisted domain:
        // exactly UNLISTED_DOMAIN_CHUNK_CAP of the unlisted domain's chunks
        // survive (boundary), the rest are dropped, and the boosted chunk is
        // untouched.
        let mut chunks = vec![chunk("https://en.wikipedia.org/wiki/Test", "reference")];
        chunks.extend((0..5).map(|i| chunk(&format!("https://aggregator.example/{i}"), "text")));
        let out = cap_unlisted_domain_chunks(&chunks);
        let boosted_count = out.iter().filter(|c| c.url.contains("wikipedia")).count();
        let unlisted_count = out.iter().filter(|c| c.url.contains("aggregator")).count();
        assert_eq!(boosted_count, 1);
        assert_eq!(unlisted_count, UNLISTED_DOMAIN_CHUNK_CAP);
    }

    #[test]
    fn cap_leaves_penalized_domains_untouched() {
        // A classified-but-not-boost domain (penalize) is not the cap's
        // target: credibility fusion already governs it upstream, and this
        // stage only shapes unlisted-domain crowding.
        let mut chunks = vec![chunk("https://en.wikipedia.org/wiki/Test", "reference")];
        chunks.extend((0..5).map(|i| chunk(&format!("https://9to5answer.com/{i}"), "text")));
        let out = cap_unlisted_domain_chunks(&chunks);
        let penalized_count = out.iter().filter(|c| c.url.contains("9to5answer")).count();
        assert_eq!(penalized_count, 5);
    }

    #[test]
    fn assemble_context_applies_unlisted_domain_cap_when_boost_present() {
        // End-to-end: the capped candidate chunks feed the URL grouping, so
        // the aggregator's dropped chunks mean its source blocks disappear
        // too, while retrieved information is never discarded wholesale (the
        // aggregator still contributes up to the cap).
        let mut chunks = vec![chunk("https://en.wikipedia.org/wiki/Test", "reference")];
        chunks.extend((0..5).map(|i| chunk(&format!("https://aggregator.example/{i}"), "text")));
        let blocks = assemble_context(&chunks, 16384);
        assert_eq!(blocks.len(), 1 + UNLISTED_DOMAIN_CHUNK_CAP);
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
