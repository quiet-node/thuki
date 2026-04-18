//! `/search` pipeline module.
//!
//! Public surface:
//! - [`SearchEvent`] — the streamed event type used on the frontend IPC
//!   channel.
//! - [`search_pipeline`] — the single Tauri command that owns the entire
//!   classify -> route -> answer flow.
//!
//! Everything else is internal. The pipeline shares Ollama streaming
//! primitives with the main chat path (`commands::stream_ollama_chat`) and
//! persists completed turns into the shared [`ConversationHistory`] so that
//! subsequent user messages see the full conversational state regardless of
//! whether they went through `/search` or the normal chat command.

use tauri::{ipc::Channel, State};
use tokio_util::sync::CancellationToken;

use crate::commands::{
    ConversationHistory, GenerationState, ModelConfig, SystemPrompt, DEFAULT_OLLAMA_URL,
};

pub mod config;
pub mod errors;
mod llm;
mod pipeline;
mod rerank;
mod searxng;
mod types;

pub use types::{
    Action, IterationStage, IterationTrace, JudgeVerdict, RouterJudgeOutput, SearchEvent,
    SearchMetadata, SearchWarning, Sufficiency,
};

/// Umbrella Tauri command implementing the full `/search` pipeline.
///
/// The frontend passes in the user's raw query plus a typed
/// [`tauri::ipc::Channel`] to receive [`SearchEvent`]s. The backend is the
/// sole owner of routing state, history mutation, cancellation, and error
/// presentation — the frontend is a pure renderer of whichever events arrive.
///
/// Reuses the shared [`GenerationState`] so a single `cancel_generation`
/// invocation cancels either a chat or a search turn, whichever is active.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn search_pipeline(
    message: String,
    on_event: Channel<SearchEvent>,
    client: State<'_, reqwest::Client>,
    generation: State<'_, GenerationState>,
    history: State<'_, ConversationHistory>,
    system_prompt: State<'_, SystemPrompt>,
    model_config: State<'_, ModelConfig>,
) -> Result<(), String> {
    let ollama_endpoint = format!("{}/api/chat", DEFAULT_OLLAMA_URL.trim_end_matches('/'));
    let cancel_token = CancellationToken::new();
    generation.set_token(cancel_token.clone());

    let today = pipeline::today_iso();
    let result = pipeline::run(
        &ollama_endpoint,
        searxng::SEARXNG_ENDPOINT,
        &model_config.active,
        &client,
        cancel_token.clone(),
        &system_prompt.0,
        &history,
        message,
        &today,
        |event| {
            let _ = on_event.send(event);
        },
    )
    .await;

    if let Err(e) = result {
        // Cancelled is already surfaced via the Cancelled event by `run`; we
        // only emit an Error event for true failure paths.
        if e != types::SearchError::Cancelled && e != types::SearchError::EmptyQuery {
            let _ = on_event.send(SearchEvent::Error {
                message: e.user_message(),
            });
        }
    }

    generation.clear_token();
    Ok(())
}
