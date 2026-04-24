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

use tauri::{ipc::Channel, State};
use tokio_util::sync::CancellationToken;

use crate::commands::{ConversationHistory, GenerationState};
use crate::config::AppConfig;

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

pub use llm::JudgeSource;
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
pub async fn search_pipeline(
    message: String,
    on_event: Channel<SearchEvent>,
    client: State<'_, reqwest::Client>,
    generation: State<'_, GenerationState>,
    history: State<'_, ConversationHistory>,
    app_config: State<'_, AppConfig>,
) -> Result<(), String> {
    // Build the runtime search config from TOML values. Defaults match the
    // compiled constants in search/config.rs so a missing [search] section
    // in config.toml produces identical behavior to previous builds.
    let runtime_config = config::SearchRuntimeConfig {
        searxng_url: app_config.search.searxng_url.clone(),
        reader_url: app_config.search.reader_url.clone(),
        max_iterations: app_config.search.max_iterations as usize,
        top_k_urls: app_config.search.top_k_urls as usize,
        search_timeout_s: app_config.search.search_timeout_s,
        reader_per_url_timeout_s: app_config.search.reader_per_url_timeout_s,
        reader_batch_timeout_s: app_config.search.reader_batch_timeout_s,
        judge_timeout_s: app_config.search.judge_timeout_s,
        router_timeout_s: app_config.search.router_timeout_s,
    };
    let searxng_endpoint = runtime_config.searxng_endpoint();

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

    let ollama_endpoint = format!(
        "{}/api/chat",
        app_config.model.ollama_url.trim_end_matches('/')
    );
    let active_model = app_config.model.active().to_string();
    let cancel_token = CancellationToken::new();
    generation.set_token(cancel_token.clone());

    let today = pipeline::today_iso();

    let router = pipeline::DefaultRouterJudge::new(
        ollama_endpoint.clone(),
        active_model.clone(),
        (*client).clone(),
        cancel_token.clone(),
        today.clone(),
        runtime_config.router_timeout_s,
    );
    let judge = pipeline::DefaultJudge::new(
        ollama_endpoint.clone(),
        active_model.clone(),
        (*client).clone(),
        cancel_token.clone(),
        runtime_config.judge_timeout_s,
    );

    let result = pipeline::run_agentic(
        &ollama_endpoint,
        &searxng_endpoint,
        &runtime_config.reader_url,
        &active_model,
        &client,
        cancel_token.clone(),
        &app_config.prompt.resolved_system,
        &history,
        message,
        &today,
        &|event| {
            let _ = on_event.send(event);
        },
        &router,
        &judge,
        &runtime_config,
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

    generation.clear_token();
    Ok(())
}
