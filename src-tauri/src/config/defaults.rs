//! Compiled default values for the application configuration.
//!
//! This is the ONE place where Thuki's default configuration lives. Every
//! other subsystem reads the resolved values from `AppConfig` via Tauri state.
//! Changing a default here propagates to a fresh first-run config file and to
//! any field a user has left unset or left empty in their existing file.

use std::ops::RangeInclusive;

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

/// Timeout (seconds) bounding the built-in engine shutdown the `sigwait` thread
/// runs on a polite stop (Ctrl+C or the SIGTERM macOS sends every app at restart,
/// issue #296). The durable clean-exit write always lands first; this only bounds
/// the sidecar kill, so a wedged engine can never keep the thread from re-raising
/// the caught signal and leave the app unresponsive past the macOS restart
/// deadline. A sidecar that outlives the bound is reaped at the next launch. Not
/// user-tunable: an internal shutdown-path safety valve, and the kill is normally
/// near-instant (the child dies on an unblockable SIGKILL).
pub const SHUTDOWN_SIGNAL_ENGINE_KILL_TIMEOUT_SECS: u64 = 3;

/// Grace period (milliseconds) between the polite `SIGTERM` and the escalating
/// `SIGKILL` the startup reaper sends an orphaned `llama-server` left behind by
/// a previous Thuki (issue #296). `llama-server` exits promptly on `SIGTERM`, so
/// this is only the window a survivor gets before it is force-killed; the full
/// orphan predicate is re-checked after the wait so a recycled pid is never
/// killed. Not user-tunable: an internal shutdown-hygiene timing constant.
pub const ORPHAN_REAP_SIGTERM_GRACE_MS: u64 = 2000;

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

/// Host-memory prompt-cache RAM bound (MiB) for the bundled llama-server
/// (`-cram` / `--cache-ram`). Upstream defaults to 8192 MiB, which competes
/// with Metal model weights on a 24GB unified-memory host (gpt-oss-20b alone
/// is ~11GB) and has a documented Metal-OOM failure mode that poisons the
/// backend until process restart. Two short system prefixes (classifier +
/// chat) plus a conversation prefix fit in a few hundred MB; 512 MiB is
/// generous headroom without unbounded cache growth. Raise only after live
/// cache-entry sizes are measured.
///
/// Not user-tunable: an engine spawn constant sized for the ship hardware
/// envelope, not a quality knob.
pub const LLAMA_SERVER_CACHE_RAM_MIB: u32 = 512;

/// Decode slot count for the bundled llama-server (`--parallel`). Always 1:
/// Thuki is single-user, multi-slot splits ctx and historically caused cold
/// first-turn prefill when warm-up and the user message landed on different
/// slots. Do not raise without a new latency package and memory budget.
///
/// Not user-tunable: architectural constant of the single-slot design.
pub const LLAMA_SERVER_PARALLEL_SLOTS: u32 = 1;

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

/// Maximum number of model downloads allowed to transfer bytes at the same
/// time. A start beyond this waits (queued) for a slot before opening its HTTP
/// transfer. Not user-tunable: a defense-in-depth bound against the
/// resource-exhaustion this closes (issue #296, where unbounded parallel
/// downloads plus an auto-load froze a memory-constrained Mac). Exposing it
/// would let a user re-introduce the very failure the cap exists to prevent.
pub const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 3;

/// Free-space headroom kept above a download's own byte needs, both in the
/// pre-download preflight and the periodic mid-transfer re-check. 2 GiB leaves
/// room for the OS, the app, and other writers so filling the volume to the
/// brim during a multi-GB model pull cannot wedge the machine. Not
/// user-tunable: a defense-in-depth floor against the disk-fill failure mode of
/// issue #296, not a preference; lowering it would re-open that failure.
pub const DEFAULT_DOWNLOAD_DISK_HEADROOM_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// How many bytes a transfer writes between successive free-disk re-checks. A
/// long download can fill the volume long after the preflight passed (other
/// apps writing, a second model downloading), so free space is re-probed every
/// this many bytes and the transfer aborts cleanly (keeping its `.partial` for
/// resume) if it falls below the headroom floor. 256 MiB bounds the statfs call
/// rate to a handful per GB while still catching a fill within a few hundred MB.
/// Not user-tunable: internal safety-probe cadence with no user-visible effect.
pub const DEFAULT_DOWNLOAD_DISK_RECHECK_INTERVAL_BYTES: u64 = 256 * 1024 * 1024;

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

/// Interval between background polls of Ollama `/api/ps` for external VRAM
/// changes (user-initiated `ollama stop`, TTL expiry, daemon restart). Not
/// user-tunable: tuning this trades responsiveness against localhost load but
/// the 5 s value is already generous for a loopback call.
pub const VRAM_POLL_INTERVAL_SECS: u64 = 5;

/// Whether the unified trace recorder writes forensic per-conversation
/// trace files for the chat layer (which includes the built-in web-search
/// turns that the `/search` command and the auto-search pre-pass drive).
///
/// Off by default. Intended for local quality investigation only: when on,
/// the recorder writes every chat turn (user message, assistant streaming
/// tokens + final answer body, screen captures, conversation lifecycle, and
/// the search skip/decision/retrieval/escalation/requery/citation-audit
/// records the built-in search emits) to JSON-Lines files under
/// `~/Library/Application Support/com.quietnode.thuki/traces/chat/<conversation_id>.jsonl`.
/// Toggleable from the Settings panel (Diagnostics). Off by default.
pub const DEFAULT_DEBUG_TRACE_ENABLED: bool = false;

/// How many days trace files are retained before the startup / on-change prune
/// deletes them. Default 7 days: long enough to revisit a recent investigation,
/// short enough that forensic files carrying sensitive text do not accumulate
/// on disk indefinitely.
///
/// The sentinel [`TRACE_RETENTION_FOREVER`] (`-1`) disables pruning entirely.
/// Any positive value is bounded by [`BOUNDS_TRACE_RETENTION_DAYS`]. Signed so
/// the `-1` sentinel is representable. Toggleable from the Settings panel
/// (Diagnostics).
pub const DEFAULT_TRACE_RETENTION_DAYS: i64 = 7;

/// Accepted positive range for `trace_retention_days` (in days). One day floor,
/// ten-year ceiling. The [`TRACE_RETENTION_FOREVER`] sentinel is handled
/// separately and is intentionally outside this range; every other value
/// outside it (including `0`) resets to [`DEFAULT_TRACE_RETENTION_DAYS`].
pub const BOUNDS_TRACE_RETENTION_DAYS: (i64, i64) = (1, 3650);

/// Sentinel `trace_retention_days` value meaning "keep trace files forever"
/// (never prune). Kept as a named constant so the loader clamp and the prune
/// caller agree on the one magic number rather than both hardcoding `-1`.
pub const TRACE_RETENTION_FOREVER: i64 = -1;

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

/// When `true` (default), the built-in engine may open the web on a plain turn
/// when the classifier decides live facts are needed. When `false`, plain
/// turns stay local-only and only an explicit `/search` (`force_search`) runs
/// the web pipeline. Independent of auto-replace / auto-close. Toggleable from
/// Settings › Behavior.
pub const DEFAULT_AUTO_SEARCH: bool = true;

/// When `false` (default), the ask bar shows a non-blocking notice explaining
/// that queries leave the device for search services. Not tied to a search
/// turn: it shows until acknowledged. Set `true` after the user taps "Got it"
/// (or equivalent) so the card never returns. Independent of `auto_search`.
pub const DEFAULT_SEARCH_NOTICE_ACKNOWLEDGED: bool = false;

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

/// Upper bound on the declared part count of a multi-part (split) GGUF model,
/// i.e. the `MMMMM` field of a `<prefix>-NNNNN-of-MMMMM.gguf` shard name. The
/// 5-digit `gguf-split` format technically permits up to 99999 parts; this
/// defense-in-depth cap rejects an absurd or hostile count from an untrusted
/// Hugging Face listing before it drives any sibling-completeness check. Real
/// models split into at most a few dozen parts, so the bound is never reached in
/// practice.
pub const MAX_SPLIT_PARTS: u32 = 999;

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

/// Fraction of live available memory a model's estimated footprint may occupy
/// and still be judged to fit with healthy headroom. At or below this it is
/// "comfortable"; above it (but at or below the ceiling) it is "tight". Feeds
/// the pre-load memory gate (issue #296) and the `estimate_model_fit` command.
///
/// Not user-tunable: a defense-in-depth threshold against the auto-load freeze
/// of issue #296, not a preference. The footprint estimate is deliberately
/// approximate (weights plus a fixed overhead), so the gate is forgiving:
/// crossing this fraction only softens the verdict to "tight", never blocks.
pub const MODEL_FIT_COMFORT_FRACTION: f64 = 0.60;

/// Ceiling fraction of live available memory a model's estimated footprint may
/// occupy before the pre-load gate refuses an un-forced load. Modeled on Jan's
/// published guidance that a model should stay under ~80% of available memory,
/// leaving the remaining fifth for the OS and other apps so a load cannot wedge
/// the machine (issue #296).
///
/// Not user-tunable: a defense-in-depth OOM bound, not a preference. Because
/// the footprint estimate can be off by up to ~2x, this hard block triggers
/// only clearly above the ceiling and a user `force` always bypasses it.
pub const MODEL_FIT_CEILING_FRACTION: f64 = 0.80;

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
    ("behavior", "auto_search"),
    ("behavior", "search_notice_acknowledged"),
    // [debug]
    ("debug", "trace_enabled"),
    ("debug", "trace_retention_days"),
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
/// Filename of the JSON session record the launch circuit breaker durably
/// writes at startup (with `clean_exit: false`) and flips to `clean_exit: true`
/// only on a real quit. Lives next to `config.toml` in `app_config_dir`. Single
/// source of truth so the reader and the writer in `startup_guard` cannot
/// drift. See `src-tauri/src/startup_guard.rs`.
pub const DEFAULT_SESSION_RECORD_FILENAME: &str = "session.json";
/// Filename of the empty advisory-lock file the launch circuit breaker holds
/// for the whole process lifetime. The kernel releases the lock on process
/// death by ANY cause, which is how crash detection works without a clean-exit
/// signal. Lives next to `config.toml` in `app_config_dir`. See
/// `src-tauri/src/startup_guard.rs`.
pub const DEFAULT_SESSION_LOCK_FILENAME: &str = "session.lock";
/// Number of consecutive abnormal launches that trips the startup circuit
/// breaker into safe mode. A launch is "abnormal" when the previous session's
/// record was still `clean_exit: false` at this launch, meaning the previous
/// process died without a clean exit (freeze, SIGKILL, OS OOM-kill, panic, or
/// power loss): the crash-loop signature from issue #296. Threshold 2 = enter
/// safe mode only after TWO consecutive abnormal sessions, so a single hard
/// reboot or `kill -9` does not nag the user, mirroring Firefox's
/// `toolkit.startup.max_resumed_crashes` pattern. Not user-tunable: it is a
/// crash-loop safety mechanism, not a preference, and letting a user raise it
/// would re-arm the freeze this guard exists to break.
pub const DEFAULT_STARTUP_SAFE_MODE_THRESHOLD: u32 = 2;
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

// ─── SSRF-safe HTTP transport ────────────────────────────────────────────────

/// Hard cap on the decompressed body Thuki reads from any single outbound
/// request in the web-search stack (bytes). The transport streams the response
/// and aborts once this many bytes have accumulated, so a hostile server (or a
/// gzip bomb, since the cap counts post-decompression bytes) cannot exhaust
/// memory. 4 MiB is far above any real HTML page or vertical-API JSON payload.
///
/// Not user-tunable: a defense-in-depth bound on attacker-controlled response
/// size, not a latency or quality knob.
pub const MAX_HTTP_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Maximum number of redirect hops the transport follows before failing the
/// request. Every hop is re-screened by the SSRF guard, so this is a
/// belt-and-suspenders bound on redirect loops and redirect-based latency, not
/// the security boundary itself. 5 covers legitimate chains (e.g. Google News
/// RSS opaque-token redirects) with margin.
///
/// Not user-tunable: a protocol-hardening bound on attacker-controlled
/// redirect chains.
pub const MAX_HTTP_REDIRECTS: usize = 5;

/// Backstop wall-clock timeout for a single outbound request (seconds). This
/// is a coarse safety net so a stuck connection cannot hang forever; callers
/// that need a tighter per-request or whole-fan-out deadline (e.g. the page
/// fetcher's ~6 s global budget) impose it themselves with `tokio::time`.
///
/// Not user-tunable: an internal robustness bound, not a user-facing knob.
pub const HTTP_REQUEST_TIMEOUT_S: u64 = 15;

/// Connection-establishment timeout for a single outbound request (seconds).
/// Tighter than the overall request timeout so an unreachable host fails fast
/// during engine rotation instead of stalling the whole turn.
///
/// Not user-tunable: an internal robustness bound.
pub const HTTP_CONNECT_TIMEOUT_S: u64 = 8;

// ─── Web-search decision (pre-filter + classifier) ───────────────────────────

/// Maximum number of leading characters of the user's message the deterministic
/// search pre-filter scans for its keyword and phrase signals. The request text
/// that carries a temporal or freshness signal is short and lives at the front;
/// a signal buried deep inside a large pasted document is better resolved by the
/// classifier than force-matched here. Bounding the scan keeps the pre-filter's
/// tokenisation strictly linear in a small constant, so a multi-megabyte pasted
/// message cannot turn the per-turn decision into a CPU-bound denial-of-service.
///
/// Not user-tunable: a defense-in-depth bound on attacker-controlled input size,
/// not a quality knob.
pub const PREFILTER_MAX_SCAN_CHARS: usize = 4096;

/// Maximum number of most-recent conversation turns the persona-free classifier
/// embeds as context when rewriting a follow-up into a standalone question. Only
/// a few turns are needed to resolve pronouns ("what about there?"); embedding
/// the whole history would bloat the classifier prompt and slow the warm-slot
/// decision without improving disambiguation.
///
/// Not user-tunable: a classifier-prompt shape constant.
pub const CLASSIFIER_HISTORY_TURNS: usize = 4;

/// Maximum number of leading characters of each embedded *assistant* answer the
/// classifier sees when rewriting a follow-up into a standalone question. The
/// referent for an elliptical follow-up ("how about him?", "what about X?")
/// usually lives in the assistant's previous answer, not the user's question, so
/// the answers must be embedded; but a full answer can run to hundreds of tokens
/// and would blow the warm-slot classifier budget if embedded whole. The opening
/// sentences carry the named entities that resolve the reference, so a bounded
/// prefix is enough. User turns are embedded whole (they are short questions).
///
/// Not user-tunable: a classifier-prompt shape constant, same rationale as
/// [`CLASSIFIER_HISTORY_TURNS`].
pub const CLASSIFIER_ASSISTANT_PREFIX_CHARS: usize = 300;

/// Token cap for the grammar-constrained classifier response. The JSON itself is
/// tiny (~60 tokens), but reasoning-family models (e.g. gpt-oss) spend internal
/// tokens on chain-of-thought before emitting the JSON, and ignore the
/// `enable_thinking:false` hint the structured-output path sets. If the budget is
/// too small the reasoning exhausts it before any JSON is produced, yielding an
/// empty body that degrades to a `no` decision: silent under-searching, the exact
/// failure the two-stage trigger exists to prevent.
///
/// The persona-free classifier prompt is a richer task (a few-shot classification
/// header) than the old single-line pre-pass instruction, and a richer prompt
/// draws *more* reasoning from these models, so the budget carries generous
/// headroom over the ~500 reasoning tokens measured on the older prompt. The cap
/// only bites when reasoning would otherwise truncate the JSON; the real
/// wall-clock guard is [`PREPASS_TIMEOUT_S`], so erring high costs nothing on
/// normal turns.
///
/// Not user-tunable: part of the classifier prompt/parse contract, not a latency
/// or quality knob the user should tune.
pub const PREPASS_MAX_TOKENS: i32 = 1536;

/// Per-call wall-clock timeout for the classifier call (seconds). Sized to fit
/// [`PREPASS_MAX_TOKENS`] at the observed ~60 tok/s decode of the largest
/// bundled model plus prefill headroom: a reasoning-family model that ignores
/// the thinking-off hint can legitimately need most of the token budget, and a
/// timeout tighter than the budget silently converts those calls into failed
/// decisions (observed live on gpt-oss-20b at 20 s). The `Reasoning: low`
/// directive in the classifier prompt keeps typical calls far below this;
/// exceeding it means the engine is wedged, and the caller degrades rather
/// than stalling.
///
/// Not user-tunable: an internal robustness bound.
pub const PREPASS_TIMEOUT_S: u64 = 35;

/// TTL (seconds) for the multi-turn source cache: how long the sources of the
/// most recent successful search stay reusable for a `cached` classifier
/// decision (a follow-up that repeats or rephrases the question just
/// answered) before a later turn falls back to a fresh search. 10 minutes
/// covers the realistic follow-up window without risking a stale answer on a
/// slow-moving conversation. The cache holds at most one entry (the most
/// recent search only, replaced whole by every new one), so this TTL is its
/// only expiry mechanism.
///
/// Not user-tunable: an internal robustness bound, the same rationale as
/// [`PREPASS_TIMEOUT_S`].
pub const SEARCH_CACHE_TTL_S: u64 = 600;

/// Token cap for the grammar-constrained sufficiency-judge response. The judge
/// decides whether a retrieved vertical block actually answers the specific
/// question before the pipeline commits to it, so an insufficient fast-path
/// result escalates to the scraped engines instead of dead-ending on a "the
/// sources do not contain that" refusal.
///
/// Matched to [`PREPASS_MAX_TOKENS`], NOT sized down on the theory that a yes/no
/// is a lighter task than the classifier's three-way route. Reasoning-family
/// models (gpt-oss) spend internal tokens on chain-of-thought before the JSON
/// and IGNORE the `enable_thinking:false` hint, and their reasoning volume
/// tracks the model's effort on the task, not the prompt's length: "do these
/// sources contain X" is not obviously less reasoning than a route decision.
/// This budget went the same 768 -> 1536 route the classifier's did after a
/// truncating body was observed live degrading to an empty parse. Here that same
/// truncation is MORE dangerous, not less: an empty judge body degrades to
/// "sufficient" (commit the block), silently restoring the exact dead-end this
/// stage exists to remove, and it only surfaces on gpt-oss (gemma emits JSON
/// with no reasoning tokens and never truncates, so a gemma smoke would show a
/// false green). The cap only bites on truncation; the real wall-clock guard is
/// [`SUFFICIENCY_JUDGE_TIMEOUT_S`], so erring high costs nothing on normal turns.
///
/// Not user-tunable: part of the judge prompt/parse contract, same rationale as
/// [`PREPASS_MAX_TOKENS`].
pub const SUFFICIENCY_JUDGE_MAX_TOKENS: i32 = 1536;

/// Per-call wall-clock timeout for the sufficiency-judge call (seconds). Matched
/// to [`PREPASS_TIMEOUT_S`] to fit [`SUFFICIENCY_JUDGE_MAX_TOKENS`] at the
/// observed decode rate plus prefill: a timeout tighter than the token budget
/// silently converts a reasoning-heavy judge call into a failure. A judge
/// failure degrades to "sufficient" (commit the fast-path block), so an
/// over-tight timeout would only ever suppress an escalation, never wall the
/// user; the `Reasoning: low` directive in the judge prompt keeps typical calls
/// well under this.
///
/// Not user-tunable: an internal robustness bound.
pub const SUFFICIENCY_JUDGE_TIMEOUT_S: u64 = 35;

/// Freshness markers that disqualify a question from the Wikipedia vertical even
/// when the classifier routed it there. Wikipedia's lead summary describes the
/// stable subject, not its live state, so a question carrying any of these words
/// (a volatile "latest/current status of X" phrasing) must never be answered
/// from a static encyclopedia extract; it falls through to the news / engine
/// tiers instead. Matched as whole tokens of the lowercased standalone question.
///
/// `anniversary` covers the age/biography class documented on
/// [`WIKI_VOLATILITY_PHRASES`]: "X years since" a fixed past date is a duration
/// that changes every year, so it needs the same fresh grounding as an explicit
/// "latest"/"current" question.
///
/// Not user-tunable: a prompt/routing contract guarding a model-routed decision
/// against a known non-answer failure mode, not a quality knob.
pub const WIKI_VOLATILITY_MARKERS: &[&str] = &[
    "latest",
    "current",
    "status",
    "today",
    "recent",
    "upcoming",
    "anniversary",
    // Non-English single-token "today/now" forms. Multi-word forms live in
    // [`WIKI_VOLATILITY_PHRASES`] (e.g. "hôm nay", "aujourd hui"). Without
    // these, non-English "today" questions never arm DDG date bias or recency
    // fusion (2026-07-14 gold-price smoke: VI "hôm nay" was invisible).
    "heute",
    "hoje",
    "hoy",
    "oggi",
    "vandaag",
    "сегодня",
    "今日",
    "今天",
    "오늘",
    "วันนี้",
];

/// Multi-word freshness phrases that disqualify the Wikipedia vertical, matched
/// as whole phrases of the lowercased standalone question. Split out from
/// [`WIKI_VOLATILITY_MARKERS`] because they span a word boundary.
///
/// `"how old is"`, `"what age is"`, and `"s age"` (the tokenised form of the
/// possessive `"'s age"`, since tokenisation splits on the apostrophe) are the
/// age/biography class: a present-tense age question ("how old is Tom Cruise")
/// is computed from the subject's birth date and the CURRENT date, so it is a
/// duration that changes every year exactly like an explicit "latest"/"current"
/// question — the live-smoke regression this addition fixes (2026-07-11:
/// Tom Cruise's age answered stale/wrong because no freshness signal fired).
/// `"how long ago"` is the same duration-from-a-fixed-past-date shape.
///
/// Two related phrasings are deliberately NOT included, each for a reason
/// specific to this guard (not a general safety net):
/// - **Past-tense age** ("how old WAS Napoleon when he died", "what age WAS
///   Einstein") names a duration between two fixed historical dates, which
///   never changes; flagging it would wrongly disqualify the Wikipedia vertical
///   for a question it answers well (see the `historical_attribute` rows in
///   `search_decision_eval.jsonl`, which exist to keep exactly these turns wiki-
///   eligible).
/// - **Birth date itself** ("when was Einstein born") names a fixed historical
///   date with no yearly-changing component, so it carries no freshness need;
///   Wikipedia's lead paragraph is the best source for it and should stay
///   eligible.
///
/// A bare `"age of"` is also deliberately excluded rather than added: idiomatic
/// English overwhelmingly uses it for eternal/historical-era facts ("age of the
/// universe", "Age of Enlightenment", "age of consent"), not living people, so it
/// would trigger far more over-matches than the "how old is" pattern it would be
/// meant to catch.
///
/// This module accepts one known over-match without a guard: `"how old is"`
/// still fires on an eternal-fact subject ("how old is the universe/Earth/the
/// pyramids"). No cheap deterministic check tells "a person" from "an era"
/// apart, and building real subject detection is out of scope for a keyword
/// guard. The cost of firing anyway is bounded and never wrong: it only adds a
/// mild recency bias to the engine tier (see `DDG_FRESHNESS_DF_VALUE`,
/// `NEWS_FRESHNESS_OPERATOR`, `RECENCY_ALPHA`) and, when the classifier had
/// routed to `wiki`, sends the turn to the engines instead of the static
/// summary — never an incorrect answer, at most a slightly less direct one.
///
/// Not user-tunable: same routing-contract rationale as the single-word markers.
pub const WIKI_VOLATILITY_PHRASES: &[&str] = &[
    "right now",
    "this year",
    "how old is",
    "what age is",
    "s age",
    "how long ago",
    // Multilingual "today / right now / latest" (token-joined phrases).
    "hôm nay",
    "mới nhất",
    "hiện nay",
    "bây giờ",
    "hari ini",
    "maintenant",
    "ahora mismo",
    // "aujourd'hui" splits on the apostrophe into these two tokens.
    "aujourd hui",
];

/// Tokens that mark a **price / market-quote** question. Any one present forces
/// the same freshness path as [`WIKI_VOLATILITY_MARKERS`] (DDG date bias,
/// recency fusion) and enables the price-intent evidence filters (numeric
/// utility, stale path-year drop). Without this, "giá vàng" with no "today"
/// word would skip temporal ranking and lose to number-bearing SEO scrapes.
///
/// Not user-tunable: a routing/evidence-contract list, same rationale as the
/// volatility markers.
pub const PRICE_INTENT_MARKERS: &[&str] = &[
    "price",
    "prices",
    "pricing",
    "cost",
    "costs",
    "rate",
    "rates",
    "quote",
    "spot",
    "ticker",
    // Vietnamese
    "giá",
    // Other supported-lang common forms (single token after lowercasing).
    "precio",
    "precios",
    "prix",
    "preis",
    "preço",
    "preco",
    "prezzo",
    "цена",
    "价格",
    "價錢",
    "价钱",
    "値段",
    "価格",
    "가격",
    "ราคา",
    "harga",
];

/// Path-year lag for freshness-gated stale URL demotion: a URL path segment
/// `/YYYY/` with `YYYY <= now_year - STALE_PATH_YEAR_LAG` is treated as
/// multi-year stale evidence on a live-price/freshness turn (e.g. `/2020/` in
/// 2026). Lag `2` keeps last calendar year eligible while dropping older
/// archive paths that SEO scrapes love for "today" price queries.
///
/// Not user-tunable: an evidence-pipeline constant tied to the freshness
/// contract, not a user preference.
pub const STALE_PATH_YEAR_LAG: u32 = 2;

/// Minimum consecutive ASCII digits that count as a price-like figure in a
/// chunk when applying the price-intent numeric utility filter. `2` admits
/// "80" (triệu) and "45" while rejecting lone list indices; independent of
/// [`STATISTIC_MIN_DIGIT_RUN`] (which is the GEO nudge threshold on boosted
/// domains only).
///
/// Not user-tunable: a ranking-algorithm heuristic bound.
pub const PRICE_LIKE_MIN_DIGIT_RUN: usize = 2;

/// Earliest 4-digit year that reads as a present/future freshness signal in a
/// standalone question, disqualifying the Wikipedia vertical. A year at or above
/// this is about the live world, which a static encyclopedia extract cannot
/// answer; a year below it is history, which Wikipedia serves well.
///
/// Not user-tunable: a routing-contract bound tied to the volatility guard.
pub const WIKI_VOLATILITY_MIN_YEAR: u32 = 2025;

/// Deterministic keyword-to-league map for the sports vertical (ESPN's public
/// scoreboard API): `(keyword, sport, league)`, where `sport`/`league` are the
/// path segments of `https://site.api.espn.com/apis/site/v2/sports/{sport}/{league}/scoreboard`.
/// Matched against the lowercased standalone question: a multi-word keyword
/// (containing a space) matches as a whole phrase, a single-word keyword
/// matches as a whole token. The first match wins; no match means the sports
/// vertical does not run for this turn.
///
/// Not user-tunable: a routing contract mapping known competitions/leagues to
/// their ESPN path segments, the same rationale as [`WIKI_VOLATILITY_MARKERS`].
/// Exposing it as a knob would let a bad edit silently break the vertical for a
/// whole league.
pub const SPORTS_LEAGUE_MAP: &[(&str, &str, &str)] = &[
    ("world cup", "soccer", "fifa.world"),
    ("premier league", "soccer", "eng.1"),
    ("champions league", "soccer", "uefa.champions"),
    ("nba", "basketball", "nba"),
    ("nfl", "football", "nfl"),
    ("mlb", "baseball", "mlb"),
    ("nhl", "hockey", "nhl"),
    ("f1", "racing", "f1"),
    ("formula 1", "racing", "f1"),
];

/// Length, in days, of the forward date window the sports vertical requests from
/// ESPN's scoreboard (`?dates=<today>-<today+N>`). ESPN's default scoreboard
/// returns only the current day's slate, which cannot answer "when is the next
/// match" once today's fixtures are all live or finished; requesting a window
/// forward from today makes the next fixture part of the same one response.
///
/// Not user-tunable: a pipeline-shape constant. Wide enough to always contain
/// the next fixture of an active competition, narrow enough to keep the block
/// within its source budget (the per-event listing the block renders is capped
/// independently in the sports module).
pub const SPORTS_SCHEDULE_WINDOW_DAYS: i64 = 7;

// ─── Web-search engine results ───────────────────────────────────────────────

/// Maximum result rows kept from one keyless search-engine query after dedupe.
/// Enough breadth for the fetch stage to pick the top pages plus snippet
/// fallbacks, without flooding the extractive filter. Matches the 8-10 band
/// every surveyed pipeline converges on.
///
/// Not user-tunable: a pipeline-shape constant, not a latency knob.
pub const SERP_MAX_RESULTS_PER_QUERY: usize = 10;

/// Maximum results kept from any single domain in one query, so a
/// content-farm that owns the whole first page cannot crowd out diverse
/// sources before the fetch/extract stages run.
///
/// Not user-tunable: a result-diversity bound.
pub const SERP_MAX_RESULTS_PER_DOMAIN: usize = 2;

/// Hard ceiling on the raw row count kept from a SINGLE engine's parsed SERP
/// before it is cached and fed into cross-engine fusion. A keyless engine's
/// `html` endpoint returns on the order of 30 organic rows, so this generous cap
/// never truncates a normal result page: every real row still reaches Reciprocal
/// Rank Fusion, preserving recall. It exists only to bound the pathological case
/// where an oversized or format-changed response parses into an unbounded row
/// list (one `SearchHit` per DOM node), which
/// would otherwise let a single response cache an arbitrarily large `Vec` under
/// up to [`SERP_CACHE_MAX_ENTRIES`] keys for [`SERP_CACHE_TTL_S`]. Sits above the
/// post-fusion [`SERP_MAX_RESULTS_PER_QUERY`] output cap on purpose: fusion still
/// sees the full page, and only the final fused list is trimmed to the output
/// ceiling.
///
/// Not user-tunable: a defense-in-depth bound on external, attacker-influenceable
/// SERP HTML, not a recall/latency knob.
pub const SERP_MAX_RAW_HITS_PER_QUERY: usize = 64;

/// Hit count at which the orchestrator stops issuing further search queries for
/// the turn. The classifier may emit up to 3 queries, but firing them all
/// back-to-back is a self-inflicted burst that trips the keyless engines' rate
/// limits (observed live: Mojeek served queries 1-2 then throttled query 3).
/// Once one query has returned this many usable hits, the remaining queries add
/// burst risk and latency for marginal recall, so they are skipped.
///
/// Not user-tunable: a rate-limit-survival bound on third-party request volume.
pub const SERP_EARLY_STOP_HITS: usize = 8;

/// Reciprocal Rank Fusion constant `k`. The keyless engine tier races all live
/// engines for a query and fuses their ranked lists with RRF: each URL scores
/// `sum over engines of 1 / (RRF_K + rank)`, where `rank` is its 1-based
/// position in that engine's list. `k = 60` is the parameter-free value from the
/// original RRF paper (Cormack et al., 2009) and the same constant Elasticsearch
/// ships as its default.
///
/// Not user-tunable: RRF is famously insensitive to `k`, so it is a fixed
/// algorithm constant, not a quality knob a user would ever benefit from turning.
pub const RRF_K: u32 = 60;

/// Rank offset added to a credibility-penalized URL's position in RRF fusion, so
/// a listed spam or copycat domain contributes `1 / (RRF_K + rank + this)` per
/// list instead of `1 / (RRF_K + rank)`. RRF at `k = 60` is nearly flat at the
/// top, so a score multiplier would do almost nothing; a rank offset is the sound
/// lever. This is a soft penalty, not a drop, because the penalize set is
/// bulk-imported and unaudited: a false-positive domain must still surface when
/// it is the only real answer. At `40` a rank-1 spam page (`1 / (60 + 1 + 40) =
/// 0.0099`) sits below a rank-10 page agreed on by two engines (`2 / (60 + 10) =
/// 0.0286`), so cross-engine agreement always beats a single-engine penalized hit.
///
/// Not user-tunable: an algorithm constant of the fusion step, tuned against the
/// RRF math, not a latency or quality knob.
pub const CREDIBILITY_PENALTY_RANK_OFFSET: u32 = 40;

// ─── Web-search freshness operators ──────────────────────────────────────────

/// DuckDuckGo `df` (date filter) value applied to the POST form and mirrored as
/// a `df` cookie when the standalone question carries a freshness signal (see
/// [`WIKI_VOLATILITY_MARKERS`]). `"w"` restricts results to the past week. The
/// dual form+cookie placement matches the common DuckDuckGo HTML client pattern,
/// which sets the filter both ways because the HTML endpoint honours either.
///
/// Not user-tunable: a fixed protocol convention of an external service, not a
/// quality knob.
pub const DDG_FRESHNESS_DF_VALUE: &str = "w";

/// Google News RSS search-operator suffix appended to the query when the
/// standalone question carries a freshness signal. `when:7d` narrows the feed
/// to the past 7 days, correcting the feed's default ordering, which otherwise
/// skews stale.
///
/// Not user-tunable: a fixed protocol convention of an external service.
pub const NEWS_FRESHNESS_OPERATOR: &str = "when:7d";

// ─── Web-search language parity ──────────────────────────────────────────────

/// The language every search channel falls back to when a query's language is
/// neither detectable from its script nor readable from the user's locale, and
/// the language whose request shapes are the compiled-in default everywhere.
///
/// Not user-tunable: it is the anchor of the allowlist in
/// `crate::websearch::lang`, and a value outside that allowlist would have no
/// verified request shape on any channel.
pub const SEARCH_LANG_DEFAULT: &str = "en";

/// Minimum share of a query's alphabetic characters that must belong to one
/// script before that script decides the query's language (Han, Kana, Hangul,
/// Thai, Arabic, Hebrew, Greek).
///
/// A presence check would be wrong: "what does 中 mean" is an English question
/// that happens to quote one Han character, and a single character must never
/// flip the whole request to Chinese. At `0.30` that query scores `0.08` and
/// stays English, while a genuinely mixed query ("iPhone 16 レビュー", `0.40`)
/// still resolves to its own language.
///
/// Not user-tunable: a defense-in-depth bound on how loudly one stray character
/// may speak for a whole query, not a quality knob.
pub const SEARCH_LANG_SCRIPT_RATIO_MIN: f64 = 0.30;

/// Minimum share of a query's whitespace tokens that must contain a
/// Vietnamese-distinctive character (see
/// `crate::websearch::script::is_vietnamese_marker`) before the query resolves
/// to Vietnamese.
///
/// Vietnamese is Latin script, so it has no script signal, only a diacritic
/// one, and the same diacritics ride into English on loanwords. The threshold
/// is set above the loanword case: "what does phở mean" scores `0.25` (one
/// token of four) and must stay English, while real Vietnamese queries carrying
/// two or more marked tokens ("thời tiết Hà Nội hôm nay", `0.50`) clear it. A
/// Vietnamese query below the bar is not lost, it falls through to the user's
/// locale, which is the correct signal for a Vietnamese-locale user.
///
/// Not user-tunable: same defense-in-depth rationale as
/// [`SEARCH_LANG_SCRIPT_RATIO_MIN`].
pub const SEARCH_LANG_VI_TOKEN_RATIO_MIN: f64 = 0.30;

/// DuckDuckGo `kl` (region) value used when the query's language does not
/// resolve, or resolves to [`SEARCH_LANG_DEFAULT`]. `wt-wt` is DuckDuckGo's own
/// "worldwide, no region bias" code: strictly better than the `us-en` this
/// replaced, which forced United States results onto every unresolved query.
///
/// Not user-tunable: a fixed protocol convention of an external service.
pub const DDG_DEFAULT_REGION: &str = "wt-wt";

/// `Accept-Language` header sent when the query resolves to
/// [`SEARCH_LANG_DEFAULT`]. DuckDuckGo's `kl` selects a REGION, not a language,
/// and its HTML endpoint exposes no language selector, so this header is the
/// only language lever the engine tier has and it must follow the resolved
/// language (see `crate::websearch::lang::accept_language`).
///
/// Not user-tunable: a fixed protocol convention of an external service.
pub const SEARCH_DEFAULT_ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";

/// Suffix appended to a non-English `Accept-Language` header, so a page with no
/// edition in the resolved language still ranks its English edition ahead of an
/// arbitrary third language rather than being excluded outright.
///
/// Not user-tunable: a fixed protocol convention of an external service.
pub const SEARCH_ACCEPT_LANGUAGE_FALLBACK: &str = ",en;q=0.5";

/// Mojeek `lbb` (language bias boost) percentage sent alongside `lb` on a
/// non-English query. `100` is the documented maximum: full weight on the
/// requested language.
///
/// Not user-tunable: a fixed protocol convention of an external service.
pub const MOJEEK_LANGUAGE_BIAS_BOOST: &str = "100";

/// How long a search engine is skipped after it returns a bot challenge or
/// rate-limit response (seconds), keyed per engine. Re-hammering a blocked
/// engine wastes a request per query, adds latency, and feeds the very volume
/// signal that keeps the block alive (DuckDuckGo's IP block is multi-hour and
/// volume-triggered, per the T1 spike). DuckDuckGo gets a long cooldown to
/// match its observed multi-hour blocks; the fallback engines get a short one
/// because their throttles are soft and clear quickly.
///
/// Not user-tunable: rate-limit-survival bounds on third-party request volume.
pub const ENGINE_COOLDOWN_PRIMARY_S: u64 = 1800;
/// Cooldown for fallback engines (seconds). See [`ENGINE_COOLDOWN_PRIMARY_S`].
pub const ENGINE_COOLDOWN_FALLBACK_S: u64 = 120;

// ─── Web-search in-memory result cache (process-lifetime, never persisted) ────

/// How long a per-engine SERP result list stays reusable in the in-memory web
/// cache (seconds). A repeat scrape of the same query within this window is
/// served from memory instead of re-hitting the keyless engine, which both cuts
/// latency and starves the engines' volume-triggered rate limits (a burst of
/// identical requests is exactly what earns a multi-hour DuckDuckGo IP block).
/// 5 minutes matches the realistic turn-to-turn repeat window while keeping SERP
/// freshness tight, since ranked results shift faster than page bodies.
///
/// Not user-tunable: an internal robustness bound, the same rationale as
/// [`SEARCH_CACHE_TTL_S`] and [`ENGINE_COOLDOWN_PRIMARY_S`].
pub const SERP_CACHE_TTL_S: u64 = 300;

/// How long an extracted page body stays reusable in the in-memory web cache
/// (seconds). Longer than [`SERP_CACHE_TTL_S`] because article text drifts more
/// slowly than the ranked result set that points at it, so a fetched page is
/// safe to reuse across a longer window.
///
/// Not user-tunable: an internal robustness bound, the same rationale as
/// [`SERP_CACHE_TTL_S`].
pub const PAGE_CACHE_TTL_S: u64 = 900;

/// Hard cap on the number of per-engine SERP lists held in the in-memory web
/// cache at once. When the cache is full the oldest-inserted entry is evicted to
/// make room, so the cache's memory footprint is bounded regardless of how many
/// distinct queries a session runs. Sized to comfortably cover a session's
/// recent-query working set without letting a long session grow the map without
/// limit.
///
/// Not user-tunable: an internal memory-safety bound.
pub const SERP_CACHE_MAX_ENTRIES: usize = 64;

/// Hard cap on the number of extracted page bodies held in the in-memory web
/// cache at once. Larger than [`SERP_CACHE_MAX_ENTRIES`] because a single SERP
/// fans out to several fetched pages, so the page working set is larger than the
/// query working set. Oldest-inserted entries are evicted at the cap, bounding
/// memory.
///
/// Not user-tunable: an internal memory-safety bound.
pub const PAGE_CACHE_MAX_ENTRIES: usize = 128;

// ─── Web-search fetch + extract ──────────────────────────────────────────────

/// `num_ctx` at or above which the fetch stage is allowed the larger page
/// budget. Below it, only a couple of extracted pages plus snippets fit
/// alongside the conversation; at or above it, more full pages fit.
///
/// Not user-tunable: a pipeline-shape threshold tied to the context budget, not
/// a user knob (the user tunes `num_ctx`, and this reads it).
pub const FETCH_LARGE_CTX_THRESHOLD: u32 = 16384;

/// Pages fully fetched and extracted per turn on a small context window
/// (`num_ctx` < [`FETCH_LARGE_CTX_THRESHOLD`]). The rest of the SERP contributes
/// snippets only, so recall is preserved without overrunning the budget.
///
/// Not user-tunable: derived pipeline shape gated by `num_ctx`.
pub const FETCH_MAX_PAGES_SMALL_CTX: usize = 2;

/// Pages fully fetched and extracted per turn on a large context window
/// (`num_ctx` >= [`FETCH_LARGE_CTX_THRESHOLD`]).
///
/// Not user-tunable: derived pipeline shape gated by `num_ctx`.
pub const FETCH_MAX_PAGES_LARGE_CTX: usize = 5;

/// Per-URL wall-clock timeout for a single page fetch (seconds). Each of the
/// budgeted page fetches is capped here; a URL that misses it degrades to its
/// SERP snippet. This is the hard backstop on any one fetch: [`FETCH_SOFT_DEADLINE_MS`]
/// can end the fan-out sooner, but nothing extends a single fetch past this.
///
/// Not user-tunable: an internal latency bound on the fetch fan-out.
pub const FETCH_PER_URL_TIMEOUT_S: u64 = 5;

/// Number of budgeted page fetches that must complete before the fetch stage
/// proceeds to ranking, out of up to [`FETCH_MAX_PAGES_LARGE_CTX`] raced
/// concurrently. Waiting on every one of them means one slow host holds up
/// the whole turn even though [`FETCH_PER_URL_TIMEOUT_S`] already bounds how
/// slow "slow" can be; racing to the first few completions instead bounds tail
/// latency on the common case where most hosts answer quickly. Whichever
/// hits fewer complete first (see [`FETCH_SOFT_DEADLINE_MS`]) applies; a
/// smaller fetch budget (`FETCH_MAX_PAGES_SMALL_CTX`) is capped by
/// `to_fetch.len()` at the call site, so this never blocks on more pages than
/// are actually being fetched.
///
/// Not user-tunable: an internal latency bound on the fetch fan-out.
pub const FETCH_FIRST_K_COMPLETIONS: usize = 3;

/// Soft aggregate deadline (milliseconds) for the whole page-fetch fan-out.
/// Once this elapses the fetch stage proceeds with whatever has completed so
/// far, regardless of [`FETCH_FIRST_K_COMPLETIONS`]; still-in-flight fetches
/// are abandoned and degrade to their SERP snippet exactly like a genuine
/// per-URL failure. This only ever shortens the wait: [`FETCH_PER_URL_TIMEOUT_S`]
/// remains the hard cap on any single fetch, so this soft deadline never
/// extends it.
///
/// Not user-tunable: an internal latency bound on the fetch fan-out.
pub const FETCH_SOFT_DEADLINE_MS: u64 = 2000;

/// Hard cap on DOM elements the readability extractor will parse from one page,
/// also reused as the cheap pre-parse element-count estimate that gates BOTH
/// the readability extraction and the freshness-gated published-date parse
/// (see `websearch::fetch::estimate_element_count`) before either pays for a
/// real parse. Defense-in-depth beyond [`MAX_HTTP_RESPONSE_BYTES`]: the byte
/// cap bounds download size (up to several MB), but a pathological page well
/// within that size can still contain far more elements than a real article,
/// and building a DOM tree at all (readability's own internal check only
/// stops the post-build algorithm, not the initial parse) is the expensive
/// step this cap is meant to avoid paying for twice. 9 000 covers real
/// articles with wide margin.
///
/// Not user-tunable: a defense-in-depth bound on attacker-controlled page
/// structure.
pub const FETCH_MAX_ELEMENTS_TO_PARSE: usize = 9000;

// ─── Web-search extractive filter (chunking + BM25) ──────────────────────────

/// Target size, in whitespace-separated words, of one page chunk fed to the
/// extractive filter. ~350 words lands in the 300-500 token band the retrieval
/// literature converges on: large enough to hold a coherent passage, small
/// enough that the ranker can discard the irrelevant remainder of a page.
///
/// Not user-tunable: a retrieval-pipeline shape constant.
pub const CHUNK_TARGET_WORDS: usize = 350;

/// Target size, in characters, of one page chunk when the page is written in an
/// unspaced script (Chinese, Japanese, Thai, Lao, Khmer, Burmese), where
/// whitespace does not delimit words and the word target above is meaningless.
/// ~500 Han characters carry roughly the information of the ~350 English words
/// [`CHUNK_TARGET_WORDS`] targets, so both paths land in the same retrieval
/// band.
///
/// Not user-tunable: a retrieval-pipeline shape constant.
pub const CHUNK_CJK_TARGET_CHARS: usize = 500;

/// Hard character ceiling for a single chunk on the unspaced-script path. A
/// sentence longer than this (a paragraph with no sentence terminator, the
/// normal shape of Thai prose) is split at this width, which guarantees forward
/// progress and makes a degenerate whole-page chunk impossible by construction.
/// Set above [`CHUNK_CJK_TARGET_CHARS`] so ordinary sentence packing, not the
/// hard split, decides chunk boundaries whenever the text has any.
///
/// Not user-tunable: a retrieval-pipeline shape constant.
pub const CHUNK_CJK_MAX_CHARS: usize = 700;

/// Sentence terminators the unspaced-script chunker splits on, kept with the
/// sentence they end. Covers the full-width CJK forms and the ASCII marks that
/// appear in mixed text; the full-width comma and the ideographic comma are
/// deliberately absent, as they separate clauses, not sentences.
///
/// Not user-tunable: a retrieval-pipeline shape constant.
pub const CHUNK_CJK_SENTENCE_TERMINATORS: [char; 7] = ['。', '！', '？', '；', '．', '!', '?'];

/// Minimum fraction of a page's non-whitespace characters that must belong to
/// an unspaced script before the page chunks on characters instead of words.
/// Above 0.3 the text is dominated by a script whose words carry no whitespace
/// delimiter, so `split_whitespace` returns a handful of enormous units; below
/// it, the page is mostly whitespace-delimited text (including Korean, which is
/// spaced) and the word path is correct. Deliberately low so a CJK page carrying
/// Latin markup, URLs, and numbers still takes the character path.
///
/// Not user-tunable: a retrieval-pipeline shape constant.
pub const CHUNK_UNSPACED_RATIO_MIN: f64 = 0.3;

/// BM25 term-frequency saturation parameter `k1`. The Okapi default; higher
/// values let repeated query terms keep raising a chunk's score, lower values
/// saturate sooner. 1.5 is the standard baseline.
///
/// Not user-tunable: a ranking-algorithm constant.
pub const BM25_K1: f64 = 1.5;

/// BM25 length-normalisation parameter `b`. The Okapi default; 1.0 fully
/// penalises long chunks, 0.0 ignores length. 0.75 is the standard baseline.
///
/// Not user-tunable: a ranking-algorithm constant.
pub const BM25_B: f64 = 0.75;

/// Maximum chunks kept from any single page after ranking, so one long page
/// cannot dominate the citation budget and source diversity is preserved.
///
/// Not user-tunable: a retrieval-pipeline diversity bound.
pub const RANK_MAX_CHUNKS_PER_PAGE: usize = 3;

/// Deterministic BM25 score nudge added to a chunk from a credibility-boosted
/// reference-grade domain when the chunk also carries a quote or an inline
/// statistic. GEO (arXiv:2311.09735) found LLM answer synthesis preferentially
/// cites quote- and statistic-bearing passages; without this nudge a reference
/// domain's plainer prose can lose a close BM25 tie to a distractor chunk that
/// happens to phrase the same fact more citably. Additive and only applied to
/// a chunk that already scored above zero (a real term match), so it can never
/// resurrect an irrelevant chunk, only tip an already-relevant one higher.
/// Moderate relative to typical query-matched BM25 scores so it tips ties
/// without overriding a genuinely stronger relevance match elsewhere.
///
/// Not user-tunable: a ranking-algorithm constant.
pub const QUOTE_STAT_SCORE_NUDGE: f64 = 0.5;

/// Minimum run of consecutive ASCII digits that counts as an inline statistic
/// for [`QUOTE_STAT_SCORE_NUDGE`] (a count, year, or other reported figure;
/// `%`-suffixed figures are recognized separately regardless of digit count).
/// Three digits excludes single- and double-digit incidental numbers (list
/// markers, small counts) while still catching years, percentages written
/// without a `%`, and larger reported figures.
///
/// Not user-tunable: a ranking-algorithm heuristic bound.
pub const STATISTIC_MIN_DIGIT_RUN: usize = 3;

// ─── Web-search recency-prior fusion ─────────────────────────────────────────

/// Weight given to a source's recency in the freshness-gated fusion score:
/// `final_score = RECENCY_ALPHA * recency + (1 - RECENCY_ALPHA) * relevance_norm`
/// (see [`crate::websearch::recency`]). `0.3` lets a clearly newer source
/// out-rank a marginally more relevant one without letting recency alone
/// override a strong relevance gap; the pass only ever reorders sources that
/// already survived credibility filtering and BM25 relevance thresholding, it
/// never introduces or resurrects one.
///
/// Not user-tunable: a corpus-sensitive ranking parameter. This is a
/// conservative first guess (see arXiv:2509.19376), pending tuning against a
/// real evaluation corpus rather than a value a user could sensibly set.
pub const RECENCY_ALPHA: f64 = 0.3;

/// Half-life, in days, of the exponential recency decay
/// `recency = exp(-ln(2) * age_days / RECENCY_HALF_LIFE_DAYS)`. A source
/// published exactly one half-life ago scores `0.5`, the same value assigned
/// to an undated source (see [`RECENCY_NEUTRAL_SCORE`]), so an undated source
/// is treated exactly as "moderately fresh" rather than favoured or
/// penalised. 14 days keeps last week's coverage strongly favoured while
/// still letting a several-week-old primary source compete on relevance.
///
/// Not user-tunable: a corpus-sensitive ranking parameter, a conservative
/// first guess pending tuning against a real evaluation corpus.
pub const RECENCY_HALF_LIFE_DAYS: f64 = 14.0;

/// Recency score assigned to a source with no extractable published or
/// modified date. Never `0.0` (an undated source is not evidence of
/// staleness) and never high enough to look like a fresh cracker: `0.5`
/// exactly matches the recency of a source published one half-life ago (see
/// [`RECENCY_HALF_LIFE_DAYS`]), so an undated source competes purely on
/// relevance instead of being dropped or boosted for a fetch-stage extraction
/// gap.
///
/// Not user-tunable: an algorithm invariant of the fusion formula, not a
/// tuning knob.
pub const RECENCY_NEUTRAL_SCORE: f64 = 0.5;

/// Clock-skew tolerance, in hours, applied when validating an extracted
/// published/modified date against the current time. A date more than this
/// far in the future is untrustworthy (a misconfigured server clock or a
/// malformed/hostile timestamp) and is treated as undated rather than
/// assigned a nonsensical negative age.
///
/// Not user-tunable: a defense-in-depth bound on attacker-controlled page
/// metadata.
pub const RECENCY_FUTURE_TOLERANCE_HOURS: i64 = 24;

// ─── Web-search context assembly ─────────────────────────────────────────────

/// Hard ceiling on the retrieved-source context injected into the writer call,
/// in estimated tokens. The effective budget is the smaller of this and a
/// fraction of `num_ctx` (see [`CONTEXT_BUDGET_CTX_PERCENT`]), so retrieval
/// never crowds out the conversation or the answer even on a huge context
/// window. 4 000 tokens holds several substantial source passages.
///
/// Not user-tunable: a pipeline budget bound derived alongside `num_ctx`.
pub const CONTEXT_MAX_TOKENS: usize = 4000;

/// Fraction of `num_ctx`, as a percentage, that retrieved sources may occupy.
/// Combined with [`CONTEXT_MAX_TOKENS`] via a min, this leaves the majority of
/// the window for the system prompt, conversation, and the generated answer.
///
/// Not user-tunable: a pipeline budget bound derived alongside `num_ctx`.
pub const CONTEXT_BUDGET_CTX_PERCENT: usize = 40;

/// Rough characters-per-token divisor for estimating token counts of source
/// text without invoking a tokenizer. ~4 characters per token is the standard
/// English approximation; the budget rounds up (over-estimates) so the real
/// token count stays under the ceiling.
///
/// Not user-tunable: an internal estimation constant.
pub const CHARS_PER_TOKEN: usize = 4;

/// Maximum chunks a domain absent from the credibility list ("unlisted") may
/// contribute to the assembled context once a credibility-boosted
/// reference-grade domain has already contributed at least one chunk.
/// Realistic-RAG research (arXiv:2505.15561) found that distracting passages
/// admitted into the top-K context, not their rank position, are what drive an
/// LLM to cite junk over a reference source sitting right beside it; capping a
/// thin unlisted aggregator's share once a reference is present keeps it from
/// crowding out that reference without discarding the aggregator outright (it
/// still contributes up to this many chunks). The cap never fires on a result
/// set with no boosted domain, since there is no reference chunk yet to
/// protect.
///
/// Not user-tunable: a retrieval-pipeline diversity bound conditioned on an
/// upstream credibility signal, not a user preference.
pub const UNLISTED_DOMAIN_CHUNK_CAP: usize = 2;

/// Length, in lowercase hex characters, of the per-request random token that
/// wraps retrieved web sources in the writer prompt (see
/// `websearch::writer`). The token is minted fresh from a CSPRNG for every
/// search turn so an attacker page, authored before the request exists, cannot
/// know it and therefore cannot forge the closing delimiter to break out of the
/// quoted untrusted-content region (prompt-injection spotlighting). 32 hex
/// characters carry the full 122 random bits of a v4 UUID: astronomically
/// unguessable, and the model never has to reason about the token's contents.
///
/// Not user-tunable: a defense-in-depth parameter over attacker-controlled web
/// content; exposing it could only weaken the delimiter, never help a user.
pub const SOURCE_DELIMITER_TOKEN_HEX_LEN: usize = 32;

/// Support-score threshold at or above which a citation is classified
/// "supported": at least this fraction of the citing sentence's content tokens
/// appear in the cited source's text. A baked-in heuristic bound for the
/// post-generation citation audit (a diagnostic that measures how often the
/// writer's bracket citations are actually backed by the cited source), not a
/// user preference.
///
/// Not user-tunable: an internal audit heuristic bound.
pub const CITE_SUPPORTED_MIN: f64 = 0.6;

/// Support-score threshold at or above which a citation is classified "weak"
/// (below [`CITE_SUPPORTED_MIN`]); below this it is "unsupported". A baked-in
/// heuristic bound for the post-generation citation audit, not a user
/// preference.
///
/// Not user-tunable: an internal audit heuristic bound.
pub const CITE_WEAK_MIN: f64 = 0.3;

/// Defensive upper bound, in bytes, on an answer the post-generation citation
/// audit will scan. Real grounded answers are far smaller; this only guards
/// against a runaway stream so the audit's work can never grow unbounded. An
/// answer past this size is skipped entirely (logged as skipped) rather than
/// audited.
///
/// Not user-tunable: an internal defensive bound.
pub const CITE_AUDIT_MAX_ANSWER_BYTES: usize = 262_144;

/// Cap on each source's body text written into a forensic
/// [`crate::trace::RecorderEvent::SearchRetrieved`] record. Tracing is
/// opt-in and evaluation-oriented: the full retrieved chunk is what the
/// writer and citation audit saw, so re-eval offline needs it, but a
/// multi-source turn must not grow a single JSONL line without bound.
///
/// Not user-tunable: forensic dump size bound (only when `trace_enabled`).
pub const TRACE_SOURCE_TEXT_MAX_BYTES: usize = 24_576;

/// Cap on the audited answer body embedded in a
/// [`crate::trace::RecorderEvent::CitationAudit`] record (the text the
/// audit actually scored, including pre-repair streams).
///
/// Not user-tunable: forensic dump size bound.
pub const TRACE_AUDIT_ANSWER_MAX_BYTES: usize = 32_768;

/// Cap on per-citation claim text in a forensic citation-detail row.
///
/// Not user-tunable: forensic dump size bound.
pub const TRACE_AUDIT_CLAIM_MAX_CHARS: usize = 512;

/// Maximum number of targeted writer repair rounds after a citation audit
/// finds unsupported claims. Each round is one extra full writer stream
/// (costly on reasoning models: multi-second thinking before any rewrite).
/// 0 means never repair (strip-only path). Cap is 1 on purpose: a second
/// repair rarely recovers total-failure cases (live gpt-oss traces still
/// failed after two) while adding ~5–15s of "Verifying" UX; one attempt plus
/// strip / honest note is enough.
///
/// Not user-tunable: fixed product budget for the grounded-answer repair loop.
pub const CITE_REPAIR_MAX_ATTEMPTS: u32 = 1;

/// Minimum byte length a cited source's fetched text must reach before the
/// post-generation citation audit will score a claim against it. Below this
/// (including a source whose text is empty), there is not enough substantive
/// content to run a meaningful lexical or numeric check, so the citation is
/// classified "unverifiable" rather than "unsupported": this is the
/// live-observed shape of a JS-widget single-page-app result (a Binance or
/// MEXC price page, for example), whose readable-text extraction succeeds but
/// collapses to a short loading placeholder or an empty SERP-snippet
/// fallback, never real content to check a claim against.
///
/// Not user-tunable: a defense-in-depth bound over externally fetched web
/// content, not a user preference.
pub const CITE_UNVERIFIABLE_MIN_SOURCE_BYTES: usize = 20;

/// Unicode code-point range of the fullwidth ASCII digits (`０`-`９`,
/// U+FF10-U+FF19, the digit run of the Halfwidth and Fullwidth Forms block).
/// The citation audit's content-token filter treats a run as number-like if
/// any of its characters fall in this range, alongside plain ASCII digits:
/// Japanese and Chinese source pages routinely render numerals fullwidth, and
/// without this a short fullwidth numeral (a lone `２`, or `３人`) never
/// clears the `> 3` character length rule and is silently dropped as a
/// content token, even though the same numeral in ASCII form would be kept.
///
/// Not user-tunable: a fixed Unicode block boundary, not a preference.
pub const CITE_FULLWIDTH_DIGITS: RangeInclusive<char> = '\u{FF10}'..='\u{FF19}';

/// Attached letter magnitude suffixes the citation audit's numeric-consistency
/// guard recognizes directly after a digit run (`615B`, `1.2mn`), paired with
/// the power-of-ten exponent each one adds. Checked in this order, but order
/// does not affect correctness: a truncated match (matching `b` when the
/// text is actually `bn`) always fails its own word-boundary check and falls
/// through to the longer entry.
///
/// Not user-tunable: fixed English financial-shorthand vocabulary for a
/// parsing guard, not a preference. Editing it would silently change which
/// figures the guard recognizes as matching.
pub const CITE_MAGNITUDE_ABBREVIATIONS: [(&str, u32); 7] = [
    ("bn", 9),
    ("mn", 6),
    ("tn", 12),
    ("b", 9),
    ("m", 6),
    ("t", 12),
    ("k", 3),
];

/// Spelled-out magnitude words the citation audit's numeric-consistency guard
/// recognizes after a digit run and whitespace (`615 billion`), paired with
/// the power-of-ten exponent each one adds.
///
/// Not user-tunable: fixed English magnitude vocabulary for a parsing guard,
/// same rationale as [`CITE_MAGNITUDE_ABBREVIATIONS`].
pub const CITE_MAGNITUDE_WORDS: [(&str, u32); 4] = [
    ("thousand", 3),
    ("million", 6),
    ("billion", 9),
    ("trillion", 12),
];

/// English month names, lowercase, paired with their calendar month number.
/// Used by the citation audit's numeric-consistency guard to recognize a
/// `July 9, 2026`-style date mention alongside the numeric `M/D/YYYY` and
/// ISO `YYYY-MM-DD` forms.
///
/// Not user-tunable: fixed English calendar vocabulary for a parsing guard,
/// same rationale as [`CITE_MAGNITUDE_ABBREVIATIONS`].
pub const CITE_MONTH_NAMES: [(&str, u32); 12] = [
    ("january", 1),
    ("february", 2),
    ("march", 3),
    ("april", 4),
    ("may", 5),
    ("june", 6),
    ("july", 7),
    ("august", 8),
    ("september", 9),
    ("october", 10),
    ("november", 11),
    ("december", 12),
];

// ─── Web-search engine-tier requery ──────────────────────────────────────────

/// Maximum number of bounded requeries the engine tier's own sufficiency judge
/// (`crate::websearch::orchestrator::judge_and_requery`) may fire per turn.
/// After the engine tier assembles its sources for the standalone question,
/// one judge call checks whether they actually answer it; on a confident
/// insufficient verdict naming what is missing, the orchestrator fires one
/// requery round (preferring the judge's keyword `requery_queries`, else
/// standalone + capped `missing`), merges the new sources in, then runs a
/// **second** judge only to set `still_missing` / conflict on the merged set.
/// There is no third requery. The flow has no loop back into the requery, so
/// `1` is the only value that fires a requery; `0` disables the requery
/// outright (the first judge still runs and its verdict is still recorded, but
/// an insufficient result simply commits round-one's sources) and is the
/// gate's only other meaningful setting.
///
/// Not user-tunable: fixed LLM-call budget per turn is a product invariant.
pub const ENGINE_REQUERY_MAX: usize = 1;

/// Maximum characters of the sufficiency judge's `missing` phrase appended to
/// the standalone question when `judge_and_requery` builds its one bounded
/// requery **fallback** (only when the judge omitted `requery_queries`).
/// `missing` is free-form model prose and can run to a full sentence; a long
/// tail of prose degrades keyless-engine SERP quality far more than a whole
/// trailing word left out does, so the appended text is truncated at the last
/// word boundary within this cap
/// (`crate::websearch::orchestrator::truncate_missing`). The trace's
/// `RecorderEvent::SearchRequeried::missing` field still carries the judge's
/// full, uncapped phrase; only the text actually searched is capped.
///
/// Not user-tunable: engine query hygiene is a pipeline-shape constant, not a
/// preference.
pub const REQUERY_MISSING_MAX_CHARS: usize = 80;

/// Maximum number of keyword SERP queries the engine-tier requery may issue
/// from a judge `requery_queries` list (or the single fallback concat query).
/// Matches the early-stop fan-out budget used by the primary engine tier's
/// multi-query loop; more than two rarely helps and doubles third-party burst.
///
/// Not user-tunable: fixed per-turn network budget.
pub const REQUERY_QUERY_MAX: usize = 2;

/// Maximum characters of each judge-authored requery keyword string after
/// trim. Longer free-form strings degrade keyless SERP quality the same way
/// an uncapped `missing` phrase does.
///
/// Not user-tunable: engine query hygiene is a pipeline-shape constant.
pub const REQUERY_QUERY_MAX_CHARS: usize = 120;

/// Maximum characters of HTML table text the page extractor may append to a
/// readability article body (or use alone when readability returns nothing).
/// Tables hold level/amount figures that Mozilla-style readability often
/// drops; this bound keeps token budget and attacker-controlled HTML in check.
///
/// Not user-tunable: extract size is a pipeline-shape constant.
pub const TABLE_EXTRACT_MAX_CHARS: usize = 4000;

/// Maximum number of `<table>` elements whose cells are harvested into the
/// table extract. Further tables are ignored.
///
/// Not user-tunable: extract size is a pipeline-shape constant.
pub const TABLE_EXTRACT_MAX_TABLES: usize = 8;

/// Maximum cells read from one table during table extract (row-major).
///
/// Not user-tunable: extract size is a pipeline-shape constant.
pub const TABLE_EXTRACT_MAX_CELLS_PER_TABLE: usize = 200;
