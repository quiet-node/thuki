//! Typed shape of the Thuki configuration file.
//!
//! Serde derives the TOML mapping automatically. Each section struct carries
//! `#[serde(default)]` so a partial file (missing whole sections or fields)
//! deserializes cleanly: missing fields inherit the compiled defaults via the
//! manual `Default` impls below.
//!
//! Section structs use manual `Default` impls (NOT `#[derive(Default)]`)
//! because deriving Default would fill fields with zero/empty values
//! (`String::default() == ""`, `u64::default() == 0`), which is the opposite
//! of what the user expects. `AppConfig` itself uses `#[derive(Default)]`
//! because it delegates entirely to each section's own `Default` impl.

use serde::{Deserialize, Serialize};

use super::defaults::{
    DEFAULT_COLLAPSED_HEIGHT, DEFAULT_HIDE_COMMIT_DELAY_MS, DEFAULT_JUDGE_TIMEOUT_S,
    DEFAULT_MAX_CHAT_HEIGHT, DEFAULT_MAX_ITERATIONS, DEFAULT_MODEL_NAME, DEFAULT_OLLAMA_URL,
    DEFAULT_OVERLAY_WIDTH, DEFAULT_QUOTE_MAX_CONTEXT_LENGTH, DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
    DEFAULT_QUOTE_MAX_DISPLAY_LINES, DEFAULT_READER_BATCH_TIMEOUT_S,
    DEFAULT_READER_PER_URL_TIMEOUT_S, DEFAULT_READER_URL, DEFAULT_ROUTER_TIMEOUT_S,
    DEFAULT_SEARCH_TIMEOUT_S, DEFAULT_SEARXNG_MAX_RESULTS, DEFAULT_SEARXNG_URL, DEFAULT_TOP_K_URLS,
};

/// Model configuration. The first entry of `available` is the active model
/// used for all inference. Reorder the list (or use the future settings panel)
/// to switch models. Keeping a single list instead of separate `active` and
/// `available` fields eliminates the mismatch failure mode entirely.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ModelSection {
    /// Ollama models Thuki knows about. First entry is active.
    pub available: Vec<String>,
    /// HTTP base URL of the local Ollama instance.
    pub ollama_url: String,
}

impl Default for ModelSection {
    fn default() -> Self {
        Self {
            available: vec![DEFAULT_MODEL_NAME.to_string()],
            ollama_url: DEFAULT_OLLAMA_URL.to_string(),
        }
    }
}

impl ModelSection {
    /// Returns the active model (first entry). Falls back to the compiled
    /// default if the list is somehow empty at call time; the loader also
    /// guarantees this never happens by calling `resolve` during load.
    pub fn active(&self) -> &str {
        self.available
            .first()
            .map(String::as_str)
            .unwrap_or(DEFAULT_MODEL_NAME)
    }
}

/// Prompt configuration. `system` holds only the user-editable base text.
/// The slash-command appendix is composed at load time into `resolved_system`
/// and is never written back to the file. `resolved_system` is computed, not
/// serialized.
///
/// Note: `#[derive(Default)]` is correct here because both fields genuinely
/// start empty: `system` empty means "use the built-in persona", and
/// `resolved_system` is populated by the loader before any consumer reads it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct PromptSection {
    /// User-editable persona prompt. Empty means "use the built-in default".
    pub system: String,
    /// Composed runtime value (base prompt plus slash-command appendix).
    /// Not serialized; computed by the loader.
    #[serde(skip)]
    pub resolved_system: String,
}

/// Overlay window geometry and animation timing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowSection {
    /// Logical width of the overlay window.
    pub overlay_width: f64,
    /// Height of the collapsed (AskBar) state.
    pub collapsed_height: f64,
    /// Maximum height the expanded chat window is allowed to grow to.
    pub max_chat_height: f64,
    /// Delay before actually hiding the NSPanel after the exit animation starts.
    pub hide_commit_delay_ms: u64,
}

impl Default for WindowSection {
    fn default() -> Self {
        Self {
            overlay_width: DEFAULT_OVERLAY_WIDTH,
            collapsed_height: DEFAULT_COLLAPSED_HEIGHT,
            max_chat_height: DEFAULT_MAX_CHAT_HEIGHT,
            hide_commit_delay_ms: DEFAULT_HIDE_COMMIT_DELAY_MS,
        }
    }
}

/// Selected-text quote display configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct QuoteSection {
    pub max_display_lines: u32,
    pub max_display_chars: u32,
    pub max_context_length: u32,
}

impl Default for QuoteSection {
    fn default() -> Self {
        Self {
            max_display_lines: DEFAULT_QUOTE_MAX_DISPLAY_LINES,
            max_display_chars: DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
            max_context_length: DEFAULT_QUOTE_MAX_CONTEXT_LENGTH,
        }
    }
}

/// Search pipeline and service configuration.
///
/// Service URLs control where the SearXNG and reader sidecar processes live.
/// The defaults match the Docker sandbox bindings in `sandbox/docker-compose.yml`.
/// Users who remap ports or run the services on a different host set these in
/// `[search]` in config.toml; no rebuild required.
///
/// Pipeline tuning knobs (`max_iterations`, `top_k_urls`) let users trade
/// search quality against latency. Timeout fields cover slow networks and slow
/// local hardware. Values that would create an inconsistency (e.g.
/// `reader_batch_timeout_s <= reader_per_url_timeout_s`) are silently corrected
/// by the loader.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SearchSection {
    /// Base URL of the SearXNG instance (scheme + host + port, no path).
    /// The `/search` endpoint is appended automatically.
    pub searxng_url: String,
    /// Base URL of the reader/extractor sidecar (scheme + host + port, no path).
    pub reader_url: String,
    /// Maximum number of search-refine iterations before the pipeline gives up.
    pub max_iterations: u32,
    /// Number of top-ranked URLs forwarded to the reader after reranking.
    pub top_k_urls: u32,
    /// Maximum number of results each SearXNG query contributes to the
    /// reranker. Acts before rerank to bound prompt size and latency: lower
    /// values trade recall for speed; higher values give the reranker more
    /// candidates per query.
    pub searxng_max_results: u32,
    /// Seconds before a SearXNG query is abandoned.
    pub search_timeout_s: u64,
    /// Seconds allowed for a single URL fetch inside the reader.
    pub reader_per_url_timeout_s: u64,
    /// Seconds allowed for the full parallel reader batch to complete.
    /// Must exceed `reader_per_url_timeout_s`; the loader corrects violations.
    pub reader_batch_timeout_s: u64,
    /// Seconds before the judge LLM call is abandoned.
    pub judge_timeout_s: u64,
    /// Seconds before the router LLM call is abandoned.
    pub router_timeout_s: u64,
}

impl Default for SearchSection {
    fn default() -> Self {
        Self {
            searxng_url: DEFAULT_SEARXNG_URL.to_string(),
            reader_url: DEFAULT_READER_URL.to_string(),
            max_iterations: DEFAULT_MAX_ITERATIONS,
            top_k_urls: DEFAULT_TOP_K_URLS,
            searxng_max_results: DEFAULT_SEARXNG_MAX_RESULTS,
            search_timeout_s: DEFAULT_SEARCH_TIMEOUT_S,
            reader_per_url_timeout_s: DEFAULT_READER_PER_URL_TIMEOUT_S,
            reader_batch_timeout_s: DEFAULT_READER_BATCH_TIMEOUT_S,
            judge_timeout_s: DEFAULT_JUDGE_TIMEOUT_S,
            router_timeout_s: DEFAULT_ROUTER_TIMEOUT_S,
        }
    }
}

/// Top-level application configuration. Managed Tauri state; every subsystem
/// reads from `State<AppConfig>` and nowhere else. The loader resolves all
/// empty strings and out-of-bounds numerics to compiled defaults before the
/// `AppConfig` is installed, so every field here holds a usable value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct AppConfig {
    pub model: ModelSection,
    pub prompt: PromptSection,
    pub window: WindowSection,
    pub quote: QuoteSection,
    pub search: SearchSection,
}
