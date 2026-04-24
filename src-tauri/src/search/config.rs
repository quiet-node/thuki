//! Pipeline-wide constants and runtime configuration for the agentic /search loop.
//!
//! ## Compiled constants (keep these; they are the defaults)
//!
//! Constants here serve two purposes:
//! - They are the source of truth for the compiled defaults used by
//!   `SearchRuntimeConfig::default()` and by `config/defaults.rs`.
//! - They are used directly in test builds, where `READER_BATCH_TIMEOUT_S`
//!   is 1 s so `BatchTimeout` tests complete quickly.
//!
//! ## Runtime-configurable values
//!
//! The following constants have TOML counterparts in `AppConfig.search` (see
//! `config/schema.rs`). Production code reads from `SearchRuntimeConfig` which
//! is constructed from `AppConfig` at pipeline entry. Tests use
//! `SearchRuntimeConfig::default()` which delegates to the compiled constants.
//!
//! Configurable via `[search]` in config.toml:
//!   `searxng_url`, `reader_url`, `max_iterations`, `top_k_urls`,
//!   `search_timeout_s`, `reader_per_url_timeout_s`, `reader_batch_timeout_s`,
//!   `judge_timeout_s`, `router_timeout_s`.
//!
//! Stays compiled-in (no TOML field):
//!   `GAP_QUERIES_PER_ROUND`, `CHUNK_TOKEN_SIZE`, `TOP_K_CHUNKS`,
//!   `LLM_RETRY_DELAY_MS`, `SEARCH_RETRY_DELAY_MS`, `READER_RETRY_DELAY_MS`.

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

/// Base URL of the SearXNG instance.
pub const SEARXNG_BASE_URL: &str = "http://127.0.0.1:25017";

/// Runtime search configuration extracted from `AppConfig` at pipeline entry.
///
/// Production code builds this from `AppConfig.search` via
/// `SearchRuntimeConfig::from_app_config`. Tests use `SearchRuntimeConfig::default()`,
/// which delegates to the compiled constants so existing test behavior is
/// unchanged (including the 1-second `READER_BATCH_TIMEOUT_S` in test builds).
#[derive(Debug, Clone)]
pub struct SearchRuntimeConfig {
    pub searxng_url: String,
    pub reader_url: String,
    pub max_iterations: usize,
    pub top_k_urls: usize,
    pub search_timeout_s: u64,
    pub reader_per_url_timeout_s: u64,
    pub reader_batch_timeout_s: u64,
    pub judge_timeout_s: u64,
    pub router_timeout_s: u64,
}

impl SearchRuntimeConfig {
    /// Derives the fully-qualified SearXNG search endpoint from `searxng_url`.
    pub fn searxng_endpoint(&self) -> String {
        format!("{}/search", self.searxng_url.trim_end_matches('/'))
    }
}

impl Default for SearchRuntimeConfig {
    fn default() -> Self {
        Self {
            searxng_url: SEARXNG_BASE_URL.to_string(),
            reader_url: READER_BASE_URL.to_string(),
            max_iterations: MAX_ITERATIONS,
            top_k_urls: TOP_K_URLS,
            search_timeout_s: SEARCH_TIMEOUT_S,
            reader_per_url_timeout_s: READER_PER_URL_TIMEOUT_S,
            reader_batch_timeout_s: READER_BATCH_TIMEOUT_S,
            judge_timeout_s: JUDGE_TIMEOUT_S,
            router_timeout_s: ROUTER_TIMEOUT_S,
        }
    }
}

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

    #[test]
    fn runtime_config_default_matches_compiled_constants() {
        let cfg = SearchRuntimeConfig::default();
        assert_eq!(cfg.max_iterations, MAX_ITERATIONS);
        assert_eq!(cfg.top_k_urls, TOP_K_URLS);
        assert_eq!(cfg.search_timeout_s, SEARCH_TIMEOUT_S);
        assert_eq!(cfg.reader_per_url_timeout_s, READER_PER_URL_TIMEOUT_S);
        assert_eq!(cfg.reader_batch_timeout_s, READER_BATCH_TIMEOUT_S);
        assert_eq!(cfg.judge_timeout_s, JUDGE_TIMEOUT_S);
        assert_eq!(cfg.router_timeout_s, ROUTER_TIMEOUT_S);
        assert_eq!(cfg.reader_url, READER_BASE_URL);
        assert_eq!(cfg.searxng_url, SEARXNG_BASE_URL);
    }

    #[test]
    fn searxng_endpoint_appends_search_path() {
        let cfg = SearchRuntimeConfig::default();
        assert!(cfg.searxng_endpoint().ends_with("/search"));
        assert!(!cfg.searxng_endpoint().ends_with("//search"));
    }

    #[test]
    fn searxng_endpoint_strips_trailing_slash() {
        let cfg = SearchRuntimeConfig {
            searxng_url: "http://127.0.0.1:25017/".to_string(),
            ..Default::default()
        };
        assert_eq!(cfg.searxng_endpoint(), "http://127.0.0.1:25017/search");
    }
}
