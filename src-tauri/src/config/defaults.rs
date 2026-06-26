//! Compiled default values for the application configuration.
//!
//! This is the ONE place where Thuki's default configuration lives. Every
//! other subsystem reads the resolved values from `AppConfig` via Tauri state.
//! Changing a default here propagates to a fresh first-run config file and to
//! any field a user has left unset or left empty in their existing file.

/// Default Ollama HTTP endpoint (loopback, standard port). Seed value for the
/// Ollama provider's `base_url` on a fresh install or after a migration.
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

/// Stable provider ids. `active_provider` references one of these.
pub const PROVIDER_ID_BUILTIN: &str = "builtin";
pub const PROVIDER_ID_OLLAMA: &str = "ollama";
/// Fixed id of the (at most one) OpenAI-compatible provider record. A single
/// record mirrors the single Ollama URL: one external server at a time.
pub const PROVIDER_ID_OPENAI: &str = "openai";

/// Provider kinds understood by the loader. Providers with any other kind are
/// dropped during resolution. Recognized kinds: `"builtin"`, `"ollama"`,
/// `"openai"`.
pub const PROVIDER_KIND_BUILTIN: &str = "builtin";
pub const PROVIDER_KIND_OLLAMA: &str = "ollama";
/// Any OpenAI-compatible local or remote inference server (LM Studio, Jan,
/// llama-server, etc.). Requires a valid http(s) `base_url`; providers with
/// an empty or non-http(s) URL are dropped rather than healed (unlike Ollama,
/// there is no sensible localhost default for arbitrary /v1 servers).
pub const PROVIDER_KIND_OPENAI: &str = "openai";

/// Human-readable provider labels shown in Settings.
pub const DEFAULT_BUILTIN_LABEL: &str = "Built-in";
pub const DEFAULT_OLLAMA_LABEL: &str = "Ollama";
/// Fallback label for an OpenAI-compatible provider added with no label.
pub const DEFAULT_OPENAI_LABEL: &str = "OpenAI-compatible";

/// Provider Thuki sends inference to on a fresh install.
///
/// Thuki bundles the llama.cpp engine, so a new install starts on the
/// built-in provider and onboarding offers a starter model download. Configs
/// that already persisted an `active_provider` (including the older
/// Ollama-only default) are never rewritten; only fresh or dangling pointers
/// land here.
pub const DEFAULT_ACTIVE_PROVIDER: &str = PROVIDER_ID_BUILTIN;

/// Default inactivity window before Thuki releases the active model from local
/// memory. Unified across both local providers (built-in engine and Ollama);
/// not applicable to a remote OpenAI-compatible server, whose residency Thuki
/// does not manage.
/// 0 means use the provider's natural short default (~5 min): Ollama defers to
/// its own 5-minute timer, the built-in engine applies its own ~5-minute timer
/// (see `DEFAULT_BUILTIN_IDLE_MINUTES`).
/// -1 means keep resident indefinitely. Positive values are minutes (1..=1440).
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

/// Upper bound on a model's context window that Thuki will trust and display
/// from external GGUF metadata (the `context_length` field of an arbitrary
/// Hugging Face repo, shown in the Browse-all listing). Defense-in-depth: the
/// field is attacker-controllable and editable (`gguf_set_metadata.py`), so a
/// value above this sane ceiling is treated as untrustworthy and dropped rather
/// than rendered. Mirrors the [`BOUNDS_NUM_CTX`] upper bound: 1 M tokens covers
/// every current model. Why not tunable: it bounds attacker-controlled data, a
/// security guard rather than a user preference.
pub const MAX_MODEL_CONTEXT_LENGTH: u32 = 1_048_576;

/// Accepted range for `keep_warm_inactivity_minutes`.
/// -1 = keep resident forever, 0 = provider's natural short default (~5 min),
/// 1..=1440 = explicit timeout. Values below -1 or above 1440 are clamped to
/// the compiled default.
pub const BOUNDS_KEEP_WARM_INACTIVITY_MINUTES: (i32, i32) = (-1, 1440);

/// The built-in engine's idle-unload timer (minutes) for the unified
/// `keep_warm_inactivity_minutes = 0` sentinel. The built-in engine has no
/// external daemon to defer to, so `0` ("use the provider's natural short
/// default") resolves to this fixed ~5-minute timer. Baked in, not tunable:
/// it is the fixed translation of one sentinel value, not a preference; users
/// who want a different timeout set `keep_warm_inactivity_minutes` directly
/// (`N` minutes or `-1` for forever). `warmup::builtin_idle_minutes` maps the
/// sentinel onto the runner's `idle_minutes` convention.
pub const DEFAULT_BUILTIN_IDLE_MINUTES: u32 = 5;

// Built-in engine lifecycle constants: baked in because they define the
// engine runner's startup and idle-check contract, not a user preference.

/// Wall-clock deadline (seconds) for a freshly spawned built-in engine to
/// pass its `/health` check before the spawn is declared failed. Large GGUF
/// models on a cold disk can take minutes to load, so the deadline is
/// generous. Not user-tunable: it bounds the worst-case "warming up" wait the
/// UI can present, so changing it alters the UX contract.
pub const ENGINE_HEALTH_DEADLINE_SECS: u64 = 300;

/// Interval (milliseconds) between `/health` probes while the built-in
/// engine starts up. Not user-tunable: pure loopback-load tuning; 250 ms
/// detects readiness promptly without hammering the local server while it is
/// busy loading the model.
pub const ENGINE_HEALTH_POLL_INTERVAL_MS: u64 = 250;

/// Timeout (seconds) for a single `/health` GET inside the poll loop. Bounds
/// a server that has accepted the TCP connection but stopped responding: a
/// wedged-but-connected server would otherwise park the poll loop indefinitely.
/// Loopback health probes are normally instant; 5 s is generous. Not
/// user-tunable: internal lifecycle contract between the runner and the engine
/// process; the poll interval and deadline are the user-facing knobs.
pub const ENGINE_HEALTH_PROBE_TIMEOUT_SECS: u64 = 5;

/// Interval (seconds) between idle-unload checks in the engine runner. Not
/// user-tunable: internal timer granularity behind the user-facing
/// `keep_warm_inactivity_minutes` knob; 30 s keeps the unload within a
/// minute-scale setting's precision at negligible cost.
pub const ENGINE_IDLE_CHECK_INTERVAL_SECS: u64 = 30;

/// Capacity of the engine runner command queue. Not user-tunable: bounds
/// memory under command bursts; 64 slots is ample for all UI-driven traffic
/// (Ensure, Touch, SetIdleMinutes, Shutdown) with no back-pressure under
/// normal use.
pub const ENGINE_COMMAND_QUEUE_CAPACITY: usize = 64;

/// Number of trailing `llama-server` stderr lines the runner retains so a
/// crash can report the engine's own reason (e.g. "unknown model
/// architecture") instead of a generic message. Not user-tunable:
/// defense-in-depth bound on subprocess output; 20 lines covers the final
/// load-error block llama.cpp prints without retaining its whole log.
pub const ENGINE_STDERR_TAIL_LINES: usize = 20;

/// Maximum bytes buffered (and retained) per captured engine stderr line. Not
/// user-tunable: defense-in-depth bound so one pathological newline-less line
/// (e.g. an enormous architecture string echoed from crafted GGUF metadata)
/// cannot force an unbounded read allocation; bytes past the cap are dropped.
pub const ENGINE_STDERR_TAIL_LINE_MAX_BYTES: usize = 500;

/// Reason reported when the built-in engine process exits without leaving any
/// stderr we could capture (e.g. an external SIGKILL). Not user-tunable:
/// internal diagnostic fallback surfaced only when the real reason is
/// unavailable.
pub const ENGINE_CRASH_FALLBACK_MESSAGE: &str = "engine process exited unexpectedly";

/// Minimum interval between Progress events emitted during a model download.
/// Bounds IPC channel traffic: a fast local connection can deliver thousands
/// of chunks per second and the UI only needs a few updates per second. Not
/// user-tunable: pure IPC hygiene, invisible below the UI refresh rate.
pub const DOWNLOAD_PROGRESS_MIN_INTERVAL_MS: u64 = 500;

/// Read-buffer size for streaming a downloaded blob through SHA-256 when the
/// hash cannot be computed live: a full-length partial already on disk, or
/// seeding the hasher with a resumed download's existing prefix. A few-MB
/// buffer turns a multi-GB read into a few hundred syscalls instead of hundreds
/// of thousands. Not user-tunable: an internal I/O buffer whose only effect is
/// verify speed.
pub const BLOB_HASH_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// Maximum accepted length of a single Server-Sent-Events line from a /v1
/// streaming response. Bounds attacker-controlled data from a chat server
/// (a malicious or broken server cannot grow a single line unboundedly).
pub const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;

/// Built-in secretary persona prompt. User overrides via `[prompt] system` in
/// the config file. The slash-command appendix is composed on top at load time
/// and is never written back to the file.
pub const DEFAULT_SYSTEM_PROMPT_BASE: &str = include_str!("../../prompts/system_prompt.txt");

/// Generated appendix listing supported slash commands. Composed on top of
/// the user-editable base prompt at load time so built-in command knowledge
/// stays in sync with the registry even when the persona prompt is overridden.
pub const SLASH_COMMAND_PROMPT_APPENDIX: &str =
    include_str!("../../prompts/generated/slash_commands.txt");

/// Whether the user has explicitly saved a system prompt via Settings. Starts
/// `false`, which marks the persisted `system` as a non-authoritative cached
/// default: the loader refreshes it to `DEFAULT_SYSTEM_PROMPT_BASE` on every
/// load (healing old configs where `system = ""` and propagating later prompt
/// edits) until the user saves through the Settings UI and flips this to `true`.
pub const DEFAULT_SYSTEM_CUSTOMIZED: bool = false;

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
/// Base font size (in CSS pixels) for chat text and the AskBar input.
/// Drives the `--thuki-text-base` CSS variable on `<html>`, which the AI
/// markdown body, the user chat bubble text, and the AskBar textarea +
/// caret-tracking mirror all read. Other surfaces (Settings panel,
/// onboarding) keep fixed sizes. Raise for easier-to-read conversation
/// text; lower to fit more text on screen.
pub const DEFAULT_TEXT_BASE_PX: f64 = 15.0;

/// Line-height multiplier applied to chat + AskBar text. Drives the
/// `--thuki-text-line-height` CSS variable. 1.5 sits between the AskBar
/// default (~1.25) and the previous AI-prose default (1.6); users can dial
/// up for airier prose or down for denser screens.
pub const DEFAULT_TEXT_LINE_HEIGHT: f64 = 1.5;

/// Letter spacing applied to chat + AskBar text, in CSS pixels. Drives the
/// `--thuki-text-letter-spacing` CSS variable. 0 keeps Nunito's native
/// tracking; raise for airier characters, drop below zero to tighten.
pub const DEFAULT_TEXT_LETTER_SPACING_PX: f64 = 0.0;

/// Numeric CSS `font-weight` applied to chat + AskBar text. Drives the
/// `--thuki-text-font-weight` CSS variable. Only the four loaded Nunito
/// weights are accepted; intermediate values would silently fall back to
/// the nearest loaded glyph set, making the slider misleading.
pub const DEFAULT_TEXT_FONT_WEIGHT: u32 = 500;
pub const ALLOWED_FONT_WEIGHTS: &[u32] = &[400, 500, 600, 700];

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
/// Accepted range for `window.text_base_px`. 11 px is the floor for legibility
/// on a retina panel; 22 px is the ceiling before line wrapping in the AskBar
/// stops looking right at the default overlay width. Values outside the range,
/// or non-finite values, are reset to `DEFAULT_TEXT_BASE_PX` by the loader.
pub const BOUNDS_TEXT_BASE_PX: (f64, f64) = (11.0, 22.0);

/// Accepted range for `window.text_line_height` (unitless CSS multiplier).
/// 1.0 collapses lines to glyph height (legibility floor); 2.5 is well past
/// any reasonable airy-prose setting.
pub const BOUNDS_TEXT_LINE_HEIGHT: (f64, f64) = (1.0, 2.5);

/// Accepted range for `window.text_letter_spacing_px` (CSS pixels). Negative
/// values tighten the typography; positive values airy it out.
pub const BOUNDS_TEXT_LETTER_SPACING_PX: (f64, f64) = (-0.5, 2.0);
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
/// Maximum tokens the sufficiency judge can generate per call. Larger than
/// ROUTER_MAX_TOKENS because thinking-capable models spend internal tokens on
/// chain-of-thought before emitting JSON content; 512 exhausts the budget on
/// thinking and leaves nothing for the JSON output, causing a parse failure
/// and a synthetic-partial fallback. 2048 gives headroom for ~1500 thinking
/// tokens plus ~200 JSON tokens. Not user-tunable: changing this value alters
/// the parse-success rate (a quality property), not just latency.
pub const JUDGE_MAX_TOKENS: i32 = 2048;
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

/// Whether the unified trace recorder writes forensic per-conversation
/// trace files for the chat layer AND the `/search` pipeline.
///
/// Off by default. Intended for local quality investigation only: when on,
/// the recorder writes every chat turn (user message, assistant streaming
/// tokens, screen captures, conversation lifecycle) AND every search-pipeline
/// step (LLM requests/responses, SearXNG queries, reader batches, judge
/// verdicts) to JSON-Lines files under
/// `~/Library/Application Support/com.quietnode.thuki/traces/`. Files are
/// grouped by domain (`traces/chat/<conversation_id>.jsonl` and
/// `traces/search/<conversation_id>.jsonl`) so an analysis agent can be
/// pointed at exactly the slice it cares about. Toggleable from the
/// Settings panel (Web tab, Diagnostics section). Off in shipped builds
/// by default.
pub const DEFAULT_DEBUG_TRACE_ENABLED: bool = false;

/// Whether `/rewrite` and `/refine` results are written straight back into the
/// source app (replacing the selection) the moment the model finishes,
/// without the user clicking the in-chat Replace button.
///
/// Off by default: auto-replace mutates text in another app, so the
/// conservative default is to require an explicit click. When on, the Replace
/// button still renders as a manual re-trigger. Toggleable from the Settings
/// panel (Behavior tab).
pub const DEFAULT_AUTO_REPLACE: bool = false;

/// When `true`, the Thuki overlay dismisses itself immediately after a
/// `/rewrite` or `/refine` result is replaced back into the source app, whether
/// the replace was automatic (see [`DEFAULT_AUTO_REPLACE`]) or a manual Replace
/// click. Only closes on a *successful* replace; a skipped write (no target /
/// secure field) leaves the overlay open.
///
/// Off by default. Independent of auto-replace: usable with either trigger.
/// Toggleable from the Settings panel (Behavior tab).
pub const DEFAULT_AUTO_CLOSE: bool = false;

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

/// Maximum accepted body size for Hugging Face API responses (repo file
/// listings). Bounds attacker-controlled data from a remote service,
/// mirroring MAX_OLLAMA_TAGS_BODY_BYTES.
pub const MAX_HF_API_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Per-request timeout (seconds) for Hugging Face API metadata calls.
pub const HF_API_TIMEOUT_SECS: u64 = 15;

/// Per-request timeout (seconds) for an OpenAI-compatible server's
/// `/v1/models` listing. Tighter than the Hugging Face timeout because the
/// server is local or LAN-hosted in the common case and the Settings model
/// dropdown blocks on this probe.
pub const OPENAI_MODELS_TIMEOUT_SECS: u64 = 5;

/// Canonical Hugging Face origin used for both model metadata calls and blob
/// downloads. Not user-tunable: the sha256-pinning + provenance model assumes
/// the canonical Hub; pointing downloads at an arbitrary mirror would bypass
/// the integrity guarantees that make the curated starter registry safe.
pub const HF_BASE_URL: &str = "https://huggingface.co";

/// Page size for the in-app Hugging Face GGUF model search. The Discover
/// "Load more" control raises the requested limit in multiples of this value.
/// Baked-in: the per-page step for the browser, not a user preference.
pub const HF_SEARCH_LIMIT: usize = 30;

/// Hard cap on a single Hugging Face search request's page size. "Load more"
/// grows the requested limit in [`HF_SEARCH_LIMIT`] steps; this bounds the
/// largest single request so a runaway page count cannot ask the Hub for an
/// unbounded result set. Baked-in: defense-in-depth bound on request size.
pub const HF_SEARCH_LIMIT_MAX: usize = 120;

/// Approximate resident-memory overhead in GiB added on top of a model's
/// weights size when estimating whether it fits in this Mac's RAM (the KV
/// cache at the default context plus runtime buffers). Baked-in: feeds the
/// RAM-fit *hint* in Library/Discover only; the authoritative per-starter
/// estimates live in the model registry.
pub const RUNTIME_OVERHEAD_GB: f64 = 2.0;

/// Maximum accepted byte length for a Hugging Face search query before it is
/// sent upstream. Defense-in-depth bound on attacker-influenced input: the
/// query reaches the fixed Hub host (no SSRF) and is percent-encoded by the
/// client, but an unbounded string is still rejected to cap request size.
pub const MAX_HF_SEARCH_QUERY_LEN: usize = 200;

/// Maximum accepted byte length for a model slug passed to `set_active_model`.
/// Real Ollama slugs are a handful of characters; 256 is generous while still
/// capping adversarial inputs long before any network or database work.
pub const MAX_MODEL_SLUG_LEN: usize = 256;

/// Maximum metadata key-value pairs the GGUF reader will scan before giving
/// up. Real GGUF models carry a few dozen KV entries; 4096 never truncates a
/// legitimate header while bounding a malformed `metadata_kv_count` so the
/// reasoning-classifier scan cannot loop on a corrupt or hostile file.
pub const MAX_GGUF_KV_COUNT: u64 = 4096;

/// Maximum accepted byte length for a single GGUF metadata key. Keys are short
/// dotted identifiers (`tokenizer.chat_template`); 1 KiB is far above any real
/// key and stops a corrupt length field from forcing a huge allocation.
pub const MAX_GGUF_KEY_BYTES: u64 = 1024;

/// Maximum accepted byte length for a GGUF string value the reader actually
/// materializes (the chat template and architecture). Real chat templates run
/// a few KB to ~100 KB; 4 MiB never truncates one while bounding the memory a
/// corrupt or hostile length field can demand.
pub const MAX_GGUF_STRING_BYTES: u64 = 4 * 1024 * 1024;

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
    // [inference] — active_provider and the providers array are not flat fields;
    // they are written via set_active_model / set_ollama_url, not set_config_field.
    ("inference", "keep_warm_inactivity_minutes"),
    ("inference", "num_ctx"),
    // [prompt]
    ("prompt", "system"),
    // [window]
    ("window", "overlay_width"),
    ("window", "max_chat_height"),
    ("window", "max_images"),
    ("window", "text_base_px"),
    ("window", "text_line_height"),
    ("window", "text_letter_spacing_px"),
    ("window", "text_font_weight"),
    // [quote]
    ("quote", "max_display_lines"),
    ("quote", "max_display_chars"),
    ("quote", "max_context_length"),
    // [behavior]
    ("behavior", "auto_replace"),
    ("behavior", "auto_close"),
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
    ("search", "pipeline_wall_clock_budget_s"),
    // [debug]
    ("debug", "trace_enabled"),
    // [updater]
    ("updater", "auto_check"),
    ("updater", "check_interval_hours"),
    ("updater", "manifest_url"),
];

/// Authoritative allowlist of section names accepted by `reset_config`.
/// Mirrors the top-level structure of `AppConfig`.
pub const ALLOWED_SECTIONS: &[&str] = &[
    "inference",
    "prompt",
    "window",
    "quote",
    "behavior",
    "search",
    "debug",
    "updater",
];

// Updater
/// Whether Thuki polls for new releases automatically at startup and periodically.
pub const DEFAULT_UPDATER_AUTO_CHECK: bool = true;
/// Hours between automatic background update checks. Bound to 1..168 (one week).
pub const DEFAULT_UPDATER_CHECK_INTERVAL_HOURS: u64 = 24;
/// Accepted range for `check_interval_hours`. 1 h minimum keeps checks meaningful;
/// 168 h (one week) is the practical ceiling for a desktop update poller.
pub const BOUNDS_UPDATER_CHECK_INTERVAL_HOURS: (u64, u64) = (1, 168);
/// URL of the Tauri updater JSON manifest. Points to the latest GitHub release asset.
pub const DEFAULT_UPDATER_MANIFEST_URL: &str =
    "https://github.com/quiet-node/thuki/releases/latest/download/latest.json";

// Email capture
/// Public proxy endpoint that the optional "Help shape Thuki" email ask POSTs to.
/// The proxy holds the email-service key; Thuki sends only `{ email, source }`
/// and never sees a secret. Not user-tunable: it is a fixed external-service
/// endpoint, not a knob, and pointing it elsewhere would silently break the
/// subscribe flow.
pub const DEFAULT_SUBSCRIBE_ENDPOINT: &str = "https://thuki.app/api/subscribe";
/// Per-request timeout for the optional email-subscribe POST, in seconds. Caps
/// how long the "Help shape Thuki" button can sit in its sending state if the
/// proxy stalls without responding, so the request fails into the generic
/// retryable error instead of hanging indefinitely. Not user-tunable: it is an
/// internal robustness bound on a one-shot network call, not a preference.
pub const DEFAULT_SUBSCRIBE_TIMEOUT_SECS: u64 = 15;
/// Filename of the JSON sidecar that persists snooze deadlines across restarts.
/// Lives next to `config.toml` in `app_config_dir`. Single source of truth so
/// the writer (commands.rs) and the loader (lib.rs) cannot drift.
pub const DEFAULT_UPDATER_STATE_FILENAME: &str = "updater_state.json";
/// Defense-in-depth upper bound on snooze duration accepted from the frontend
/// IPC boundary (in hours). One year is far longer than any UI-driven snooze
/// the app exposes today, but small enough that `hours * 3600` cannot overflow
/// `u64` even when added to a future Unix timestamp. Saturating arithmetic in
/// the command handlers makes this defensive rather than load-bearing.
pub const MAX_UPDATER_SNOOZE_HOURS: u64 = 8760;

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
