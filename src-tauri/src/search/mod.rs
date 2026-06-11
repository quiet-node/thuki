//! `/search` pipeline module.
//!
//! Public surface:
//! - [`SearchEvent`] - the streamed event type used on the frontend IPC
//!   channel.
//! - [`search_pipeline`] - the single Tauri command that owns the entire
//!   classify -> route -> answer flow.
//!
//! Everything else is internal. The pipeline shares Ollama streaming
//! primitives with the main chat path (`commands::stream_ollama_chat`) and
//! persists completed turns into the shared [`ConversationHistory`] so that
//! subsequent user messages see the full conversational state regardless of
//! whether they went through `/search` or the normal chat command.

use std::sync::Arc;

use tauri::{ipc::Channel, State};
use tokio_util::sync::CancellationToken;

use crate::commands::{ConversationHistory, GenerationState};
use crate::config::AppConfig;
use crate::models::ActiveModelState;
use crate::trace::{BoundRecorder, ConversationId, LiveTraceRecorder, TraceRecorder};

pub mod chunker;
pub mod config;
pub mod errors;
pub mod judge;
mod llm;
pub mod pipeline;
pub mod probe;
pub mod reader;
mod rerank;
mod searxng;
mod types;

pub use llm::{JudgeSource, JudgeStage};
pub use pipeline::{run_agentic, JudgeCaller, RouterJudgeCaller};
pub use probe::probe;
pub use types::{
    Action, IterationStage, IterationTrace, JudgeVerdict, RouterJudgeOutput, SearchError,
    SearchEvent, SearchMetadata, SearchWarning, Sufficiency,
};

/// Umbrella Tauri command implementing the full `/search` agentic pipeline.
///
/// The frontend passes in the user's raw query plus a typed
/// [`tauri::ipc::Channel`] to receive [`SearchEvent`]s. The backend is the
/// sole owner of routing state, history mutation, cancellation, and error
/// presentation - the frontend is a pure renderer of whichever events arrive.
///
/// Reuses the shared [`GenerationState`] so a single `cancel_generation`
/// invocation cancels either a chat or a search turn, whichever is active.
///
/// Dispatches to [`pipeline::run_agentic`] using [`pipeline::DefaultRouterJudge`]
/// and [`pipeline::DefaultJudge`] as the production LLM callers.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
#[allow(clippy::too_many_arguments)]
pub async fn search_pipeline(
    message: String,
    conversation_id: String,
    is_first_turn: bool,
    displayed_content: Option<String>,
    on_event: Channel<SearchEvent>,
    client: State<'_, reqwest::Client>,
    generation: State<'_, GenerationState>,
    history: State<'_, ConversationHistory>,
    app_config: State<'_, parking_lot::RwLock<AppConfig>>,
    active_model_state: State<'_, ActiveModelState>,
    trace_recorder: State<'_, Arc<LiveTraceRecorder>>,
    db: State<'_, crate::history::Database>,
    model_store: State<'_, crate::models::storage::ModelStore>,
    engine: State<'_, crate::engine::runner::EngineHandle>,
    secrets: State<'_, crate::keychain::Secrets>,
) -> Result<(), String> {
    // Snapshot the config once so the entire pipeline sees a consistent view
    // even if the user edits Settings while a search is in flight.
    let app_config = app_config.read().clone();

    // Route by the active provider's kind, mirroring `ask_model`. A builtin
    // provider with no model picked surfaces as `NoModelSelected` here, so
    // the frontend keeps `is_first_turn` armed exactly like the
    // ActiveModelState bail below.
    let route = match crate::commands::resolve_chat_route(&app_config.inference) {
        Ok(route) => route,
        Err(err) => {
            let _ = on_event.send(route_failure_event(err));
            return Ok(());
        }
    };

    // Resolve the runtime search view from the loaded TOML. The single
    // source of truth lives in `config::defaults`; the loader has already
    // clamped and resolved every field by the time we read it here.
    let runtime_config = config::SearchRuntimeConfig::from_app_config(&app_config);
    let searxng_endpoint = runtime_config.searxng_endpoint();

    // Snapshot the active model slug once from the picker-backed
    // ActiveModelState; drop the guard before any `.await` so we never
    // hold a `MutexGuard` across an await point. Builtin routes carry their
    // model in the provider config instead (see `commands::model_for_route`).
    let active_model = {
        let guard = active_model_state.0.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };
    let Some(model_name) = crate::commands::model_for_route(&route, active_model) else {
        // Mirrors the chat-path gate: refuse to dispatch with no active
        // model. The frontend strip already steers the user to the picker
        // before this point, so this branch is defense-in-depth for the
        // race where the user's last installed model was removed mid-run.
        // Emit a dedicated typed event (not a generic Error) so the frontend
        // can keep `is_first_turn` armed: this bail returns before
        // `ConversationStart` is recorded, so the next attempt must still
        // open the trace as a first turn.
        let _ = on_event.send(SearchEvent::NoModelSelected);
        return Ok(());
    };

    // Pre-flight: verify both sandbox services are reachable before touching
    // the LLM or SearXNG. A 2-second probe prevents a long wait when the
    // containers are simply not running.
    if let Err(_e) = probe(
        &client,
        &runtime_config.searxng_url,
        &runtime_config.reader_url,
    )
    .await
    {
        let _ = on_event.send(SearchEvent::SandboxUnavailable);
        return Ok(());
    }

    // Resolve the wire transport. For the builtin route this marks engine
    // activity and ensures the sidecar serves the selected model before any
    // pipeline stage issues an LLM call.
    let transport = match crate::commands::resolve_llm_transport(
        route,
        &db,
        &model_store,
        &engine,
        secrets.0.as_ref(),
        app_config.inference.num_ctx,
    )
    .await
    {
        Ok(transport) => transport,
        Err(err) => {
            let _ = on_event.send(transport_failure_event(err));
            return Ok(());
        }
    };
    let cancel_token = CancellationToken::new();
    generation.set_token(cancel_token.clone());

    let today = pipeline::today_iso();

    // Pull the per-conversation forensic recorder from the global
    // trace registry. When the dev-only `[debug] trace_enabled` flag is
    // off (production default) the registry is a `NoopRecorder` so this
    // resolves to a zero-cost noop wrapped in `BoundRecorder`. When on,
    // every pipeline step records into the conversation's
    // `traces/search/<conversation_id>.jsonl` file via the registry's
    // lazy-insert path.
    let conv_id = ConversationId::new(conversation_id);
    let live: Arc<LiveTraceRecorder> = Arc::clone(trace_recorder.inner());
    // Coerce the concrete `Arc<LiveTraceRecorder>` to the
    // `Arc<dyn TraceRecorder>` shape `BoundRecorder` expects. The
    // coercion happens at the binding site; calling `record()` on
    // the bound recorder still goes through the live wrapper, so a
    // mid-stream trace toggle takes effect on the next event.
    let live_inner: Arc<dyn TraceRecorder> = live;
    let recorder = Arc::new(BoundRecorder::new(live_inner, conv_id));

    // Mirror the user-perceived turn into the chat-domain trace so the
    // `traces/chat/<conversation_id>.jsonl` file is the canonical
    // user-facing timeline regardless of whether a turn used `/search`
    // or hit `ask_model` directly. Symmetric with what
    // `commands::ask_model` records at its hook sites; the deep
    // search-pipeline internals (LLM calls, judge verdicts, SearXNG
    // queries) stay in the search-domain file via the same conv id.
    crate::commands::record_conversation_start_if_first_turn(
        &recorder,
        is_first_turn,
        model_name.clone(),
        app_config.prompt.resolved_system.clone(),
    );
    // Tell the frontend the trace was opened. Sent unconditionally so
    // the hook can retire its `is_first_turn` flag even if a previous
    // first-turn attempt was cancelled before any token arrived.
    let _ = on_event.send(SearchEvent::TurnAccepted);
    // `displayed_content` is what the user actually typed on screen
    // (e.g. "/search who is Elon Musk?"); `message` is the stripped
    // query the search engine receives. The chat file uses the
    // displayed text for symmetry with non-search turns, where
    // `user_message.content` is the literal user input.
    let user_visible_content = displayed_content.as_deref().unwrap_or(&message).to_owned();
    recorder.record(crate::trace::RecorderEvent::UserMessage {
        content: user_visible_content,
        attached_images: Vec::new(),
        slash_command: Some("/search".to_owned()),
    });
    let stream_started_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let token_count = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    let router = pipeline::DefaultRouterJudge::new(
        transport.clone(),
        model_name.clone(),
        (*client).clone(),
        cancel_token.clone(),
        today.clone(),
        runtime_config.router_timeout_s,
        app_config.inference.num_ctx,
        Arc::clone(&recorder),
    );
    let judge = pipeline::DefaultJudge::new(
        transport.clone(),
        model_name.clone(),
        (*client).clone(),
        cancel_token.clone(),
        runtime_config.judge_timeout_s,
        app_config.inference.num_ctx,
        Arc::clone(&recorder),
    );

    let recorder_for_pump = Arc::clone(&recorder);
    let token_count_for_pump = Arc::clone(&token_count);
    let result = pipeline::run_agentic(
        &transport,
        &searxng_endpoint,
        &runtime_config.reader_url,
        &model_name,
        &client,
        cancel_token.clone(),
        &app_config.prompt.resolved_system,
        &history,
        message,
        &today,
        &|event| {
            // Mirror synthesized-answer tokens into the chat-domain
            // trace so the chat file's `assistant_tokens` stream
            // matches what the user reads on screen, exactly like a
            // non-search turn. Other `SearchEvent` variants (status
            // pills, source URLs, warnings) stay in the search-domain
            // file; they were intentionally dropped from the chat
            // mirror to keep chat turns shape-symmetric across normal
            // and `/search` paths.
            if let SearchEvent::Token { content } = &event {
                token_count_for_pump.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                recorder_for_pump.record(crate::trace::RecorderEvent::AssistantTokens {
                    chunk: content.clone(),
                });
            }
            let _ = on_event.send(event);
        },
        &router,
        &judge,
        &runtime_config,
        app_config.inference.num_ctx,
        &recorder,
    )
    .await;

    if let Err(e) = result {
        // Cancelled is already surfaced via the Cancelled event by `run_agentic`;
        // only emit an Error event for true failure paths.
        if e != types::SearchError::Cancelled && e != types::SearchError::EmptyQuery {
            // SandboxUnavailable gets its own typed event so the frontend can
            // render the setup-guidance card rather than the generic error bubble.
            if e == types::SearchError::SandboxUnavailable {
                let _ = on_event.send(SearchEvent::SandboxUnavailable);
            } else {
                let _ = on_event.send(SearchEvent::Error {
                    message: e.user_message(),
                });
            }
        }
    }

    // Close the chat-domain user-perceived turn even on error paths so
    // the chat file's `assistant_complete` always pairs with the
    // earlier `user_message`. `total_tokens` reflects the synthesized
    // tokens streamed to the user (zero on early-bail paths like
    // `SandboxUnavailable`).
    let stream_ended_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    recorder.record(crate::trace::RecorderEvent::AssistantComplete {
        total_tokens: token_count.load(std::sync::atomic::Ordering::Relaxed),
        latency_ms: stream_ended_ms.saturating_sub(stream_started_ms),
    });

    generation.clear_token();
    Ok(())
}

/// Maps a [`crate::commands::resolve_chat_route`] failure onto the search
/// event stream. A builtin provider with no model picked must surface as the
/// typed `NoModelSelected` event (keeping the frontend's `is_first_turn`
/// armed), not as a generic error bubble; every other route failure carries
/// its user-facing message.
fn route_failure_event(err: crate::commands::EngineError) -> SearchEvent {
    if err.kind == crate::commands::EngineErrorKind::NoModelSelected {
        SearchEvent::NoModelSelected
    } else {
        SearchEvent::Error {
            message: err.message,
        }
    }
}

/// Maps a [`crate::commands::resolve_llm_transport`] failure onto the search
/// event stream. `Superseded` means a newer settings change preempted the
/// engine ensure: a cancellation, never an error. Engine failures (start
/// failure, missing manifest row) carry their user-facing message.
fn transport_failure_event(err: crate::commands::TransportError) -> SearchEvent {
    match err {
        crate::commands::TransportError::Superseded => SearchEvent::Cancelled,
        crate::commands::TransportError::Engine(e) => SearchEvent::Error { message: e.message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{ChatRoute, EngineError, EngineErrorKind, TransportError};

    #[test]
    fn route_failure_event_maps_no_model_to_typed_event() {
        let event = route_failure_event(EngineError {
            kind: EngineErrorKind::NoModelSelected,
            message: "No model selected\nPick or download a model in Settings.".to_string(),
        });
        assert!(matches!(event, SearchEvent::NoModelSelected));
    }

    #[test]
    fn route_failure_event_maps_other_kinds_to_error_message() {
        let event = route_failure_event(EngineError {
            kind: EngineErrorKind::Other,
            message: "Something went wrong\nThe active provider has an unknown kind.".to_string(),
        });
        assert!(matches!(
            event,
            SearchEvent::Error { message } if message.contains("unknown kind")
        ));
    }

    #[test]
    fn transport_failure_event_maps_superseded_to_cancelled() {
        assert!(matches!(
            transport_failure_event(TransportError::Superseded),
            SearchEvent::Cancelled
        ));
    }

    #[test]
    fn transport_failure_event_maps_engine_error_to_message() {
        let event = transport_failure_event(TransportError::Engine(EngineError {
            kind: EngineErrorKind::EngineStartFailed,
            message: "Thuki's engine could not start.\nspawn boom".to_string(),
        }));
        assert!(matches!(
            event,
            SearchEvent::Error { message } if message.contains("could not start")
        ));
    }
}
