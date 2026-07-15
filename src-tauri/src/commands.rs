use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, Emitter, State};
use tokio_util::sync::CancellationToken;

use crate::config::defaults::STRIP_PATTERNS;
use crate::config::AppConfig;
use crate::models::{Capabilities, ModelCapabilitiesCache};

/// Removes special turn-boundary tokens (see [`STRIP_PATTERNS`]) and ASCII
/// control characters from assistant content before it is persisted to
/// history. Whitespace control chars (`\n`, `\t`, `\r`) are preserved so
/// markdown rendering and code blocks survive intact.
///
/// Pure function: same input always yields the same output. No allocation
/// happens when the input is already clean.
pub fn sanitize_assistant_content(input: &str) -> String {
    let mut out = input.to_string();
    for pattern in STRIP_PATTERNS {
        if out.contains(pattern) {
            out = out.replace(pattern, "");
        }
    }
    if out.chars().any(is_unsafe_control_char) {
        out = out
            .chars()
            .filter(|c| !is_unsafe_control_char(*c))
            .collect();
    }
    out
}

/// True for ASCII control characters in `0x00..=0x1F` except the three
/// whitespace controls Thuki actively renders (`\n`, `\t`, `\r`).
fn is_unsafe_control_char(c: char) -> bool {
    let code = c as u32;
    code <= 0x1F && c != '\n' && c != '\t' && c != '\r'
}

/// Counts of items stripped by [`apply_capability_filter`]. Returned to the
/// caller for telemetry only; the filter itself acts on the snapshot in
/// place. Storage is never mutated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FilterStats {
    /// Total images dropped across every message in the snapshot. A single
    /// message contributing N images to the strip increments by N.
    pub stripped_images: usize,
}

/// Per-request filter that aligns a snapshot of conversation history with
/// what the active model can actually consume. Storage is never touched:
/// the caller passes the working snapshot, this function trims it in
/// place, and on the next turn the caller rebuilds the snapshot from full
/// stored history again. Switching back to a capable model later restores
/// the full original payload because nothing was lost.
///
/// Today this strips images for non-vision models and trims per-message
/// image counts to a vision model's `max_images` cap. Multi-image trim
/// keeps the FIRST `max` images per message to preserve the order the
/// user attached them (OQ-1, doc decision).
pub fn apply_capability_filter(messages: &mut [ChatMessage], caps: &Capabilities) -> FilterStats {
    let mut stats = FilterStats::default();
    if !caps.vision {
        for msg in messages.iter_mut() {
            if let Some(imgs) = msg.images.take() {
                stats.stripped_images += imgs.len();
            }
        }
        return stats;
    }
    if let Some(max) = caps.max_images {
        let max = max as usize;
        for msg in messages.iter_mut() {
            if let Some(imgs) = msg.images.as_mut() {
                if imgs.len() > max {
                    let dropped = imgs.len() - max;
                    imgs.truncate(max);
                    stats.stripped_images += dropped;
                }
            }
        }
    }
    stats
}

/// Classifies the kind of error returned from the Ollama backend.
/// Used by the frontend to pick accent bar color and display copy.
#[derive(Clone, Serialize, PartialEq, Debug)]
#[serde(rename_all = "PascalCase")]
pub enum EngineErrorKind {
    /// Ollama process is not running (connection refused / timeout).
    EngineUnreachable,
    /// The bundled engine's sidecar process failed to launch or crashed before
    /// passing its health check.
    EngineStartFailed,
    /// The selected model's architecture is not supported by the bundled
    /// engine build, so `llama-server` refused to load it. A setup nudge (try
    /// another model), not a crash: the frontend renders it with the amber
    /// warning accent rather than the red failure accent.
    ModelUnsupported,
    /// The selected model's estimated footprint does not fit the memory
    /// available right now (issue #296). A soft, force-overridable refusal
    /// rather than a crash: the frontend renders it with the amber warning
    /// accent and offers a "load anyway" retry, sourcing the exact figures
    /// from the `estimate_model_fit` command.
    InsufficientMemory,
    /// The requested model has not been pulled yet (HTTP 404).
    ModelNotFound,
    /// No active model has been selected. The user must pick a model from
    /// the in-app picker before any chat request can be issued. Distinct
    /// from `ModelNotFound`, which fires when the daemon answered 404 for
    /// a slug we did try to use.
    NoModelSelected,
    /// Any other unexpected error.
    Other,
}

/// Builds the structured error returned when `ActiveModelState` holds `None`
/// at the time `ask_model` is invoked. Pulled out as a free function so the
/// exact title + body wording lives in one place and the branch is testable
/// without a full Tauri runtime.
pub fn no_model_selected_error() -> EngineError {
    EngineError {
        kind: EngineErrorKind::NoModelSelected,
        message: "No model selected\nPick a model in the picker.".to_string(),
    }
}

/// Builds the [`EngineErrorKind::InsufficientMemory`] error returned when the
/// pre-load memory gate refuses an un-forced load (issue #296). `required` and
/// `available` are the gate's estimate and the memory it judged available; both
/// are woven into the copy as approximate GiB so the user sees why. The exact
/// machine-readable figures for the "load anyway" affordance come from the
/// `estimate_model_fit` command, keeping one numeric source of truth.
pub fn insufficient_memory_error(required: u64, available: u64) -> EngineError {
    let gib = |bytes: u64| bytes as f64 / (1u64 << 30) as f64;
    EngineError {
        kind: EngineErrorKind::InsufficientMemory,
        message: format!(
            "This model may not fit in memory\nIt needs about {:.1} GB but only about {:.1} GB is free. Close some apps, pick a smaller model, or load it anyway.",
            gib(required),
            gib(available),
        ),
    }
}

/// Structured error emitted over the streaming channel.
/// Rust owns all user-facing copy; the frontend only uses `kind` for styling.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct EngineError {
    pub kind: EngineErrorKind,
    /// Final user-facing string. First line is the title, remainder is the subtitle.
    pub message: String,
}

/// How a chat turn reaches its inference backend, decided once per request
/// from the active provider's kind.
#[derive(Debug, PartialEq, Eq)]
pub enum ChatRoute {
    /// Native Ollama `/api/chat` streaming at the provider's base URL.
    OllamaNative {
        /// Full `<base>/api/chat` endpoint.
        endpoint: String,
    },
    /// Generic OpenAI-compatible `/v1` streaming at the provider's base URL.
    /// The API key is fetched later by provider id so the Keychain read
    /// happens only on the path that needs it.
    V1 {
        base_url: String,
        api_key_provider: Option<String>,
    },
    /// The bundled engine: resolve the installed model, ensure the sidecar
    /// serves it, then stream via the `/v1` client at the engine's port.
    Builtin {
        /// The active provider's `model` field: the manifest id.
        model_id: String,
    },
}

/// Decides the chat route from the resolved config. Pure so the routing
/// decision is unit-tested even though `ask_model` is coverage-off.
///
/// Errors:
/// - unknown/empty kind (defensive; the loader drops unknown kinds and
///   repairs a dangling `active_provider` pointer),
/// - `builtin` with an empty model (`NoModelSelected`, pointing the user at
///   the Settings model pick).
pub fn resolve_chat_route(
    inference: &crate::config::schema::InferenceSection,
) -> Result<ChatRoute, EngineError> {
    use crate::config::defaults::{
        PROVIDER_KIND_BUILTIN, PROVIDER_KIND_OLLAMA, PROVIDER_KIND_OPENAI,
    };
    match inference.active_provider_kind() {
        PROVIDER_KIND_OLLAMA => Ok(ChatRoute::OllamaNative {
            endpoint: format!(
                "{}/api/chat",
                inference.active_provider_base_url().trim_end_matches('/')
            ),
        }),
        PROVIDER_KIND_OPENAI => Ok(ChatRoute::V1 {
            base_url: inference
                .active_provider_base_url()
                .trim_end_matches('/')
                .to_string(),
            api_key_provider: Some(inference.active_provider.clone()),
        }),
        PROVIDER_KIND_BUILTIN => {
            let model = inference.active_provider_model();
            if model.is_empty() {
                return Err(EngineError {
                    kind: EngineErrorKind::NoModelSelected,
                    message: "No model selected\nPick or download a model in Settings.".to_string(),
                });
            }
            Ok(ChatRoute::Builtin {
                model_id: model.to_string(),
            })
        }
        _ => Err(EngineError {
            kind: EngineErrorKind::Other,
            message: "Something went wrong\nThe active provider has an unknown kind.".to_string(),
        }),
    }
}

/// Maps an installed-model manifest row onto the engine [`Target`] the
/// runner spawns: the model path, the optional mmproj path, plus the configured
/// context size.
///
/// A single-file model loads its content-addressed blob directly. A multi-part
/// (split) model cannot: llama.cpp rejoins a split by reading sibling shards
/// named `<prefix>-NNNNN-of-MMMMM.gguf`, but the blob store names every file by
/// its sha. So a split model is loaded through a symlink shim
/// ([`crate::models::storage::ModelStore::materialize_split_shim`]) whose first
/// shard's symlink becomes `model_path`; the engine then finds the rest of the
/// set beside it. A shim failure (an unreadable cache dir, or an invalid shard
/// name from the untrusted listing) surfaces as an engine-start error.
///
/// [`Target`]: crate::engine::state::Target
pub fn builtin_target(
    conn: &rusqlite::Connection,
    store: &crate::models::storage::ModelStore,
    model_id: &str,
    num_ctx: u32,
) -> Result<crate::engine::state::Target, EngineError> {
    let row = crate::models::manifest::get(conn, model_id).map_err(|e| EngineError {
        kind: EngineErrorKind::Other,
        message: format!("Something went wrong\nCould not read the installed-model manifest: {e}"),
    })?;
    let Some(model) = row else {
        return Err(EngineError {
            kind: EngineErrorKind::ModelNotFound,
            message: "The selected model is not installed.\nPick or download a model in Settings."
                .to_string(),
        });
    };
    let model_path = if model.parts.is_empty() {
        store.blob_path(&model.sha256)
    } else {
        store
            .materialize_split_shim(&model.parts)
            .map_err(|e| EngineError {
                kind: EngineErrorKind::Other,
                message: format!(
                    "Something went wrong\nCould not prepare the split model for loading: {e}"
                ),
            })?
    };
    Ok(crate::engine::state::Target {
        model_path,
        mmproj_path: model
            .mmproj_sha256
            .as_deref()
            .map(|sha| store.blob_path(sha)),
        num_ctx,
    })
}

/// Runs the pre-load memory gate (issue #296) for the built-in model
/// `model_id` about to be loaded at `target_path`. Assembles the inputs the
/// pure [`crate::models::memory::decide_load_gate`] needs and delegates every
/// decision to it (the single source of the block decision, shared with the
/// frontend fit affordance so the two can never drift, issue #296):
/// - the target's weights bytes from the manifest,
/// - live available memory from the mach VM statistics,
/// - the currently-resident model's path (so a same-model reload is a no-op and
///   a different resident model's footprint is credited back, since the engine
///   evicts before loading), read from the engine status,
/// - the installed rows mapped to `(weights_bytes, blob_path)` for that credit.
///
/// Forgiving on failure: an unreadable manifest returns [`MemoryGate::Proceed`]
/// rather than blocking a load on a database hiccup. Coverage-off: pure wiring
/// over tested functions; the gate logic lives in `models::memory`.
///
/// [`MemoryGate::Proceed`]: crate::models::memory::MemoryGate::Proceed
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn preflight_memory_gate(
    conn: &rusqlite::Connection,
    store: &crate::models::storage::ModelStore,
    engine: &crate::engine::runner::EngineHandle,
    model_id: &str,
    target_path: &std::path::Path,
    forced: bool,
) -> crate::models::memory::MemoryGate {
    use crate::models::memory;
    // Read the live engine status once: it feeds both the already-loading bypass
    // and the resident-credit path, both applied inside `decide_load_gate`.
    let status = engine.current_status();
    // Cannot size the target -> do not block on a database hiccup.
    let target_weights = match crate::models::manifest::get(conn, model_id) {
        Ok(Some(row)) => memory::model_weights_bytes(&row),
        _ => return memory::MemoryGate::Proceed,
    };
    // Map every installed row to (weights_bytes, weights blob path) so a
    // resident model can be matched by path and its footprint credited back.
    let installed: Vec<(u64, std::path::PathBuf)> = crate::models::manifest::list(conn)
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            (
                memory::model_weights_bytes(&row),
                store.blob_path(&row.sha256),
            )
        })
        .collect();
    // A live "loaded" status names the resident model's path; anything else
    // means nothing is resident to credit.
    let resident = (status.state == "loaded" && !status.model_path.is_empty())
        .then(|| std::path::PathBuf::from(&status.model_path));
    // Single source of the block decision, shared with `estimate_model_fit`'s
    // `build_model_fit_estimate` so the gate and the frontend fit affordance can
    // never diverge (issue #296). It folds in the already-loading bypass.
    memory::decide_load_gate(
        &status.state,
        &status.model_path,
        target_weights,
        memory::available_memory_bytes(),
        resident.as_deref(),
        target_path,
        &installed,
        forced,
    )
}

/// Parses llama-server's `GET /props` response and reports whether the
/// loaded model accepts image input. The flag lives at `modalities.vision`;
/// an absent field, a non-boolean value, or a malformed body all collapse to
/// `false` so the gate fails closed (images are stripped rather than letting
/// llama-server reject the whole request).
pub(crate) fn parse_props_vision(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("modalities")
                .and_then(|m| m.get("vision"))
                .and_then(|b| b.as_bool())
        })
        .unwrap_or(false)
}

/// Asks the serving llama-server whether the loaded model accepts images
/// (`GET /props`). Any transport or read failure collapses to `false`,
/// matching [`parse_props_vision`]'s fail-closed contract.
async fn fetch_builtin_vision(client: &reqwest::Client, base_url: &str) -> bool {
    match client.get(format!("{base_url}/props")).send().await {
        Ok(resp) => match resp.bytes().await {
            Ok(bytes) => parse_props_vision(&bytes),
            Err(_) => false,
        },
        Err(_) => false,
    }
}

/// Condenses a multi-line engine failure detail into the single most
/// informative line for the error subtitle (which renders as one paragraph).
/// The captured stderr tail can be many timestamped lines, so this prefers the
/// FIRST line that reads like an actual error message ("error:", "error
/// loading", "failed to", "failed:") over one that merely contains the word
/// (a startup banner such as "log level: error"). llama.cpp prints the specific
/// root cause first ("error loading model: <reason>") then generic trailers
/// ("failed to load", "exiting due to model loading error"), so the first
/// actionable match is the one to show. It falls back to any error/failure
/// mention, then to the first non-empty line; a single-line detail (e.g. a
/// health-check message) is returned unchanged. The first line is preferred
/// over the last because llama.cpp prints the real cause early and trails it
/// with unrelated lines (a dyld image line, a generic "exiting" notice), which
/// the last-line fallback would surface instead. Classification upstream still
/// sees the full detail.
fn concise_detail(detail: &str) -> String {
    let lines: Vec<&str> = detail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    match lines.as_slice() {
        [] => detail.trim().to_string(),
        [single] => (*single).to_string(),
        many => many
            .iter()
            .find(|line| {
                let lower = line.to_ascii_lowercase();
                lower.contains("error:")
                    || lower.contains("error loading")
                    || lower.contains("failed to")
                    || lower.contains("failed:")
            })
            .or_else(|| {
                many.iter().find(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower.contains("error") || lower.contains("failed")
                })
            })
            .copied()
            .unwrap_or(many[0])
            .to_string(),
    }
}

/// True when the engine's stderr is a dynamic-linker failure resolving a symbol
/// in a macOS system framework (e.g. `Symbol not found: _OBJC_CLASS_$_MTL... ;
/// Expected in: /System/Library/Frameworks/Metal.framework`). dyld emits this
/// when the engine binary needs a framework symbol the running macOS does not
/// provide, which for the bundled engine means the OS predates the engine's
/// build target. Both markers are required so an internal-symbol mismatch is
/// not misreported as an OS-version problem.
fn is_os_incompatible(lower_detail: &str) -> bool {
    lower_detail.contains("symbol not found") && lower_detail.contains(".framework")
}

/// Maps a built-in engine start failure (the engine's own captured stderr, or
/// a health-check message) onto a user-facing [`EngineError`]. A llama.cpp
/// "unknown model architecture" failure means the bundled engine cannot run
/// this model, so it becomes a `ModelUnsupported` nudge to pick another model;
/// a dyld system-framework symbol failure means the engine cannot run on this
/// macOS at all, so it surfaces a clear "update macOS" message instead of the
/// raw linker line; every other failure surfaces the concise reason as the
/// bare message, which the frontend renders verbatim under a fixed "couldn't
/// start this model" title, so OOM, context-size, and projector mismatches
/// stay actionable without a duplicated heading.
///
/// Pure so the classification and exact copy are unit-tested without a Tauri
/// runtime. Shared by `stream_builtin_chat` and `resolve_llm_transport`.
pub fn engine_start_error(detail: &str) -> EngineError {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("unknown model architecture") || lower.contains("unknown architecture") {
        EngineError {
            kind: EngineErrorKind::ModelUnsupported,
            message: "Unsupported model\nThuki's engine doesn't support this model's architecture yet. Try another model; support expands as the engine updates.".to_string(),
        }
    } else if is_os_incompatible(&lower) {
        EngineError {
            kind: EngineErrorKind::EngineStartFailed,
            message: "Thuki's engine could not start.\nYour version of macOS is too old for the built-in engine. Update macOS to use it.".to_string(),
        }
    } else {
        EngineError {
            kind: EngineErrorKind::EngineStartFailed,
            message: concise_detail(detail),
        }
    }
}

/// Runs the built-in-engine stage of a chat turn: mark activity, ensure the
/// engine serves `target`, then stream via the `/v1` client at the engine's
/// port. An engine activity guard is held for the whole turn (ensure,
/// `/props` gate, and body streaming) so the idle sweep never kills the
/// sidecar mid-generation. Pulled out of [`ask_model`] so the ensure-error
/// mapping is covered by tests:
/// - a cancel while the engine is still loading becomes a terminal
///   `Cancelled` (the load itself continues in the background so the next
///   message reuses the warm engine),
/// - `Superseded` becomes a terminal `Cancelled` (a newer settings change
///   preempted this request; never an engine-start failure),
/// - `StartFailed` becomes a typed `EngineStartFailed` error.
///
/// When the outgoing messages carry images, the serving llama-server is asked
/// whether the loaded model actually accepts them (`/props` runtime gate);
/// a non-vision model gets the images stripped through the same
/// [`apply_capability_filter`] path and stderr notice the cache-driven filter
/// uses, instead of letting the whole request fail.
///
/// This request's own first content chunk (`Token`/`ThinkingToken`) is
/// authoritative proof the model is warm, independent of the proactive
/// warm-up prime (`crate::warmup::warm_builtin`), which can still be queued
/// behind this same request at the engine's single execution slot. On that
/// first chunk, `warm_state.mark_warmed_by_real_request` is consulted and
/// `on_warmed` fires at most once, so a caller wired to emit
/// `warmup:builtin-warmed` from it never leaves the Settings status stuck on
/// "warming" for the duration of a response that raced ahead of its own
/// prime.
///
/// Returns the accumulated assistant content (empty on the error paths) so
/// the caller's persistence tail treats every route identically.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_builtin_chat(
    engine: &crate::engine::runner::EngineHandle,
    target: crate::engine::state::Target,
    model_id: String,
    think: bool,
    mut messages: Vec<ChatMessage>,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    warm_state: &crate::warmup::BuiltinWarmState,
    on_warmed: impl Fn(),
    on_chunk: impl Fn(StreamChunk),
) -> String {
    engine.touch();
    let _activity = engine.activity_guard();
    // Race the engine ensure against the user's cancel: a Stop press during
    // a cold model load must end the turn immediately, not after the load
    // completes. The runner tolerates dropped reply waiters, so the load
    // keeps running in the background and the next message reuses it.
    let ensured = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => None,
        result = engine.ensure_loaded(target) => Some(result),
    };
    match ensured {
        None => {
            on_chunk(StreamChunk::Cancelled);
            String::new()
        }
        Some(Ok(port)) => {
            let base_url = format!("http://127.0.0.1:{port}");
            let carries_images = messages
                .iter()
                .any(|m| m.images.as_ref().is_some_and(|imgs| !imgs.is_empty()));
            if carries_images && !fetch_builtin_vision(client, &base_url).await {
                let stats = apply_capability_filter(&mut messages, &Capabilities::default());
                if stats.stripped_images > 0 {
                    eprintln!(
                        "thuki: [capability filter] model={} stripped_images={}",
                        model_id, stats.stripped_images
                    );
                }
            }
            let warmed_announced = std::sync::atomic::AtomicBool::new(false);
            let on_chunk = |chunk: StreamChunk| {
                if !warmed_announced.load(std::sync::atomic::Ordering::Relaxed)
                    && matches!(chunk, StreamChunk::Token(_) | StreamChunk::ThinkingToken(_))
                {
                    warmed_announced.store(true, std::sync::atomic::Ordering::Relaxed);
                    if warm_state.mark_warmed_by_real_request(port) {
                        on_warmed();
                    }
                }
                on_chunk(chunk);
            };
            crate::openai::stream_openai_chat(
                crate::openai::OpenAiChatParams {
                    base_url,
                    model: model_id,
                    messages,
                    api_key: None,
                    flavor: crate::openai::V1Flavor::Builtin,
                    enable_thinking: think,
                },
                client,
                cancel_token,
                on_chunk,
            )
            .await
        }
        Some(Err(crate::engine::runner::EnsureError::Superseded)) => {
            on_chunk(StreamChunk::Cancelled);
            String::new()
        }
        Some(Err(crate::engine::runner::EnsureError::StartFailed(detail))) => {
            on_chunk(StreamChunk::Error(engine_start_error(&detail)));
            String::new()
        }
    }
}

/// Outcome of the built-in search pre-pass + pipeline for one turn.
enum BuiltinSearchResult {
    /// Search grounded the answer: stream these writer messages (which already
    /// embed the delimited sources) instead of the plain chat messages.
    /// `sources` carries the resolved citation blocks so the streamed answer can
    /// be citation-audited afterward; it is empty for an unreachable-search turn
    /// (grounded messaging, but nothing to cite).
    Grounded {
        messages: Vec<ChatMessage>,
        sources: Vec<crate::websearch::assemble::SourceBlock>,
        /// Instant the search pipeline started (submit), used to measure
        /// writer TTFT (submit → first answer token) once streaming begins.
        search_submit: std::time::Instant,
    },
    /// No search this turn (a `no` decision, an infra failure, or nothing worth
    /// citing): stream the original plain messages.
    Plain,
    /// The user cancelled during the pipeline: emit `Cancelled`, stream nothing.
    Cancelled,
}

/// Today's date as `YYYY-MM-DD`, in the device's local timezone when it can
/// be determined, else UTC. Kept at day granularity (not the fuller
/// [`local_datetime_context`] line): this exact string also feeds
/// [`crate::websearch::prefilter::prefilter`]'s current-or-future-year check
/// and the sports vertical's as-of line, both of which parse a bare
/// `YYYY-MM-DD`. Coverage-excluded: a thin clock wrapper.
#[cfg_attr(coverage_nightly, coverage(off))]
fn today_string() -> String {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    time::OffsetDateTime::now_utc()
        .to_offset(offset)
        .date()
        .to_string()
}

/// Best-effort user locale from `$LANG` (e.g. `en_US`), falling back to
/// `en-US`, for the writer's localisation hint. Coverage-excluded: a thin
/// environment wrapper.
#[cfg_attr(coverage_nightly, coverage(off))]
fn user_locale() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|lang| lang.split('.').next().map(str::to_string))
        .filter(|locale| !locale.is_empty())
        .unwrap_or_else(|| "en-US".to_string())
}

/// Best-effort IANA timezone name, read from the `/etc/localtime` symlink
/// target (macOS's canonical timezone pointer, e.g.
/// `/usr/share/zoneinfo/America/Chicago` -> `"America/Chicago"`). `None` when
/// the symlink is missing, unreadable, or does not resolve under a
/// `zoneinfo/` prefix (falls back to a numeric UTC-offset label; see
/// [`format_datetime_context`]). Coverage-excluded: a thin filesystem
/// wrapper.
#[cfg_attr(coverage_nightly, coverage(off))]
fn zone_label() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let target = target.to_str()?;
    target.split("zoneinfo/").nth(1).map(str::to_string)
}

/// Formats `offset` as `UTC+HH:MM` / `UTC-HH:MM`: the zone label used when the
/// IANA name in [`zone_label`] could not be resolved.
fn offset_label(offset: time::UtcOffset) -> String {
    let total_seconds = offset.whole_seconds();
    let sign = if total_seconds < 0 { '-' } else { '+' };
    let total_minutes = total_seconds.unsigned_abs() / 60;
    format!(
        "UTC{sign}{:02}:{:02}",
        total_minutes / 60,
        total_minutes % 60
    )
}

/// Formats the current local-datetime context line injected into the persona
/// system prompt (see [`system_prompt_with_datetime`]): weekday, ISO date,
/// 24-hour time, and a timezone label, e.g. `"Friday, 2026-07-10, 01:15
/// (America/Chicago)"`. `now_utc` and `offset` are injected so both branches
/// are unit-tested without depending on process/thread state:
/// - `offset` is `None` when the local offset could not be soundly determined
///   (see `local_datetime_context`), in which case the line reports the UTC
///   time labelled `"UTC"` rather than guessing;
/// - `zone` is the best-effort IANA name; `None` falls back to a numeric
///   `UTC±HH:MM` label ([`offset_label`]) so the line always carries a zone
///   marker.
fn format_datetime_context(
    now_utc: time::OffsetDateTime,
    offset: Option<time::UtcOffset>,
    zone: Option<&str>,
) -> String {
    let (local, label) = match offset {
        Some(offset) => (
            now_utc.to_offset(offset),
            zone.map(str::to_string)
                .unwrap_or_else(|| offset_label(offset)),
        ),
        None => (now_utc, "UTC".to_string()),
    };
    format!(
        "{}, {:04}-{:02}-{:02}, {:02}:{:02} ({label})",
        local.weekday(),
        local.year(),
        u8::from(local.month()),
        local.day(),
        local.hour(),
        local.minute(),
    )
}

/// Captures the real clock/offset/zone and formats the current local-datetime
/// context line (see [`format_datetime_context`]). Coverage-excluded: a thin
/// wrapper over the tested pure formatter; the only untested behaviour is
/// reading the real clock and `/etc/localtime`.
#[cfg_attr(coverage_nightly, coverage(off))]
fn local_datetime_context() -> String {
    let offset = time::UtcOffset::current_local_offset().ok();
    let zone = zone_label();
    format_datetime_context(time::OffsetDateTime::now_utc(), offset, zone.as_deref())
}

/// Appends the current local-datetime context line to the persona system
/// prompt, plus — when a place-qualified clock question resolved this turn
/// (see [`resolve_clock_place_time`] / `websearch::clock`) — the resolved
/// place-time line and a standing instruction that timezone arithmetic is
/// never the model's job. This is composed once, before the route/search
/// branch, so both the plain chat stream and the search writer/unreachable
/// prompts (which reuse this same system message; see `run_builtin_search`'s
/// `system_prompt` argument) can answer date/time questions directly from
/// context instead of guessing or, worse, searching the web for the clock.
///
/// The instruction line is unconditional (present whether or not a place
/// resolved) so the model has one consistent rule: read a resolved line back
/// verbatim when one is present, and otherwise share only the local time
/// above rather than compute a conversion itself. Small local models are
/// unreliable at arithmetic, so the conversion is never left to them; see
/// [`crate::websearch::clock`] for where it is actually computed.
///
/// This is deliberately the ONLY place the local datetime (and any resolved
/// place time) is threaded into: the persona-free search classifier
/// (`websearch::prepass`) builds its own short system prompt and never sees
/// the persona, so neither line can leak into its standalone-question
/// rewrite or outbound search queries. The writer's own `today`
/// (`YYYY-MM-DD`) date context is separate and unaffected (see
/// `today_string`).
pub(crate) fn system_prompt_with_datetime(
    persona: &str,
    datetime_context: &str,
    place_time_line: Option<&str>,
) -> String {
    let mut out = format!("{persona}\n\nCurrent local date and time: {datetime_context}.");
    if let Some(line) = place_time_line {
        out.push('\n');
        out.push_str(line);
    }
    out.push_str(
        "\nWhen asked what time it is somewhere else, use a resolved time line above \
         verbatim if one is present and never compute the timezone conversion yourself; \
         if none is present, you can only share the local time above, so say that rather \
         than guess.",
    );
    out
}

/// Resolves a place-qualified clock question's current time for this turn
/// ("what time is it in San Francisco"), or `None` on any miss: not a clock
/// question, no place named, the place did not geocode (including a bare
/// abbreviation like "SF", which Open-Meteo does not resolve), a transport
/// error, or an unresolvable timezone. A miss injects nothing extra; the
/// model falls back to its own local-time context (see
/// [`system_prompt_with_datetime`]). Never triggers a web search: this is a
/// pure geocode-plus-computation, independent of the search decision.
///
/// Coverage-excluded: thin async glue delegating every decision to
/// [`crate::websearch::prefilter::clock_question_place`] (pure, tested) and
/// [`crate::websearch::clock::resolve_place_time`] (tested against
/// [`crate::net::transport::FakeHttpTransport`]); the only untested lines
/// here are wiring the real [`crate::net::transport::ReqwestTransport`] and
/// the real clock.
#[cfg_attr(coverage_nightly, coverage(off))]
async fn resolve_clock_place_time(message: &str) -> Option<String> {
    let place = crate::websearch::prefilter::clock_question_place(message)?;
    let transport = crate::net::transport::ReqwestTransport::new().ok()?;
    crate::websearch::clock::resolve_place_time(&transport, &place, time::OffsetDateTime::now_utc())
        .await
}

/// Maps a resolved [`crate::websearch::orchestrator::SearchOutcome`] to a short
/// stable label for the J6 diagnostic stderr line in [`run_builtin_search`].
/// This is a diagnostic hook for an unreproduced bug (first-turn submissions
/// completing instantly with zero tokens): the auto-search pre-pass shares the
/// engine port and cancel token with the writer, so naming which search path
/// resolved lets a live zero-token occurrence be correlated with the branch
/// that ran. A cache hit is not distinguished here by design: it resolves into
/// `Answer` inside the fully-tested `run_search`, so surfacing it would mean
/// instrumenting a different function. Pure so it stays unit-tested while its
/// coverage-excluded call site does not.
fn builtin_search_outcome_label(
    outcome: &crate::websearch::orchestrator::SearchOutcome,
) -> &'static str {
    use crate::websearch::orchestrator::SearchOutcome;
    match outcome {
        SearchOutcome::Answer { .. } => "Answer",
        SearchOutcome::Unreachable { .. } => "Unreachable",
        SearchOutcome::NoSearch => "NoSearch",
        SearchOutcome::Cancelled => "Cancelled",
    }
}

/// Transform slash commands that must never run auto-search or the search
/// classifier/prefilter. Kept in lockstep with FE `skipsAutoSearch` /
/// `Command.skipSearch` in `src/config/commands.ts`.
///
/// Not skipped: `/search` (force), `/explain`, `/think`, `/screen`.
fn slash_skips_auto_search(slash_command: Option<&str>) -> bool {
    matches!(
        slash_command,
        Some("/rewrite")
            | Some("/refine")
            | Some("/translate")
            | Some("/tldr")
            | Some("/bullets")
            | Some("/todos")
            | Some("/extract")
    )
}

/// Decision for whether a built-in turn enters `run_builtin_search`.
#[derive(Debug, PartialEq, Eq)]
enum BuiltinSearchGate {
    /// Stay on the plain chat path (no classifier, no engines).
    Plain,
    /// Enter the search pipeline; `force` is the `/search` engines-only flag.
    Run { force: bool },
}

/// Pure gate for built-in web search before any pipeline work.
///
/// Order: non-vision image turns stay plain (capability strip path); `/search`
/// always runs forced; transform slash commands always plain; otherwise Auto
/// search Settings decide. Vision + images may enter the pipeline so the
/// classifier and writer can see the photo.
fn builtin_search_gate(
    turn_has_images: bool,
    model_is_vision: bool,
    force_search: bool,
    skip_search: bool,
    auto_search: bool,
) -> BuiltinSearchGate {
    // Non-vision models cannot use image-aware search; keep today's strip path.
    if turn_has_images && !model_is_vision {
        BuiltinSearchGate::Plain
    } else if force_search {
        BuiltinSearchGate::Run { force: true }
    } else if skip_search {
        BuiltinSearchGate::Plain
    } else if auto_search {
        BuiltinSearchGate::Run { force: false }
    } else {
        BuiltinSearchGate::Plain
    }
}

/// Closed `reason` strings for [`crate::trace::RecorderEvent::SearchSkipped`].
/// Keep in lockstep with gate order and docs consumers of the JSONL.
fn search_skip_reason_for_plain_gate(
    turn_has_images: bool,
    model_is_vision: bool,
    skip_search: bool,
) -> &'static str {
    if turn_has_images && !model_is_vision {
        "non_vision_images"
    } else if skip_search {
        "transform_slash"
    } else {
        "auto_off"
    }
}

/// Caps the final answer body stored on [`RecorderEvent::AssistantComplete`] so
/// a pathological long answer cannot balloon the JSONL. Prefer a char boundary.
fn capped_trace_final_content(content: &str) -> String {
    let max = crate::config::defaults::CITE_AUDIT_MAX_ANSWER_BYTES;
    if content.len() <= max {
        return content.to_string();
    }
    let mut end = max;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated]", &content[..end])
}

/// Runs the built-in search pre-pass and, if it fires, the retrieval pipeline
/// on the warm engine, emitting progress through `on_chunk`. The prompt inputs
/// MUST be the same strings the plain chat path streams (system prompt, filtered
/// history, quote-wrapped user turn) so the pre-pass and writer reuse
/// llama-server's warm KV prefix. `cache_scope` is the conversation epoch
/// snapshotted at the start of this turn, scoping reads/writes of the
/// multi-turn source cache to this conversation (see `websearch::cache`).
/// `force_search` (set by the `/search` slash command) forces the search on
/// regardless of the pre-pass decision, with cache read-bypass, write-through
/// semantics; `false` lets the invisible auto-search pre-pass decide.
///
/// Coverage-excluded: glue that wires the real engine port, HTTP transport, and
/// BM25 scorer into the fully-tested [`crate::websearch::orchestrator::run_search`]
/// and maps its outcome to a wire result. Every decision lives in `run_search`.
#[cfg_attr(coverage_nightly, coverage(off))]
#[allow(clippy::too_many_arguments)]
async fn run_builtin_search(
    engine: &crate::engine::runner::EngineHandle,
    target: &crate::engine::state::Target,
    model_id: &str,
    client: &reqwest::Client,
    num_ctx: u32,
    system_prompt: &str,
    history: &[ChatMessage],
    latest_user: &str,
    // Base64 images for the latest turn when the model is vision-capable.
    // Forwarded to prepass + writer only; never to search engines.
    latest_images: Option<&[String]>,
    cancel: &CancellationToken,
    recorder: &std::sync::Arc<crate::trace::BoundRecorder>,
    on_chunk: &(impl Fn(StreamChunk) + Send + Sync),
    cache_scope: u64,
    force_search: bool,
) -> BuiltinSearchResult {
    // The engine is already warm (the caller holds an activity guard); this
    // re-ensure just reads back the live port for the pre-pass and writer.
    let Ok(port) = engine.ensure_loaded(target.clone()).await else {
        // Gate said Run but the engine is not loadable: still mark the skip so
        // traces do not look like the turn never considered search.
        recorder.record(crate::trace::RecorderEvent::SearchSkipped {
            reason: "engine_unavailable".to_string(),
        });
        return BuiltinSearchResult::Plain;
    };
    let Ok(transport) = crate::net::transport::ReqwestTransport::new() else {
        recorder.record(crate::trace::RecorderEvent::SearchSkipped {
            reason: "engine_unavailable".to_string(),
        });
        return BuiltinSearchResult::Plain;
    };
    let prepass = crate::websearch::prepass::BuiltinPrePass::new(
        client.clone(),
        format!("http://127.0.0.1:{port}"),
        model_id.to_string(),
        crate::config::defaults::PREPASS_TIMEOUT_S,
    );
    // The sufficiency judge shares the warm engine: it decides whether a
    // vertical's answer actually contains what the question asked before the
    // pipeline commits to it, escalating an insufficient block to the scraped
    // engines instead of dead-ending (see `websearch::judge`).
    let judge = crate::websearch::judge::BuiltinSufficiencyJudge::new(
        client.clone(),
        format!("http://127.0.0.1:{port}"),
        model_id.to_string(),
        crate::config::defaults::SUFFICIENCY_JUDGE_TIMEOUT_S,
    );
    let scorer = crate::websearch::rank::Bm25Scorer;
    // The device IANA timezone, so the sports vertical can localize scheduled
    // kickoff times; `None` (unreadable /etc/localtime) degrades to date-only
    // event lines.
    let local_zone = zone_label();
    // One timing bag per search turn: orchestrator records stage ms and flushes
    // SearchTimings; writer_ttft is appended on the first answer token below.
    let timing_bag = crate::websearch::stage_timing::TimingBag::new();
    let deps = crate::websearch::orchestrator::SearchDeps {
        prepass: &prepass,
        judge: &judge,
        transport: &transport,
        scorer: &scorer,
        health: crate::websearch::engine::global_engine_health(),
        recorder: recorder.as_ref(),
        // Scoped to the conversation epoch at the start of this turn: the
        // same epoch `reset_conversation`/`load_conversation` bump on every
        // conversation boundary, so a cache entry from one conversation is
        // never served to another (see `websearch::cache` module docs).
        cache: crate::websearch::cache::global_search_cache(),
        cache_scope,
        // Process-wide in-memory SERP + page cache, shared across turns so a
        // repeat scrape is served from memory (see `websearch::serp_cache`).
        web_cache: crate::websearch::serp_cache::global_web_cache(),
        local_zone: local_zone.as_deref(),
        // Forced on by the `/search` slash command: search this turn regardless
        // of the pre-pass decision, with cache read-bypass, write-through
        // semantics (see `SearchDeps::force_search`).
        force_search,
        // Vision turns only: classifier + writer keep the photo; engines stay text.
        latest_images,
        timings: &timing_bag,
    };
    let status = |phase| on_chunk(StreamChunk::SearchStatus { phase });
    let outcome = if force_search {
        crate::websearch::orchestrator::run_search_forced(
            &deps,
            system_prompt,
            history,
            latest_user,
            num_ctx,
            &today_string(),
            &user_locale(),
            cancel,
            &status,
        )
        .await
    } else {
        crate::websearch::orchestrator::run_search(
            &deps,
            system_prompt,
            history,
            latest_user,
            num_ctx,
            &today_string(),
            &user_locale(),
            cancel,
            &status,
        )
        .await
    };
    // Diagnostic hook for the unreproduced J6 zero-token bug: log which outcome
    // variant resolved so a live occurrence can be correlated with the search
    // path that ran. One stderr line per search-turn (not gated to turn 1);
    // never surfaced to the frontend.
    eprintln!(
        "search: run_builtin_search resolved outcome={}",
        builtin_search_outcome_label(&outcome)
    );
    let search_submit = timing_bag.submit_instant();
    match outcome {
        crate::websearch::orchestrator::SearchOutcome::Answer { messages, sources } => {
            on_chunk(StreamChunk::SearchSources(source_metas(&sources)));
            BuiltinSearchResult::Grounded {
                messages,
                sources,
                search_submit,
            }
        }
        // Search wanted but produced nothing: emit a reliable, typed failure
        // signal (independent of the model's prose) so the frontend always shows
        // the right note, then stream the disclosure-bearing messages (no sources
        // to attach) so the answer also names its unverified freshness.
        crate::websearch::orchestrator::SearchOutcome::Unreachable { messages, reason } => {
            on_chunk(StreamChunk::SearchFailed { reason });
            BuiltinSearchResult::Grounded {
                messages,
                sources: Vec::new(),
                search_submit,
            }
        }
        crate::websearch::orchestrator::SearchOutcome::NoSearch => BuiltinSearchResult::Plain,
        crate::websearch::orchestrator::SearchOutcome::Cancelled => BuiltinSearchResult::Cancelled,
    }
}

/// Records writer TTFT (submit → first answer token) to stderr and the trace.
/// Pure except for the recorder/stderr side effects; coverage-off thin glue.
#[cfg_attr(coverage_nightly, coverage(off))]
fn record_writer_ttft(
    search_submit: std::time::Instant,
    recorder: &std::sync::Arc<crate::trace::BoundRecorder>,
) {
    use crate::websearch::stage_timing::{elapsed_ms, format_timing_line, STAGE_WRITER_TTFT};
    let ms = elapsed_ms(search_submit);
    eprintln!("{}", format_timing_line(STAGE_WRITER_TTFT, ms));
    recorder.record(crate::trace::RecorderEvent::SearchTimings {
        stages: vec![crate::trace::StageTiming {
            stage: STAGE_WRITER_TTFT.to_string(),
            ms,
        }],
    });
}

/// Records one citation audit to the trace + stderr, then returns the audit.
/// Skips (returns `None`) when there are no sources or the answer exceeds the
/// defensive size cap. Coverage-off: thin I/O glue over pure `cite_check`.
#[cfg_attr(coverage_nightly, coverage(off))]
fn record_citation_audit(
    answer: &str,
    sources: &[crate::websearch::assemble::SourceBlock],
    recorder: &std::sync::Arc<crate::trace::BoundRecorder>,
) -> Option<crate::websearch::cite_check::CitationAudit> {
    if sources.is_empty() {
        return None;
    }
    if answer.len() > crate::config::defaults::CITE_AUDIT_MAX_ANSWER_BYTES {
        eprintln!(
            "[search] citation audit: skipped (answer {} bytes exceeds {} byte cap)",
            answer.len(),
            crate::config::defaults::CITE_AUDIT_MAX_ANSWER_BYTES
        );
        return None;
    }
    let audit = crate::websearch::cite_check::audit_citations(answer, sources);
    let answer_for_trace = crate::trace::truncate_for_trace(
        answer,
        crate::config::defaults::TRACE_AUDIT_ANSWER_MAX_BYTES,
    );
    let details = audit
        .details
        .iter()
        .map(|d| crate::trace::CitationDetail {
            index: d.index,
            class: d.class.clone(),
            claim: d.claim.clone(),
            lexical_score: d.lexical_score.clone(),
            numeric_checked: d.numeric_checked,
            numeric_matched: d.numeric_matched,
            numeric_missing: d.numeric_missing,
        })
        .collect();
    recorder.record(crate::trace::RecorderEvent::CitationAudit {
        cited: audit.cited,
        supported: audit.supported,
        weak: audit.weak,
        unsupported: audit.unsupported,
        unsupported_indices: audit.unsupported_indices.clone(),
        numeric_checked: audit.numeric_checked,
        numeric_matched: audit.numeric_matched,
        numeric_missing: audit.numeric_missing,
        unverifiable: audit.unverifiable,
        answer: answer_for_trace,
        details,
    });
    eprintln!(
        "[search] citation audit: cited={} supported={} weak={} unsupported={} unverifiable={} numeric_checked={} numeric_matched={} numeric_missing={}",
        audit.cited,
        audit.supported,
        audit.weak,
        audit.unsupported,
        audit.unverifiable,
        audit.numeric_checked,
        audit.numeric_matched,
        audit.numeric_missing
    );
    Some(audit)
}

/// Builds the writer-follow-up messages for one citation repair round: the
/// original grounded writer transcript, the previous answer as assistant, and
/// a pure-code critique naming the failing `[n]` indices.
fn build_repair_messages(
    writer_messages: Vec<ChatMessage>,
    previous_answer: &str,
    audit: &crate::websearch::cite_check::CitationAudit,
) -> Vec<ChatMessage> {
    let mut messages = writer_messages;
    messages.push(ChatMessage {
        role: "assistant".into(),
        content: previous_answer.into(),
        images: None,
    });
    messages.push(ChatMessage {
        role: "user".into(),
        content: crate::websearch::cite_check::repair_critique(audit),
        images: None,
    });
    messages
}

/// After the first grounded writer stream, audit → repair (up to
/// [`CITE_REPAIR_MAX_ATTEMPTS`]) → strip leftover bad citations. Coverage-off:
/// orchestration glue around pure `cite_check` and `stream_builtin_chat`.
#[cfg_attr(coverage_nightly, coverage(off))]
#[allow(clippy::too_many_arguments)]
async fn refine_grounded_answer(
    mut content: String,
    sources: &[crate::websearch::assemble::SourceBlock],
    writer_messages: Vec<ChatMessage>,
    engine: &crate::engine::runner::EngineHandle,
    target: crate::engine::state::Target,
    model_id: String,
    think: bool,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    warm_state: &crate::warmup::BuiltinWarmState,
    recorder: &std::sync::Arc<crate::trace::BoundRecorder>,
    on_terminal: &impl Fn(StreamChunk),
) -> String {
    if sources.is_empty() || content.is_empty() {
        return content;
    }
    if content.len() > crate::config::defaults::CITE_AUDIT_MAX_ANSWER_BYTES {
        return content;
    }

    for attempt in 0..crate::config::defaults::CITE_REPAIR_MAX_ATTEMPTS {
        if cancel_token.is_cancelled() {
            break;
        }
        let Some(audit) = record_citation_audit(&content, sources, recorder) else {
            return content;
        };
        // Success only when every present [n] is ok AND the answer cited
        // something. cited==0 with sources present is a silent failure mode
        // (gpt-oss often names Bloomberg/Forbes in prose without [n]).
        if !crate::websearch::cite_check::needs_citation_repair(&audit, true) {
            return content;
        }
        eprintln!(
            "[search] citation repair: attempt {}/{} cited={} unsupported={:?}",
            attempt + 1,
            crate::config::defaults::CITE_REPAIR_MAX_ATTEMPTS,
            audit.cited,
            audit.unsupported_indices
        );
        let repair_messages = build_repair_messages(writer_messages.clone(), &content, &audit);
        // Mute answer tokens on repair rounds: only the final cleaned answer
        // is shown. Forward cancel/error so the UI still leaves streaming.
        let repaired = stream_builtin_chat(
            engine,
            target.clone(),
            model_id.clone(),
            think,
            repair_messages,
            client,
            cancel_token.clone(),
            warm_state,
            || {},
            |chunk| match chunk {
                StreamChunk::Cancelled | StreamChunk::Error(_) => on_terminal(chunk),
                StreamChunk::Done
                | StreamChunk::Token(_)
                | StreamChunk::ThinkingToken(_)
                | StreamChunk::TurnAccepted
                | StreamChunk::SearchStatus { .. }
                | StreamChunk::SearchSources(_)
                // Never emitted on a repair round (a chat stream, not the search
                // pipeline), but the match must stay exhaustive.
                | StreamChunk::SearchFailed { .. }
                | StreamChunk::SetContent(_) => {}
            },
        )
        .await;
        if cancel_token.is_cancelled() {
            break;
        }
        if repaired.is_empty() {
            break;
        }
        content = repaired;
    }

    // Final audit after repairs (or after exhausting attempts).
    match record_citation_audit(&content, sources, recorder) {
        Some(audit) => crate::websearch::cite_check::finalize_answer_after_audit(&content, &audit),
        None => content,
    }
}

/// Decides the final persisted answer content and the terminal chunk(s) to
/// emit once a built-in turn's token stream (and optional citation refine)
/// has finished.
///
/// `done_pending` is `false` when the stream ended by cancellation or error
/// instead of `Done`; nothing more is emitted (the frontend already left
/// streaming). Answer tokens stream live during the first writer pass so the
/// UI keeps the streaming feel; `streamed` is that live-accumulated body.
/// When citation audit/repair changes the body, emit one
/// [`StreamChunk::SetContent`] so the bubble snaps to the cleaned text before
/// `Done`. When the body is unchanged, only `Done` is emitted.
fn finalize_builtin_stream(
    content: String,
    streamed: &str,
    done_pending: bool,
) -> (String, Vec<StreamChunk>) {
    if !done_pending {
        return (content, Vec::new());
    }
    let mut chunks = Vec::new();
    if content != streamed {
        chunks.push(StreamChunk::SetContent(content.clone()));
    }
    chunks.push(StreamChunk::Done);
    (content, chunks)
}

/// Sets `flag` when `chunk` carries reasoning output. The built-in runtime
/// backstop wires this into the chunk pump so it learns whether a model emitted
/// reasoning tokens even though reasoning was requested OFF.
pub(crate) fn observe_reasoning_chunk(chunk: &StreamChunk, flag: &std::sync::atomic::AtomicBool) {
    if matches!(chunk, StreamChunk::ThinkingToken(_)) {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Decides whether the runtime backstop should mark a built-in model as
/// always-reasoning. True only when reasoning was requested OFF (`!think`) yet
/// the model still streamed reasoning (`reasoning_seen`), the manifest does not
/// already record it as always (`!current_reasoning_always`), and the model is
/// not a curated starter (`!is_curated`, whose class is registry truth and must
/// never be overridden from behavior).
pub(crate) fn should_backstop_mark(
    think: bool,
    reasoning_seen: bool,
    current_reasoning_always: bool,
    is_curated: bool,
) -> bool {
    !think && reasoning_seen && !current_reasoning_always && !is_curated
}

/// Best-effort runtime backstop for the built-in engine: when a chat streamed
/// reasoning while reasoning was OFF, persist `reasoning_always` so the picker
/// badge and `/think` gate self-correct on the next read. Coverage-off: the
/// decision lives in [`should_backstop_mark`]; this wrapper only reads the row
/// and writes the flag. Never fails the turn (every error is logged and
/// swallowed).
#[cfg_attr(coverage_nightly, coverage(off))]
fn backstop_mark_reasoning_always(
    db: &crate::history::Database,
    model_id: &str,
    think: bool,
    reasoning_seen: bool,
) {
    // Cheap exit before locking: only an OFF request that still saw reasoning
    // can change anything.
    if think || !reasoning_seen {
        return;
    }
    let Ok(conn) = db.0.lock() else { return };
    let Ok(Some(row)) = crate::models::manifest::get(&conn, model_id) else {
        return;
    };
    let is_curated = crate::models::curated_reasoning_flags(&row.repo, &row.file_name).is_some();
    if should_backstop_mark(think, reasoning_seen, row.reasoning_always, is_curated) {
        match crate::models::manifest::mark_reasoning_always(&conn, model_id) {
            Ok(()) => {
                eprintln!("thuki: [models] reasoning backstop: marked {model_id} always-reasoning")
            }
            Err(e) => {
                eprintln!("thuki: [models] reasoning backstop: failed to mark {model_id}: {e}")
            }
        }
    }
}

/// Reads the API key for an `openai`-kind provider from the secret store.
/// Errors degrade to `None` with a stderr log: a missing or unreadable key
/// must not block a keyless local `/v1` server.
pub(crate) fn resolve_provider_api_key(
    store: &dyn crate::keychain::SecretStore,
    provider_id: Option<&str>,
) -> Option<String> {
    let id = provider_id?;
    match store.get(id) {
        Ok(key) => key,
        Err(e) => {
            eprintln!("thuki: [keychain] failed to read the api key for provider '{id}': {e}");
            None
        }
    }
}

/// How LLM calls reach the active provider, decided once per pipeline turn.
///
/// Downstream of [`ChatRoute`]: the route names the provider kind, the
/// transport is the resolved wire target. `Builtin` routes collapse into
/// `V1` here because once the engine sidecar is serving, it is just another
/// keyless OpenAI-compatible server at a loopback port.
#[derive(Clone, PartialEq)]
pub enum LlmTransport {
    /// Native Ollama `/api/chat` at the provider's base URL.
    OllamaNative {
        /// Full `<base>/api/chat` endpoint.
        endpoint: String,
    },
    /// Generic OpenAI-compatible `/v1` server: an `openai`-kind provider
    /// (key already resolved) or the built-in engine (keyless, engine port).
    V1 {
        base_url: String,
        api_key: Option<String>,
        /// Which `/v1` flavor this transport targets, decided where the
        /// provider kind is known so downstream error copy matches the
        /// provider (builtin vs remote).
        flavor: crate::openai::V1Flavor,
    },
}

impl LlmTransport {
    /// Human-readable endpoint label for forensic trace records.
    pub fn endpoint_label(&self) -> String {
        match self {
            LlmTransport::OllamaNative { endpoint } => endpoint.clone(),
            LlmTransport::V1 { base_url, .. } => format!("{base_url}/v1/chat/completions"),
        }
    }
}

impl std::fmt::Debug for LlmTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmTransport::OllamaNative { endpoint } => f
                .debug_struct("OllamaNative")
                .field("endpoint", endpoint)
                .finish(),
            LlmTransport::V1 {
                base_url,
                api_key,
                flavor,
            } => f
                .debug_struct("V1")
                .field("base_url", base_url)
                .field("api_key", &api_key.as_ref().map(|_| "<redacted>"))
                .field("flavor", flavor)
                .finish(),
        }
    }
}

/// Picks the model slug for a pipeline turn. `Builtin` routes carry their
/// model in the provider config (already validated non-empty by
/// `resolve_chat_route`); every other kind keeps the picker-backed fallback
/// whose `None` means "no model selected".
///
/// Used by both the search pipeline and title generation so the selection
/// logic stays in one place.
pub fn model_for_route(route: &ChatRoute, fallback: Option<String>) -> Option<String> {
    match route {
        ChatRoute::Builtin { model_id } => Some(model_id.clone()),
        _ => fallback,
    }
}

/// Acquires an engine activity guard when (and only when) the route targets
/// the built-in engine. The caller holds the returned guard across every LLM
/// call of the turn (the search pipeline issues several with gaps between
/// them; title generation issues one) so the idle sweep treats the whole
/// turn as continuous activity. Non-builtin routes get `None`: they must not
/// pin a possibly-loaded sidecar in memory.
pub(crate) fn route_activity_guard(
    route: &ChatRoute,
    engine: &crate::engine::runner::EngineHandle,
) -> Option<crate::engine::runner::ActivityGuard> {
    matches!(route, ChatRoute::Builtin { .. }).then(|| engine.activity_guard())
}

/// How [`resolve_llm_transport`] responds when the pre-load memory gate (issue
/// #296) judges the built-in model too large for the memory available now.
///
/// The gate is shared by two builtin callers with opposite needs, so the
/// response is the caller's to choose:
/// - `/search` is user-initiated inference, so it must surface the same
///   user-facing "insufficient memory" error the chat path shows, with a
///   `forced` escape hatch for the user's explicit "load anyway".
/// - Background history title generation must never surface an error; an
///   over-large model simply skips the title.
///
/// Only ever consulted on the built-in arm; Ollama and OpenAI-compatible
/// routes carry no local memory footprint and ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OversizePolicy {
    /// Refuse an over-large load with the user-facing insufficient-memory
    /// error. `forced` is the user's "load anyway" and bypasses the gate.
    ///
    /// Production `ask_model` runs the same gate inline via
    /// `preflight_memory_gate` + `allow_oversized`. This variant remains for
    /// [`resolve_llm_transport`]'s unit tests (and any future non-command
    /// caller that wants the user-facing block path); the only live production
    /// constructor today is [`SilentSkip`] from background title generation.
    #[allow(dead_code)]
    Block {
        /// Whether the user explicitly opted to load the over-large model.
        forced: bool,
    },
    /// Skip an over-large load silently, yielding
    /// [`TransportError::SkippedInsufficientMemory`] for the caller to treat as
    /// a benign no-op rather than an error.
    SilentSkip,
}

/// Error from [`resolve_llm_transport`]. Splits the engine-ensure outcomes so
/// each caller can map them into its own vocabulary: `Cancelled` and
/// `Superseded` are cancellations (the user stopped the turn, or a newer
/// settings change preempted the request; never failures), `Engine` carries
/// a typed user-facing error, and `SkippedInsufficientMemory` is a benign skip
/// signal (never surfaced to the user).
#[derive(Debug, PartialEq)]
pub enum TransportError {
    /// The caller's cancel token fired while the engine ensure was in flight.
    Cancelled,
    /// A newer settings change preempted the engine ensure.
    Superseded,
    /// A typed engine error (start failure, missing manifest row, ...).
    Engine(EngineError),
    /// The memory gate blocked an over-large built-in load under
    /// [`OversizePolicy::SilentSkip`]. Not a failure: a background caller
    /// (history title generation) treats it as "skip this optional work",
    /// never as a user-facing error.
    SkippedInsufficientMemory,
}

/// Resolves a [`ChatRoute`] into the [`LlmTransport`] a pipeline turn streams
/// through. `OllamaNative` passes through; `V1` resolves the provider's API
/// key; `Builtin` maps the manifest row to an engine [`Target`], marks
/// activity, and ensures the sidecar serves it, yielding a keyless `V1`
/// transport at the engine's loopback port.
///
/// `num_ctx` is consumed only by the builtin arm: the context size is a
/// launch property of the llama-server process, not a per-request knob.
/// `cancel_token` is also builtin-only: the ensure is raced against it so a
/// Stop press during a cold model load ends the turn immediately (the load
/// continues in the background and the next request reuses it). Callers with
/// no cancel affordance pass a fresh, never-cancelled token.
///
/// `policy` is the builtin-only pre-load memory gate response (issue #296),
/// run BEFORE the sidecar loads so an over-large model never cold-loads and
/// freezes the machine. On the gate's `Block` outcome the response depends on
/// `policy`: [`OversizePolicy::Block`] (unforced) yields the user-facing
/// insufficient-memory error, [`OversizePolicy::Block`] with `forced` proceeds
/// to load anyway, and [`OversizePolicy::SilentSkip`] yields
/// [`TransportError::SkippedInsufficientMemory`] for a background caller to
/// swallow. When the model fits, or the exact model is already resident, the
/// load proceeds exactly as before. Non-builtin routes ignore `policy`.
///
/// [`Target`]: crate::engine::state::Target
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_llm_transport(
    route: ChatRoute,
    db: &crate::history::Database,
    store: &crate::models::storage::ModelStore,
    engine: &crate::engine::runner::EngineHandle,
    secrets: &dyn crate::keychain::SecretStore,
    num_ctx: u32,
    cancel_token: &CancellationToken,
    policy: OversizePolicy,
) -> Result<LlmTransport, TransportError> {
    match route {
        ChatRoute::OllamaNative { endpoint } => Ok(LlmTransport::OllamaNative { endpoint }),
        ChatRoute::V1 {
            base_url,
            api_key_provider,
        } => Ok(LlmTransport::V1 {
            base_url,
            api_key: resolve_provider_api_key(secrets, api_key_provider.as_deref()),
            flavor: crate::openai::V1Flavor::Remote,
        }),
        ChatRoute::Builtin { model_id } => {
            // Resolve the manifest row and run the pre-load memory gate inside a
            // single scope so the connection guard drops before the ensure
            // await. `builtin_target` runs first so a missing/unreadable row
            // still surfaces its typed error before the gate. A poisoned lock is
            // recovered: the connection itself is not invalidated by an
            // unrelated panic. Only `Block { forced: true }` is a forced load.
            let forced = matches!(policy, OversizePolicy::Block { forced: true });
            let (target, gate) = {
                let conn = match db.0.lock() {
                    Ok(conn) => conn,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let target = builtin_target(&conn, store, &model_id, num_ctx)
                    .map_err(TransportError::Engine)?;
                let gate = preflight_memory_gate(
                    &conn,
                    store,
                    engine,
                    &model_id,
                    &target.model_path,
                    forced,
                );
                (target, gate)
            };
            if let crate::models::memory::MemoryGate::Block {
                required_bytes,
                available_bytes,
            } = gate
            {
                // Reached only for `Block { forced: false }` and `SilentSkip`:
                // a forced load resolves to `Proceed` in the gate above.
                return match policy {
                    OversizePolicy::Block { .. } => Err(TransportError::Engine(
                        insufficient_memory_error(required_bytes, available_bytes),
                    )),
                    OversizePolicy::SilentSkip => Err(TransportError::SkippedInsufficientMemory),
                };
            }
            engine.touch();
            // Race the ensure against the caller's cancel token, mirroring
            // `stream_builtin_chat`: the load is not aborted, only this
            // turn's wait for it.
            let ensured = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => None,
                result = engine.ensure_loaded(target) => Some(result),
            };
            match ensured {
                None => Err(TransportError::Cancelled),
                Some(Ok(port)) => Ok(LlmTransport::V1 {
                    base_url: format!("http://127.0.0.1:{port}"),
                    api_key: None,
                    flavor: crate::openai::V1Flavor::Builtin,
                }),
                Some(Err(crate::engine::runner::EnsureError::Superseded)) => {
                    Err(TransportError::Superseded)
                }
                Some(Err(crate::engine::runner::EnsureError::StartFailed(detail))) => {
                    Err(TransportError::Engine(engine_start_error(&detail)))
                }
            }
        }
    }
}

/// Pulls the human-readable reason out of an Ollama error payload. Ollama
/// returns `{"error":"..."}` on every non-2xx status from `/api/chat`; when
/// the body is empty, malformed, or missing the `error` key we return
/// `None` so the caller can fall back to the bare status code.
pub fn extract_ollama_error_message(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Maps an HTTP status code (plus the response body for non-404 paths) to a
/// user-friendly `EngineError`. The `model_name` is woven into the
/// `ModelNotFound` hint so the user sees the exact command to run; for every
/// other status we surface the concrete reason Ollama returned (e.g. "this
/// model only supports one image while more than one image requested") so
/// the user can act on it instead of staring at a bare HTTP code.
pub fn classify_http_error(status: u16, model_name: &str, body: &str) -> EngineError {
    match status {
        404 => EngineError {
            kind: EngineErrorKind::ModelNotFound,
            message: format!("Model not found\nRun: ollama pull {model_name} in a terminal."),
        },
        _ => {
            let detail =
                extract_ollama_error_message(body).unwrap_or_else(|| format!("HTTP {status}"));
            // Backend filter is best-effort: if the capability cache lied
            // (e.g. user pulled a re-tagged variant we have not refreshed)
            // and Ollama still rejects on image/vision grounds, point the
            // user at the picker instead of letting them stare at a raw
            // upstream string. Substring check is intentionally loose so
            // we catch the half-dozen phrasings Ollama uses across model
            // families ("does not support images", "vision capability
            // required", "only supports one image", ...).
            let lower = body.to_ascii_lowercase();
            let mentions_image_or_vision = lower.contains("image") || lower.contains("vision");
            let message = if mentions_image_or_vision {
                format!(
                    "Something went wrong\n{detail}\nTry switching to a vision model from the picker chip."
                )
            } else {
                format!("Something went wrong\n{detail}")
            };
            EngineError {
                kind: EngineErrorKind::Other,
                message,
            }
        }
    }
}

/// Maps a reqwest connection/transport error to a user-friendly `EngineError`.
pub fn classify_stream_error(e: &reqwest::Error) -> EngineError {
    if e.is_connect() || e.is_timeout() {
        EngineError {
            kind: EngineErrorKind::EngineUnreachable,
            message: "Ollama isn't running\nStart Ollama and try again.".to_string(),
        }
    } else {
        EngineError {
            kind: EngineErrorKind::Other,
            message: "Something went wrong\nCould not reach Ollama.".to_string(),
        }
    }
}

/// Payload emitted back to the frontend per token chunk.
#[derive(Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum StreamChunk {
    /// A single token chunk string.
    Token(String),
    /// A single thinking/reasoning token chunk string.
    ThinkingToken(String),
    /// Indicates the stream has fully completed.
    Done,
    /// The user explicitly cancelled generation.
    Cancelled,
    /// A structured, user-friendly error occurred during processing.
    Error(EngineError),
    /// Emitted exactly once per turn, after the backend has cleared every
    /// pre-`ConversationStart` gate (no-model bail, model lookup, etc.) and
    /// committed to opening the trace for this `conversation_id`. Carries
    /// no payload; the frontend uses it as the unambiguous signal to
    /// retire its `is_first_turn` flag without relying on token-arrival
    /// ordering. Does not appear in the trace itself.
    TurnAccepted,
    /// Progress of the invisible web-search pipeline, streamed before any answer
    /// token so the UI can show what the model is doing on the warm slot.
    SearchStatus {
        phase: crate::websearch::orchestrator::SearchPhase,
    },
    /// The resolved citation sources for a source-grounded answer, emitted once
    /// just before the answer tokens. Never emitted on a plain (non-search) turn.
    SearchSources(Vec<SourceMeta>),
    /// A wanted web search produced no citable answer. Emitted once, before the
    /// (parametric-knowledge) answer tokens, so the frontend can show a reliable
    /// failure note that does not depend on the model's own prose. `reason`
    /// serializes as `"unreachable"` or `"no_results"` (see
    /// [`crate::websearch::orchestrator::SearchFailReason`]). Carries no engine
    /// names, URLs, or error text: user-facing signal only.
    SearchFailed {
        reason: crate::websearch::orchestrator::SearchFailReason,
    },
    /// Replace the assistant bubble's full answer text. Used after live
    /// streaming when citation repair or strip produces a different final body.
    SetContent(String),
}

/// Citation metadata for one resolved web source, sent to the frontend to
/// render the numbered sources list. The source's extracted text stays
/// server-side (it lives only in the writer prompt), so only the citation index,
/// origin, and any required licence attribution cross the IPC boundary.
#[derive(Clone, Serialize)]
pub struct SourceMeta {
    pub index: usize,
    pub url: String,
    pub title: String,
    /// Optional markdown attribution line (licence / provider credit). Present
    /// for verticals that require a user-visible hyperlink (Open-Meteo, Wikipedia).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
}

/// Licence / provider credit for known attribution-required verticals, keyed by
/// registration host. Markdown so the Sources footer can render real hyperlinks.
pub(crate) fn source_attribution_for_url(url: &str) -> Option<String> {
    let domain = crate::websearch::domain_of(url);
    if domain == "open-meteo.com" || domain.ends_with(".open-meteo.com") {
        return Some(crate::websearch::OPEN_METEO_ATTRIBUTION.to_string());
    }
    if domain == "wikipedia.org" || domain.ends_with(".wikipedia.org") {
        return Some(crate::websearch::WIKIPEDIA_ATTRIBUTION.to_string());
    }
    None
}

/// Projects the assembled source blocks to the citation metadata the UI needs.
pub(crate) fn source_metas(blocks: &[crate::websearch::assemble::SourceBlock]) -> Vec<SourceMeta> {
    blocks
        .iter()
        .map(|block| SourceMeta {
            index: block.index,
            url: block.url.clone(),
            title: block.title.clone(),
            attribution: source_attribution_for_url(&block.url),
        })
        .collect()
}

/// A single message in the Ollama `/api/chat` conversation format.
///
/// The optional `images` field carries base64-encoded image data for
/// multimodal models. When absent or empty, the message is text-only.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
}

/// Sampling parameters for Ollama `/api/chat`, following Google's recommended
/// configuration for Gemma4 models.
#[derive(Serialize)]
struct OllamaOptions {
    temperature: f64,
    top_p: f64,
    top_k: u32,
    /// Context window size. Must match the warmup request so Ollama reuses
    /// the same runner instance and its cached KV prefix for the system prompt.
    num_ctx: u32,
}

/// Request payload for Ollama `/api/chat` endpoint.
#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    think: bool,
    options: OllamaOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_alive: Option<String>,
}

/// Groups the per-request parameters for `stream_ollama_chat` to keep the
/// function signature within clippy's argument-count limit.
pub struct OllamaChatParams {
    pub endpoint: String,
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub think: bool,
    pub keep_alive: Option<String>,
    /// Context window size in tokens. Must match the warmup request so Ollama
    /// reuses the same runner instance and its cached KV prefix.
    pub num_ctx: u32,
}

/// Nested message object in Ollama `/api/chat` response chunks.
#[derive(Deserialize)]
struct OllamaChatResponseMessage {
    content: Option<String>,
    thinking: Option<String>,
}

/// Expected structured response chunk from Ollama `/api/chat`.
#[derive(Deserialize)]
struct OllamaChatResponse {
    message: Option<OllamaChatResponseMessage>,
    done: Option<bool>,
}

/// Holds the active cancellation token for the current generation request.
///
/// Only one generation runs at a time - starting a new request replaces the
/// previous token. `cancel_generation` cancels whatever is currently active.
#[derive(Default)]
pub struct GenerationState {
    token: Mutex<Option<CancellationToken>>,
}

impl GenerationState {
    /// Creates a new empty generation state with no active token.
    pub fn new() -> Self {
        Self {
            token: Mutex::new(None),
        }
    }

    /// Stores a new cancellation token, replacing any previous one.
    pub fn set_token(&self, token: CancellationToken) {
        *self.token.lock().unwrap() = Some(token);
    }

    /// Cancels the active generation, if any, and clears the stored token.
    pub fn cancel(&self) {
        if let Some(token) = self.token.lock().unwrap().take() {
            token.cancel();
        }
    }

    /// Clears the stored token without cancelling it (used on natural completion).
    pub fn clear_token(&self) {
        *self.token.lock().unwrap() = None;
    }
}

/// Backend-managed conversation history with an epoch counter to prevent
/// stale writes after a reset. The Rust side is the source of truth; the
/// frontend sends only new user messages and receives streamed tokens.
pub struct ConversationHistory {
    pub messages: Mutex<Vec<ChatMessage>>,
    pub epoch: AtomicU64,
}

impl Default for ConversationHistory {
    fn default() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            epoch: AtomicU64::new(0),
        }
    }
}

impl ConversationHistory {
    /// Creates a new empty conversation history at epoch 0.
    pub fn new() -> Self {
        Self::default()
    }
}

// `get_config` lives in `crate::settings_commands` so all configuration-touching
// commands share one module. The Settings panel uses the same command via
// `invoke('get_config')`; this is the single source of truth across the app.

/// Core streaming logic for Ollama `/api/chat`, separated from the Tauri
/// command for testability. Uses `tokio::select!` to race each chunk read
/// against the cancellation token, ensuring the HTTP connection is dropped
/// immediately when the user cancels - which signals Ollama to stop inference.
/// Returns the accumulated assistant response so the caller can persist it.
pub async fn stream_ollama_chat(
    params: OllamaChatParams,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    on_chunk: impl Fn(StreamChunk),
) -> String {
    let OllamaChatParams {
        endpoint,
        model,
        messages,
        think,
        keep_alive,
        num_ctx,
    } = params;
    let request_payload = OllamaChatRequest {
        model,
        messages,
        stream: true,
        think,
        options: OllamaOptions {
            temperature: 1.0,
            top_p: 0.95,
            top_k: 64,
            num_ctx,
        },
        keep_alive,
    };

    let mut accumulated = String::new();
    // Tracks whether a terminal Done was already emitted, so the stream-end
    // branch can emit one when Ollama closes without a done:true line without
    // double-emitting on the normal completion path.
    let mut done_emitted = false;

    let res = client.post(endpoint).json(&request_payload).send().await;

    match res {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status().as_u16();
                // Drain the body so the user sees Ollama's own reason
                // (e.g. "this model only supports one image while more
                // than one image requested") instead of a bare HTTP code.
                // A failed read collapses to an empty string and the
                // classifier falls back to the status code.
                let body = response.text().await.unwrap_or_default();
                on_chunk(StreamChunk::Error(classify_http_error(
                    status,
                    &request_payload.model,
                    &body,
                )));
                return accumulated;
            }

            let mut stream = response.bytes_stream();
            let mut buffer: Vec<u8> = Vec::new();

            loop {
                tokio::select! {
                    biased;
                    _ = cancel_token.cancelled() => {
                        // Drop the stream - closes the HTTP connection,
                        // which signals Ollama to stop inference.
                        drop(stream);
                        on_chunk(StreamChunk::Cancelled);
                        return accumulated;
                    }
                    chunk_opt = stream.next() => {
                        match chunk_opt {
                            Some(Ok(bytes)) => {
                                buffer.extend_from_slice(&bytes);

                                while let Some(idx) = buffer.iter().position(|&b| b == b'\n') {
                                    let line_bytes = buffer.drain(..=idx).collect::<Vec<u8>>();
                                    if let Ok(line_text) = String::from_utf8(line_bytes) {
                                        let trimmed = line_text.trim();
                                        if trimmed.is_empty() {
                                            continue;
                                        }

                                        if let Ok(json) =
                                            serde_json::from_str::<OllamaChatResponse>(trimmed)
                                        {
                                            if let Some(ref msg) = json.message {
                                                if let Some(ref thinking) = msg.thinking {
                                                    if !thinking.is_empty() {
                                                        on_chunk(StreamChunk::ThinkingToken(
                                                            thinking.clone(),
                                                        ));
                                                    }
                                                }
                                                if let Some(ref token) = msg.content {
                                                    if !token.is_empty() {
                                                        accumulated.push_str(token);
                                                        on_chunk(StreamChunk::Token(
                                                            token.clone(),
                                                        ));
                                                    }
                                                }
                                            }
                                            if let Some(true) = json.done {
                                                on_chunk(StreamChunk::Done);
                                                done_emitted = true;
                                            }
                                        }
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                on_chunk(StreamChunk::Error(classify_stream_error(&e)));
                                return accumulated;
                            }
                            None => {
                                // Ollama can drop the stream without its
                                // terminal done:true line (e.g. a small model
                                // degenerating on a long repetitive token run
                                // and the runner closing the connection). Emit
                                // a terminal Done so the frontend always leaves
                                // its streaming state instead of spinning
                                // forever on the missing terminal event.
                                if !done_emitted {
                                    on_chunk(StreamChunk::Done);
                                }
                                return accumulated;
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            on_chunk(StreamChunk::Error(classify_stream_error(&e)));
        }
    }

    accumulated
}

/// Mirrors a streaming chunk into the chat-domain trace recorder. Pulled out
/// of [`ask_model`] so the per-token routing logic and the token-count
/// increment are exercised by the unit-test suite rather than the
/// coverage-off Tauri command body. `Done`, `Cancelled`, and `Error` chunks
/// are intentionally noops here: those terminal events are summarized by
/// `AssistantComplete` after the stream returns.
pub(crate) fn record_chunk_to_trace(
    chunk: &StreamChunk,
    recorder: &std::sync::Arc<crate::trace::BoundRecorder>,
    token_count: &AtomicU64,
) {
    match chunk {
        StreamChunk::Token(text) => {
            token_count.fetch_add(1, Ordering::Relaxed);
            recorder.record(crate::trace::RecorderEvent::AssistantTokens {
                chunk: text.clone(),
            });
        }
        StreamChunk::ThinkingToken(text) => {
            recorder.record(crate::trace::RecorderEvent::AssistantThinking {
                chunk: text.clone(),
            });
        }
        StreamChunk::SetContent(text) => {
            // One synthetic chunk for the replacement body (not per-token).
            token_count.fetch_add(1, Ordering::Relaxed);
            recorder.record(crate::trace::RecorderEvent::AssistantTokens {
                chunk: text.clone(),
            });
        }
        StreamChunk::Done
        | StreamChunk::Cancelled
        | StreamChunk::Error(_)
        | StreamChunk::TurnAccepted
        | StreamChunk::SearchStatus { .. }
        | StreamChunk::SearchSources(_)
        // A UI-only failure signal: the search outcome is already recorded to
        // the trace by the pipeline (SearchRetrieved/SearchSkipped), so this
        // chunk contributes no token or trace event of its own.
        | StreamChunk::SearchFailed { .. } => {}
    }
}

/// Emits `ConversationStart` to the trace recorder iff this is the first
/// turn of the conversation. Pulled out of [`ask_model`] and the search
/// pipeline so the gate is covered by tests instead of the coverage-off
/// Tauri command body.
pub(crate) fn record_conversation_start_if_first_turn(
    recorder: &std::sync::Arc<crate::trace::BoundRecorder>,
    is_first_turn: bool,
    model: String,
    system_prompt: String,
) {
    if is_first_turn {
        recorder.record(crate::trace::RecorderEvent::ConversationStart {
            model,
            system_prompt,
        });
    }
}

/// Streams a chat response from the local Ollama backend. Appends the user
/// message and assistant response to conversation history after completion
/// or cancellation (retaining context for follow-up requests). Uses an epoch
/// counter to prevent stale writes after a reset.
///
/// `conversation_id` flows from the frontend (`useConversationHistory.ts`).
/// `is_first_turn` lets the frontend tell the backend "emit
/// `ConversationStart` before this turn's `UserMessage`" without the backend
/// needing to track per-conversation state. Both feed the unified trace
/// recorder when `[debug] trace_enabled = true`; off by default they collapse
/// to noop calls.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
#[allow(clippy::too_many_arguments)]
pub async fn ask_model(
    message: String,
    quoted_text: Option<String>,
    image_paths: Option<Vec<String>>,
    think: bool,
    conversation_id: String,
    is_first_turn: bool,
    slash_command: Option<String>,
    // The user's "load anyway" override for the pre-load memory gate (issue
    // #296). `Some(true)` bypasses the block; `None`/`Some(false)` (the default,
    // and what existing frontend invokes that omit the field deserialize to)
    // leaves the gate active.
    allow_oversized: Option<bool>,
    // When true, force built-in engines-only web search (`/search` alias).
    // Non-builtin providers receive a capability error below.
    force_search: bool,
    on_event: Channel<StreamChunk>,
    client: State<'_, reqwest::Client>,
    generation: State<'_, GenerationState>,
    history: State<'_, ConversationHistory>,
    config: State<'_, parking_lot::RwLock<AppConfig>>,
    active_model: State<'_, crate::models::ActiveModelState>,
    capabilities_cache: State<'_, ModelCapabilitiesCache>,
    trace_recorder: State<'_, std::sync::Arc<crate::trace::LiveTraceRecorder>>,
    db: State<'_, crate::history::Database>,
    model_store: State<'_, crate::models::storage::ModelStore>,
    engine: State<'_, crate::engine::runner::EngineHandle>,
    secrets: State<'_, crate::keychain::Secrets>,
    app: tauri::AppHandle,
    warm_state: State<'_, crate::warmup::BuiltinWarmState>,
) -> Result<(), String> {
    // Snapshot the config once so all downstream reads (endpoint, prompt, model)
    // see a consistent view even if the user edits Settings mid-stream.
    let config = config.read().clone();

    // Route by the active provider's kind: native Ollama, the built-in
    // engine, or a generic OpenAI-compatible server. The decision is made
    // once here; the streaming dispatch below consumes it.
    let route = match resolve_chat_route(&config.inference) {
        Ok(route) => route,
        Err(err) => {
            let _ = on_event.send(StreamChunk::Error(err));
            return Ok(());
        }
    };

    // `/search` force-search is built-in only (same residency premise as auto-search).
    if force_search && !matches!(route, ChatRoute::Builtin { .. }) {
        let _ = on_event.send(StreamChunk::Error(EngineError {
            kind: EngineErrorKind::Other,
            message: "Web search needs the built-in engine\nSwitch to Built-in in Settings to use /search."
                .to_string(),
        }));
        return Ok(());
    }

    // Snapshot the picker-backed active model; drop the guard before any
    // `.await`. It is only a fallback: `Builtin` routes carry their model in
    // the provider config (kept fresh by `persist_active_provider_model`), so
    // a builtin chat must not depend on this snapshot.
    let snapshot = {
        let guard = active_model.0.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };
    let Some(model_name) = model_for_route(&route, snapshot) else {
        // Defense in depth: the onboarding gate already refuses to open the
        // overlay without a selected model, so this branch only fires if the
        // user removed their last installed model with `ollama rm` between
        // launches and the picker hasn't been opened yet. Surface a typed
        // error so the frontend can route the user to the picker.
        let _ = on_event.send(StreamChunk::Error(no_model_selected_error()));
        return Ok(());
    };
    let cancel_token = CancellationToken::new();
    generation.set_token(cancel_token.clone());

    // Bind the trace recorder to this conversation. When tracing is on,
    // every event for this turn flows to
    // `traces/chat/<conversation_id>.jsonl` via the registry. When off,
    // each `record()` is a constant-time noop. The bound recorder is
    // cheap to clone and is captured by the streaming-pump closure so
    // per-token emits skip the registry lookup on the hot path.
    let live: std::sync::Arc<crate::trace::LiveTraceRecorder> =
        std::sync::Arc::clone(trace_recorder.inner());
    let live_inner: std::sync::Arc<dyn crate::trace::TraceRecorder> = live;
    let bound_recorder = std::sync::Arc::new(crate::trace::BoundRecorder::new(
        live_inner,
        crate::trace::ConversationId::new(conversation_id),
    ));

    // Emit ConversationStart at the moment we know the model + resolved
    // system prompt. The frontend's `is_first_turn` flag prevents this
    // event from firing on subsequent turns of the same conversation.
    record_conversation_start_if_first_turn(
        &bound_recorder,
        is_first_turn,
        model_name.clone(),
        config.prompt.resolved_system.clone(),
    );
    // Tell the frontend the trace was opened for this conversation_id.
    // Sent unconditionally (regardless of `is_first_turn`) so the hook
    // can retire its flag the moment ANY turn lands, even if a previous
    // first-turn attempt was cancelled before any token arrived.
    let _ = on_event.send(StreamChunk::TurnAccepted);

    // Snapshot the raw typed message before it is (possibly) moved into the
    // quote-wrapped `content` below, for the place-time clock resolution:
    // that check needs the user's bare turn, never the "[Highlighted
    // Text]..." wrapper, which would never pass `clock_question_place`'s
    // high-precision gate anyway.
    let clock_probe = message.clone();

    // Build user message content.  When quoted text is present, label it
    // explicitly so the model knows the highlighted text is the primary
    // subject and any attached images provide surrounding context.
    let content = match quoted_text {
        Some(ref qt) if !qt.trim().is_empty() => {
            format!("[Highlighted Text]\n\"{}\"\n\n[Request]\n{}", qt, message)
        }
        _ => message,
    };

    // Emit UserMessage before any image base64 work, so the trace
    // captures the user's intent even if encoding fails. Image paths
    // are recorded as strings (matching the IPC contract); image bytes
    // never enter the JSONL.
    bound_recorder.record(crate::trace::RecorderEvent::UserMessage {
        content: content.clone(),
        attached_images: image_paths.clone().unwrap_or_default(),
        slash_command: slash_command.clone(),
    });

    // Whether this turn attaches images. Non-vision image turns skip search
    // (capability strip path); vision turns enter the pipeline with pixels on
    // the classifier and writer. Capture before `image_paths` moves.
    let turn_has_images = image_paths.as_ref().is_some_and(|paths| !paths.is_empty());

    // The `/search` slash command forces a web search this turn (see
    // `run_builtin_search`'s `force_search`), turning the invisible auto-search
    // into an always-on, cache-bypassing search with the same "look it up again"
    // semantics. Transform utilities (`slash_skips_auto_search`) never enter the
    // pipeline. `/explain` and `/think` still honor Auto search Settings.
    let force_search = slash_command.as_deref() == Some("/search");
    let skip_search = slash_skips_auto_search(slash_command.as_deref());

    // Base64-encode attached images for the Ollama multimodal API.
    let images = match image_paths {
        Some(ref paths) if !paths.is_empty() => {
            Some(crate::images::encode_images_as_base64(paths)?)
        }
        _ => None,
    };

    let user_msg = ChatMessage {
        role: "user".to_string(),
        content,
        images,
    };

    // Deterministic place-time resolution for a clock question naming a
    // place ("what time is it in San Francisco"): resolves the place's IANA
    // timezone via geocoding and computes its current wall-clock time in
    // code (see `resolve_clock_place_time` / `websearch::clock`), so the
    // model performs zero timezone arithmetic itself. Runs before the
    // route/search branch below so every provider (built-in, Ollama,
    // OpenAI-compatible) sees the same resolved line; it is a pure geocode
    // + system-tzdata computation, never a model call, and never a reason
    // to search the web.
    let place_time_line = resolve_clock_place_time(&clock_probe).await;

    // Snapshot the current epoch and build the messages array for Ollama.
    // The user message is NOT yet committed to history - it is only added
    // after a response (including partial/cancelled) to prevent orphaned
    // messages on errors. The system message carries the current
    // local-datetime context (see `system_prompt_with_datetime`), captured
    // once here so every downstream call this turn (plain stream, search
    // writer, unreachable disclosure) sees byte-identical text.
    let (epoch_at_start, mut messages) = {
        let conv = history.messages.lock().unwrap();
        let epoch = history.epoch.load(Ordering::SeqCst);
        let mut msgs = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt_with_datetime(
                &config.prompt.resolved_system,
                &local_datetime_context(),
                place_time_line.as_deref(),
            ),
            images: None,
        }];
        msgs.extend(conv.clone());
        msgs.push(user_msg.clone());
        (epoch, msgs)
    };

    // Per-request capability filter. The snapshot is the working copy;
    // stored history (`conv`) is never mutated. On a cache miss we leave
    // the payload untouched and trust Ollama to surface a structured error
    // through `classify_http_error`'s picker hint, which the user can act on.
    let provider_id = config.inference.active_provider.clone();
    let cache_hit = capabilities_cache
        .0
        .lock()
        .ok()
        .and_then(|guard| guard.get(&(provider_id, model_name.clone())).cloned());
    if let Some(caps) = cache_hit {
        let stats = apply_capability_filter(&mut messages, &caps);
        if stats.stripped_images > 0 {
            eprintln!(
                "thuki: [capability filter] model={} stripped_images={}",
                model_name, stats.stripped_images
            );
        }
    } else {
        eprintln!(
            "thuki: [capability filter] cache miss for model={}, sending payload as-is",
            model_name
        );
    }

    let keep_alive = if config.inference.keep_warm_inactivity_minutes == 0 {
        None
    } else {
        Some(crate::warmup::keep_alive_string(
            config.inference.keep_warm_inactivity_minutes,
        ))
    };

    let stream_started_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let token_count_atomic = std::sync::Arc::new(AtomicU64::new(0));
    let token_count_for_pump = std::sync::Arc::clone(&token_count_atomic);
    let recorder_for_pump = std::sync::Arc::clone(&bound_recorder);

    // Mirror every user-visible chunk into the trace before forwarding it
    // to the frontend. Token / ThinkingToken chunks land as discrete trace
    // events; terminal chunks are summarized below by `AssistantComplete`.
    // Captures by reference only, so the closure is Copy and each route arm
    // can consume it.
    let pump = |chunk: StreamChunk| {
        record_chunk_to_trace(&chunk, &recorder_for_pump, &token_count_for_pump);
        let _ = on_event.send(chunk);
    };

    // Every arm returns the accumulated assistant content (empty when the
    // turn ended in a pre-stream error), so the persistence tail below is
    // identical for all three routes.
    let accumulated = match route {
        ChatRoute::OllamaNative { endpoint } => {
            stream_ollama_chat(
                OllamaChatParams {
                    endpoint,
                    model: model_name,
                    messages,
                    think,
                    keep_alive,
                    num_ctx: config.inference.num_ctx,
                },
                &client,
                cancel_token.clone(),
                pump,
            )
            .await
        }
        ChatRoute::Builtin { model_id } => {
            // Resolve the manifest row to blob-store paths and run the pre-load
            // memory gate inside a scope so the connection guard drops before
            // any `.await`. The gate (issue #296) refuses an un-forced load
            // whose estimated footprint does not fit the memory available now;
            // `allow_oversized == Some(true)` is the user's "load anyway".
            let resolved = {
                let conn = db.0.lock().map_err(|e| e.to_string())?;
                builtin_target(&conn, &model_store, &model_id, config.inference.num_ctx).map(
                    |target| {
                        let gate = preflight_memory_gate(
                            &conn,
                            &model_store,
                            &engine,
                            &model_id,
                            &target.model_path,
                            allow_oversized == Some(true),
                        );
                        (target, gate)
                    },
                )
            };
            match resolved {
                Ok((
                    _,
                    crate::models::memory::MemoryGate::Block {
                        required_bytes,
                        available_bytes,
                    },
                )) => {
                    pump(StreamChunk::Error(insufficient_memory_error(
                        required_bytes,
                        available_bytes,
                    )));
                    String::new()
                }
                Ok((target, crate::models::memory::MemoryGate::Proceed)) => {
                    // Hold an activity guard across the search pre-pass and the
                    // stream so the idle sweep cannot unload the engine between
                    // resolving the port for the pre-pass and streaming.
                    let _activity = engine.activity_guard();
                    engine.touch();

                    // Built-in web search. Non-vision image turns stay plain
                    // (strip path). Vision + images enter the pipeline so the
                    // classifier and writer see the photo. Transform slash
                    // commands skip entirely. On-demand mode
                    // (`behavior.auto_search = false`) skips on plain turns;
                    // only `force_search` (`/search`) still runs. Prompt inputs
                    // mirror the plain path so the pre-pass and writer share
                    // the warm KV prefix. Do NOT let source-augmented writer
                    // messages reach history below.
                    let model_is_vision = if turn_has_images {
                        // Fail closed: unknown vision capability keeps the
                        // non-vision strip path rather than shipping images to
                        // a text-only classifier.
                        match engine.ensure_loaded(target.clone()).await {
                            Ok(port) => {
                                let base = format!("http://127.0.0.1:{port}");
                                fetch_builtin_vision(&client, &base).await
                            }
                            Err(_) => false,
                        }
                    } else {
                        // No images: vision flag is irrelevant to the gate.
                        true
                    };
                    let search_images = if turn_has_images && model_is_vision {
                        user_msg.images.as_deref()
                    } else {
                        None
                    };
                    let search = match builtin_search_gate(
                        turn_has_images,
                        model_is_vision,
                        force_search,
                        skip_search,
                        config.behavior.auto_search,
                    ) {
                        BuiltinSearchGate::Plain => {
                            bound_recorder.record(crate::trace::RecorderEvent::SearchSkipped {
                                reason: search_skip_reason_for_plain_gate(
                                    turn_has_images,
                                    model_is_vision,
                                    skip_search,
                                )
                                .to_string(),
                            });
                            BuiltinSearchResult::Plain
                        }
                        BuiltinSearchGate::Run { force } => {
                            let history = &messages[1..messages.len().saturating_sub(1)];
                            run_builtin_search(
                                &engine,
                                &target,
                                &model_id,
                                &client,
                                config.inference.num_ctx,
                                &messages[0].content,
                                history,
                                &user_msg.content,
                                search_images,
                                &cancel_token,
                                &bound_recorder,
                                &pump,
                                epoch_at_start,
                                force,
                            )
                            .await
                        }
                    };

                    match search {
                        BuiltinSearchResult::Cancelled => {
                            pump(StreamChunk::Cancelled);
                            String::new()
                        }
                        grounded_or_plain => {
                            // Keep the resolved sources (empty on a plain or
                            // unreachable turn) so the streamed answer can be
                            // citation-audited after the stream completes.
                            let (stream_messages, audit_sources, search_submit) =
                                match grounded_or_plain {
                                    BuiltinSearchResult::Grounded {
                                        messages,
                                        sources,
                                        search_submit,
                                    } => (messages, sources, Some(search_submit)),
                                    _ => (messages, Vec::new(), None),
                                };
                            // Observe whether reasoning streamed this turn so the
                            // runtime backstop can mark a model that reasons even
                            // with reasoning requested OFF (see
                            // `backstop_mark_reasoning_always`).
                            let reasoning_seen =
                                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                            let seen_for_pump = std::sync::Arc::clone(&reasoning_seen);
                            let backstop_model_id = model_id.clone();
                            // Stream answer tokens live for the first writer
                            // pass. Withhold `Done` so citation audit + optional
                            // repair can finish; if the cleaned body differs,
                            // emit SetContent then Done (see finalize_builtin_stream).
                            let done_pending =
                                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                            let done_pending_for_pump = std::sync::Arc::clone(&done_pending);
                            let writer_ttft_recorded =
                                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                            let writer_ttft_flag = std::sync::Arc::clone(&writer_ttft_recorded);
                            let recorder_for_ttft = std::sync::Arc::clone(&bound_recorder);
                            let builtin_pump = move |chunk: StreamChunk| {
                                observe_reasoning_chunk(&chunk, &seen_for_pump);
                                // First answer token after a search: record writer TTFT.
                                if let Some(submit) = search_submit {
                                    if matches!(chunk, StreamChunk::Token(_))
                                        && !writer_ttft_flag
                                            .swap(true, std::sync::atomic::Ordering::Relaxed)
                                    {
                                        record_writer_ttft(submit, &recorder_for_ttft);
                                    }
                                }
                                if matches!(chunk, StreamChunk::Done) {
                                    done_pending_for_pump
                                        .store(true, std::sync::atomic::Ordering::Relaxed);
                                    return;
                                }
                                pump(chunk);
                            };
                            let writer_messages_for_repair = stream_messages.clone();
                            let streamed_content = stream_builtin_chat(
                                &engine,
                                target.clone(),
                                model_id.clone(),
                                think,
                                stream_messages,
                                &client,
                                cancel_token.clone(),
                                &warm_state,
                                || {
                                    let _ = app.emit("warmup:builtin-warmed", ());
                                },
                                builtin_pump,
                            )
                            .await;
                            backstop_mark_reasoning_always(
                                &db,
                                &backstop_model_id,
                                think,
                                reasoning_seen.load(std::sync::atomic::Ordering::Relaxed),
                            );
                            // Audit → up to CITE_REPAIR_MAX_ATTEMPTS rewrites →
                            // strip leftover bad [n] (honest note only on total fail).
                            // Repair rewrites stay muted (see refine_grounded_answer).
                            // Emit Verifying so the FE sources pill shows work is
                            // still in flight after the last answer token.
                            let content = if !audit_sources.is_empty()
                                && !cancel_token.is_cancelled()
                                && done_pending.load(std::sync::atomic::Ordering::Relaxed)
                            {
                                pump(StreamChunk::SearchStatus {
                                    phase: crate::websearch::orchestrator::SearchPhase::Verifying,
                                });
                                refine_grounded_answer(
                                    streamed_content.clone(),
                                    &audit_sources,
                                    writer_messages_for_repair,
                                    &engine,
                                    target,
                                    model_id,
                                    think,
                                    &client,
                                    cancel_token.clone(),
                                    &warm_state,
                                    &bound_recorder,
                                    &pump,
                                )
                                .await
                            } else {
                                streamed_content.clone()
                            };
                            let (content, terminal_chunks) = finalize_builtin_stream(
                                content,
                                &streamed_content,
                                done_pending.load(std::sync::atomic::Ordering::Relaxed),
                            );
                            for chunk in terminal_chunks {
                                pump(chunk);
                            }
                            content
                        }
                    }
                }
                Err(err) => {
                    pump(StreamChunk::Error(err));
                    String::new()
                }
            }
        }
        ChatRoute::V1 {
            base_url,
            api_key_provider,
        } => {
            let api_key = resolve_provider_api_key(secrets.0.as_ref(), api_key_provider.as_deref());
            crate::openai::stream_openai_chat(
                crate::openai::OpenAiChatParams {
                    base_url,
                    model: model_name,
                    messages,
                    api_key,
                    flavor: crate::openai::V1Flavor::Remote,
                    // `/think` reasoning control is built-in only; a remote
                    // OpenAI-compatible server uses its own server-side defaults.
                    enable_thinking: false,
                },
                &client,
                cancel_token.clone(),
                pump,
            )
            .await
        }
    };

    let stream_ended_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    bound_recorder.record(crate::trace::RecorderEvent::AssistantComplete {
        total_tokens: token_count_atomic.load(Ordering::Relaxed),
        latency_ms: stream_ended_ms.saturating_sub(stream_started_ms),
        // Final body after repair/`SetContent`, not the raw stream only.
        final_content: capped_trace_final_content(&sanitize_assistant_content(&accumulated)),
    });

    // Persist user + assistant messages to in-memory history when the epoch
    // has not changed (no reset during streaming) and we received content.
    // This includes cancelled generations so that subsequent requests retain
    // the conversational context (the user message and any partial response).
    let current_epoch = history.epoch.load(Ordering::SeqCst);
    if current_epoch == epoch_at_start && !accumulated.is_empty() {
        let mut conv = history.messages.lock().unwrap();
        // Preserve images in history so that follow-up messages can still
        // reference earlier screenshots or attachments.  The full conversation
        // (including base64 blobs) is replayed to Ollama on every turn, which
        // is fine for a localhost-only setup.
        conv.push(user_msg);
        conv.push(ChatMessage {
            role: "assistant".to_string(),
            content: sanitize_assistant_content(&accumulated),
            images: None,
        });
    }

    generation.clear_token();
    Ok(())
}

/// Opens a URL in the system default browser (macOS `open` command).
///
/// Only `http://` and `https://` URLs are accepted; all other schemes are
/// rejected to prevent command injection and accidental protocol handler abuse.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("Only http/https URLs are supported".to_string());
    }
    std::process::Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|e| format!("Failed to open URL: {e}"))?;
    Ok(())
}

/// Cancels the currently active generation, if any.
///
/// Signals the `CancellationToken` stored in `GenerationState`, which causes the
/// `stream_ollama_chat` loop to exit immediately and drop the HTTP connection.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn cancel_generation(generation: State<'_, GenerationState>) -> Result<(), String> {
    generation.cancel();
    Ok(())
}

/// Clears the backend conversation history and increments the epoch counter.
/// The epoch increment prevents any in-flight `ask_model` from writing stale
/// messages into the freshly cleared history.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn reset_conversation(history: State<'_, ConversationHistory>) {
    history.epoch.fetch_add(1, Ordering::SeqCst);
    history.messages.lock().unwrap().clear();
}

/// Frontend-driven `ConversationEnd` emission.
///
/// The chat-domain trace lifecycle is owned by the frontend because
/// Thuki's window-close intercept hides instead of quits, and the same
/// conversation can resume on the next hotkey activation. Emitting
/// `ConversationEnd` from the backend on window-hide would falsely mark
/// every still-open conversation ended on every dismiss. The frontend
/// invokes this command exactly when the user-perceived conversation
/// terminates: clicking "New conversation", loading a different
/// conversation from history, or quitting from the tray.
///
/// The command is a thin trace-only signal; it does NOT mutate
/// `ConversationHistory` (that is `reset_conversation`'s job) and does
/// NOT touch the SQLite-backed history UI.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn record_conversation_end(
    conversation_id: String,
    reason: String,
    trace_recorder: State<'_, std::sync::Arc<crate::trace::LiveTraceRecorder>>,
) {
    use crate::trace::TraceRecorder;
    trace_recorder.record(
        &crate::trace::ConversationId::new(conversation_id),
        crate::trace::RecorderEvent::ConversationEnd { reason },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::defaults::DEFAULT_NUM_CTX;
    use std::sync::{Arc, Mutex as StdMutex};

    // ── local-datetime context ────────────────────────────────────────────

    /// 2026-07-10 01:15:30 UTC (a Friday), the fixed instant every
    /// `format_datetime_context` test formats.
    fn fixed_utc() -> time::OffsetDateTime {
        time::macros::datetime!(2026-07-10 01:15:30 UTC)
    }

    #[test]
    fn builtin_search_outcome_label_covers_all_variants() {
        use crate::websearch::orchestrator::SearchOutcome;
        assert_eq!(
            builtin_search_outcome_label(&SearchOutcome::NoSearch),
            "NoSearch"
        );
        assert_eq!(
            builtin_search_outcome_label(&SearchOutcome::Cancelled),
            "Cancelled"
        );
        assert_eq!(
            builtin_search_outcome_label(&SearchOutcome::Answer {
                messages: vec![],
                sources: vec![],
            }),
            "Answer"
        );
        assert_eq!(
            builtin_search_outcome_label(&SearchOutcome::Unreachable {
                messages: vec![],
                reason: crate::websearch::orchestrator::SearchFailReason::NoResults,
            }),
            "Unreachable"
        );
    }

    #[test]
    fn slash_skips_auto_search_covers_transform_set_only() {
        for trigger in [
            "/rewrite",
            "/refine",
            "/translate",
            "/tldr",
            "/bullets",
            "/todos",
            "/extract",
        ] {
            assert!(
                slash_skips_auto_search(Some(trigger)),
                "{trigger} must skip auto-search"
            );
        }
        for trigger in ["/search", "/explain", "/think", "/screen"] {
            assert!(
                !slash_skips_auto_search(Some(trigger)),
                "{trigger} must not skip auto-search"
            );
        }
        assert!(!slash_skips_auto_search(None));
        assert!(!slash_skips_auto_search(Some("/unknown")));
    }

    #[test]
    fn builtin_search_gate_orders_images_force_skip_and_auto() {
        // Non-vision + images: always plain, even under force or auto.
        assert_eq!(
            builtin_search_gate(true, false, true, false, true),
            BuiltinSearchGate::Plain
        );
        // Vision + images + auto: enter auto pipeline.
        assert_eq!(
            builtin_search_gate(true, true, false, false, true),
            BuiltinSearchGate::Run { force: false }
        );
        // Vision + images + force: engines-only path.
        assert_eq!(
            builtin_search_gate(true, true, true, false, false),
            BuiltinSearchGate::Run { force: true }
        );
        // `/search` forces the pipeline even when Auto search is off.
        assert_eq!(
            builtin_search_gate(false, true, true, false, false),
            BuiltinSearchGate::Run { force: true }
        );
        // Transform slash: plain even when Auto search is on (no classifier).
        assert_eq!(
            builtin_search_gate(false, true, false, true, true),
            BuiltinSearchGate::Plain
        );
        // Force wins over skip if both were ever set (defensive).
        assert_eq!(
            builtin_search_gate(false, true, true, true, true),
            BuiltinSearchGate::Run { force: true }
        );
        // Plain chat with Auto search on → auto pipeline.
        assert_eq!(
            builtin_search_gate(false, true, false, false, true),
            BuiltinSearchGate::Run { force: false }
        );
        // Plain chat with Auto search off → plain.
        assert_eq!(
            builtin_search_gate(false, true, false, false, false),
            BuiltinSearchGate::Plain
        );
    }

    #[test]
    fn search_skip_reason_matches_gate_order() {
        assert_eq!(
            search_skip_reason_for_plain_gate(true, false, false),
            "non_vision_images"
        );
        assert_eq!(
            search_skip_reason_for_plain_gate(false, true, true),
            "transform_slash"
        );
        assert_eq!(
            search_skip_reason_for_plain_gate(false, true, false),
            "auto_off"
        );
    }

    #[test]
    fn capped_trace_final_content_leaves_short_text() {
        assert_eq!(capped_trace_final_content("hello"), "hello");
    }

    #[test]
    fn capped_trace_final_content_truncates_oversize() {
        let max = crate::config::defaults::CITE_AUDIT_MAX_ANSWER_BYTES;
        let big = "a".repeat(max + 10);
        let out = capped_trace_final_content(&big);
        assert!(out.ends_with("…[truncated]"));
        // Body before the marker is at most `max` bytes.
        let body = out.trim_end_matches("…[truncated]");
        assert!(body.len() <= max);
        assert_eq!(body.len(), max);
    }

    #[test]
    fn capped_trace_final_content_respects_utf8_char_boundary() {
        // Place a multi-byte character straddling the cap so the walk-back
        // branch runs and the cut stays on a char boundary.
        let max = crate::config::defaults::CITE_AUDIT_MAX_ANSWER_BYTES;
        let prefix = "a".repeat(max - 1);
        let big = format!("{prefix}éxxx");
        let out = capped_trace_final_content(&big);
        assert!(out.ends_with("…[truncated]"));
        let body = out.trim_end_matches("…[truncated]");
        assert!(body.is_char_boundary(body.len()));
        assert!(body.len() < max);
    }

    #[test]
    fn offset_label_formats_positive_and_negative_offsets() {
        assert_eq!(
            offset_label(time::UtcOffset::from_hms(5, 30, 0).unwrap()),
            "UTC+05:30"
        );
        assert_eq!(
            offset_label(time::UtcOffset::from_hms(-5, 0, 0).unwrap()),
            "UTC-05:00"
        );
        assert_eq!(offset_label(time::UtcOffset::UTC), "UTC+00:00");
    }

    #[test]
    fn format_datetime_context_uses_iana_zone_when_known() {
        let offset = time::UtcOffset::from_hms(-5, 0, 0).unwrap();
        let got = format_datetime_context(fixed_utc(), Some(offset), Some("America/Chicago"));
        // 01:15 UTC-5 = the previous day, 20:15.
        assert_eq!(got, "Thursday, 2026-07-09, 20:15 (America/Chicago)");
    }

    #[test]
    fn format_datetime_context_falls_back_to_numeric_offset_label_when_zone_unknown() {
        let offset = time::UtcOffset::from_hms(9, 0, 0).unwrap();
        let got = format_datetime_context(fixed_utc(), Some(offset), None);
        assert_eq!(got, "Friday, 2026-07-10, 10:15 (UTC+09:00)");
    }

    #[test]
    fn format_datetime_context_falls_back_to_utc_when_offset_unavailable() {
        // The soundness-restricted path: no local offset could be determined,
        // so the line reports UTC time labelled "UTC" rather than guessing.
        // A zone name (even if somehow present) is irrelevant here: with no
        // offset, `now_utc` is used verbatim and the label is always "UTC".
        let got = format_datetime_context(fixed_utc(), None, Some("America/Chicago"));
        assert_eq!(got, "Friday, 2026-07-10, 01:15 (UTC)");
    }

    #[test]
    fn system_prompt_with_datetime_appends_context_line() {
        let got =
            system_prompt_with_datetime("You are Thuki.", "Friday, 2026-07-10, 01:15 (UTC)", None);
        assert_eq!(
            got,
            "You are Thuki.\n\nCurrent local date and time: Friday, 2026-07-10, 01:15 (UTC).\n\
             When asked what time it is somewhere else, use a resolved time line above \
             verbatim if one is present and never compute the timezone conversion yourself; \
             if none is present, you can only share the local time above, so say that rather \
             than guess."
        );
    }

    #[test]
    fn system_prompt_with_datetime_includes_resolved_place_line_when_present() {
        let place_line =
            "Current time in San Francisco (America/Los_Angeles): 06:42, Friday, 2026-07-10.";
        let got = system_prompt_with_datetime(
            "You are Thuki.",
            "Friday, 2026-07-10, 08:42 (America/Chicago)",
            Some(place_line),
        );
        assert_eq!(
            got,
            "You are Thuki.\n\nCurrent local date and time: Friday, 2026-07-10, 08:42 (America/Chicago).\n\
             Current time in San Francisco (America/Los_Angeles): 06:42, Friday, 2026-07-10.\n\
             When asked what time it is somewhere else, use a resolved time line above \
             verbatim if one is present and never compute the timezone conversion yourself; \
             if none is present, you can only share the local time above, so say that rather \
             than guess."
        );
    }

    #[test]
    fn source_metas_projects_index_url_title() {
        let blocks = vec![
            crate::websearch::assemble::SourceBlock {
                index: 1,
                url: "https://a/".into(),
                title: "A".into(),
                text: "body a".into(),
            },
            crate::websearch::assemble::SourceBlock {
                index: 2,
                url: "https://b/".into(),
                title: "B".into(),
                text: "body b".into(),
            },
        ];
        let metas = source_metas(&blocks);
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].index, 1);
        assert_eq!(metas[0].url, "https://a/");
        assert_eq!(metas[1].title, "B");
        assert!(metas[0].attribution.is_none());
    }

    #[test]
    fn source_metas_includes_required_attribution_for_weather_and_wiki() {
        let blocks = vec![
            crate::websearch::assemble::SourceBlock {
                index: 1,
                url: "https://open-meteo.com/".into(),
                title: "Weather".into(),
                text: "report".into(),
            },
            crate::websearch::assemble::SourceBlock {
                index: 2,
                url: "https://en.wikipedia.org/wiki/Photosynthesis".into(),
                title: "Photosynthesis".into(),
                text: "extract".into(),
            },
        ];
        let metas = source_metas(&blocks);
        let weather = metas[0].attribution.as_deref().unwrap();
        assert!(weather.contains("Weather data by Open-Meteo.com"));
        assert!(weather.contains("https://open-meteo.com/"));
        let wiki = metas[1].attribution.as_deref().unwrap();
        assert!(wiki.contains("CC BY-SA 4.0"));
        assert!(wiki.contains("creativecommons.org/licenses/by-sa/4.0"));
    }

    fn collect_chunks() -> (Arc<StdMutex<Vec<StreamChunk>>>, impl Fn(StreamChunk)) {
        let chunks: Arc<StdMutex<Vec<StreamChunk>>> = Arc::new(StdMutex::new(Vec::new()));
        let chunks_clone = chunks.clone();
        let callback = move |chunk: StreamChunk| {
            chunks_clone.lock().unwrap().push(chunk);
        };
        (chunks, callback)
    }

    /// Shared `stream_builtin_chat` `on_warmed` no-op for tests that never
    /// reach a real streamed token (ensure fails/cancels, or the mocked
    /// response has no content chunk). One source location shared across
    /// every such call site, so `stream_builtin_chat_announces_warmed_*`
    /// invoking the equivalent counting closure below is enough to prove
    /// this shape is reachable - none of these individual call sites need to
    /// invoke it themselves for coverage.
    fn noop_on_warmed() -> impl Fn() {
        || {}
    }

    /// Builds an `on_warmed` counter for tests: the returned closure
    /// increments a shared count so a test can assert exactly how many times
    /// `stream_builtin_chat` announced a warm-up.
    fn warmed_counter() -> (Arc<AtomicU64>, impl Fn()) {
        let count = Arc::new(AtomicU64::new(0));
        let count_cb = Arc::clone(&count);
        (count, move || {
            count_cb.fetch_add(1, Ordering::Relaxed);
        })
    }

    /// Helper: builds a `/api/chat` response line from content + done flag.
    fn chat_line(content: &str, done: bool) -> String {
        format!(
            "{{\"message\":{{\"role\":\"assistant\",\"content\":\"{}\"}},\"done\":{}}}\n",
            content, done
        )
    }

    #[test]
    fn engine_start_error_unknown_architecture_is_model_unsupported() {
        let err = engine_start_error(
            "error loading model: unknown model architecture: 'deepseek4_mtp_support'",
        );
        assert_eq!(err.kind, EngineErrorKind::ModelUnsupported);
        assert_eq!(
            err.message,
            "Unsupported model\nThuki's engine doesn't support this model's architecture yet. Try another model; support expands as the engine updates."
        );
    }

    #[test]
    fn engine_start_error_matches_short_unknown_architecture_phrasing() {
        assert_eq!(
            engine_start_error("llama_model_load: unknown architecture").kind,
            EngineErrorKind::ModelUnsupported
        );
    }

    #[test]
    fn engine_start_error_metal_symbol_failure_is_os_incompatible() {
        let detail = "dyld[123]: Symbol not found: _OBJC_CLASS_$_MTLResidencySetDescriptor\n  Referenced from: <UUID> /Applications/Thuki.app/Contents/Frameworks/libggml-metal.0.dylib\n  Expected in: <UUID> /System/Library/Frameworks/Metal.framework/Versions/A/Metal";
        let err = engine_start_error(detail);
        assert_eq!(err.kind, EngineErrorKind::EngineStartFailed);
        assert_eq!(
            err.message,
            "Thuki's engine could not start.\nYour version of macOS is too old for the built-in engine. Update macOS to use it."
        );
    }

    #[test]
    fn engine_start_error_symbol_failure_without_framework_marker_stays_generic() {
        // "Symbol not found" alone (no system-framework marker) is not treated
        // as an OS-version problem; it falls through to the concise reason.
        // The message is bare for `EngineStartFailed`: the frontend
        // `ErrorCard` supplies the fixed "couldn't start this model" title.
        let err = engine_start_error("Symbol not found: _some_internal_symbol");
        assert_eq!(err.kind, EngineErrorKind::EngineStartFailed);
        assert_eq!(err.message, "Symbol not found: _some_internal_symbol");
    }

    #[test]
    fn engine_start_error_other_failures_surface_raw_reason() {
        let err = engine_start_error("engine health check returned HTTP 500");
        assert_eq!(err.kind, EngineErrorKind::EngineStartFailed);
        // The message is the bare reason: the frontend supplies the title.
        assert_eq!(err.message, "engine health check returned HTTP 500");
    }

    #[test]
    fn engine_start_error_condenses_a_multiline_non_arch_tail() {
        let detail = "0.06 I log_info: loading\n0.06 E common_init: error loading model: out of memory\n0.06 I srv exiting";
        let err = engine_start_error(detail);
        assert_eq!(err.kind, EngineErrorKind::EngineStartFailed);
        assert_eq!(
            err.message,
            "0.06 E common_init: error loading model: out of memory"
        );
    }

    #[test]
    fn concise_detail_returns_a_single_line_unchanged() {
        assert_eq!(
            concise_detail("engine did not become healthy before the deadline"),
            "engine did not become healthy before the deadline"
        );
    }

    #[test]
    fn concise_detail_falls_back_to_the_first_line_without_an_error_marker() {
        // No line reads like an error, so the first non-empty line wins: it is
        // the real cause llama.cpp prints early, never a trailing dyld image
        // line or "exiting" notice that the last-line fallback would surface.
        assert_eq!(concise_detail("first\nsecond\nthird"), "first");
    }

    #[test]
    fn concise_detail_prefers_the_first_error_line_over_a_generic_trailer() {
        // llama.cpp prints the root cause first, then generic "exiting due to
        // ... error" trailers: the first match must win.
        let tail = "I loading model\nE error loading model: out of memory\nE failed to load model\nE exiting due to model loading error";
        assert_eq!(concise_detail(tail), "E error loading model: out of memory");
    }

    #[test]
    fn concise_detail_skips_a_benign_error_word_for_the_real_cause() {
        // A startup banner mentions "error" as a log level; the actionable line
        // is the real loading failure further down. The banner must not win.
        let tail = "I log level set to error\nI loading model\nE error loading model: bad magic";
        assert_eq!(concise_detail(tail), "E error loading model: bad magic");
    }

    #[test]
    fn concise_detail_falls_back_to_any_failure_mention() {
        // No "error:"/"error loading"/"failed to" line, but a bare mention is
        // still more informative than the last line, so it is preferred.
        let tail = "I starting up\nW cuda error detected\nI shutting down";
        assert_eq!(concise_detail(tail), "W cuda error detected");
    }

    #[test]
    fn concise_detail_empty_detail_is_empty() {
        assert_eq!(concise_detail("  \n  "), "");
    }

    #[tokio::test]
    async fn streams_tokens_from_valid_response() {
        let mut server = mockito::Server::new_async().await;
        let body = format!(
            "{}{}{}",
            chat_line("Hello", false),
            chat_line(" world", false),
            chat_line("", true),
        );
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            images: None,
        }];

        let accumulated = stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages,
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Token(t) if t == "Hello"));
        assert!(matches!(&chunks[1], StreamChunk::Token(t) if t == " world"));
        assert_eq!(
            std::mem::discriminant(&chunks[2]),
            std::mem::discriminant(&StreamChunk::Done)
        );
        assert_eq!(
            chunks.len(),
            3,
            "a single terminal Done; the stream-end branch must not emit a duplicate"
        );
        assert_eq!(accumulated, "Hello world");
    }

    /// Ollama can end the response stream without its usual terminal
    /// `done:true` line (observed when a small model degenerates on a long
    /// repetitive token run and the runner drops the connection). The loop
    /// must still emit a terminal `Done` so the frontend exits its streaming
    /// state instead of spinning forever.
    #[tokio::test]
    async fn emits_done_when_stream_ends_without_done_marker() {
        let mut server = mockito::Server::new_async().await;
        // Note: no `chat_line("", true)` line; the stream just stops.
        let body = format!(
            "{}{}",
            chat_line("Hello", false),
            chat_line(" world", false)
        );
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![ChatMessage {
                    role: "user".to_string(),
                    content: "hi".to_string(),
                    images: None,
                }],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Token(t) if t == "Hello"));
        assert!(matches!(&chunks[1], StreamChunk::Token(t) if t == " world"));
        assert_eq!(
            std::mem::discriminant(&chunks[2]),
            std::mem::discriminant(&StreamChunk::Done),
            "stream ending without done:true must still produce a terminal Done"
        );
        assert_eq!(chunks.len(), 3);
        assert_eq!(accumulated, "Hello world");
    }

    #[tokio::test]
    async fn handles_http_500() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            std::mem::discriminant(&chunks[0]),
            std::mem::discriminant(&StreamChunk::Error(EngineError {
                kind: EngineErrorKind::Other,
                message: String::new(),
            }))
        );
        assert!(accumulated.is_empty());
    }

    #[tokio::test]
    async fn handles_connection_refused() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            OllamaChatParams {
                endpoint: "http://127.0.0.1:1/api/chat".to_string(),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            std::mem::discriminant(&chunks[0]),
            std::mem::discriminant(&StreamChunk::Error(EngineError {
                kind: EngineErrorKind::Other,
                message: String::new(),
            }))
        );
        assert!(accumulated.is_empty());
    }

    #[tokio::test]
    async fn handles_malformed_json() {
        let mut server = mockito::Server::new_async().await;
        let body = format!("not json at all\n{}", chat_line("ok", true));
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[tokio::test]
    async fn handles_empty_response_body() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_body("")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        // An empty 200 body still ends the stream: emit a single terminal Done
        // so the frontend leaves its streaming state, with no content.
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            std::mem::discriminant(&chunks[0]),
            std::mem::discriminant(&StreamChunk::Done)
        );
        assert!(accumulated.is_empty());
    }

    #[tokio::test]
    async fn tokens_arrive_in_order() {
        let mut server = mockito::Server::new_async().await;
        let body = format!(
            "{}{}{}{}",
            chat_line("A", false),
            chat_line("B", false),
            chat_line("C", false),
            chat_line("", true),
        );
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        let tokens: Vec<&str> = chunks
            .iter()
            .filter_map(|c| match c {
                StreamChunk::Token(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tokens, vec!["A", "B", "C"]);
        assert_eq!(accumulated, "ABC");
    }

    #[tokio::test]
    async fn handles_invalid_utf8_in_stream() {
        let mut server = mockito::Server::new_async().await;
        let mut body = b"\xFF\xFE\n".to_vec();
        body.extend_from_slice(chat_line("ok", true).as_bytes());
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[tokio::test]
    async fn handles_mid_stream_network_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 4096];
            let _ = stream.read(&mut req_buf).await;

            let first_line = chat_line("A", false);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\n\r\n{}",
                first_line.len() + 64,
                first_line
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        });

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("http://127.0.0.1:{}/api/chat", port),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(chunks
            .iter()
            .any(|chunk| matches!(chunk, StreamChunk::Token(token) if token == "A")));
        let error = chunks.iter().find_map(|chunk| match chunk {
            StreamChunk::Error(error) => Some(error),
            _ => None,
        });
        assert!(error.is_some());
        assert_eq!(error.unwrap().kind, EngineErrorKind::Other);
        assert!(chunks
            .iter()
            .all(|chunk| !matches!(chunk, StreamChunk::Done)));
        assert_eq!(accumulated, "A");
    }

    #[tokio::test]
    async fn http_500_with_empty_body() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(500)
            .with_body("")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(
            matches!(&chunks[0], StreamChunk::Error(e) if e.kind == EngineErrorKind::Other && e.message.contains("500"))
        );
    }

    #[tokio::test]
    async fn whitespace_only_lines_are_skipped() {
        let mut server = mockito::Server::new_async().await;
        let body = format!("   \n{}", chat_line("hi", true));
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[tokio::test]
    async fn message_field_absent_emits_only_done() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_body("{\"done\":true}\n")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().all(|c| !matches!(c, StreamChunk::Token(_))));
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[tokio::test]
    async fn cancellation_stops_stream_and_emits_cancelled() {
        use std::sync::Arc;
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;
        use tokio::time::{timeout, Duration};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_done = Arc::new(tokio::sync::Notify::new());
        let server_done_clone = server_done.clone();

        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let (mut stream, _) = listener.accept().await.unwrap();
            // Consume the HTTP request so hyper doesn't see an UnexpectedMessage error
            // when it gets the response before its send is acknowledged.
            let mut req_buf = [0u8; 4096];
            let _ = stream.read(&mut req_buf).await;
            let first_line = chat_line("A", false);
            // Large Content-Length keeps the stream open after the first token so
            // the cancel fires mid-stream rather than at connection-close time.
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: 1048576\r\n\r\n{}",
                first_line
            );
            let _ = stream.write_all(header.as_bytes()).await;
            server_done_clone.notified().await;
        });

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let chunks: Arc<StdMutex<Vec<StreamChunk>>> = Arc::new(StdMutex::new(Vec::new()));
        let chunks_clone = chunks.clone();
        let first_token_seen = Arc::new(tokio::sync::Notify::new());
        let first_token_seen_clone = first_token_seen.clone();
        let callback = move |chunk: StreamChunk| {
            if matches!(&chunk, StreamChunk::Token(token) if token == "A") {
                first_token_seen_clone.notify_one();
            }
            chunks_clone.lock().unwrap().push(chunk);
        };

        let cancel_task = tokio::spawn(async move {
            timeout(Duration::from_secs(5), first_token_seen.notified())
                .await
                .expect("expected first token before cancellation");
            token_clone.cancel();
        });

        timeout(
            Duration::from_secs(5),
            stream_ollama_chat(
                OllamaChatParams {
                    endpoint: format!("http://127.0.0.1:{}/api/chat", port),
                    model: "test-model".to_string(),
                    messages: vec![],
                    think: false,
                    keep_alive: None,
                    num_ctx: DEFAULT_NUM_CTX,
                },
                &client,
                token,
                callback,
            ),
        )
        .await
        .expect("expected stream cancellation path to complete");

        cancel_task.await.unwrap();

        {
            let chunks = chunks.lock().unwrap();
            assert!(chunks
                .iter()
                .any(|c| matches!(c, StreamChunk::Token(t) if t == "A")));
            assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Cancelled)));
            assert!(chunks.iter().all(|c| !matches!(c, StreamChunk::Done)));
        }

        server_done.notify_one();
        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn pre_cancelled_token_emits_cancelled_immediately() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/api/chat")
            .with_body(chat_line("Hello", true))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        token.cancel();

        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Cancelled)));
    }

    #[tokio::test]
    async fn sends_messages_array_in_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"messages":[{"role":"system","content":"Be helpful"},{"role":"user","content":"hi"}]}"#.to_string(),
            ))
            .with_body(chat_line("", true))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (_, callback) = collect_chunks();
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "Be helpful".to_string(),
                images: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
                images: None,
            },
        ];

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages,
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn message_content_absent_emits_only_done() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_body("{\"message\":{\"role\":\"assistant\"},\"done\":true}\n")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().all(|c| !matches!(c, StreamChunk::Token(_))));
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[test]
    fn generation_state_set_and_cancel() {
        let state = GenerationState::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();

        state.set_token(token);
        assert!(!token_clone.is_cancelled());

        state.cancel();
        assert!(token_clone.is_cancelled());
    }

    #[test]
    fn generation_state_cancel_when_empty() {
        let state = GenerationState::new();
        state.cancel();
    }

    #[test]
    fn generation_state_clear_does_not_cancel() {
        let state = GenerationState::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();

        state.set_token(token);
        state.clear_token();
        assert!(!token_clone.is_cancelled());
    }

    #[test]
    fn generation_state_set_replaces_previous() {
        let state = GenerationState::new();
        let first = CancellationToken::new();
        let first_clone = first.clone();
        let second = CancellationToken::new();
        let second_clone = second.clone();

        state.set_token(first);
        state.set_token(second);

        state.cancel();
        assert!(!first_clone.is_cancelled());
        assert!(second_clone.is_cancelled());
    }

    // Note: CSV/whitespace/empty parsing of the previous THUKI_SUPPORTED_AI_MODELS
    // env var was covered by 7 env-mutating tests here. Those assertions now live
    // in src/config/tests.rs expressed as TOML input fixtures (resolve_empty_*,
    // resolve_whitespace_only_entries_are_filtered, resolve_entry_whitespace_is_trimmed).

    // ── sampling options test ────────────────────────────────────────────────

    #[tokio::test]
    async fn sends_sampling_options_in_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"options":{"temperature":1.0,"top_p":0.95,"top_k":64}}"#.to_string(),
            ))
            .with_body(chat_line("", true))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (_, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn sends_num_ctx_in_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(format!(
                r#"{{"options":{{"num_ctx":{}}}}}"#,
                DEFAULT_NUM_CTX
            )))
            .with_body(chat_line("", true))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (_, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
    }

    // Note: THUKI_SYSTEM_PROMPT env-var handling was covered by 3 tests here
    // and compose_system_prompt by 2. Those assertions now live in
    // src/config/tests.rs (resolve_empty_system_prompt_uses_built_in_base_plus_appendix,
    // resolve_custom_system_prompt_flows_through_with_appendix,
    // compose_system_prompt_*).

    #[test]
    fn conversation_history_new_starts_at_epoch_zero() {
        let h = ConversationHistory::new();
        assert_eq!(h.epoch.load(Ordering::SeqCst), 0);
        assert!(h.messages.lock().unwrap().is_empty());
    }

    #[test]
    fn conversation_history_epoch_increments_on_clear() {
        let h = ConversationHistory::new();
        h.messages.lock().unwrap().push(ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            images: None,
        });

        h.epoch.fetch_add(1, Ordering::SeqCst);
        h.messages.lock().unwrap().clear();

        assert_eq!(h.epoch.load(Ordering::SeqCst), 1);
        assert!(h.messages.lock().unwrap().is_empty());
    }

    // ─── EngineError classification ───────────────────────────────────────────

    #[test]
    fn classify_http_404_returns_model_not_found() {
        let err = classify_http_error(404, "gemma4:e2b", "");
        assert_eq!(err.kind, EngineErrorKind::ModelNotFound);
        assert!(err.message.contains("gemma4:e2b"));
    }

    /// The exact Ollama 404 copy is part of the IPC contract with ErrorCard
    /// (the `ollama pull` substring is wrapped in a code element). Pinned
    /// byte-for-byte so provider-aware copy work never drifts it.
    #[test]
    fn classify_http_404_pins_exact_ollama_copy() {
        let err = classify_http_error(404, "gemma4:e2b", "");
        assert_eq!(
            err.message,
            "Model not found\nRun: ollama pull gemma4:e2b in a terminal."
        );
    }

    /// The exact Ollama unreachable copy is rendered verbatim by ErrorCard.
    /// Pinned byte-for-byte so provider-aware copy work never drifts it.
    #[tokio::test]
    async fn classify_stream_error_pins_exact_ollama_copy() {
        // Bind then drop a listener so the port is closed; the resulting
        // reqwest error is a real connect failure.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let e = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .unwrap_err();
        let err = classify_stream_error(&e);
        assert_eq!(err.kind, EngineErrorKind::EngineUnreachable);
        assert_eq!(
            err.message,
            "Ollama isn't running\nStart Ollama and try again."
        );
    }

    #[test]
    fn classify_http_404_includes_requested_model_name_in_hint() {
        let err = classify_http_error(404, "custom:model", "");
        assert_eq!(err.kind, EngineErrorKind::ModelNotFound);
        assert!(err.message.contains("custom:model"));
    }

    #[test]
    fn classify_http_500_with_empty_body_falls_back_to_status_code() {
        let err = classify_http_error(500, "gemma4:e2b", "");
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err.message.contains("500"));
    }

    #[test]
    fn classify_http_500_surfaces_ollama_error_text_when_present() {
        let body =
            r#"{"error":"this model only supports one image while more than one image requested"}"#;
        let err = classify_http_error(500, "llama3.2-vision:11b", body);
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err
            .message
            .contains("only supports one image while more than one image requested"));
        assert!(!err.message.contains("HTTP 500"));
    }

    #[test]
    fn classify_http_500_falls_back_to_status_when_body_is_not_json() {
        let err = classify_http_error(500, "any", "<html>oops</html>");
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err.message.contains("500"));
    }

    #[test]
    fn classify_http_500_falls_back_to_status_when_error_field_is_missing() {
        let err = classify_http_error(500, "any", r#"{"detail":"nope"}"#);
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err.message.contains("500"));
    }

    #[test]
    fn classify_http_500_falls_back_to_status_when_error_field_is_blank() {
        let err = classify_http_error(500, "any", r#"{"error":"   "}"#);
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err.message.contains("500"));
    }

    #[test]
    fn extract_ollama_error_message_handles_known_shapes() {
        assert_eq!(extract_ollama_error_message(""), None);
        assert_eq!(extract_ollama_error_message("   "), None);
        assert_eq!(extract_ollama_error_message("not json"), None);
        assert_eq!(extract_ollama_error_message(r#"{}"#), None);
        assert_eq!(
            extract_ollama_error_message(r#"{"error":""}"#),
            None,
            "blank error string should be treated as missing",
        );
        assert_eq!(
            extract_ollama_error_message(r#"{"error":"boom"}"#).as_deref(),
            Some("boom"),
        );
    }

    #[test]
    fn classify_http_401_returns_other_with_status() {
        let err = classify_http_error(401, "gemma4:e2b", "");
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err.message.contains("401"));
    }

    #[test]
    fn no_model_selected_error_uses_typed_kind_and_actionable_message() {
        // The frontend keys off `kind` to route to the picker; the message
        // is rendered verbatim. Both are part of the IPC contract: lock
        // them down so accidental wording drift does not silently break
        // the recovery path.
        let err = no_model_selected_error();
        assert_eq!(err.kind, EngineErrorKind::NoModelSelected);
        assert!(
            err.message.contains("Pick a model"),
            "message should steer the user to the picker, got: {}",
            err.message,
        );
    }

    #[test]
    fn insufficient_memory_error_reports_typed_kind_and_gib_figures() {
        // The frontend keys off `kind` to raise the "load anyway" affordance;
        // the message carries approximate GiB so the user sees why. Lock both
        // down: 6 GiB required, 4 GiB available render as "6.0 GB" / "4.0 GB".
        let err = insufficient_memory_error(6 * (1 << 30), 4 * (1 << 30));
        assert_eq!(err.kind, EngineErrorKind::InsufficientMemory);
        assert!(
            err.message.contains("6.0 GB") && err.message.contains("4.0 GB"),
            "message should carry both GiB figures, got: {}",
            err.message,
        );
    }

    #[test]
    fn engine_error_kinds_serialize_as_pascal_case() {
        // Wire format contract: every kind must serialize verbatim in
        // PascalCase so the React side (ErrorCard.barColors, useModel) can match
        // on stable literal strings. Drift here silently breaks accent styling
        // and error routing without failing any other test.
        let cases = [
            (EngineErrorKind::EngineUnreachable, "EngineUnreachable"),
            (EngineErrorKind::EngineStartFailed, "EngineStartFailed"),
            (EngineErrorKind::ModelNotFound, "ModelNotFound"),
            (EngineErrorKind::NoModelSelected, "NoModelSelected"),
            (EngineErrorKind::Other, "Other"),
        ];
        for (kind, expected) in cases {
            let v = serde_json::to_value(kind).unwrap();
            assert_eq!(v, serde_json::Value::String(expected.to_string()));
        }
    }

    #[tokio::test]
    async fn connection_refused_emits_not_running_error() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: "http://127.0.0.1:1/api/chat".to_string(),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(
            matches!(&chunks[0], StreamChunk::Error(e) if e.kind == EngineErrorKind::EngineUnreachable)
        );
    }

    #[tokio::test]
    async fn http_404_emits_model_not_found_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(404)
            .with_body("")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(
            matches!(&chunks[0], StreamChunk::Error(e) if e.kind == EngineErrorKind::ModelNotFound)
        );
    }

    #[test]
    fn thinking_token_serializes_correctly() {
        let chunk = StreamChunk::ThinkingToken("reasoning step".to_string());
        let json = serde_json::to_value(&chunk).unwrap();
        assert_eq!(json["type"], "ThinkingToken");
        assert_eq!(json["data"], "reasoning step");
    }

    #[test]
    fn search_failed_serializes_reason_to_the_wire_strings() {
        use crate::websearch::orchestrator::SearchFailReason;
        let unreachable = serde_json::to_value(StreamChunk::SearchFailed {
            reason: SearchFailReason::Unreachable,
        })
        .unwrap();
        assert_eq!(unreachable["type"], "SearchFailed");
        assert_eq!(unreachable["data"]["reason"], "unreachable");

        let no_results = serde_json::to_value(StreamChunk::SearchFailed {
            reason: SearchFailReason::NoResults,
        })
        .unwrap();
        assert_eq!(no_results["data"]["reason"], "no_results");
    }

    #[test]
    fn ollama_chat_request_sends_think_false_explicitly() {
        let req = OllamaChatRequest {
            model: "test".to_string(),
            messages: vec![],
            stream: true,
            think: false,
            options: OllamaOptions {
                temperature: 1.0,
                top_p: 0.95,
                top_k: 64,
                num_ctx: DEFAULT_NUM_CTX,
            },
            keep_alive: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["think"], false);
    }

    #[test]
    fn ollama_chat_request_includes_think_when_true() {
        let req = OllamaChatRequest {
            model: "test".to_string(),
            messages: vec![],
            stream: true,
            think: true,
            options: OllamaOptions {
                temperature: 1.0,
                top_p: 0.95,
                top_k: 64,
                num_ctx: DEFAULT_NUM_CTX,
            },
            keep_alive: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["think"], true);
    }

    #[test]
    fn ollama_response_message_deserializes_thinking_field() {
        let json = r#"{"content":"hello","thinking":"let me think"}"#;
        let msg: OllamaChatResponseMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.content.unwrap(), "hello");
        assert_eq!(msg.thinking.unwrap(), "let me think");
    }

    #[test]
    fn ollama_response_message_thinking_absent() {
        let json = r#"{"content":"hello"}"#;
        let msg: OllamaChatResponseMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.content.unwrap(), "hello");
        assert!(msg.thinking.is_none());
    }

    #[tokio::test]
    async fn http_500_emits_other_error_with_status() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(
            matches!(&chunks[0], StreamChunk::Error(e) if e.kind == EngineErrorKind::Other && e.message.contains("500"))
        );
    }

    #[tokio::test]
    async fn http_500_surfaces_ollama_error_body_through_stream() {
        let mut server = mockito::Server::new_async().await;
        let body =
            r#"{"error":"this model only supports one image while more than one image requested"}"#;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "llama3.2-vision:11b".to_string(),
                messages: vec![],
                think: false,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(
            &chunks[0],
            StreamChunk::Error(e)
                if e.kind == EngineErrorKind::Other
                && e.message.contains("only supports one image")
                && !e.message.contains("HTTP 500")
        ));
    }

    /// Helper: builds a `/api/chat` response line with both thinking and content fields.
    fn chat_line_with_thinking(thinking: &str, content: &str, done: bool) -> String {
        format!(
            "{{\"message\":{{\"role\":\"assistant\",\"content\":\"{}\",\"thinking\":\"{}\"}},\"done\":{}}}\n",
            content, thinking, done
        )
    }

    #[tokio::test]
    async fn stream_ollama_chat_emits_thinking_tokens() {
        let mut server = mockito::Server::new_async().await;
        let body = format!(
            "{}{}{}",
            chat_line_with_thinking("step 1", "", false),
            chat_line_with_thinking("", "Hello", false),
            chat_line_with_thinking("", "", true),
        );
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: true,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();

        // ThinkingToken emitted for thinking field
        assert!(matches!(&chunks[0], StreamChunk::ThinkingToken(t) if t == "step 1"));
        // Token emitted for content field
        assert!(matches!(&chunks[1], StreamChunk::Token(t) if t == "Hello"));
        // Done emitted
        assert_eq!(
            std::mem::discriminant(&chunks[2]),
            std::mem::discriminant(&StreamChunk::Done)
        );

        // Accumulated return value contains only content, not thinking
        assert_eq!(accumulated, "Hello");
    }

    #[tokio::test]
    async fn stream_ollama_chat_sends_think_true_in_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"think":true}"#.to_string(),
            ))
            .with_body(chat_line("", true))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (_, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: true,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn stream_ollama_chat_empty_thinking_not_emitted() {
        let mut server = mockito::Server::new_async().await;
        let body = format!(
            "{}{}",
            chat_line_with_thinking("", "Hello", false),
            chat_line_with_thinking("", "", true),
        );
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            OllamaChatParams {
                endpoint: format!("{}/api/chat", server.url()),
                model: "test-model".to_string(),
                messages: vec![],
                think: true,
                keep_alive: None,
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();

        // No ThinkingToken emitted for empty thinking field
        assert!(chunks
            .iter()
            .all(|c| !matches!(c, StreamChunk::ThinkingToken(_))));
        // Content token still emitted
        assert!(chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::Token(t) if t == "Hello")));
    }

    // ─── sanitize_assistant_content ──────────────────────────────────────────

    #[test]
    fn sanitize_returns_clean_input_unchanged() {
        let input = "Hello **world**\n\n```rust\nlet x = 1;\n```\nDone.";
        assert_eq!(sanitize_assistant_content(input), input);
    }

    #[test]
    fn sanitize_strips_every_known_pattern() {
        for pattern in STRIP_PATTERNS {
            let dirty = format!("before{pattern}after");
            assert_eq!(
                sanitize_assistant_content(&dirty),
                "beforeafter",
                "pattern {pattern} should be removed"
            );
        }
    }

    #[test]
    fn sanitize_strips_multiple_occurrences() {
        let dirty = "<|im_start|>a<|im_start|>b<|im_end|>c";
        assert_eq!(sanitize_assistant_content(dirty), "abc");
    }

    #[test]
    fn sanitize_drops_unsafe_control_chars_but_keeps_whitespace() {
        let dirty = "a\x00b\x07c\nd\te\rf\x1Fg";
        assert_eq!(sanitize_assistant_content(dirty), "abc\nd\te\rfg");
    }

    #[test]
    fn sanitize_preserves_unicode_and_emoji() {
        let input = "héllo 世界 🚀\nline two";
        assert_eq!(sanitize_assistant_content(input), input);
    }

    #[test]
    fn sanitize_handles_empty_string() {
        assert_eq!(sanitize_assistant_content(""), "");
    }

    // ─── apply_capability_filter ─────────────────────────────────────────────

    fn msg(role: &str, content: &str, images: Option<Vec<String>>) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            images,
        }
    }

    #[test]
    fn filter_strips_images_when_vision_false() {
        let mut messages = vec![
            msg(
                "user",
                "first",
                Some(vec!["a".to_string(), "b".to_string()]),
            ),
            msg("assistant", "reply", None),
            msg("user", "again", Some(vec!["c".to_string()])),
        ];
        let caps = Capabilities {
            vision: false,
            thinking: false,
            reasoning_always: false,
            max_images: None,
        };
        let stats = apply_capability_filter(&mut messages, &caps);
        assert_eq!(stats.stripped_images, 3);
        assert!(messages.iter().all(|m| m.images.is_none()));
    }

    #[test]
    fn filter_preserves_images_when_vision_true_and_no_cap() {
        let mut messages = vec![msg(
            "user",
            "x",
            Some(vec!["a".to_string(), "b".to_string()]),
        )];
        let caps = Capabilities {
            vision: true,
            thinking: false,
            reasoning_always: false,
            max_images: None,
        };
        let stats = apply_capability_filter(&mut messages, &caps);
        assert_eq!(stats.stripped_images, 0);
        assert_eq!(messages[0].images.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn filter_truncates_to_max_images_keeping_first() {
        let mut messages = vec![msg(
            "user",
            "x",
            Some(vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string(),
            ]),
        )];
        let caps = Capabilities {
            vision: true,
            thinking: false,
            reasoning_always: false,
            max_images: Some(1),
        };
        let stats = apply_capability_filter(&mut messages, &caps);
        assert_eq!(stats.stripped_images, 2);
        let imgs = messages[0].images.as_ref().unwrap();
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0], "first");
    }

    #[test]
    fn filter_no_op_when_under_max_images() {
        let mut messages = vec![msg("user", "x", Some(vec!["only".to_string()]))];
        let caps = Capabilities {
            vision: true,
            thinking: false,
            reasoning_always: false,
            max_images: Some(2),
        };
        let stats = apply_capability_filter(&mut messages, &caps);
        assert_eq!(stats.stripped_images, 0);
        assert_eq!(messages[0].images.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn filter_handles_text_only_messages_under_vision_false() {
        let mut messages = vec![msg("user", "hi", None), msg("assistant", "hello", None)];
        let caps = Capabilities {
            vision: false,
            thinking: false,
            reasoning_always: false,
            max_images: None,
        };
        let stats = apply_capability_filter(&mut messages, &caps);
        assert_eq!(stats.stripped_images, 0);
    }

    #[test]
    fn filter_skips_messages_without_images_under_max_cap() {
        let mut messages = vec![
            msg("user", "no imgs", None),
            msg(
                "user",
                "two imgs",
                Some(vec!["a".to_string(), "b".to_string()]),
            ),
        ];
        let caps = Capabilities {
            vision: true,
            thinking: false,
            reasoning_always: false,
            max_images: Some(1),
        };
        let stats = apply_capability_filter(&mut messages, &caps);
        assert_eq!(stats.stripped_images, 1);
        assert!(messages[0].images.is_none());
        assert_eq!(messages[1].images.as_ref().unwrap().len(), 1);
    }

    // ─── classify_http_error: capability picker hint ─────────────────────────

    #[test]
    fn classify_http_500_appends_picker_hint_when_body_mentions_image() {
        let body = r#"{"error":"this model only supports one image"}"#;
        let err = classify_http_error(500, "any", body);
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err.message.contains("only supports one image"));
        assert!(err.message.contains("picker chip"));
    }

    #[test]
    fn classify_http_500_appends_picker_hint_when_body_mentions_vision() {
        let body = r#"{"error":"vision capability required"}"#;
        let err = classify_http_error(500, "any", body);
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err.message.contains("vision capability required"));
        assert!(err.message.contains("picker chip"));
    }

    #[test]
    fn classify_http_500_omits_picker_hint_for_unrelated_errors() {
        let body = r#"{"error":"context window exceeded"}"#;
        let err = classify_http_error(500, "any", body);
        assert!(!err.message.contains("picker chip"));
    }

    #[test]
    fn classify_http_500_omits_picker_hint_when_body_is_empty() {
        let err = classify_http_error(500, "any", "");
        assert!(!err.message.contains("picker chip"));
        assert!(err.message.contains("500"));
    }

    #[test]
    fn classify_http_404_does_not_append_picker_hint() {
        let err = classify_http_error(404, "vision-model", "image required");
        assert_eq!(err.kind, EngineErrorKind::ModelNotFound);
        assert!(!err.message.contains("picker chip"));
    }

    // ─── Trace orchestration helpers ────────────────────────────────────────

    /// Builds a `BoundRecorder` over a `MockRecorder` so each helper test
    /// can inspect what got recorded without going through the file system.
    fn mock_bound_recorder(
        conv_id: &str,
    ) -> (
        Arc<crate::trace::BoundRecorder>,
        Arc<crate::trace::recorder::MockRecorder>,
    ) {
        let mock = Arc::new(crate::trace::recorder::MockRecorder::new());
        let inner: Arc<dyn crate::trace::TraceRecorder> = mock.clone();
        let bound = Arc::new(crate::trace::BoundRecorder::new(
            inner,
            crate::trace::ConversationId::new(conv_id),
        ));
        (bound, mock)
    }

    #[test]
    fn record_chunk_to_trace_emits_assistant_tokens_and_increments_count() {
        let (bound, mock) = mock_bound_recorder("conv-token");
        let counter = AtomicU64::new(0);
        record_chunk_to_trace(&StreamChunk::Token("hi".to_string()), &bound, &counter);
        record_chunk_to_trace(&StreamChunk::Token(" there".to_string()), &bound, &counter);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
        let snapshot = mock.snapshot();
        assert_eq!(snapshot.len(), 2);
        for (id, _) in &snapshot {
            assert_eq!(id.as_str(), "conv-token");
        }
        assert!(matches!(
            snapshot[0].1,
            crate::trace::RecorderEvent::AssistantTokens { ref chunk } if chunk == "hi"
        ));
        assert!(matches!(
            snapshot[1].1,
            crate::trace::RecorderEvent::AssistantTokens { ref chunk } if chunk == " there"
        ));
    }

    #[test]
    fn record_chunk_to_trace_emits_assistant_thinking_without_increment() {
        let (bound, mock) = mock_bound_recorder("conv-think");
        let counter = AtomicU64::new(0);
        record_chunk_to_trace(
            &StreamChunk::ThinkingToken("planning".to_string()),
            &bound,
            &counter,
        );
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        let snapshot = mock.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert!(matches!(
            snapshot[0].1,
            crate::trace::RecorderEvent::AssistantThinking { ref chunk } if chunk == "planning"
        ));
    }

    #[test]
    fn record_chunk_to_trace_records_set_content_as_one_token_chunk() {
        let (bound, mock) = mock_bound_recorder("conv-set");
        let counter = AtomicU64::new(0);
        record_chunk_to_trace(
            &StreamChunk::SetContent("cleaned body".to_string()),
            &bound,
            &counter,
        );
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        let snapshot = mock.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert!(matches!(
            snapshot[0].1,
            crate::trace::RecorderEvent::AssistantTokens { ref chunk } if chunk == "cleaned body"
        ));
    }

    #[test]
    fn record_chunk_to_trace_skips_terminal_chunks() {
        let (bound, mock) = mock_bound_recorder("conv-term");
        let counter = AtomicU64::new(0);
        record_chunk_to_trace(&StreamChunk::Done, &bound, &counter);
        record_chunk_to_trace(&StreamChunk::Cancelled, &bound, &counter);
        record_chunk_to_trace(
            &StreamChunk::Error(no_model_selected_error()),
            &bound,
            &counter,
        );
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        assert_eq!(mock.snapshot().len(), 0);
    }

    #[test]
    fn finalize_builtin_stream_no_done_pending_never_emits() {
        // Cancelled/error paths: the frontend already left streaming, so
        // nothing more is emitted and content is returned unmodified.
        let (content, chunks) =
            finalize_builtin_stream("partial answer".to_string(), "partial answer", false);
        assert_eq!(content, "partial answer");
        assert!(chunks.is_empty());
    }

    #[test]
    fn finalize_builtin_stream_unchanged_body_only_done() {
        // Live-streamed answer matches post-audit body: only Done.
        let body = "The answer [1].";
        let (content, chunks) = finalize_builtin_stream(body.to_string(), body, true);
        assert_eq!(content, body);
        assert_eq!(chunks.len(), 1);
        assert!(matches!(chunks[0], StreamChunk::Done));
    }

    #[test]
    fn finalize_builtin_stream_changed_body_sets_content_then_done() {
        // Repair/strip changed the live draft: replace bubble then Done.
        let streamed = "Bad cite [9] here.";
        let cleaned = "Clean answer [1].";
        let (content, chunks) = finalize_builtin_stream(cleaned.to_string(), streamed, true);
        assert_eq!(content, cleaned);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::SetContent(t) if t == cleaned));
        assert!(matches!(chunks[1], StreamChunk::Done));
    }

    /// Cross-feature: conflict directive (J1) still shapes the writer prompt,
    /// and post-generation audit (J3) now strips unsupported citations (or
    /// adds an honest total-failure note) instead of a guilt hedge footer.
    /// Neither feature suppresses the other.
    #[test]
    fn conflicting_verdict_and_audit_cleanup_compose_in_one_grounded_turn() {
        use crate::websearch::assemble::SourceBlock;
        use crate::websearch::cite_check::{
            audit_citations, finalize_answer_after_audit, is_total_citation_failure,
        };
        use crate::websearch::judge::{InsufficiencyReason, SufficiencyVerdict};
        use crate::websearch::writer::build_writer_appendix;

        let verdict = SufficiencyVerdict {
            sufficient: false,
            missing: "the two sources report different revenue figures".into(),
            reason: InsufficiencyReason::Conflicting,
            requery_queries: Vec::new(),
        };
        assert!(verdict.conflicting());

        let source = SourceBlock {
            index: 1,
            url: "https://example.test/report".into(),
            title: "Quarterly report".into(),
            text: "The company announced a new phone at its event with several colours.".into(),
        };
        let blocks = [source.clone()];

        let conflict_appendix = build_writer_appendix(
            &blocks,
            "2026-07-10",
            "en-US",
            "deadbeef",
            false, /* is_cache_tier */
            verdict.conflicting(),
            None,
        );
        let plain_appendix = build_writer_appendix(
            &blocks,
            "2026-07-10",
            "en-US",
            "deadbeef",
            false,
            false,
            None,
        );
        assert!(conflict_appendix.contains("The sources disagree on a value the question asks for"));
        assert!(!plain_appendix.contains("The sources disagree on a value the question asks for"));

        // Fabricated figure: audit flags [1] as total failure → strip + honest note.
        let answer = "The phone costs 499 dollars launching in 2027 quarter [1].";
        let audit = audit_citations(answer, &blocks);
        assert_eq!(audit.unsupported_indices, vec![1]);
        assert!(is_total_citation_failure(&audit));
        let cleaned = finalize_answer_after_audit(answer, &audit);
        assert!(cleaned.contains("found sources but could not verify"));
        assert!(!cleaned.contains("[1]"));

        // Live draft differed after audit cleanup: SetContent then Done.
        let (content, chunks) = finalize_builtin_stream(cleaned.clone(), answer, true);
        assert_eq!(content, cleaned);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::SetContent(t) if t == &cleaned));
        assert!(matches!(chunks[1], StreamChunk::Done));
    }

    #[test]
    fn build_repair_messages_appends_assistant_then_critique() {
        use crate::websearch::cite_check::CitationAudit;
        let base = vec![ChatMessage {
            role: "user".into(),
            content: "sources here".into(),
            images: None,
        }];
        let audit = CitationAudit {
            cited: 1,
            supported: 0,
            weak: 0,
            unsupported: 1,
            unverifiable: 0,
            unsupported_indices: vec![3],
            numeric_checked: 0,
            numeric_matched: 0,
            numeric_missing: 0,
            details: vec![],
        };
        let msgs = build_repair_messages(base, "bad answer [3]", &audit);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "bad answer [3]");
        assert_eq!(msgs[2].role, "user");
        assert!(msgs[2].content.contains("[3]"));
        assert!(msgs[2].content.contains("Rewrite the full answer"));
    }

    #[test]
    fn build_repair_messages_zero_cite_demands_bracket_markers() {
        use crate::websearch::cite_check::CitationAudit;
        let base = vec![ChatMessage {
            role: "user".into(),
            content: "sources here".into(),
            images: None,
        }];
        // Elon-shaped: prose outlet names, no [n] markers.
        let audit = CitationAudit {
            cited: 0,
            supported: 0,
            weak: 0,
            unsupported: 0,
            unverifiable: 0,
            unsupported_indices: vec![],
            numeric_checked: 0,
            numeric_matched: 0,
            numeric_missing: 0,
            details: vec![],
        };
        let prose = "Worth $913B according to Bloomberg and $917B per Forbes.";
        let msgs = build_repair_messages(base, prose, &audit);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1].content, prose);
        assert!(msgs[2].content.contains("no [n] source"));
        assert!(msgs[2].content.contains("Naming a publisher in prose"));
    }

    #[test]
    fn record_conversation_start_if_first_turn_emits_when_true() {
        let (bound, mock) = mock_bound_recorder("conv-start");
        record_conversation_start_if_first_turn(
            &bound,
            true,
            "model-a".to_string(),
            "you are helpful".to_string(),
        );
        let snapshot = mock.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert!(matches!(
            snapshot[0].1,
            crate::trace::RecorderEvent::ConversationStart {
                ref model,
                ref system_prompt,
            } if model == "model-a" && system_prompt == "you are helpful"
        ));
    }

    #[test]
    fn record_conversation_start_if_first_turn_skips_when_false() {
        let (bound, mock) = mock_bound_recorder("conv-skip");
        record_conversation_start_if_first_turn(
            &bound,
            false,
            "model-a".to_string(),
            "ignored".to_string(),
        );
        assert_eq!(mock.snapshot().len(), 0);
    }

    // ─── resolve_chat_route ─────────────────────────────────────────────

    /// Helper: an `InferenceSection` whose single provider `p1` is active.
    fn inference_with_provider(
        kind: &str,
        base_url: &str,
        model: &str,
    ) -> crate::config::schema::InferenceSection {
        use crate::config::schema::{InferenceSection, Provider};
        InferenceSection {
            active_provider: "p1".to_string(),
            providers: vec![Provider {
                id: "p1".to_string(),
                kind: kind.to_string(),
                label: "Test".to_string(),
                base_url: base_url.to_string(),
                model: model.to_string(),
                vision: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn resolve_chat_route_ollama() {
        use crate::config::defaults::PROVIDER_KIND_OLLAMA;
        let inference =
            inference_with_provider(PROVIDER_KIND_OLLAMA, "http://127.0.0.1:11434/", "");
        assert_eq!(
            resolve_chat_route(&inference).unwrap(),
            ChatRoute::OllamaNative {
                endpoint: "http://127.0.0.1:11434/api/chat".to_string(),
            }
        );
    }

    #[test]
    fn resolve_chat_route_openai() {
        use crate::config::defaults::PROVIDER_KIND_OPENAI;
        let inference =
            inference_with_provider(PROVIDER_KIND_OPENAI, "http://localhost:8080/", "qwen3");
        assert_eq!(
            resolve_chat_route(&inference).unwrap(),
            ChatRoute::V1 {
                base_url: "http://localhost:8080".to_string(),
                api_key_provider: Some("p1".to_string()),
            }
        );
    }

    #[test]
    fn resolve_chat_route_builtin() {
        use crate::config::defaults::PROVIDER_KIND_BUILTIN;
        let inference = inference_with_provider(PROVIDER_KIND_BUILTIN, "", "org/repo:m.gguf");
        assert_eq!(
            resolve_chat_route(&inference).unwrap(),
            ChatRoute::Builtin {
                model_id: "org/repo:m.gguf".to_string(),
            }
        );
    }

    #[test]
    fn resolve_chat_route_no_model_selected() {
        use crate::config::defaults::PROVIDER_KIND_BUILTIN;
        let inference = inference_with_provider(PROVIDER_KIND_BUILTIN, "", "");
        let err = resolve_chat_route(&inference).unwrap_err();
        assert_eq!(err.kind, EngineErrorKind::NoModelSelected);
        assert!(err.message.contains("Settings"));
    }

    #[test]
    fn resolve_chat_route_unknown_kind() {
        let inference = inference_with_provider("weird", "http://x", "m");
        let err = resolve_chat_route(&inference).unwrap_err();
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err.message.contains("unknown kind"));
    }

    // ─── builtin_target ─────────────────────────────────────────────────

    /// Helper: a complete manifest row keyed by `id` with the given hashes.
    fn installed_model(
        id: &str,
        sha256: &str,
        mmproj_sha256: Option<&str>,
    ) -> crate::models::manifest::InstalledModel {
        crate::models::manifest::InstalledModel {
            id: id.to_string(),
            display_name: format!("Model {id}"),
            repo: "org/repo".to_string(),
            revision: "a".repeat(40),
            file_name: format!("{id}.gguf"),
            sha256: sha256.to_string(),
            size_bytes: 1_000_000,
            quant: "Q4_K_M".to_string(),
            vision: mmproj_sha256.is_some(),
            thinking: false,
            reasoning_always: false,
            mmproj_file: mmproj_sha256.map(|_| format!("{id}-mmproj.gguf")),
            mmproj_sha256: mmproj_sha256.map(str::to_string),
            parts: Vec::new(),
        }
    }

    #[test]
    fn builtin_target_maps_manifest_row() {
        let conn = crate::database::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = crate::models::storage::ModelStore::new(dir.path().to_path_buf()).unwrap();
        crate::models::manifest::insert(
            &conn,
            &installed_model("org/repo:v.gguf", "sha_w", Some("sha_mm")),
        )
        .unwrap();
        crate::models::manifest::insert(&conn, &installed_model("org/repo:t.gguf", "sha_t", None))
            .unwrap();

        let vision = builtin_target(&conn, &store, "org/repo:v.gguf", 4096).unwrap();
        assert_eq!(vision.model_path, store.blob_path("sha_w"));
        assert_eq!(vision.mmproj_path, Some(store.blob_path("sha_mm")));
        assert_eq!(vision.num_ctx, 4096);

        let text = builtin_target(&conn, &store, "org/repo:t.gguf", DEFAULT_NUM_CTX).unwrap();
        assert_eq!(text.model_path, store.blob_path("sha_t"));
        assert_eq!(text.mmproj_path, None);
        assert_eq!(text.num_ctx, DEFAULT_NUM_CTX);
    }

    #[test]
    fn builtin_target_missing_row_is_model_not_found() {
        let conn = crate::database::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = crate::models::storage::ModelStore::new(dir.path().to_path_buf()).unwrap();
        let err = builtin_target(&conn, &store, "org/repo:gone.gguf", 4096).unwrap_err();
        assert_eq!(err.kind, EngineErrorKind::ModelNotFound);
        assert!(err.message.contains("Settings"));
    }

    #[test]
    fn builtin_target_manifest_read_error_is_other() {
        // A bare connection without the schema makes `manifest::get` fail.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = crate::models::storage::ModelStore::new(dir.path().to_path_buf()).unwrap();
        let err = builtin_target(&conn, &store, "org/repo:m.gguf", 4096).unwrap_err();
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err.message.contains("manifest"));
    }

    /// A multi-part manifest row whose `parts` carry the given `(file, sha)`
    /// shard pairs; the representative sha256 is the first shard's.
    fn multipart_model(
        id: &str,
        shards: &[(&str, &str)],
    ) -> crate::models::manifest::InstalledModel {
        crate::models::manifest::InstalledModel {
            sha256: shards[0].1.to_string(),
            parts: shards
                .iter()
                .map(|(file, sha)| crate::models::HfGgufPart {
                    file: file.to_string(),
                    sha256: sha.to_string(),
                    size_bytes: 100,
                })
                .collect(),
            ..installed_model(id, shards[0].1, None)
        }
    }

    #[test]
    fn builtin_target_multipart_model_loads_through_the_split_shim() {
        let conn = crate::database::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = crate::models::storage::ModelStore::new(dir.path().to_path_buf()).unwrap();
        // Write blobs for both shards so the shim's symlinks resolve.
        std::fs::write(store.blob_path("shaA"), b"a").unwrap();
        std::fs::write(store.blob_path("shaB"), b"b").unwrap();
        crate::models::manifest::insert(
            &conn,
            &multipart_model(
                "org/repo:split",
                &[
                    ("model-00001-of-00002.gguf", "shaA"),
                    ("model-00002-of-00002.gguf", "shaB"),
                ],
            ),
        )
        .unwrap();

        let target = builtin_target(&conn, &store, "org/repo:split", DEFAULT_NUM_CTX).unwrap();
        // model_path is the first shard's symlink under the shim dir, NOT the
        // raw blob path.
        assert!(target
            .model_path
            .ends_with("shims/shaA/model-00001-of-00002.gguf"));
        assert_ne!(target.model_path, store.blob_path("shaA"));
    }

    #[test]
    fn builtin_target_multipart_invalid_shard_name_is_other_error() {
        let conn = crate::database::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = crate::models::storage::ModelStore::new(dir.path().to_path_buf()).unwrap();
        // An attacker-controlled shard name that is not a valid split shape
        // makes the shim refuse, which surfaces as an engine-start error.
        crate::models::manifest::insert(
            &conn,
            &multipart_model("org/repo:evil", &[("../escape.gguf", "shaE")]),
        )
        .unwrap();

        let err = builtin_target(&conn, &store, "org/repo:evil", DEFAULT_NUM_CTX).unwrap_err();
        assert_eq!(err.kind, EngineErrorKind::Other);
        assert!(err.message.contains("split model"));
    }

    // ─── resolve_provider_api_key ───────────────────────────────────────

    #[test]
    fn resolve_provider_api_key_reads_key_and_misses_to_none() {
        use crate::keychain::SecretStore;
        let store = crate::keychain::FakeSecretStore::new();
        store.set("p1", "sk-test").unwrap();
        assert_eq!(
            resolve_provider_api_key(&store, Some("p1")),
            Some("sk-test".to_string())
        );
        assert_eq!(resolve_provider_api_key(&store, Some("absent")), None);
        assert_eq!(resolve_provider_api_key(&store, None), None);
    }

    /// A secret store whose reads always fail, for the degrade-to-None path.
    struct FailingSecretStore;

    impl crate::keychain::SecretStore for FailingSecretStore {
        fn set(&self, _provider_id: &str, _secret: &str) -> Result<(), String> {
            Err("locked".to_string())
        }
        fn get(&self, _provider_id: &str) -> Result<Option<String>, String> {
            Err("locked".to_string())
        }
        fn delete(&self, _provider_id: &str) -> Result<(), String> {
            Err("locked".to_string())
        }
    }

    #[test]
    fn resolve_provider_api_key_error_degrades_to_none() {
        use crate::keychain::SecretStore;
        assert_eq!(
            resolve_provider_api_key(&FailingSecretStore, Some("p1")),
            None
        );
        // The other trait methods fail too; the chat path never calls them.
        assert!(FailingSecretStore.set("p1", "sk").is_err());
        assert!(FailingSecretStore.delete("p1").is_err());
    }

    // ─── Ollama native path regression ──────────────────────────────────

    /// Locks the native `/api/chat` wire contract across the routing change:
    /// the exact request body (model, messages, stream, think, options
    /// {temperature, top_p, top_k, num_ctx}, keep_alive) must be identical
    /// to the payload Thuki sent before provider routing was introduced.
    #[tokio::test]
    async fn ollama_request_body_unchanged() {
        use crate::config::defaults::PROVIDER_KIND_OLLAMA;
        let mut server = mockito::Server::new_async().await;

        // The endpoint comes from the route resolver, exactly as `ask_model`
        // dispatches it.
        let inference =
            inference_with_provider(PROVIDER_KIND_OLLAMA, &format!("{}/", server.url()), "");
        let endpoint = format!("{}/api/chat", server.url());
        assert_eq!(
            resolve_chat_route(&inference).unwrap(),
            ChatRoute::OllamaNative {
                endpoint: endpoint.clone(),
            }
        );

        let expected_body = serde_json::json!({
            "model": "gemma3:12b",
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "hi"}
            ],
            "stream": true,
            "think": false,
            "options": {
                "temperature": 1.0,
                "top_p": 0.95,
                "top_k": 64,
                "num_ctx": DEFAULT_NUM_CTX
            },
            "keep_alive": "10m"
        });
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::Json(expected_body))
            .with_body(chat_line("", true))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let (_, callback) = collect_chunks();
        stream_ollama_chat(
            OllamaChatParams {
                endpoint,
                model: "gemma3:12b".to_string(),
                messages: vec![
                    ChatMessage {
                        role: "system".to_string(),
                        content: "sys".to_string(),
                        images: None,
                    },
                    ChatMessage {
                        role: "user".to_string(),
                        content: "hi".to_string(),
                        images: None,
                    },
                ],
                think: false,
                keep_alive: Some("10m".to_string()),
                num_ctx: DEFAULT_NUM_CTX,
            },
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        mock.assert_async().await;
    }

    // ─── stream_builtin_chat ────────────────────────────────────────────

    /// Scriptable [`crate::engine::process::EngineProcess`] for the built-in
    /// route tests: hands out a fixed port, optionally fails every spawn,
    /// and either answers health probes with 200 or hangs them forever so a
    /// test can preempt the in-flight ensure.
    struct ScriptedEngineProcess {
        port: u16,
        spawn_error: Option<String>,
        healthy: bool,
    }

    struct ScriptedChild {
        exit_tx: tokio::sync::watch::Sender<bool>,
        exit_rx: tokio::sync::watch::Receiver<bool>,
    }

    #[async_trait::async_trait]
    impl crate::engine::process::EngineChild for ScriptedChild {
        async fn wait_exit(&mut self) {
            let _ = self.exit_rx.wait_for(|exited| *exited).await;
        }
        async fn kill(&mut self) {
            let _ = self.exit_tx.send(true);
        }
        fn stderr_tail(&self) -> String {
            String::new()
        }
    }

    #[test]
    fn scripted_child_has_no_stderr_tail() {
        let (exit_tx, exit_rx) = tokio::sync::watch::channel(false);
        let child = ScriptedChild { exit_tx, exit_rx };
        assert_eq!(crate::engine::process::EngineChild::stderr_tail(&child), "");
    }

    #[async_trait::async_trait]
    impl crate::engine::process::EngineProcess for ScriptedEngineProcess {
        async fn spawn(
            &self,
            _args: &crate::engine::process::SpawnArgs,
        ) -> Result<Box<dyn crate::engine::process::EngineChild>, String> {
            if let Some(ref message) = self.spawn_error {
                return Err(message.clone());
            }
            let (exit_tx, exit_rx) = tokio::sync::watch::channel(false);
            Ok(Box::new(ScriptedChild { exit_tx, exit_rx }))
        }
        fn free_port(&self) -> Result<u16, String> {
            Ok(self.port)
        }
        async fn health_probe(&self, _port: u16) -> Result<u16, String> {
            if !self.healthy {
                // Hangs until the poll task is dropped by a kill; the
                // answer below is only ever reached on the healthy path.
                std::future::pending::<()>().await;
            }
            Ok(200)
        }
    }

    /// Helper: an [`crate::engine::state::Target`] with placeholder paths.
    fn engine_target() -> crate::engine::state::Target {
        crate::engine::state::Target {
            model_path: std::path::PathBuf::from("/tmp/m.gguf"),
            mmproj_path: None,
            num_ctx: DEFAULT_NUM_CTX,
        }
    }

    /// Helper: an [`crate::engine::runner::EngineHandle`] over a scripted
    /// process with idle unload disabled.
    fn spawn_engine(process: ScriptedEngineProcess) -> crate::engine::runner::EngineHandle {
        crate::engine::runner::EngineHandle::spawn(
            Arc::new(process),
            0,
            std::time::Duration::from_secs(3600),
        )
    }

    #[tokio::test]
    async fn stream_builtin_chat_streams_from_engine_port() {
        let mut server = mockito::Server::new_async().await;
        let port: u16 = server
            .url()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .expect("mockito url ends in a port");
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_header("content-type", "text/event-stream")
            .with_body("data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n")
            .create_async()
            .await;

        let engine = spawn_engine(ScriptedEngineProcess {
            port,
            spawn_error: None,
            healthy: true,
        });
        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_builtin_chat(
            &engine,
            engine_target(),
            "org/repo:m.gguf".to_string(),
            false,
            vec![],
            &client,
            CancellationToken::new(),
            &crate::warmup::BuiltinWarmState::default(),
            noop_on_warmed(),
            callback,
        )
        .await;

        mock.assert_async().await;
        assert_eq!(accumulated, "Hi");
        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Token(t) if t == "Hi"));
        assert_eq!(
            std::mem::discriminant(&chunks[1]),
            std::mem::discriminant(&StreamChunk::Done)
        );
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn stream_builtin_chat_announces_warmed_exactly_once_on_first_token() {
        let mut server = mockito::Server::new_async().await;
        let port: u16 = server
            .url()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .expect("mockito url ends in a port");
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_header("content-type", "text/event-stream")
            .with_body(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n\
                 data: {\"choices\":[{\"delta\":{\"content\":\" there\"}}]}\n\n\
                 data: [DONE]\n",
            )
            .create_async()
            .await;

        let engine = spawn_engine(ScriptedEngineProcess {
            port,
            spawn_error: None,
            healthy: true,
        });
        let client = reqwest::Client::new();
        let (_chunks, callback) = collect_chunks();
        let warm_state = crate::warmup::BuiltinWarmState::default();
        let (warmed_count, on_warmed) = warmed_counter();
        stream_builtin_chat(
            &engine,
            engine_target(),
            "org/repo:m.gguf".to_string(),
            false,
            vec![],
            &client,
            CancellationToken::new(),
            &warm_state,
            on_warmed,
            callback,
        )
        .await;

        mock.assert_async().await;
        assert_eq!(
            warmed_count.load(Ordering::Relaxed),
            1,
            "two tokens stream but on_warmed fires only for the first"
        );
        assert!(
            !warm_state.try_begin(port),
            "the real request's first token marked this port as warmed"
        );
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn stream_builtin_chat_skips_on_warmed_when_the_port_is_already_marked() {
        let mut server = mockito::Server::new_async().await;
        let port: u16 = server
            .url()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .expect("mockito url ends in a port");
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_header("content-type", "text/event-stream")
            .with_body("data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n")
            .create_async()
            .await;

        let engine = spawn_engine(ScriptedEngineProcess {
            port,
            spawn_error: None,
            healthy: true,
        });
        let client = reqwest::Client::new();
        let (_chunks, callback) = collect_chunks();
        let warm_state = crate::warmup::BuiltinWarmState::default();
        // A proactive prime already announced this port as warmed before the
        // real request's first token arrives.
        assert!(warm_state.mark_warmed_by_real_request(port));
        let (warmed_count, on_warmed) = warmed_counter();
        stream_builtin_chat(
            &engine,
            engine_target(),
            "org/repo:m.gguf".to_string(),
            false,
            vec![],
            &client,
            CancellationToken::new(),
            &warm_state,
            on_warmed,
            callback,
        )
        .await;

        mock.assert_async().await;
        assert_eq!(
            warmed_count.load(Ordering::Relaxed),
            0,
            "the port was already announced warmed; no redundant emit"
        );
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn superseded_ensure_emits_cancelled() {
        // Health probes hang, so the ensure stays in flight until the
        // unload preempts it.
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 1,
            spawn_error: None,
            healthy: false,
        });
        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();

        let task = {
            let engine = engine.clone();
            tokio::spawn(async move {
                stream_builtin_chat(
                    &engine,
                    engine_target(),
                    "org/repo:m.gguf".to_string(),
                    false,
                    vec![],
                    &client,
                    CancellationToken::new(),
                    &crate::warmup::BuiltinWarmState::default(),
                    noop_on_warmed(),
                    callback,
                )
                .await
            })
        };

        // Wait until the spawn landed and the health poll is in flight,
        // then preempt the waiting ensure.
        let mut status = engine.status();
        status
            .wait_for(|s| s.state == "starting")
            .await
            .expect("actor is running");
        engine.unload().await;

        let accumulated = task.await.unwrap();
        assert_eq!(accumulated, "");
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1, "exactly one terminal chunk");
        assert_eq!(
            std::mem::discriminant(&chunks[0]),
            std::mem::discriminant(&StreamChunk::Cancelled)
        );
        engine.shutdown().await;
    }

    /// A Stop press while the engine is still cold-loading must terminate
    /// the chat turn immediately with a terminal `Cancelled`, not after the
    /// load completes. The load itself keeps running in the background so
    /// the next message reuses it.
    #[tokio::test]
    async fn cancel_during_ensure_emits_cancelled_and_keeps_load_running() {
        // Health probes hang, so the ensure stays in flight until cancelled.
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 1,
            spawn_error: None,
            healthy: false,
        });
        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let cancel_token = CancellationToken::new();

        let task = {
            let engine = engine.clone();
            let cancel_token = cancel_token.clone();
            tokio::spawn(async move {
                stream_builtin_chat(
                    &engine,
                    engine_target(),
                    "org/repo:m.gguf".to_string(),
                    false,
                    vec![],
                    &client,
                    cancel_token,
                    &crate::warmup::BuiltinWarmState::default(),
                    noop_on_warmed(),
                    callback,
                )
                .await
            })
        };

        // Wait until the spawn landed and the health poll is in flight,
        // then cancel the turn.
        let mut status = engine.status();
        status
            .wait_for(|s| s.state == "starting")
            .await
            .expect("actor is running");
        cancel_token.cancel();

        let accumulated = task.await.unwrap();
        assert_eq!(accumulated, "");
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1, "exactly one terminal chunk");
        assert_eq!(
            std::mem::discriminant(&chunks[0]),
            std::mem::discriminant(&StreamChunk::Cancelled)
        );
        // The load was not aborted: the engine is still starting.
        assert_eq!(engine.status().borrow().state, "starting");
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn start_failed_maps_engine_start_failed() {
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 1,
            spawn_error: Some("spawn boom".to_string()),
            healthy: true,
        });
        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_builtin_chat(
            &engine,
            engine_target(),
            "org/repo:m.gguf".to_string(),
            false,
            vec![],
            &client,
            CancellationToken::new(),
            &crate::warmup::BuiltinWarmState::default(),
            noop_on_warmed(),
            callback,
        )
        .await;

        assert_eq!(accumulated, "");
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1, "exactly one terminal chunk");
        assert!(matches!(
            &chunks[0],
            StreamChunk::Error(e)
                if e.kind == EngineErrorKind::EngineStartFailed                && e.message.contains("spawn boom")
        ));
        engine.shutdown().await;
    }

    // ─── /props runtime vision gate ─────────────────────────────────────

    #[test]
    fn parse_props_vision_true_false_absent_malformed() {
        assert!(parse_props_vision(br#"{"modalities":{"vision":true}}"#));
        assert!(!parse_props_vision(br#"{"modalities":{"vision":false}}"#));
        assert!(!parse_props_vision(br#"{"modalities":{}}"#), "absent flag");
        assert!(!parse_props_vision(br#"{}"#), "absent modalities");
        assert!(
            !parse_props_vision(br#"{"modalities":{"vision":"yes"}}"#),
            "non-boolean flag fails closed"
        );
        assert!(!parse_props_vision(b"not json"), "malformed body");
    }

    #[test]
    fn observe_reasoning_chunk_sets_flag_only_on_thinking_token() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        observe_reasoning_chunk(&StreamChunk::Token("hi".into()), &flag);
        assert!(!flag.load(std::sync::atomic::Ordering::Relaxed));
        observe_reasoning_chunk(&StreamChunk::Done, &flag);
        assert!(!flag.load(std::sync::atomic::Ordering::Relaxed));
        observe_reasoning_chunk(&StreamChunk::ThinkingToken("step".into()), &flag);
        assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn should_backstop_mark_only_fires_for_surprising_pasted_reasoning() {
        // Reasoning requested OFF, model still reasoned, not yet recorded, not
        // curated: the one case that should mark.
        assert!(should_backstop_mark(false, true, false, false));
        // /think was on: expected reasoning, never a surprise.
        assert!(!should_backstop_mark(true, true, false, false));
        // No reasoning streamed: nothing to learn.
        assert!(!should_backstop_mark(false, false, false, false));
        // Already recorded as always: no redundant write.
        assert!(!should_backstop_mark(false, true, true, false));
        // Curated starter: registry is truth, never override from behavior.
        assert!(!should_backstop_mark(false, true, false, true));
    }

    #[tokio::test]
    async fn fetch_builtin_vision_transport_error_fails_closed() {
        let client = reqwest::Client::new();
        assert!(!fetch_builtin_vision(&client, "http://127.0.0.1:1").await);
    }

    /// A 2xx `/props` response whose body dies mid-read (connection closed
    /// before the promised Content-Length) fails closed, like every other
    /// gate failure mode.
    #[tokio::test]
    async fn fetch_builtin_vision_body_read_failure_fails_closed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 8192];
            let _ = stream.read(&mut req_buf).await;
            // Promise more bytes than are sent, then shut down.
            let response =
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{\"modalities\"";
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        });

        let client = reqwest::Client::new();
        assert!(!fetch_builtin_vision(&client, &format!("http://127.0.0.1:{port}")).await);
    }

    /// Messages carrying one image, as the gate sees them after the
    /// capability snapshot is built.
    fn image_message() -> Vec<ChatMessage> {
        vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            images: Some(vec!["QUJD".to_string()]),
        }]
    }

    /// Drives `stream_builtin_chat` against a mockito server acting as the
    /// engine port, with `/props` scripted to report `vision` and the chat
    /// mock matching `expected_chat_body`. Returns once both mocks assert.
    async fn run_props_gate_case(vision: bool, expected_chat_body: &str) {
        let mut server = mockito::Server::new_async().await;
        let port: u16 = server
            .url()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .expect("mockito url ends in a port");
        let props_mock = server
            .mock("GET", "/props")
            .with_status(200)
            .with_body(format!(r#"{{"modalities":{{"vision":{vision}}}}}"#))
            .create_async()
            .await;
        let chat_mock = server
            .mock("POST", "/v1/chat/completions")
            .match_body(mockito::Matcher::PartialJsonString(
                expected_chat_body.to_string(),
            ))
            .with_header("content-type", "text/event-stream")
            .with_body("data: [DONE]\n")
            .create_async()
            .await;

        let engine = spawn_engine(ScriptedEngineProcess {
            port,
            spawn_error: None,
            healthy: true,
        });
        let client = reqwest::Client::new();
        let (_chunks, callback) = collect_chunks();
        stream_builtin_chat(
            &engine,
            engine_target(),
            "org/repo:m.gguf".to_string(),
            false,
            image_message(),
            &client,
            CancellationToken::new(),
            &crate::warmup::BuiltinWarmState::default(),
            noop_on_warmed(),
            callback,
        )
        .await;

        props_mock.assert_async().await;
        chat_mock.assert_async().await;
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn props_gate_strips_images_when_vision_unloaded() {
        // vision:false -> the image part is stripped, so the wire message
        // collapses to the plain-string content shape.
        run_props_gate_case(false, r#"{"messages":[{"role":"user","content":"hi"}]}"#).await;
    }

    #[tokio::test]
    async fn props_gate_keeps_images_when_vision_supported() {
        // vision:true -> the multipart content shape with the image part
        // reaches the wire untouched.
        run_props_gate_case(
            true,
            r#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"data:image/jpeg;base64,QUJD"}}]}]}"#,
        )
        .await;
    }

    // ─── LlmTransport / resolve_llm_transport ───────────────────────────

    #[test]
    fn llm_transport_endpoint_label_names_the_wire_target() {
        let native = LlmTransport::OllamaNative {
            endpoint: "http://127.0.0.1:11434/api/chat".to_string(),
        };
        assert_eq!(native.endpoint_label(), "http://127.0.0.1:11434/api/chat");
        let v1 = LlmTransport::V1 {
            base_url: "http://localhost:8080".to_string(),
            api_key: None,
            flavor: crate::openai::V1Flavor::Remote,
        };
        assert_eq!(
            v1.endpoint_label(),
            "http://localhost:8080/v1/chat/completions"
        );
    }

    #[test]
    fn llm_transport_debug_redacts_api_key() {
        let with_key = LlmTransport::V1 {
            base_url: "https://api.openai.com".to_string(),
            api_key: Some("sk-supersecret".to_string()),
            flavor: crate::openai::V1Flavor::Remote,
        };
        let debug = format!("{with_key:?}");
        assert!(
            !debug.contains("sk-supersecret"),
            "key must not appear in Debug output"
        );
        assert!(
            debug.contains("<redacted>"),
            "redacted placeholder must be present"
        );

        let no_key = LlmTransport::V1 {
            base_url: "http://127.0.0.1:8080".to_string(),
            api_key: None,
            flavor: crate::openai::V1Flavor::Builtin,
        };
        let debug_none = format!("{no_key:?}");
        assert!(debug_none.contains("None"), "None key must show as None");
        assert!(
            debug_none.contains("Builtin"),
            "flavor must appear in Debug output"
        );

        // OllamaNative has no key field; just verify it formats without panic.
        let native = LlmTransport::OllamaNative {
            endpoint: "http://127.0.0.1:11434/api/chat".to_string(),
        };
        let debug_native = format!("{native:?}");
        assert!(debug_native.contains("OllamaNative"));
    }

    // ─── model_for_route ────────────────────────────────────────────────────

    #[test]
    fn model_for_route_prefers_builtin_provider_model() {
        let route = ChatRoute::Builtin {
            model_id: "org/repo:m.gguf".to_string(),
        };
        assert_eq!(
            model_for_route(&route, Some("gemma3:12b".to_string())),
            Some("org/repo:m.gguf".to_string())
        );
        assert_eq!(
            model_for_route(&route, None),
            Some("org/repo:m.gguf".to_string())
        );
    }

    #[test]
    fn model_for_route_keeps_fallback_for_non_builtin_routes() {
        let ollama = ChatRoute::OllamaNative {
            endpoint: "http://127.0.0.1:11434/api/chat".to_string(),
        };
        assert_eq!(
            model_for_route(&ollama, Some("gemma3:12b".to_string())),
            Some("gemma3:12b".to_string())
        );
        assert_eq!(model_for_route(&ollama, None), None);

        let v1 = ChatRoute::V1 {
            base_url: "http://localhost:8080".to_string(),
            api_key_provider: None,
        };
        assert_eq!(
            model_for_route(&v1, Some("gpt-4o".to_string())),
            Some("gpt-4o".to_string())
        );
        assert_eq!(model_for_route(&v1, None), None);
    }

    /// Helper: a `Database` over a fresh in-memory schema.
    fn test_db() -> crate::history::Database {
        crate::history::Database(StdMutex::new(crate::database::open_in_memory().unwrap()))
    }

    /// Helper: a `ModelStore` rooted in a fresh temp dir, plus the dir guard.
    fn test_store() -> (tempfile::TempDir, crate::models::storage::ModelStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::models::storage::ModelStore::new(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    #[tokio::test]
    async fn resolve_llm_transport_passes_ollama_endpoint_through() {
        let db = test_db();
        let (_dir, store) = test_store();
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 1,
            spawn_error: None,
            healthy: true,
        });
        let secrets = crate::keychain::FakeSecretStore::new();
        let transport = resolve_llm_transport(
            ChatRoute::OllamaNative {
                endpoint: "http://127.0.0.1:11434/api/chat".to_string(),
            },
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            // Non-builtin route: the memory gate never runs, so the policy is
            // irrelevant here.
            OversizePolicy::Block { forced: false },
        )
        .await
        .unwrap();
        assert_eq!(
            transport,
            LlmTransport::OllamaNative {
                endpoint: "http://127.0.0.1:11434/api/chat".to_string(),
            }
        );
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn resolve_llm_transport_v1_resolves_api_key() {
        use crate::keychain::SecretStore;
        let db = test_db();
        let (_dir, store) = test_store();
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 1,
            spawn_error: None,
            healthy: true,
        });
        let secrets = crate::keychain::FakeSecretStore::new();
        secrets.set("p1", "sk-test").unwrap();
        let transport = resolve_llm_transport(
            ChatRoute::V1 {
                base_url: "http://localhost:8080".to_string(),
                api_key_provider: Some("p1".to_string()),
            },
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            // Non-builtin route: the memory gate never runs.
            OversizePolicy::Block { forced: false },
        )
        .await
        .unwrap();
        assert_eq!(
            transport,
            LlmTransport::V1 {
                base_url: "http://localhost:8080".to_string(),
                api_key: Some("sk-test".to_string()),
                flavor: crate::openai::V1Flavor::Remote,
            }
        );
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn resolve_llm_transport_builtin_ensures_engine() {
        let db = test_db();
        {
            let conn = db.0.lock().unwrap();
            crate::models::manifest::insert(
                &conn,
                &installed_model("org/repo:m.gguf", "sha_w", None),
            )
            .unwrap();
        }
        let (_dir, store) = test_store();
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 4242,
            spawn_error: None,
            healthy: true,
        });
        let secrets = crate::keychain::FakeSecretStore::new();
        let transport = resolve_llm_transport(
            ChatRoute::Builtin {
                model_id: "org/repo:m.gguf".to_string(),
            },
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            // Force past the memory gate: this test exercises the ensure path,
            // orthogonal to the gate (covered separately below).
            OversizePolicy::Block { forced: true },
        )
        .await
        .unwrap();
        assert_eq!(
            transport,
            LlmTransport::V1 {
                base_url: "http://127.0.0.1:4242".to_string(),
                api_key: None,
                flavor: crate::openai::V1Flavor::Builtin,
            }
        );
        // The ensure landed: the engine reports the loaded model.
        assert_eq!(engine.status().borrow().state, "loaded");
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn resolve_llm_transport_builtin_missing_row_is_engine_error() {
        let db = test_db();
        let (_dir, store) = test_store();
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 1,
            spawn_error: None,
            healthy: true,
        });
        let secrets = crate::keychain::FakeSecretStore::new();
        let err = resolve_llm_transport(
            ChatRoute::Builtin {
                model_id: "org/repo:gone.gguf".to_string(),
            },
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            // `builtin_target` errors on the missing row before the gate runs,
            // so the policy is irrelevant.
            OversizePolicy::Block { forced: false },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            TransportError::Engine(e) if e.kind == EngineErrorKind::ModelNotFound
        ));
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn resolve_llm_transport_recovers_poisoned_db_lock() {
        let db = test_db();
        {
            let conn = db.0.lock().unwrap();
            crate::models::manifest::insert(
                &conn,
                &installed_model("org/repo:m.gguf", "sha_w", None),
            )
            .unwrap();
        }
        // Poison the connection mutex with an unrelated panic; the resolver
        // must recover the guard rather than fail the turn.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = db.0.lock().unwrap();
            panic!("poison");
        }));
        assert!(db.0.lock().is_err(), "mutex must be poisoned");

        let (_dir, store) = test_store();
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 4243,
            spawn_error: None,
            healthy: true,
        });
        let secrets = crate::keychain::FakeSecretStore::new();
        let transport = resolve_llm_transport(
            ChatRoute::Builtin {
                model_id: "org/repo:m.gguf".to_string(),
            },
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            // Force past the gate: this test exercises poisoned-lock recovery.
            OversizePolicy::Block { forced: true },
        )
        .await
        .unwrap();
        assert_eq!(
            transport,
            LlmTransport::V1 {
                base_url: "http://127.0.0.1:4243".to_string(),
                api_key: None,
                flavor: crate::openai::V1Flavor::Builtin,
            }
        );
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn resolve_llm_transport_superseded_and_start_failed_map() {
        // StartFailed: every spawn errors out.
        let db = test_db();
        {
            let conn = db.0.lock().unwrap();
            crate::models::manifest::insert(
                &conn,
                &installed_model("org/repo:m.gguf", "sha_w", None),
            )
            .unwrap();
        }
        let (_dir, store) = test_store();
        let secrets = crate::keychain::FakeSecretStore::new();
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 1,
            spawn_error: Some("spawn boom".to_string()),
            healthy: true,
        });
        let err = resolve_llm_transport(
            ChatRoute::Builtin {
                model_id: "org/repo:m.gguf".to_string(),
            },
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            // Force past the gate: this asserts the spawn StartFailed mapping,
            // which a gate Block would preempt on a low-memory runner.
            OversizePolicy::Block { forced: true },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            TransportError::Engine(ref e)
                if e.kind == EngineErrorKind::EngineStartFailed                && e.message.contains("spawn boom")
        ));
        engine.shutdown().await;

        // Superseded: health probes hang, so the in-flight ensure can be
        // preempted by an unload.
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 1,
            spawn_error: None,
            healthy: false,
        });
        let task = {
            let engine = engine.clone();
            let db = test_db();
            {
                let conn = db.0.lock().unwrap();
                crate::models::manifest::insert(
                    &conn,
                    &installed_model("org/repo:m.gguf", "sha_w", None),
                )
                .unwrap();
            }
            let (_dir2, store2) = test_store();
            tokio::spawn(async move {
                let secrets = crate::keychain::FakeSecretStore::new();
                resolve_llm_transport(
                    ChatRoute::Builtin {
                        model_id: "org/repo:m.gguf".to_string(),
                    },
                    &db,
                    &store2,
                    &engine,
                    &secrets,
                    DEFAULT_NUM_CTX,
                    &CancellationToken::new(),
                    // Force past the gate: this asserts the unload-preempt
                    // Superseded mapping, orthogonal to the memory gate.
                    OversizePolicy::Block { forced: true },
                )
                .await
            })
        };
        let mut status = engine.status();
        status
            .wait_for(|s| s.state == "starting")
            .await
            .expect("actor is running");
        engine.unload().await;
        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err, TransportError::Superseded);
        engine.shutdown().await;
    }

    /// A Stop press while the builtin ensure is in flight resolves the
    /// transport as `Cancelled` immediately; the load keeps running in the
    /// background so the next pipeline turn reuses it.
    #[tokio::test]
    async fn resolve_llm_transport_cancel_during_ensure_maps_cancelled() {
        let db = test_db();
        {
            let conn = db.0.lock().unwrap();
            crate::models::manifest::insert(
                &conn,
                &installed_model("org/repo:m.gguf", "sha_w", None),
            )
            .unwrap();
        }
        let (_dir, store) = test_store();
        // Health probes hang, so the ensure stays in flight until cancelled.
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 1,
            spawn_error: None,
            healthy: false,
        });
        let cancel_token = CancellationToken::new();
        let task = {
            let engine = engine.clone();
            let cancel_token = cancel_token.clone();
            tokio::spawn(async move {
                let secrets = crate::keychain::FakeSecretStore::new();
                resolve_llm_transport(
                    ChatRoute::Builtin {
                        model_id: "org/repo:m.gguf".to_string(),
                    },
                    &db,
                    &store,
                    &engine,
                    &secrets,
                    DEFAULT_NUM_CTX,
                    &cancel_token,
                    // Force past the gate: this asserts the cancel-during-ensure
                    // mapping, orthogonal to the memory gate.
                    OversizePolicy::Block { forced: true },
                )
                .await
            })
        };
        let mut status = engine.status();
        status
            .wait_for(|s| s.state == "starting")
            .await
            .expect("actor is running");
        cancel_token.cancel();
        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err, TransportError::Cancelled);
        // The load was not aborted: the engine is still starting.
        assert_eq!(engine.status().borrow().state, "starting");
        engine.shutdown().await;
    }

    /// Builds an installed row whose weights are so large the memory gate can
    /// never fit them: `u64::MAX` saturates `estimate_required_bytes` to
    /// `u64::MAX`, which exceeds the ceiling for any live available figure > 0
    /// (guaranteed by `available_memory_bytes_reads_a_plausible_live_value`).
    /// Keeps the `Block` outcome deterministic regardless of the runner's real
    /// free memory; do not shrink this size or the block-path tests go flaky.
    fn oversized_model(id: &str) -> crate::models::manifest::InstalledModel {
        let mut model = installed_model(id, "sha_big", None);
        model.size_bytes = u64::MAX;
        model
    }

    /// `/search`'s policy (`Block { forced: false }`) on an over-large builtin
    /// model surfaces the user-facing insufficient-memory error and never
    /// loads the sidecar.
    #[tokio::test]
    async fn resolve_llm_transport_builtin_blocks_oversized_unforced() {
        let db = test_db();
        {
            let conn = db.0.lock().unwrap();
            crate::models::manifest::insert(&conn, &oversized_model("org/repo:big.gguf")).unwrap();
        }
        let (_dir, store) = test_store();
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 4444,
            spawn_error: None,
            healthy: true,
        });
        let secrets = crate::keychain::FakeSecretStore::new();
        let err = resolve_llm_transport(
            ChatRoute::Builtin {
                model_id: "org/repo:big.gguf".to_string(),
            },
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            OversizePolicy::Block { forced: false },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            TransportError::Engine(e) if e.kind == EngineErrorKind::InsufficientMemory
        ));
        // The gate ran before the ensure: the sidecar was never started.
        assert_eq!(engine.status().borrow().state, "stopped");
        engine.shutdown().await;
    }

    /// The user's "load anyway" (`Block { forced: true }`) bypasses the gate on
    /// the same over-large model and proceeds to load.
    #[tokio::test]
    async fn resolve_llm_transport_builtin_forced_loads_oversized() {
        let db = test_db();
        {
            let conn = db.0.lock().unwrap();
            crate::models::manifest::insert(&conn, &oversized_model("org/repo:big.gguf")).unwrap();
        }
        let (_dir, store) = test_store();
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 4445,
            spawn_error: None,
            healthy: true,
        });
        let secrets = crate::keychain::FakeSecretStore::new();
        let transport = resolve_llm_transport(
            ChatRoute::Builtin {
                model_id: "org/repo:big.gguf".to_string(),
            },
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            OversizePolicy::Block { forced: true },
        )
        .await
        .unwrap();
        assert_eq!(
            transport,
            LlmTransport::V1 {
                base_url: "http://127.0.0.1:4445".to_string(),
                api_key: None,
                flavor: crate::openai::V1Flavor::Builtin,
            }
        );
        assert_eq!(engine.status().borrow().state, "loaded");
        engine.shutdown().await;
    }

    /// History title generation's policy (`SilentSkip`) on an over-large model
    /// yields the benign `SkippedInsufficientMemory`, not a user-facing error,
    /// and never loads the sidecar.
    #[tokio::test]
    async fn resolve_llm_transport_builtin_silent_skip_oversized() {
        let db = test_db();
        {
            let conn = db.0.lock().unwrap();
            crate::models::manifest::insert(&conn, &oversized_model("org/repo:big.gguf")).unwrap();
        }
        let (_dir, store) = test_store();
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 4446,
            spawn_error: None,
            healthy: true,
        });
        let secrets = crate::keychain::FakeSecretStore::new();
        let err = resolve_llm_transport(
            ChatRoute::Builtin {
                model_id: "org/repo:big.gguf".to_string(),
            },
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            OversizePolicy::SilentSkip,
        )
        .await
        .unwrap_err();
        assert_eq!(err, TransportError::SkippedInsufficientMemory);
        assert_eq!(engine.status().borrow().state, "stopped");
        engine.shutdown().await;
    }

    /// When the exact model is already resident, the gate's same-model
    /// short-circuit proceeds without any memory arithmetic under BOTH
    /// policies, even for an over-large model that a cold load would block.
    /// This proves the resident short-circuit is load-bearing: the `u64::MAX`
    /// weights would otherwise force `Block`.
    #[tokio::test]
    async fn resolve_llm_transport_builtin_proceeds_when_already_resident() {
        let db = test_db();
        {
            let conn = db.0.lock().unwrap();
            crate::models::manifest::insert(&conn, &oversized_model("org/repo:big.gguf")).unwrap();
        }
        let (_dir, store) = test_store();
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 4447,
            spawn_error: None,
            healthy: true,
        });
        let secrets = crate::keychain::FakeSecretStore::new();
        // `ChatRoute` is not `Clone`, so each call rebuilds the same route.
        let builtin_route = || ChatRoute::Builtin {
            model_id: "org/repo:big.gguf".to_string(),
        };
        // Prime the sidecar with a forced load so the exact model is resident.
        resolve_llm_transport(
            builtin_route(),
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            OversizePolicy::Block { forced: true },
        )
        .await
        .unwrap();
        assert_eq!(engine.status().borrow().state, "loaded");
        let expected = LlmTransport::V1 {
            base_url: "http://127.0.0.1:4447".to_string(),
            api_key: None,
            flavor: crate::openai::V1Flavor::Builtin,
        };
        // Unforced now proceeds because the model is already resident.
        let unforced = resolve_llm_transport(
            builtin_route(),
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            OversizePolicy::Block { forced: false },
        )
        .await
        .unwrap();
        assert_eq!(unforced, expected);
        // SilentSkip likewise proceeds on the resident model.
        let skip = resolve_llm_transport(
            builtin_route(),
            &db,
            &store,
            &engine,
            &secrets,
            DEFAULT_NUM_CTX,
            &CancellationToken::new(),
            OversizePolicy::SilentSkip,
        )
        .await
        .unwrap();
        assert_eq!(skip, expected);
        engine.shutdown().await;
    }

    /// Only builtin routes pin the engine: a guard for any other kind would
    /// keep a previously loaded sidecar resident while the user chats
    /// through Ollama or a remote `/v1` server.
    #[tokio::test]
    async fn route_activity_guard_acquires_for_builtin_routes_only() {
        let engine = spawn_engine(ScriptedEngineProcess {
            port: 1,
            spawn_error: None,
            healthy: true,
        });
        let builtin = ChatRoute::Builtin {
            model_id: "org/repo:m.gguf".to_string(),
        };
        let ollama = ChatRoute::OllamaNative {
            endpoint: "http://127.0.0.1:11434/api/chat".to_string(),
        };
        let v1 = ChatRoute::V1 {
            base_url: "http://localhost:8080".to_string(),
            api_key_provider: None,
        };
        assert!(route_activity_guard(&builtin, &engine).is_some());
        assert!(route_activity_guard(&ollama, &engine).is_none());
        assert!(route_activity_guard(&v1, &engine).is_none());
        engine.shutdown().await;
    }
}
