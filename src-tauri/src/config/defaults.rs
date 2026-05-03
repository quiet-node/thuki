//! Compiled default values for the application configuration.
//!
//! This is the ONE place where Thuki's default configuration lives. Every
//! other subsystem reads the resolved values from `AppConfig` via Tauri state.
//! Changing a default here propagates to a fresh first-run config file and to
//! any field a user has left unset or left empty in their existing file.

/// Default Ollama HTTP endpoint (loopback, standard port).
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

/// Default inactivity window before Thuki tells Ollama to release the model.
/// 0 means do not manage: Ollama's own 5-minute default applies.
/// -1 means keep indefinitely. Positive values are minutes (1..=1440).
pub const DEFAULT_KEEP_WARM_INACTIVITY_MINUTES: i32 = 0;

/// Ollama context window size (tokens) sent with every /api/chat request.
/// 16 384 tokens gives the full system prompt (~4 000 tokens) plus ~12 000
/// tokens of conversation history while staying within the VRAM budget of
/// the target models. Warmup and chat MUST use the same value so Ollama
/// reuses the same runner instance and its cached KV prefix.
pub const DEFAULT_NUM_CTX: u32 = 16384;

/// Accepted range for `num_ctx`. Values below 2 048 cannot fit the built-in
/// system prompt and leave nothing for conversation history. No upper cap is
/// enforced here: Ollama silently clamps `num_ctx` to the model's physical
/// maximum, so any value is safe to pass through. The 1 048 576 (1 M) ceiling
/// is a sanity guard against TOML typos (e.g. an extra zero) and covers every
/// current consumer model including the largest 1 M-context variants.
pub const BOUNDS_NUM_CTX: (u32, u32) = (2048, 1_048_576);

/// Accepted range for `keep_warm_inactivity_minutes`.
/// -1 = never release, 0 = disabled (Ollama default), 1..=1440 = explicit timeout.
/// Values below -1 or above 1440 are clamped to the compiled default.
pub const BOUNDS_KEEP_WARM_INACTIVITY_MINUTES: (i32, i32) = (-1, 1440);

/// Built-in secretary persona prompt. User overrides via `[prompt] system` in
/// the config file. The slash-command appendix is composed on top at load time
/// and is never written back to the file.
pub const DEFAULT_SYSTEM_PROMPT_BASE: &str = include_str!("../../prompts/system_prompt.txt");

/// Generated appendix listing supported slash commands. Composed on top of
/// the user-editable base prompt at load time so built-in command knowledge
/// stays in sync with the registry even when the persona prompt is overridden.
pub const SLASH_COMMAND_PROMPT_APPENDIX: &str =
    include_str!("../../prompts/generated/slash_commands.txt");

/// Window defaults (logical pixels and counts). Only the user-tunable knobs
/// live here; the collapsed-bar height and the close-animation deadline are
/// baked into `App.tsx` because their effective range is invisible to users
/// (see the rationale comment on `WindowSection` in `schema.rs`).
pub const DEFAULT_OVERLAY_WIDTH: f64 = 600.0;
pub const DEFAULT_MAX_CHAT_HEIGHT: f64 = 648.0;
/// Maximum number of manually attached images per message. One additional
/// image from /screen capture is allowed on top of this, so the total
/// per-message image count is max_images + 1. Raise for more visual context
/// per message; lower to keep prompts compact.
pub const DEFAULT_MAX_IMAGES: u32 = 3;

/// Quote display defaults.
pub const DEFAULT_QUOTE_MAX_DISPLAY_LINES: u32 = 4;
pub const DEFAULT_QUOTE_MAX_DISPLAY_CHARS: u32 = 300;
pub const DEFAULT_QUOTE_MAX_CONTEXT_LENGTH: u32 = 4096;

/// Numeric sanity bounds used by the loader to reject values that would brick
/// the UI. Out-of-bounds values fall back to compiled defaults. The bounds
/// themselves are intentionally generous: the intent is to catch typos
/// (zeros, missing digits), not to second-guess tasteful customization.
pub const BOUNDS_OVERLAY_WIDTH: (f64, f64) = (200.0, 2000.0);
pub const BOUNDS_MAX_CHAT_HEIGHT: (f64, f64) = (200.0, 2000.0);
pub const BOUNDS_MAX_IMAGES: (u32, u32) = (1, 20);
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

/// Wall-clock budget for an entire `/search` pipeline turn (seconds). When
/// exceeded, the gap-refinement loop exits early and the pipeline force-
/// synthesizes on whatever evidence has been gathered so far, emitting a
/// `BudgetExhausted` warning. Bounds the worst-case latency a user can
/// observe regardless of how often the LLM produces fresh gap queries.
/// Raise for deeper research turns; lower for snappier interactive use.
pub const DEFAULT_PIPELINE_WALL_CLOCK_BUDGET_S: u64 = 90;

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

/// Interval between background polls of Ollama `/api/ps` for external VRAM
/// changes (user-initiated `ollama stop`, TTL expiry, daemon restart). Not
/// user-tunable: tuning this trades responsiveness against localhost load but
/// the 5 s value is already generous for a loopback call.
pub const VRAM_POLL_INTERVAL_SECS: u64 = 5;

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

/// Accepted range for the pipeline wall-clock budget (seconds). 15 s is the
/// floor: anything tighter would force budget exhaustion on every gap-loop
/// turn that needs more than one reader fetch. 600 s (10 min) is the ceiling:
/// a single user search should never tie up the daemon longer than that.
pub const BOUNDS_PIPELINE_WALL_CLOCK_BUDGET_S: (u64, u64) = (15, 600);

/// Cumulative cap on bytes of judge user-message input across all judge calls
/// in a single pipeline turn. Tracked as bytes (not tokens) because the byte
/// length of the source list is the cheapest reliable upper bound on prompt
/// size; chars-to-tokens varies per tokenizer. 200 KB ~ 50k tokens which is
/// well above what any reasonable agentic search consumes. Defense-in-depth
/// against a runaway loop that keeps fetching huge pages. Not user-tunable
/// because it bounds attacker-influenced data (page content from the reader)
/// and the wall-clock budget is the user-facing knob.
pub const PIPELINE_INPUT_CHAR_BUDGET: usize = 200_000;

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

/// Authoritative allowlist of `(section, key)` pairs the Settings GUI is
/// permitted to write via the `set_config_field` Tauri command.
///
/// This list is the security boundary between the frontend and the on-disk
/// configuration. The command rejects any `(section, key)` not present here
/// with a typed `UnknownSection` / `UnknownField` error, preventing the GUI
/// from attempting to write fields that do not exist or that are intentionally
/// not user-tunable.
///
/// A compile-time test (`config::tests::allowed_fields_match_schema`) asserts
/// the list size matches the count of tunable fields in `AppConfig` so any
/// future schema addition must extend this list explicitly.
///
/// Order matches `AppConfig` field ordering for review-friendliness.
pub const ALLOWED_FIELDS: &[(&str, &str)] = &[
    // [inference]
    ("inference", "ollama_url"),
    ("inference", "keep_warm_inactivity_minutes"),
    ("inference", "num_ctx"),
    // [prompt]
    ("prompt", "system"),
    // [window]
    ("window", "overlay_width"),
    ("window", "max_chat_height"),
    ("window", "max_images"),
    // [quote]
    ("quote", "max_display_lines"),
    ("quote", "max_display_chars"),
    ("quote", "max_context_length"),
    // [search]
    ("search", "searxng_url"),
    ("search", "reader_url"),
    ("search", "max_iterations"),
    ("search", "top_k_urls"),
    ("search", "searxng_max_results"),
    ("search", "search_timeout_s"),
    ("search", "reader_per_url_timeout_s"),
    ("search", "reader_batch_timeout_s"),
    ("search", "judge_timeout_s"),
    ("search", "router_timeout_s"),
];

/// Authoritative allowlist of section names accepted by `reset_config`.
/// Mirrors the top-level structure of `AppConfig`.
pub const ALLOWED_SECTIONS: &[&str] = &["inference", "prompt", "window", "quote", "search"];

/// Special turn-boundary tokens used by the major Ollama-served model families.
/// Ollama normally parses these out of `/api/chat` responses, but some fine-tunes
/// leak them into `message.content` as plain text. If the leaked bytes are persisted
/// into history and replayed to a model from a different family on the next turn,
/// that model treats them as garbage tokens and the conversation visibly degrades.
///
/// Stripped before persisting assistant replies and again at render time so legacy
/// on-disk content stays clean visually without a migration. Exact-string match,
/// case-sensitive: these markers are not natural English, so any false-positive
/// collision would already be a bug elsewhere.
///
/// The TypeScript mirror of this list lives in `src/utils/sanitizeAssistantContent.ts`
/// (`STRIP_PATTERNS`). Keep both in sync when adding new model families.
///
/// Not user-tunable: defense-in-depth bound on external/attacker-controlled data.
/// Exposing it would let a malformed or adversarial model response disable the
/// sanitization layer.
pub const STRIP_PATTERNS: &[&str] = &[
    "<|im_start|>",
    "<|im_end|>",
    "<|begin_of_text|>",
    "<|end_of_text|>",
    "<|start_header_id|>",
    "<|end_header_id|>",
    "<|eot_id|>",
    "[INST]",
    "[/INST]",
    "<start_of_turn>",
    "<end_of_turn>",
    "<|endoftext|>",
    "<|user|>",
    "<|assistant|>",
    "<|system|>",
    "<think>",
    "</think>",
];
