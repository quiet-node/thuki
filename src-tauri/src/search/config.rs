//! Pipeline-wide numeric knobs for the agentic /search loop.
//!
//! All values are compiled-in rather than user-configurable. Tuning requires a
//! rebuild, which is intentional: downstream prompt design and persisted
//! metadata interpretation assume these exact values.

/// Maximum number of search-refine iterations before the pipeline gives up.
pub const MAX_ITERATIONS: usize = 3;

/// Number of gap-filling queries generated per iteration round.
pub const GAP_QUERIES_PER_ROUND: usize = 3;

/// Number of top-ranked URLs forwarded to the reader after reranking.
pub const TOP_K_URLS: usize = 10;

/// Approximate token budget for each retrieved page chunk.
pub const CHUNK_TOKEN_SIZE: usize = 500;

/// Number of highest-scoring chunks passed to the synthesis prompt.
pub const TOP_K_CHUNKS: usize = 8;

/// Milliseconds to wait before retrying a failed LLM call.
pub const LLM_RETRY_DELAY_MS: u64 = 500;

/// Milliseconds to wait before retrying a failed SearXNG call.
pub const SEARCH_RETRY_DELAY_MS: u64 = 1000;

/// Milliseconds to wait before retrying a failed reader fetch.
pub const READER_RETRY_DELAY_MS: u64 = 500;

/// Seconds before the router LLM call is abandoned.
pub const ROUTER_TIMEOUT_S: u64 = 45;

/// Seconds before a SearXNG query is abandoned.
pub const SEARCH_TIMEOUT_S: u64 = 20;

/// Seconds allowed for a single URL fetch inside the reader.
pub const READER_PER_URL_TIMEOUT_S: u64 = 10;

/// Seconds allowed for the full parallel reader batch to complete.
/// Reduced to 1 second in tests so `BatchTimeout` can be triggered by a
/// slow mock without waiting 30 seconds.
#[cfg(not(test))]
pub const READER_BATCH_TIMEOUT_S: u64 = 30;
#[cfg(test)]
pub const READER_BATCH_TIMEOUT_S: u64 = 1;

/// Seconds before the judge LLM call is abandoned.
pub const JUDGE_TIMEOUT_S: u64 = 30;

/// Base URL of the local reader/extractor service.
pub const READER_BASE_URL: &str = "http://127.0.0.1:25018";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_are_sane_for_local_ollama() {
        assert!(MAX_ITERATIONS >= 1 && MAX_ITERATIONS <= 5);
        assert!(GAP_QUERIES_PER_ROUND >= 1 && GAP_QUERIES_PER_ROUND <= 5);
        assert!(TOP_K_URLS >= 1 && TOP_K_URLS <= 10);
        assert!(CHUNK_TOKEN_SIZE >= 128 && CHUNK_TOKEN_SIZE <= 2048);
        assert!(TOP_K_CHUNKS >= 1 && TOP_K_CHUNKS <= 32);
        // READER_BATCH_TIMEOUT_S is 1s in test builds (to enable BatchTimeout
        // testing); the production value (30s) is enforced by this assertion
        // only in non-test builds.
        #[cfg(not(test))]
        assert!(READER_BATCH_TIMEOUT_S >= READER_PER_URL_TIMEOUT_S);
    }

    #[test]
    fn retry_delays_are_bounded() {
        assert!(LLM_RETRY_DELAY_MS <= 2_000);
        assert!(SEARCH_RETRY_DELAY_MS <= 2_000);
        assert!(READER_RETRY_DELAY_MS <= 2_000);
    }
}
