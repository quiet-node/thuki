//! Pipeline-wide constants and runtime configuration for the agentic
//! `/search` loop.
//!
//! ## Single source of truth
//!
//! All user-configurable values live in [`crate::config::defaults`] and are
//! surfaced to the user through the `[search]` section of
//! `~/Library/Application Support/com.quietnode.thuki/config.toml`. This
//! module only owns constants that are **not** user-configurable, because
//! they are part of the prompt/retry contract: changing them at runtime
//! would silently break synthesized output rather than tune behavior.
//!
//! Production code reads runtime values from [`SearchRuntimeConfig`], which
//! is constructed from the loaded [`AppConfig`] at pipeline entry via
//! [`SearchRuntimeConfig::from_app_config`]. Tests use
//! [`SearchRuntimeConfig::default()`], which delegates to the same
//! `defaults` module so a missing `[search]` section produces identical
//! behavior to test builds.

use crate::config::defaults;
use crate::config::AppConfig;

/// Number of gap-filling queries generated per iteration round.
pub const GAP_QUERIES_PER_ROUND: usize = 3;

/// Approximate token budget for each retrieved page chunk. Drives the
/// chunker's split heuristic; downstream prompts assume this exact size.
pub const CHUNK_TOKEN_SIZE: usize = 500;

/// Number of highest-scoring chunks passed to the synthesis prompt.
pub const TOP_K_CHUNKS: usize = 8;

/// Milliseconds to wait before retrying a failed LLM call.
pub const LLM_RETRY_DELAY_MS: u64 = 500;

/// Milliseconds to wait before retrying a failed SearXNG call.
pub const SEARCH_RETRY_DELAY_MS: u64 = 1000;

/// Milliseconds to wait before retrying a failed reader fetch.
pub const READER_RETRY_DELAY_MS: u64 = 500;

/// Reader-batch timeout used by the test-only `SearchRuntimeConfig::default()`
/// override. Reduced to 1 second so `BatchTimeout` paths can be exercised
/// without sleeping for the production 30-second budget on every test run.
#[cfg(test)]
pub(crate) const TEST_READER_BATCH_TIMEOUT_S: u64 = 1;

/// Runtime search configuration resolved from [`AppConfig`] at pipeline entry.
///
/// Owning the runtime view as a flat struct (rather than threading
/// `&AppConfig` through the pipeline) keeps the search code free of any
/// dependency on the global config layout: only the loader and this struct
/// know about the `[search]` TOML schema.
#[derive(Debug, Clone)]
pub struct SearchRuntimeConfig {
    /// Base URL of the SearXNG instance (scheme + host + port, no path).
    pub searxng_url: String,
    /// Base URL of the reader/extractor sidecar (scheme + host + port, no path).
    pub reader_url: String,
    /// Maximum number of search-refine iterations before the pipeline gives up.
    pub max_iterations: usize,
    /// Number of top-ranked URLs forwarded to the reader after reranking.
    pub top_k_urls: usize,
    /// Seconds before a SearXNG query is abandoned.
    pub search_timeout_s: u64,
    /// Seconds allowed for a single URL fetch inside the reader.
    pub reader_per_url_timeout_s: u64,
    /// Seconds allowed for the full parallel reader batch to complete.
    pub reader_batch_timeout_s: u64,
    /// Seconds before the judge LLM call is abandoned.
    pub judge_timeout_s: u64,
    /// Seconds before the router LLM call is abandoned.
    pub router_timeout_s: u64,
}

impl SearchRuntimeConfig {
    /// Constructs the runtime config from the loaded [`AppConfig`].
    ///
    /// Performs the `u32 -> usize` width conversion at the boundary so the
    /// pipeline can index and count without further casts. The loader has
    /// already clamped every numeric field to its sanity bounds and replaced
    /// any empty URL with the compiled default, so all fields are guaranteed
    /// to hold usable values when this runs.
    pub fn from_app_config(cfg: &AppConfig) -> Self {
        Self {
            searxng_url: cfg.search.searxng_url.clone(),
            reader_url: cfg.search.reader_url.clone(),
            max_iterations: cfg.search.max_iterations as usize,
            top_k_urls: cfg.search.top_k_urls as usize,
            search_timeout_s: cfg.search.search_timeout_s,
            reader_per_url_timeout_s: cfg.search.reader_per_url_timeout_s,
            reader_batch_timeout_s: cfg.search.reader_batch_timeout_s,
            judge_timeout_s: cfg.search.judge_timeout_s,
            router_timeout_s: cfg.search.router_timeout_s,
        }
    }

    /// Derives the fully-qualified SearXNG search endpoint from `searxng_url`.
    /// Strips a trailing slash so concatenation never produces `//search`.
    pub fn searxng_endpoint(&self) -> String {
        format!("{}/search", self.searxng_url.trim_end_matches('/'))
    }
}

impl Default for SearchRuntimeConfig {
    /// Production defaults sourced from [`crate::config::defaults`].
    ///
    /// In test builds, `reader_batch_timeout_s` is reduced to
    /// [`TEST_READER_BATCH_TIMEOUT_S`] (1 s) so the `BatchTimeout` error
    /// path in the reader can be exercised quickly. This is the only field
    /// that diverges from production defaults, and the divergence is
    /// localised to test builds.
    fn default() -> Self {
        Self {
            searxng_url: defaults::DEFAULT_SEARXNG_URL.to_string(),
            reader_url: defaults::DEFAULT_READER_URL.to_string(),
            max_iterations: defaults::DEFAULT_MAX_ITERATIONS as usize,
            top_k_urls: defaults::DEFAULT_TOP_K_URLS as usize,
            search_timeout_s: defaults::DEFAULT_SEARCH_TIMEOUT_S,
            reader_per_url_timeout_s: defaults::DEFAULT_READER_PER_URL_TIMEOUT_S,
            #[cfg(not(test))]
            reader_batch_timeout_s: defaults::DEFAULT_READER_BATCH_TIMEOUT_S,
            #[cfg(test)]
            reader_batch_timeout_s: TEST_READER_BATCH_TIMEOUT_S,
            judge_timeout_s: defaults::DEFAULT_JUDGE_TIMEOUT_S,
            router_timeout_s: defaults::DEFAULT_ROUTER_TIMEOUT_S,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_only_constants_have_sane_bounds() {
        assert!((1..=5).contains(&GAP_QUERIES_PER_ROUND));
        assert!((128..=2048).contains(&CHUNK_TOKEN_SIZE));
        assert!((1..=32).contains(&TOP_K_CHUNKS));
    }

    #[test]
    fn retry_delays_are_bounded() {
        assert!(LLM_RETRY_DELAY_MS <= 2_000);
        assert!(SEARCH_RETRY_DELAY_MS <= 2_000);
        assert!(READER_RETRY_DELAY_MS <= 2_000);
    }

    #[test]
    fn default_matches_app_defaults_except_for_test_batch_timeout() {
        let cfg = SearchRuntimeConfig::default();
        assert_eq!(cfg.searxng_url, defaults::DEFAULT_SEARXNG_URL);
        assert_eq!(cfg.reader_url, defaults::DEFAULT_READER_URL);
        assert_eq!(
            cfg.max_iterations,
            defaults::DEFAULT_MAX_ITERATIONS as usize
        );
        assert_eq!(cfg.top_k_urls, defaults::DEFAULT_TOP_K_URLS as usize);
        assert_eq!(cfg.search_timeout_s, defaults::DEFAULT_SEARCH_TIMEOUT_S);
        assert_eq!(
            cfg.reader_per_url_timeout_s,
            defaults::DEFAULT_READER_PER_URL_TIMEOUT_S
        );
        assert_eq!(cfg.judge_timeout_s, defaults::DEFAULT_JUDGE_TIMEOUT_S);
        assert_eq!(cfg.router_timeout_s, defaults::DEFAULT_ROUTER_TIMEOUT_S);
        // Test-only override: production value is DEFAULT_READER_BATCH_TIMEOUT_S.
        assert_eq!(cfg.reader_batch_timeout_s, TEST_READER_BATCH_TIMEOUT_S);
    }

    #[test]
    fn from_app_config_copies_every_search_field() {
        let mut app = AppConfig::default();
        app.search.searxng_url = "http://10.0.0.1:9000".to_string();
        app.search.reader_url = "http://10.0.0.1:9001".to_string();
        app.search.max_iterations = 7;
        app.search.top_k_urls = 4;
        app.search.search_timeout_s = 11;
        app.search.reader_per_url_timeout_s = 12;
        app.search.reader_batch_timeout_s = 60;
        app.search.judge_timeout_s = 13;
        app.search.router_timeout_s = 14;

        let cfg = SearchRuntimeConfig::from_app_config(&app);
        assert_eq!(cfg.searxng_url, "http://10.0.0.1:9000");
        assert_eq!(cfg.reader_url, "http://10.0.0.1:9001");
        assert_eq!(cfg.max_iterations, 7);
        assert_eq!(cfg.top_k_urls, 4);
        assert_eq!(cfg.search_timeout_s, 11);
        assert_eq!(cfg.reader_per_url_timeout_s, 12);
        assert_eq!(cfg.reader_batch_timeout_s, 60);
        assert_eq!(cfg.judge_timeout_s, 13);
        assert_eq!(cfg.router_timeout_s, 14);
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
