//! Compiled default values for the application configuration.
//!
//! This is the ONE place where Thuki's default configuration lives. Every
//! other subsystem reads the resolved values from `AppConfig` via Tauri state.
//! Changing a default here propagates to a fresh first-run config file and to
//! any field a user has left unset or left empty in their existing file.

/// Default active model name, used when no config file exists yet and when a
/// user's `[model] available` list is empty after whitespace resolution.
pub const DEFAULT_MODEL_NAME: &str = "gemma4:e2b";

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

/// Latest config schema version understood by this build. Present in every
/// written file so future migrations have a fixed point to branch from.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

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
/// how many reranked URLs are forwarded to the reader. Both are overridable
/// under `[search]` in config.toml.
pub const DEFAULT_MAX_ITERATIONS: u32 = 3;
pub const DEFAULT_TOP_K_URLS: u32 = 10;

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

/// Bounds for all search timeout fields (seconds). 300 s (5 min) is the
/// ceiling: a timeout longer than that indicates a misconfiguration, not a
/// slow service.
pub const BOUNDS_TIMEOUT_S: (u64, u64) = (1, 300);
