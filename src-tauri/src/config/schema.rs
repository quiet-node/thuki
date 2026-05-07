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
    DEFAULT_DEBUG_TRACE_ENABLED, DEFAULT_JUDGE_TIMEOUT_S, DEFAULT_KEEP_WARM_INACTIVITY_MINUTES,
    DEFAULT_MAX_CHAT_HEIGHT, DEFAULT_MAX_IMAGES, DEFAULT_MAX_ITERATIONS, DEFAULT_NUM_CTX,
    DEFAULT_OLLAMA_URL, DEFAULT_OVERLAY_WIDTH, DEFAULT_PIPELINE_WALL_CLOCK_BUDGET_S,
    DEFAULT_QUOTE_MAX_CONTEXT_LENGTH, DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
    DEFAULT_QUOTE_MAX_DISPLAY_LINES, DEFAULT_READER_BATCH_TIMEOUT_S,
    DEFAULT_READER_PER_URL_TIMEOUT_S, DEFAULT_READER_URL, DEFAULT_ROUTER_TIMEOUT_S,
    DEFAULT_SEARCH_TIMEOUT_S, DEFAULT_SEARXNG_MAX_RESULTS, DEFAULT_SEARXNG_URL, DEFAULT_TOP_K_URLS,
    DEFAULT_UPDATER_AUTO_CHECK, DEFAULT_UPDATER_CHECK_INTERVAL_HOURS, DEFAULT_UPDATER_MANIFEST_URL,
};

/// Static, user-tunable inference daemon configuration.
///
/// The active model selection is NOT stored here. Active-model state is
/// runtime UI state owned by [`crate::models::ActiveModelState`] and
/// persisted in the SQLite `app_config` table under
/// [`crate::models::ACTIVE_MODEL_KEY`]. Storing a model slug in TOML would
/// duplicate ground truth from Ollama's `/api/tags` and create a staleness
/// trap: the file would happily reference a model the user has since
/// removed. This section keeps only the truly static knob, the Ollama
/// endpoint URL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct InferenceSection {
    /// HTTP base URL of the local Ollama instance.
    pub ollama_url: String,
    /// Minutes of inactivity before Thuki tells Ollama to release the model.
    /// 0 means do not manage (Ollama's 5-minute default applies).
    /// -1 means keep indefinitely. Valid range: -1 or 0..=1440.
    pub keep_warm_inactivity_minutes: i32,
    /// Context window size (in tokens) sent to Ollama with every request.
    /// Warmup and chat use the same value so Ollama reuses the same runner
    /// instance and its cached KV prefix for the system prompt. Raise to fit
    /// longer conversations in a single context; lower to use less VRAM.
    /// Valid range: 2048..=1048576.
    pub num_ctx: u32,
}

impl Default for InferenceSection {
    fn default() -> Self {
        Self {
            ollama_url: DEFAULT_OLLAMA_URL.to_string(),
            keep_warm_inactivity_minutes: DEFAULT_KEEP_WARM_INACTIVITY_MINUTES,
            num_ctx: DEFAULT_NUM_CTX,
        }
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

/// Overlay UI configuration. Holds window geometry and input attachment
/// limits. The collapsed-bar height and the close-animation deadline are
/// baked into the frontend (see `App.tsx`) because their effective range is
/// invisible to the user (collapsed height is overwritten by the
/// ResizeObserver within a frame; the hide delay sits below normal perception
/// across its usable range and creates a visible pop if dropped below the
/// exit-animation duration).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowSection {
    /// Logical width of the overlay window.
    pub overlay_width: f64,
    /// Maximum height the expanded chat window is allowed to grow to.
    pub max_chat_height: f64,
    /// Maximum number of manually attached images per message. One additional
    /// image from /screen capture is allowed on top, for a total of
    /// max_images + 1 per message.
    pub max_images: u32,
}

impl Default for WindowSection {
    fn default() -> Self {
        Self {
            overlay_width: DEFAULT_OVERLAY_WIDTH,
            max_chat_height: DEFAULT_MAX_CHAT_HEIGHT,
            max_images: DEFAULT_MAX_IMAGES,
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
    /// Wall-clock budget for the full `/search` pipeline turn (seconds).
    /// When exceeded, the gap-refinement loop bails out early and the
    /// pipeline force-synthesizes on whatever evidence it has gathered,
    /// surfacing a `BudgetExhausted` warning. Raise for deeper research;
    /// lower for snappier interactive use.
    pub pipeline_wall_clock_budget_s: u64,
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
            pipeline_wall_clock_budget_s: DEFAULT_PIPELINE_WALL_CLOCK_BUDGET_S,
        }
    }
}

/// Developer and power-user debugging knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DebugSection {
    /// Records every chat conversation and `/search` session to JSON-Lines
    /// files under `app_data_dir/traces/{chat,search}/<conversation_id>.jsonl`.
    /// Off by default; toggleable from Settings.
    pub trace_enabled: bool,
}

impl Default for DebugSection {
    fn default() -> Self {
        Self {
            trace_enabled: DEFAULT_DEBUG_TRACE_ENABLED,
        }
    }
}

/// Auto-update configuration. Determines whether and how often Thuki polls
/// for new releases via the bundled tauri-plugin-updater.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UpdaterSection {
    /// Poll for updates automatically at startup and every
    /// `check_interval_hours` hours while running.
    #[serde(default = "default_updater_auto_check")]
    pub auto_check: bool,

    /// Hours between automatic background checks. Bound to 1..168.
    #[serde(default = "default_updater_check_interval_hours")]
    pub check_interval_hours: u64,

    /// URL to fetch the update manifest from.
    #[serde(default = "default_updater_manifest_url")]
    pub manifest_url: String,
}

fn default_updater_auto_check() -> bool {
    DEFAULT_UPDATER_AUTO_CHECK
}
fn default_updater_check_interval_hours() -> u64 {
    DEFAULT_UPDATER_CHECK_INTERVAL_HOURS
}
fn default_updater_manifest_url() -> String {
    DEFAULT_UPDATER_MANIFEST_URL.to_string()
}

impl Default for UpdaterSection {
    fn default() -> Self {
        Self {
            auto_check: DEFAULT_UPDATER_AUTO_CHECK,
            check_interval_hours: DEFAULT_UPDATER_CHECK_INTERVAL_HOURS,
            manifest_url: DEFAULT_UPDATER_MANIFEST_URL.to_string(),
        }
    }
}

/// Top-level application configuration. Managed Tauri state; every subsystem
/// reads from `State<RwLock<AppConfig>>` and nowhere else. The loader resolves all
/// empty strings and out-of-bounds numerics to compiled defaults before the
/// `AppConfig` is installed, so every field here holds a usable value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct AppConfig {
    pub inference: InferenceSection,
    pub prompt: PromptSection,
    pub window: WindowSection,
    pub quote: QuoteSection,
    pub search: SearchSection,
    pub debug: DebugSection,
    #[serde(default)]
    pub updater: UpdaterSection,
}
