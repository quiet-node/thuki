//! Compiled default values for the application configuration.
//!
//! This is the ONE place where Thuki's default configuration lives. Every
//! other subsystem reads the resolved values from `AppConfig` via Tauri state.
//! Changing a default here propagates to a fresh first-run config file and to
//! any field a user has left unset or left empty in their existing file.

/// Default Ollama HTTP endpoint (loopback, standard port).
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

/// Built-in secretary persona prompt. User overrides via `[prompt] system` in
/// the config file. The slash-command appendix is composed on top at load time
/// and is never written back to the file.
pub const DEFAULT_SYSTEM_PROMPT_BASE: &str = include_str!("../../prompts/system_prompt.txt");

/// Generated appendix listing supported slash commands. Composed on top of
/// the user-editable base prompt at load time so built-in command knowledge
/// stays in sync with the registry even when the persona prompt is overridden.
pub const SLASH_COMMAND_PROMPT_APPENDIX: &str =
    include_str!("../../prompts/generated/slash_commands.txt");

/// Window defaults (logical pixels / milliseconds).
pub const DEFAULT_OVERLAY_WIDTH: f64 = 600.0;
pub const DEFAULT_COLLAPSED_HEIGHT: f64 = 80.0;
pub const DEFAULT_MAX_CHAT_HEIGHT: f64 = 648.0;
pub const DEFAULT_HIDE_COMMIT_DELAY_MS: u64 = 350;

/// Quote display defaults.
pub const DEFAULT_QUOTE_MAX_DISPLAY_LINES: u32 = 4;
pub const DEFAULT_QUOTE_MAX_DISPLAY_CHARS: u32 = 300;
pub const DEFAULT_QUOTE_MAX_CONTEXT_LENGTH: u32 = 4096;

/// Numeric sanity bounds used by the loader to reject values that would brick
/// the UI. Out-of-bounds values fall back to compiled defaults. The bounds
/// themselves are intentionally generous: the intent is to catch typos
/// (zeros, missing digits), not to second-guess tasteful customization.
pub const BOUNDS_OVERLAY_WIDTH: (f64, f64) = (200.0, 2000.0);
pub const BOUNDS_COLLAPSED_HEIGHT: (f64, f64) = (40.0, 400.0);
pub const BOUNDS_MAX_CHAT_HEIGHT: (f64, f64) = (200.0, 2000.0);
pub const BOUNDS_HIDE_COMMIT_DELAY_MS: (u64, u64) = (0, 5000);
pub const BOUNDS_QUOTE_MAX_DISPLAY_LINES: (u32, u32) = (1, 100);
pub const BOUNDS_QUOTE_MAX_DISPLAY_CHARS: (u32, u32) = (1, 10_000);
pub const BOUNDS_QUOTE_MAX_CONTEXT_LENGTH: (u32, u32) = (1, 65_536);

/// Search service default URLs. Match the Docker sandbox bindings in
/// `sandbox/docker-compose.yml`. Users running SearXNG or the reader
/// service on a different port override these in `[search]` in config.toml.
pub const DEFAULT_SEARXNG_URL: &str = "http://127.0.0.1:25017";
pub const DEFAULT_READER_URL: &str = "http://127.0.0.1:25018";

/// Default values for user-configurable search pipeline tuning knobs.
/// `max_iterations` caps the search-refine loop count; `top_k_urls` limits
/// how many reranked URLs are forwarded to the reader;
/// `searxng_max_results` caps how many results each SearXNG query
/// contributes before reranking. All are overridable under `[search]` in
/// config.toml.
pub const DEFAULT_MAX_ITERATIONS: u32 = 3;
pub const DEFAULT_TOP_K_URLS: u32 = 10;
pub const DEFAULT_SEARXNG_MAX_RESULTS: u32 = 10;

/// Defense-in-depth caps on data flowing in/out of SearXNG. These are NOT
/// exposed in config.toml: `MAX_QUERY_CHARS` bounds outgoing queries to the
/// external engines (so a malformed prompt cannot DOS them), and
/// `MAX_SNIPPET_CHARS` bounds the per-result text Thuki accepts back (so a
/// malicious search result cannot flood the rerank prompt). Both apply
/// before any user-controllable knob, in unicode scalar values.
pub const DEFAULT_MAX_SNIPPET_CHARS: usize = 500;
pub const DEFAULT_MAX_QUERY_CHARS: usize = 500;

// Pipeline-internal defaults: not exposed in config.toml because they are
// part of the prompt and retry contract. Changing these values alters output
// shape and quality, not only latency, so they are intentionally not
// user-tunable at runtime.

/// Gap-filling queries generated per iteration round. Drives the judge
/// normalization cap in `search::judge::normalize_verdict`.
pub const DEFAULT_GAP_QUERIES_PER_ROUND: usize = 3;
/// Approximate token budget for each retrieved page chunk. Drives the
/// chunker split heuristic; downstream prompts assume this exact size.
pub const DEFAULT_CHUNK_TOKEN_SIZE: usize = 500;
/// Number of highest-scoring chunks forwarded to the synthesis prompt.
pub const DEFAULT_TOP_K_CHUNKS: usize = 8;
/// Milliseconds before retrying a failed reader fetch.
pub const DEFAULT_READER_RETRY_DELAY_MS: u64 = 500;

/// Search timeout defaults (seconds).
pub const DEFAULT_SEARCH_TIMEOUT_S: u64 = 20;
pub const DEFAULT_READER_PER_URL_TIMEOUT_S: u64 = 10;
pub const DEFAULT_READER_BATCH_TIMEOUT_S: u64 = 30;
pub const DEFAULT_JUDGE_TIMEOUT_S: u64 = 30;
pub const DEFAULT_ROUTER_TIMEOUT_S: u64 = 45;

/// Bounds for search pipeline counts.
pub const BOUNDS_MAX_ITERATIONS: (u32, u32) = (1, 10);
pub const BOUNDS_TOP_K_URLS: (u32, u32) = (1, 20);
pub const BOUNDS_SEARXNG_MAX_RESULTS: (u32, u32) = (1, 20);

/// Bounds for all search timeout fields (seconds). 300 s (5 min) is the
/// ceiling: a timeout longer than that indicates a misconfiguration, not a
/// slow service.
pub const BOUNDS_TIMEOUT_S: (u64, u64) = (1, 300);

// Ollama API baked-in limits: not exposed in config.toml because they bound
// attacker-controlled data (response bodies from the local Ollama daemon) and
// keep the UI responsive when the daemon is hung. Changing either timeout
// value would require re-tuning the UX; changing the byte caps would require
// re-evaluating the memory budget.

/// Per-request timeout (in seconds) for the Ollama `/api/tags` GET. Guards
/// the IPC boundary: if the daemon accepts the TCP connection but never
/// responds, `get_model_picker_state` would otherwise block indefinitely and
/// wedge the UI. 5 seconds is generous for a localhost call.
pub const DEFAULT_OLLAMA_TAGS_REQUEST_TIMEOUT_SECS: u64 = 5;

/// Per-request timeout (in seconds) for the Ollama `/api/show` POST. Same
/// rationale as `DEFAULT_OLLAMA_TAGS_REQUEST_TIMEOUT_SECS`: local-loopback
/// HTTP is normally instant, but capping prevents a wedged daemon from
/// blocking picker rendering.
pub const DEFAULT_OLLAMA_SHOW_REQUEST_TIMEOUT_SECS: u64 = 5;

/// Maximum accepted body size for the Ollama `/api/tags` response. Guards
/// against a misbehaving or compromised localhost Ollama streaming an
/// unbounded response that would exhaust memory. 4 MiB comfortably fits
/// thousands of model entries.
pub const MAX_OLLAMA_TAGS_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Maximum accepted body size for the Ollama `/api/show` response. The full
/// Modelfile and parameters can be sizable, but 4 MiB is comfortably above
/// any real model and bounds attacker-controlled inputs.
pub const MAX_OLLAMA_SHOW_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Maximum accepted byte length for a model slug passed to `set_active_model`.
/// Real Ollama slugs are a handful of characters; 256 is generous while still
/// capping adversarial inputs long before any network or database work.
pub const MAX_MODEL_SLUG_LEN: usize = 256;
