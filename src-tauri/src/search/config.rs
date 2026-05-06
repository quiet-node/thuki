//! Runtime configuration adapter for the agentic `/search` pipeline.
//!
//! ## Single source of truth
//!
//! All default values — both user-configurable TOML knobs and pipeline-
//! internal structural constants — live in [`crate::config::defaults`].
//! This module owns only the adapter that projects [`AppConfig`] into the
//! flat [`SearchRuntimeConfig`] view consumed by the pipeline, and the
//! test-only batch-timeout override that keeps integration tests fast.
//!
//! Production code constructs [`SearchRuntimeConfig`] once at pipeline entry
//! via [`SearchRuntimeConfig::from_app_config`]. Tests use
//! [`SearchRuntimeConfig::default()`], which reads the same `defaults`
//! constants so a missing `[search]` section behaves identically to tests.

use crate::config::defaults;
use crate::config::AppConfig;

/// Reader-batch timeout used by the test-only [`SearchRuntimeConfig::default`]
/// override. Reduced to 1 s so the `BatchTimeout` error path in the reader
/// can be exercised without sleeping for the production 30-second budget.
#[cfg(test)]
pub(crate) const TEST_READER_BATCH_TIMEOUT_S: u64 = 1;

/// Runtime search configuration resolved from [`AppConfig`] at pipeline entry.
///
/// Owning the runtime view as a flat struct (rather than threading
/// `&AppConfig` through the pipeline) keeps search code free of any
/// dependency on the global config layout: only this module knows about the
/// `[search]` TOML schema. The `u32 -> usize` width conversions for
/// `max_iterations` and `top_k_urls` are performed once here so the pipeline
/// can index and count without scattered casts.
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
    /// Maximum number of results each SearXNG query contributes to the
    /// reranker.
    pub searxng_max_results: usize,
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
    /// Wall-clock budget for the whole pipeline turn (seconds). Enforced by
    /// `pipeline::PipelineBudget`; exhaustion drops the gap loop into the
    /// fallback synthesis path with a `BudgetExhausted` warning.
    pub pipeline_wall_clock_budget_s: u64,
    /// Cumulative cap on bytes of judge user-message input across all chunk-
    /// stage judge calls in a single pipeline turn. Defense-in-depth against
    /// runaway loops that keep fetching huge pages. Not exposed via TOML; the
    /// production default sources from `defaults::PIPELINE_INPUT_CHAR_BUDGET`.
    /// Tests dial this down to exercise the early-exit path without
    /// allocating hundreds of KB of source text.
    pub pipeline_input_char_budget: usize,
    /// Whether the forensic per-turn search trace recorder is on. Off in
    /// shipped builds; toggled from the Settings panel (Web tab, Diagnostics
    /// section). See [`crate::search::recorder`] for the file format.
    pub trace_enabled: bool,
}

impl SearchRuntimeConfig {
    /// Constructs the runtime config from the loaded [`AppConfig`].
    ///
    /// The loader has already clamped every numeric field to its sanity bounds
    /// and replaced any empty URL with the compiled default, so all fields are
    /// guaranteed to hold usable values when this runs.
    pub fn from_app_config(cfg: &AppConfig) -> Self {
        Self {
            searxng_url: cfg.search.searxng_url.clone(),
            reader_url: cfg.search.reader_url.clone(),
            max_iterations: cfg.search.max_iterations as usize,
            top_k_urls: cfg.search.top_k_urls as usize,
            searxng_max_results: cfg.search.searxng_max_results as usize,
            search_timeout_s: cfg.search.search_timeout_s,
            reader_per_url_timeout_s: cfg.search.reader_per_url_timeout_s,
            reader_batch_timeout_s: cfg.search.reader_batch_timeout_s,
            judge_timeout_s: cfg.search.judge_timeout_s,
            router_timeout_s: cfg.search.router_timeout_s,
            pipeline_wall_clock_budget_s: cfg.search.pipeline_wall_clock_budget_s,
            pipeline_input_char_budget: defaults::PIPELINE_INPUT_CHAR_BUDGET,
            trace_enabled: cfg.debug.trace_enabled,
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
            searxng_max_results: defaults::DEFAULT_SEARXNG_MAX_RESULTS as usize,
            search_timeout_s: defaults::DEFAULT_SEARCH_TIMEOUT_S,
            reader_per_url_timeout_s: defaults::DEFAULT_READER_PER_URL_TIMEOUT_S,
            #[cfg(not(test))]
            reader_batch_timeout_s: defaults::DEFAULT_READER_BATCH_TIMEOUT_S,
            #[cfg(test)]
            reader_batch_timeout_s: TEST_READER_BATCH_TIMEOUT_S,
            judge_timeout_s: defaults::DEFAULT_JUDGE_TIMEOUT_S,
            router_timeout_s: defaults::DEFAULT_ROUTER_TIMEOUT_S,
            pipeline_wall_clock_budget_s: defaults::DEFAULT_PIPELINE_WALL_CLOCK_BUDGET_S,
            pipeline_input_char_budget: defaults::PIPELINE_INPUT_CHAR_BUDGET,
            trace_enabled: defaults::DEFAULT_DEBUG_TRACE_ENABLED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_internal_defaults_have_sane_bounds() {
        assert!((1..=5).contains(&defaults::DEFAULT_GAP_QUERIES_PER_ROUND));
        assert!((128..=2048).contains(&defaults::DEFAULT_CHUNK_TOKEN_SIZE));
        assert!((1..=32).contains(&defaults::DEFAULT_TOP_K_CHUNKS));
        const _: () = assert!(defaults::DEFAULT_READER_RETRY_DELAY_MS <= 2_000);
        assert!((1..=2_000).contains(&defaults::DEFAULT_MAX_SNIPPET_CHARS));
        assert!((1..=2_000).contains(&defaults::DEFAULT_MAX_QUERY_CHARS));
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
        assert_eq!(
            cfg.searxng_max_results,
            defaults::DEFAULT_SEARXNG_MAX_RESULTS as usize
        );
        assert_eq!(cfg.search_timeout_s, defaults::DEFAULT_SEARCH_TIMEOUT_S);
        assert_eq!(
            cfg.reader_per_url_timeout_s,
            defaults::DEFAULT_READER_PER_URL_TIMEOUT_S
        );
        assert_eq!(cfg.judge_timeout_s, defaults::DEFAULT_JUDGE_TIMEOUT_S);
        assert_eq!(cfg.router_timeout_s, defaults::DEFAULT_ROUTER_TIMEOUT_S);
        assert_eq!(
            cfg.pipeline_wall_clock_budget_s,
            defaults::DEFAULT_PIPELINE_WALL_CLOCK_BUDGET_S
        );
        // Test-only override: production value is DEFAULT_READER_BATCH_TIMEOUT_S.
        assert_eq!(cfg.reader_batch_timeout_s, TEST_READER_BATCH_TIMEOUT_S);
        assert_eq!(cfg.trace_enabled, defaults::DEFAULT_DEBUG_TRACE_ENABLED);
    }

    #[test]
    fn from_app_config_copies_every_search_field() {
        let mut app = AppConfig::default();
        app.search.searxng_url = "http://10.0.0.1:9000".to_string();
        app.search.reader_url = "http://10.0.0.1:9001".to_string();
        app.search.max_iterations = 7;
        app.search.top_k_urls = 4;
        app.search.searxng_max_results = 6;
        app.search.search_timeout_s = 11;
        app.search.reader_per_url_timeout_s = 12;
        app.search.reader_batch_timeout_s = 60;
        app.search.judge_timeout_s = 13;
        app.search.router_timeout_s = 14;
        app.search.pipeline_wall_clock_budget_s = 120;
        app.debug.trace_enabled = true;

        let cfg = SearchRuntimeConfig::from_app_config(&app);
        assert_eq!(cfg.searxng_url, "http://10.0.0.1:9000");
        assert_eq!(cfg.reader_url, "http://10.0.0.1:9001");
        assert_eq!(cfg.max_iterations, 7);
        assert_eq!(cfg.top_k_urls, 4);
        assert_eq!(cfg.searxng_max_results, 6);
        assert_eq!(cfg.search_timeout_s, 11);
        assert_eq!(cfg.reader_per_url_timeout_s, 12);
        assert_eq!(cfg.reader_batch_timeout_s, 60);
        assert_eq!(cfg.judge_timeout_s, 13);
        assert_eq!(cfg.router_timeout_s, 14);
        assert_eq!(cfg.pipeline_wall_clock_budget_s, 120);
        assert!(cfg.trace_enabled);
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
