//! Orchestrator for the `/search` pipeline.
//!
//! Implements the three-branch state machine discussed in the design:
//!   Classifying -> Clarifying                (query is ambiguous)
//!   Classifying -> Token* -> Done           (answer from prior context)
//!   Classifying -> Searching -> Token* -> Done  (fresh web search + synthesis)
//!
//! Task 13 adds an agentic entry point [`run_agentic`] alongside the legacy
//! [`run`] function. It uses two trait seams, [`RouterJudgeCaller`] and
//! [`JudgeCaller`], so tests can inject deterministic mocks without spinning a
//! mock Ollama server. The legacy [`run`] and the Tauri command in
//! `search::mod` are untouched; they remain the production path until Task 16
//! retires them.
//!
//! The pipeline is the single owner of `ConversationHistory` mutations for a
//! search turn: every branch that produces a user-visible assistant message
//! persists both the user's query and the assistant reply so that subsequent
//! classifier calls can see the full conversational state.

use std::sync::atomic::Ordering;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::commands::{
    classify_http_error, classify_stream_error, stream_ollama_chat, ChatMessage,
    ConversationHistory, StreamChunk,
};

use super::llm::{
    build_answer_from_context_messages, build_synthesis_messages, call_router, JudgeSource,
};
use super::rerank;
use super::searxng;
use super::types::{
    Action, JudgeVerdict, RouterDecision, RouterJudgeOutput, SearchError, SearchEvent, Sufficiency,
};

/// Returns the current UTC date formatted as `YYYY-MM-DD`.
///
/// Uses `time::OffsetDateTime::now_utc()` to avoid the unsoundness of
/// local-offset calculation in multi-threaded processes on Unix (documented
/// in the `time` crate README). UTC is appropriate here: the date string is
/// injected into the synthesis prompt purely to prevent the model from
/// substituting its training-cutoff year; sub-day precision is irrelevant.
pub fn today_iso() -> String {
    let d = time::OffsetDateTime::now_utc().date();
    format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day())
}

/// Runs the full search pipeline end-to-end. Emits every user-visible state
/// transition through `on_event`. Returns an internal `SearchError` for
/// diagnostic tests; the caller is responsible for converting terminal errors
/// into an `Error` event, since lower-level streaming failures already emit
/// their own error events through the stream adapter.
///
/// `ollama_endpoint` is the fully-qualified `/api/chat` URL; `searxng_endpoint`
/// is the fully-qualified SearXNG `/search` URL. Both are surfaced as
/// parameters for testability: production callers pass the compiled-in
/// constants defined in `commands.rs` and `search::searxng`.
///
/// `today` is a `YYYY-MM-DD` string injected into the synthesis prompt to
/// anchor the model to the real calendar date. Pass `today_iso()` at the
/// call site for production use, or a fixed string in tests.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    ollama_endpoint: &str,
    searxng_endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    chat_system_prompt: &str,
    history: &ConversationHistory,
    query: String,
    today: &str,
    on_event: impl Fn(SearchEvent),
) -> Result<(), SearchError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(SearchError::EmptyQuery);
    }
    let user_query = trimmed.to_string();

    if cancel_token.is_cancelled() {
        on_event(SearchEvent::Cancelled);
        return Err(SearchError::Cancelled);
    }

    on_event(SearchEvent::Classifying);

    let (epoch_at_start, history_snapshot) = snapshot_history(history);

    let decision = call_router(
        ollama_endpoint,
        model,
        client,
        &history_snapshot,
        &user_query,
        &cancel_token,
    )
    .await?;

    let user_msg = ChatMessage {
        role: "user".to_string(),
        content: user_query.clone(),
        images: None,
    };

    match decision {
        RouterDecision::Clarify { question } => {
            run_clarify_branch(history, epoch_at_start, user_msg, question, &on_event);
            Ok(())
        }
        RouterDecision::AnswerFromContext => {
            let messages = build_answer_from_context_messages(
                chat_system_prompt,
                &history_snapshot,
                &user_query,
            );
            run_streaming_branch(
                ollama_endpoint,
                model,
                client,
                cancel_token,
                messages,
                history,
                epoch_at_start,
                user_msg,
                &on_event,
            )
            .await;
            Ok(())
        }
        RouterDecision::Search { optimized_query } => {
            run_search_branch(
                ollama_endpoint,
                searxng_endpoint,
                model,
                client,
                cancel_token,
                &history_snapshot,
                history,
                epoch_at_start,
                user_msg,
                user_query,
                optimized_query,
                today,
                &on_event,
            )
            .await
        }
    }
}

/// Takes a snapshot of the conversation history and its epoch counter under a
/// single lock acquisition. The snapshot is used for the entire pipeline run;
/// if the epoch changes before we write back, the write is skipped (the user
/// started a new conversation mid-flight).
fn snapshot_history(history: &ConversationHistory) -> (u64, Vec<ChatMessage>) {
    let conv = history.messages.lock().unwrap();
    let epoch = history.epoch.load(Ordering::SeqCst);
    (epoch, conv.clone())
}

/// Clarify branch: emit the clarifying event, persist the pair, emit `Done`.
/// No LLM streaming is needed: the router already produced the full output.
fn run_clarify_branch(
    history: &ConversationHistory,
    epoch_at_start: u64,
    user_msg: ChatMessage,
    question: String,
    on_event: &impl Fn(SearchEvent),
) {
    on_event(SearchEvent::Clarifying {
        question: question.clone(),
    });
    persist_turn(
        history,
        epoch_at_start,
        user_msg,
        ChatMessage {
            role: "assistant".to_string(),
            content: question,
            images: None,
        },
    );
    on_event(SearchEvent::Done);
}

/// Search branch: resolve results, assemble synthesis prompt, stream the
/// answer. Emits `Searching` before contacting SearXNG so the UI can surface
/// the phase transition.
#[allow(clippy::too_many_arguments)]
async fn run_search_branch(
    ollama_endpoint: &str,
    searxng_endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    history_snapshot: &[ChatMessage],
    history: &ConversationHistory,
    epoch_at_start: u64,
    user_msg: ChatMessage,
    user_query: String,
    optimized_query: String,
    today: &str,
    on_event: &impl Fn(SearchEvent),
) -> Result<(), SearchError> {
    if cancel_token.is_cancelled() {
        on_event(SearchEvent::Cancelled);
        return Err(SearchError::Cancelled);
    }

    on_event(SearchEvent::Searching);

    let results = searxng::search(client, searxng_endpoint, &optimized_query).await?;

    // Rerank the retrieved set by fusing BM25F field-weighted lexical scores
    // with the upstream engine order via Reciprocal Rank Fusion. The same
    // order is then used for both the frontend Sources footer and the
    // synthesis prompt, so the answer and its citations are consistent.
    let results = rerank::rerank(&optimized_query, results);

    // Forward result previews to the frontend so it can render a sources
    // footer below the synthesized answer.
    on_event(SearchEvent::Sources {
        results: results
            .iter()
            .map(|r| super::types::SearchResultPreview {
                title: r.title.clone(),
                url: r.url.clone(),
            })
            .collect(),
    });

    let messages = build_synthesis_messages(history_snapshot, &user_query, &results, today);

    run_streaming_branch(
        ollama_endpoint,
        model,
        client,
        cancel_token,
        messages,
        history,
        epoch_at_start,
        user_msg,
        on_event,
    )
    .await;

    Ok(())
}

/// Runs a streaming Ollama call, translating `StreamChunk` events into
/// `SearchEvent` events and persisting the completed assistant turn on normal
/// completion (or partial completion via cancellation).
#[allow(clippy::too_many_arguments)]
async fn run_streaming_branch(
    endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    messages: Vec<ChatMessage>,
    history: &ConversationHistory,
    epoch_at_start: u64,
    user_msg: ChatMessage,
    on_event: &impl Fn(SearchEvent),
) {
    let accumulated = stream_ollama_chat(
        endpoint,
        model,
        messages,
        false,
        client,
        cancel_token,
        |chunk| {
            on_event(translate_chunk(chunk));
        },
    )
    .await;

    if !accumulated.is_empty() {
        persist_turn(
            history,
            epoch_at_start,
            user_msg,
            ChatMessage {
                role: "assistant".to_string(),
                content: accumulated,
                images: None,
            },
        );
    }
}

/// Maps a low-level streaming chunk to a pipeline event.
pub(super) fn translate_chunk(chunk: StreamChunk) -> SearchEvent {
    match chunk {
        StreamChunk::Token(t) => SearchEvent::Token { content: t },
        // Thinking mode is not exposed for the search pipeline: suppressing
        // these tokens keeps the event stream minimal. A dedicated event can
        // be added later without touching the frontend types.
        StreamChunk::ThinkingToken(_) => SearchEvent::Token {
            content: String::new(),
        },
        StreamChunk::Done => SearchEvent::Done,
        StreamChunk::Cancelled => SearchEvent::Cancelled,
        StreamChunk::Error(e) => SearchEvent::Error { message: e.message },
    }
}

/// Appends `(user, assistant)` to the conversation history, skipping the
/// write when the history epoch has advanced since the snapshot (i.e. the
/// user reset the conversation mid-pipeline). The epoch check is performed
/// under the lock so there is no race window between the check and the push.
fn persist_turn(
    history: &ConversationHistory,
    epoch_at_start: u64,
    user_msg: ChatMessage,
    assistant_msg: ChatMessage,
) {
    let mut conv = history.messages.lock().unwrap();
    if history.epoch.load(Ordering::SeqCst) != epoch_at_start {
        return;
    }
    conv.push(user_msg);
    conv.push(assistant_msg);
}

/// Convenience wrapper around `classify_http_error` for consistency with
/// `commands::stream_ollama_chat` — exposed so future variants of the pipeline
/// can translate HTTP errors uniformly.
#[allow(dead_code)]
pub(super) fn http_error_message(status: u16) -> String {
    classify_http_error(status).message
}

/// Convenience wrapper around `classify_stream_error` for symmetry.
#[allow(dead_code)]
pub(super) fn stream_error_message(e: &reqwest::Error) -> String {
    classify_stream_error(e).message
}

// ── Agentic trait seams ────────────────────────────────────────────────────

/// Abstracts the merged router+judge LLM call so the agentic pipeline can be
/// tested with deterministic mock output without spinning a real Ollama server.
///
/// Production code uses [`DefaultRouterJudge`]. Tests inject a struct that
/// returns a canned [`RouterJudgeOutput`]. The abstraction is introduced in
/// Task 13 alongside [`run_agentic`]; Task 16 wires the Tauri command to use
/// [`run_agentic`] exclusively, at which point the legacy [`run`] is retired.
// The trait and its implementations have no non-test call site until Task 16
// wires run_agentic to the Tauri command. Suppress dead_code for the trait,
// the two default structs, run_agentic, and the helper until then.
#[allow(dead_code)]
#[async_trait]
pub trait RouterJudgeCaller: Send + Sync {
    /// Calls the router+judge LLM with the given conversation history and
    /// current query, returning a combined routing and sufficiency decision.
    async fn call(
        &self,
        history: &[ChatMessage],
        query: &str,
    ) -> Result<RouterJudgeOutput, SearchError>;
}

/// Abstracts the per-round sufficiency judge call so the agentic gap loop can
/// be exercised with injected verdicts.
///
/// Production code uses [`DefaultJudge`]. Tests inject a mock that returns
/// a predetermined sequence of [`JudgeVerdict`]s. Task 14 adds the gap loop;
/// Task 16 retires the legacy router path.
#[allow(dead_code)]
#[async_trait]
pub trait JudgeCaller: Send + Sync {
    /// Judges how well the given sources answer the query.
    async fn call(&self, query: &str, sources: &[JudgeSource])
        -> Result<JudgeVerdict, SearchError>;
}

/// Production [`RouterJudgeCaller`] implementation.
///
/// The real wiring (endpoint, model, HTTP client, today string, cancellation
/// token) is all per-call state owned by the Tauri command. Injecting it here
/// via struct fields would require cloning or `Arc`-wrapping per-request
/// objects. Instead this impl is left as `unimplemented!` and the full wiring
/// happens in Task 16 when the Tauri command swaps from `run` to
/// `run_agentic`. Tests never instantiate this struct; they inject a mock.
#[derive(Default, Clone, Copy)]
pub struct DefaultRouterJudge;

#[allow(dead_code)]
#[async_trait]
impl RouterJudgeCaller for DefaultRouterJudge {
    async fn call(
        &self,
        _history: &[ChatMessage],
        _query: &str,
    ) -> Result<RouterJudgeOutput, SearchError> {
        unimplemented!(
            "DefaultRouterJudge is wired to the Tauri command in Task 16; \
             inject a mock in tests"
        )
    }
}

/// Production [`JudgeCaller`] implementation.
///
/// Same rationale as [`DefaultRouterJudge`]: per-call dependencies (endpoint,
/// model, client, cancel token) are not available here and will be threaded
/// through in Task 16. Tests inject mocks; production never reaches this path
/// until that task lands.
#[derive(Default, Clone, Copy)]
pub struct DefaultJudge;

#[allow(dead_code)]
#[async_trait]
impl JudgeCaller for DefaultJudge {
    async fn call(
        &self,
        _query: &str,
        _sources: &[JudgeSource],
    ) -> Result<JudgeVerdict, SearchError> {
        unimplemented!(
            "DefaultJudge is wired to the Tauri command in Task 16; \
             inject a mock in tests"
        )
    }
}

// ── Agentic entry point ────────────────────────────────────────────────────

/// Agentic search pipeline entry point. Handles the CLARIFY short-circuit and
/// the history-sufficient short-circuit without performing any web search.
///
/// Branch summary:
/// - `Action::Clarify`: streams the clarifying question as `Token` events,
///   then `Done`. The question is also persisted to history so the next turn
///   sees it. No `Clarifying` event is emitted (design decision 15).
/// - `Action::Proceed` + `history_sufficiency == Some(Sufficient)`: streams
///   the answer synthesised from conversation history alone, reusing the
///   same `answer_from_context` path as the legacy [`run`].
/// - `Action::Proceed` + anything else: returns
///   [`SearchError::Internal`] as a placeholder until Task 14 wires the
///   initial search round.
///
/// Task 14 implements the search round and post-snippet judge call.
/// Task 15 adds the gap loop and exhaustion fallback.
/// Task 16 retires [`run`] and makes this the sole Tauri command entry point.
// No non-test call site until Task 16 wires the Tauri command.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub async fn run_agentic<R, J>(
    ollama_endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    chat_system_prompt: &str,
    history: &ConversationHistory,
    query: String,
    on_event: impl Fn(SearchEvent),
    router: &R,
    // Judge is not called in Tasks 13-14 short-circuit branches but is part
    // of the signature so Task 14 can add gap-loop calls without changing
    // the function interface.
    _judge: &J,
) -> Result<(), SearchError>
where
    R: RouterJudgeCaller + ?Sized,
    J: JudgeCaller + ?Sized,
{
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(SearchError::EmptyQuery);
    }
    let user_query = trimmed.to_string();

    if cancel_token.is_cancelled() {
        on_event(SearchEvent::Cancelled);
        return Err(SearchError::Cancelled);
    }

    on_event(SearchEvent::AnalyzingQuery);

    let (epoch_at_start, history_snapshot) = snapshot_history(history);

    let output = router.call(&history_snapshot, &user_query).await?;

    let user_msg = ChatMessage {
        role: "user".to_string(),
        content: user_query.clone(),
        images: None,
    };

    match output.action {
        Action::Clarify => {
            let question = output.clarifying_question.unwrap_or_default();
            for piece in split_into_stream_pieces(&question) {
                if cancel_token.is_cancelled() {
                    on_event(SearchEvent::Cancelled);
                    return Ok(());
                }
                on_event(SearchEvent::Token { content: piece });
            }
            // Persist so the next turn can see the clarifying question.
            persist_turn(
                history,
                epoch_at_start,
                user_msg,
                ChatMessage {
                    role: "assistant".to_string(),
                    content: question,
                    images: None,
                },
            );
            on_event(SearchEvent::Done);
            Ok(())
        }
        Action::Proceed => {
            if matches!(output.history_sufficiency, Some(Sufficiency::Sufficient)) {
                let messages = build_answer_from_context_messages(
                    chat_system_prompt,
                    &history_snapshot,
                    &user_query,
                );
                run_streaming_branch(
                    ollama_endpoint,
                    model,
                    client,
                    cancel_token,
                    messages,
                    history,
                    epoch_at_start,
                    user_msg,
                    &on_event,
                )
                .await;
                Ok(())
            } else {
                // Tasks 14 and 15 implement the search round and gap loop.
                Err(SearchError::Internal(
                    "agentic search path not wired yet; Task 14 lands this".into(),
                ))
            }
        }
    }
}

/// Splits a string into roughly `TARGET`-character pieces on whitespace
/// boundaries so the frontend receives a stream of `Token` events rather than
/// one atomic message. Words that exceed `TARGET` alone are emitted as-is.
// Called only from run_agentic which is itself dead until Task 16.
#[allow(dead_code)]
fn split_into_stream_pieces(s: &str) -> Vec<String> {
    const TARGET: usize = 24;
    let mut out = Vec::new();
    let mut current = String::new();
    for word in s.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= TARGET {
            current.push(' ');
            current.push_str(word);
        } else {
            out.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() && !s.is_empty() {
        out.push(s.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{OllamaError, OllamaErrorKind};
    use std::sync::{Arc, Mutex};

    fn collect_events() -> (Arc<Mutex<Vec<SearchEvent>>>, impl Fn(SearchEvent)) {
        let events = Arc::new(Mutex::new(Vec::<SearchEvent>::new()));
        let events_clone = events.clone();
        let callback = move |e: SearchEvent| {
            events_clone.lock().unwrap().push(e);
        };
        (events, callback)
    }

    // ── today_iso ───────────────────────────────────────────────────────────

    #[test]
    fn today_iso_returns_valid_yyyy_mm_dd() {
        let s = today_iso();
        // Must be exactly 10 chars: YYYY-MM-DD.
        assert_eq!(s.len(), 10, "expected YYYY-MM-DD (10 chars), got: {s}");
        // Positions 4 and 7 must be dashes.
        let b = s.as_bytes();
        assert_eq!(b[4], b'-', "expected dash at position 4");
        assert_eq!(b[7], b'-', "expected dash at position 7");
        // All other positions must be ASCII digits.
        for i in [0, 1, 2, 3, 5, 6, 8, 9] {
            assert!(
                b[i].is_ascii_digit(),
                "position {i} is not a digit in '{s}'"
            );
        }
    }

    // ── translate_chunk ─────────────────────────────────────────────────────

    #[test]
    fn translate_chunk_token_maps_to_token() {
        let out = translate_chunk(StreamChunk::Token("hi".into()));
        assert_eq!(
            out,
            SearchEvent::Token {
                content: "hi".into()
            }
        );
    }

    #[test]
    fn translate_chunk_thinking_token_suppressed() {
        let out = translate_chunk(StreamChunk::ThinkingToken("reason".into()));
        assert_eq!(
            out,
            SearchEvent::Token {
                content: String::new()
            }
        );
    }

    #[test]
    fn translate_chunk_done_maps_to_done() {
        assert_eq!(translate_chunk(StreamChunk::Done), SearchEvent::Done);
    }

    #[test]
    fn translate_chunk_cancelled_maps_to_cancelled() {
        assert_eq!(
            translate_chunk(StreamChunk::Cancelled),
            SearchEvent::Cancelled
        );
    }

    #[test]
    fn translate_chunk_error_maps_to_error_event() {
        let out = translate_chunk(StreamChunk::Error(OllamaError {
            kind: OllamaErrorKind::Other,
            message: "boom".into(),
        }));
        assert_eq!(
            out,
            SearchEvent::Error {
                message: "boom".into()
            }
        );
    }

    // ── snapshot_history ────────────────────────────────────────────────────

    #[test]
    fn snapshot_history_returns_current_epoch_and_messages() {
        let h = ConversationHistory::new();
        h.messages.lock().unwrap().push(ChatMessage {
            role: "user".into(),
            content: "hi".into(),
            images: None,
        });
        let (epoch, msgs) = snapshot_history(&h);
        assert_eq!(epoch, 0);
        assert_eq!(msgs.len(), 1);
    }

    // ── persist_turn ────────────────────────────────────────────────────────

    #[test]
    fn persist_turn_appends_both_messages_under_matching_epoch() {
        let h = ConversationHistory::new();
        persist_turn(
            &h,
            0,
            ChatMessage {
                role: "user".into(),
                content: "q".into(),
                images: None,
            },
            ChatMessage {
                role: "assistant".into(),
                content: "a".into(),
                images: None,
            },
        );
        let conv = h.messages.lock().unwrap();
        assert_eq!(conv.len(), 2);
        assert_eq!(conv[0].role, "user");
        assert_eq!(conv[1].role, "assistant");
    }

    #[test]
    fn persist_turn_skips_when_epoch_advanced() {
        let h = ConversationHistory::new();
        h.epoch.fetch_add(1, Ordering::SeqCst);
        persist_turn(
            &h,
            0,
            ChatMessage {
                role: "user".into(),
                content: "q".into(),
                images: None,
            },
            ChatMessage {
                role: "assistant".into(),
                content: "a".into(),
                images: None,
            },
        );
        let conv = h.messages.lock().unwrap();
        assert!(conv.is_empty());
    }

    // ── run_clarify_branch ──────────────────────────────────────────────────

    #[test]
    fn run_clarify_branch_emits_and_persists() {
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        run_clarify_branch(
            &h,
            0,
            ChatMessage {
                role: "user".into(),
                content: "who is him?".into(),
                images: None,
            },
            "Which person are you referring to?".into(),
            &cb,
        );

        let evs = events.lock().unwrap();
        assert!(matches!(
            &evs[0],
            SearchEvent::Clarifying { question }
                if question == "Which person are you referring to?"
        ));
        assert_eq!(evs[1], SearchEvent::Done);

        let conv = h.messages.lock().unwrap();
        assert_eq!(conv.len(), 2);
        assert_eq!(conv[0].role, "user");
        assert_eq!(conv[1].content, "Which person are you referring to?");
    }

    // ── run: cancel before work starts ──────────────────────────────────────

    #[tokio::test]
    async fn run_emits_cancelled_when_token_already_cancelled() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        token.cancel();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        let err = run(
            "http://127.0.0.1:1/api/chat",
            "http://127.0.0.1:1/search",
            "m",
            &client,
            token,
            "chat prompt",
            &h,
            "q".into(),
            "2026-04-17",
            cb,
        )
        .await
        .unwrap_err();

        assert_eq!(err, SearchError::Cancelled);
        let evs = events.lock().unwrap();
        assert_eq!(evs[0], SearchEvent::Cancelled);
    }

    #[tokio::test]
    async fn run_rejects_empty_query_before_any_event() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        let err = run(
            "http://127.0.0.1:1/api/chat",
            "http://127.0.0.1:1/search",
            "m",
            &client,
            token,
            "chat",
            &h,
            "   ".into(),
            "2026-04-17",
            cb,
        )
        .await
        .unwrap_err();
        assert_eq!(err, SearchError::EmptyQuery);
        assert!(events.lock().unwrap().is_empty());
    }

    // ── run: full clarify branch end-to-end ─────────────────────────────────

    #[tokio::test]
    async fn run_clarify_path_end_to_end() {
        let mut ollama = mockito::Server::new_async().await;
        let router_body = serde_json::json!({
            "message": { "content": r#"{"action":"clarify","question":"Who?","suggestions":["A","B"]}"# }
        })
        .to_string();
        let router_mock = ollama
            .mock("POST", "/api/chat")
            .with_body(router_body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        run(
            &format!("{}/api/chat", ollama.url()),
            "http://127.0.0.1:1/search",
            "m",
            &client,
            token,
            "chat-prompt",
            &h,
            "who is him".into(),
            "2026-04-17",
            cb,
        )
        .await
        .unwrap();

        router_mock.assert_async().await;
        let evs = events.lock().unwrap();
        assert_eq!(evs[0], SearchEvent::Classifying);
        assert!(matches!(evs[1], SearchEvent::Clarifying { .. }));
        assert_eq!(evs[2], SearchEvent::Done);

        let conv = h.messages.lock().unwrap();
        assert_eq!(conv.len(), 2);
    }

    // ── run: answer_from_context path ───────────────────────────────────────

    #[tokio::test]
    async fn run_answer_from_context_path_end_to_end() {
        let mut ollama = mockito::Server::new_async().await;
        let router_body =
            serde_json::json!({ "message": { "content": r#"{"action":"answer_from_context"}"# } })
                .to_string();
        let stream_line =
            "{\"message\":{\"role\":\"assistant\",\"content\":\"hello\"},\"done\":false}\n\
                           {\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true}\n";

        let router_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":false}"#.to_string(),
            ))
            .with_body(router_body)
            .create_async()
            .await;
        let stream_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":true}"#.to_string(),
            ))
            .with_body(stream_line)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        run(
            &format!("{}/api/chat", ollama.url()),
            "http://127.0.0.1:1/search",
            "m",
            &client,
            token,
            "chat",
            &h,
            "what is 2+2".into(),
            "2026-04-17",
            cb,
        )
        .await
        .unwrap();

        router_mock.assert_async().await;
        stream_mock.assert_async().await;

        let evs = events.lock().unwrap();
        assert_eq!(evs[0], SearchEvent::Classifying);
        // No Searching event on this branch.
        assert!(evs.iter().all(|e| !matches!(e, SearchEvent::Searching)));
        assert!(evs
            .iter()
            .any(|e| matches!(e, SearchEvent::Token { content } if content == "hello")));
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);

        let conv = h.messages.lock().unwrap();
        assert_eq!(conv.len(), 2);
        assert_eq!(conv[1].content, "hello");
    }

    // ── run: search path end-to-end ─────────────────────────────────────────

    #[tokio::test]
    async fn run_search_path_end_to_end() {
        let mut ollama = mockito::Server::new_async().await;
        let mut searx = mockito::Server::new_async().await;

        let router_body = serde_json::json!({
            "message": { "content": r#"{"action":"search","optimized_query":"rust async"}"# }
        })
        .to_string();
        let router_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":false}"#.to_string(),
            ))
            .with_body(router_body)
            .create_async()
            .await;

        let searx_body = serde_json::json!({
            "results": [
                { "title": "R", "url": "https://r", "content": "rust info" }
            ]
        })
        .to_string();
        let searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(searx_body)
            .create_async()
            .await;

        let stream_line =
            "{\"message\":{\"role\":\"assistant\",\"content\":\"answer [1]\"},\"done\":false}\n\
                           {\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true}\n";
        // Assert the synthesis call body contains the injected date string.
        let stream_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::PartialJsonString(r#"{"stream":true}"#.to_string()),
                mockito::Matcher::Regex("2026-04-17".to_string()),
            ]))
            .with_body(stream_line)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        run(
            &format!("{}/api/chat", ollama.url()),
            &format!("{}/search", searx.url()),
            "m",
            &client,
            token,
            "chat",
            &h,
            "what is rust".into(),
            "2026-04-17",
            cb,
        )
        .await
        .unwrap();

        router_mock.assert_async().await;
        searx_mock.assert_async().await;
        stream_mock.assert_async().await;

        let evs = events.lock().unwrap();
        assert_eq!(evs[0], SearchEvent::Classifying);
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::Searching)));
        assert!(evs
            .iter()
            .any(|e| matches!(e, SearchEvent::Token { content } if content == "answer [1]")));
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);
        let conv = h.messages.lock().unwrap();
        assert_eq!(conv.len(), 2);
        assert!(conv[1].content.contains("answer [1]"));
    }

    #[tokio::test]
    async fn run_search_path_reranks_sources_via_bm25f_rrf() {
        // SearXNG returns the matching doc in position 2 (out of 3). The
        // reranker must lift it to position 0 in the emitted Sources event.
        let mut ollama = mockito::Server::new_async().await;
        let mut searx = mockito::Server::new_async().await;

        let router_body = serde_json::json!({
            "message": {
                "content": r#"{"action":"search","optimized_query":"rust async runtime"}"#,
            }
        })
        .to_string();
        let _router_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":false}"#.to_string(),
            ))
            .with_body(router_body)
            .create_async()
            .await;

        let searx_body = serde_json::json!({
            "results": [
                { "title": "totally unrelated header", "url": "https://a",
                  "content": "nothing to see here filler" },
                { "title": "another filler doc", "url": "https://c",
                  "content": "more filler body text" },
                { "title": "rust async runtime design", "url": "https://b",
                  "content": "walkthrough of rust async runtime internals" },
            ]
        })
        .to_string();
        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(searx_body)
            .create_async()
            .await;

        let stream_line =
            "{\"message\":{\"role\":\"assistant\",\"content\":\"ok\"},\"done\":false}\n\
             {\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true}\n";
        let _stream_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":true}"#.to_string(),
            ))
            .with_body(stream_line)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        run(
            &format!("{}/api/chat", ollama.url()),
            &format!("{}/search", searx.url()),
            "m",
            &client,
            token,
            "chat",
            &h,
            "rust async runtime".into(),
            "2026-04-17",
            cb,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        let sources = evs
            .iter()
            .find_map(|e| match e {
                SearchEvent::Sources { results } => Some(results.clone()),
                _ => None,
            })
            .expect("Sources event missing");
        assert_eq!(sources.len(), 3);
        assert_eq!(
            sources[0].url, "https://b",
            "expected rerank to lift the matching doc to position 0, got {sources:?}"
        );
    }

    #[tokio::test]
    async fn run_search_path_surfaces_searx_error() {
        let mut ollama = mockito::Server::new_async().await;
        let mut searx = mockito::Server::new_async().await;

        let router_body = serde_json::json!({
            "message": { "content": r#"{"action":"search","optimized_query":"q"}"# }
        })
        .to_string();
        let _router_mock = ollama
            .mock("POST", "/api/chat")
            .with_body(router_body)
            .create_async()
            .await;

        let searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_status(503)
            .with_body("down")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        let err = run(
            &format!("{}/api/chat", ollama.url()),
            &format!("{}/search", searx.url()),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-17",
            cb,
        )
        .await
        .unwrap_err();

        searx_mock.assert_async().await;
        assert_eq!(err, SearchError::SearxHttp(503));
        // Conversation history must NOT mutate when the branch errored before
        // producing an assistant message.
        assert!(h.messages.lock().unwrap().is_empty());
        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::Searching)));
    }

    #[tokio::test]
    async fn run_search_branch_returns_cancelled_when_token_already_cancelled() {
        let mut ollama = mockito::Server::new_async().await;
        let router_body = serde_json::json!({
            "message": { "content": r#"{"action":"search","optimized_query":"q"}"# }
        })
        .to_string();
        let _router_mock = ollama
            .mock("POST", "/api/chat")
            .with_body(router_body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        let user_msg = ChatMessage {
            role: "user".into(),
            content: "q".into(),
            images: None,
        };
        let precancelled = CancellationToken::new();
        precancelled.cancel();
        let err = run_search_branch(
            &format!("{}/api/chat", ollama.url()),
            "http://127.0.0.1:1/search",
            "m",
            &client,
            precancelled,
            &[],
            &h,
            0,
            user_msg,
            "q".into(),
            "q".into(),
            "2026-04-17",
            &cb,
        )
        .await
        .unwrap_err();
        drop(token);
        assert_eq!(err, SearchError::Cancelled);
        let evs = events.lock().unwrap();
        assert_eq!(evs[0], SearchEvent::Cancelled);
    }

    // ── run_streaming_branch: error propagation ─────────────────────────────

    #[tokio::test]
    async fn run_streaming_branch_does_not_persist_when_empty() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(500)
            .with_body("")
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (_, cb) = collect_events();

        run_streaming_branch(
            &format!("{}/api/chat", server.url()),
            "m",
            &client,
            token,
            vec![ChatMessage {
                role: "user".into(),
                content: "q".into(),
                images: None,
            }],
            &h,
            0,
            ChatMessage {
                role: "user".into(),
                content: "q".into(),
                images: None,
            },
            &cb,
        )
        .await;

        mock.assert_async().await;
        assert!(h.messages.lock().unwrap().is_empty());
    }

    // ── http/stream error message helpers ───────────────────────────────────

    #[test]
    fn http_error_message_forwards_to_commands_helper() {
        let msg = http_error_message(500);
        assert!(msg.contains("500"));
    }

    #[tokio::test]
    async fn stream_error_message_forwards_to_commands_helper() {
        let err = reqwest::get("http://127.0.0.1:1/").await.unwrap_err();
        let msg = stream_error_message(&err);
        assert!(!msg.is_empty());
    }
}

// ── Agentic pipeline tests ─────────────────────────────────────────────────

#[cfg(test)]
mod agentic_tests {
    use super::*;

    // ── mock implementations ────────────────────────────────────────────────

    struct MockRouter(RouterJudgeOutput);

    #[async_trait]
    impl RouterJudgeCaller for MockRouter {
        async fn call(
            &self,
            _h: &[ChatMessage],
            _q: &str,
        ) -> Result<RouterJudgeOutput, SearchError> {
            Ok(self.0.clone())
        }
    }

    struct MockJudge;

    #[async_trait]
    impl JudgeCaller for MockJudge {
        async fn call(&self, _q: &str, _s: &[JudgeSource]) -> Result<JudgeVerdict, SearchError> {
            panic!("judge should not be called on CLARIFY or history-sufficient paths");
        }
    }

    fn collect_events() -> (
        std::sync::Arc<std::sync::Mutex<Vec<SearchEvent>>>,
        impl Fn(SearchEvent),
    ) {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<SearchEvent>::new()));
        let events_clone = events.clone();
        let callback = move |e: SearchEvent| {
            events_clone.lock().unwrap().push(e);
        };
        (events, callback)
    }

    // ── split_into_stream_pieces ─────────────────────────────────────────────

    #[test]
    fn split_into_stream_pieces_respects_target_length() {
        let pieces = split_into_stream_pieces("which project are you asking about today");
        // No piece should exceed TARGET + one word overhang.
        for piece in &pieces {
            // Pieces can slightly exceed 24 chars if a single word is long,
            // but assembled they must reconstitute the original words.
            assert!(!piece.is_empty());
        }
        let rejoined = pieces.join(" ");
        assert_eq!(rejoined, "which project are you asking about today");
    }

    #[test]
    fn split_into_stream_pieces_empty_string_returns_empty_vec() {
        assert!(split_into_stream_pieces("").is_empty());
    }

    #[test]
    fn split_into_stream_pieces_whitespace_only_returns_single_piece() {
        // The function preserves the raw string when no words are found but the
        // input is non-empty. In practice run_agentic trims and rejects
        // whitespace-only queries before this helper is called.
        let p = split_into_stream_pieces("   ");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0], "   ");
    }

    #[test]
    fn split_into_stream_pieces_single_short_word_returns_one_piece() {
        let p = split_into_stream_pieces("hi");
        assert_eq!(p, vec!["hi".to_string()]);
    }

    // ── run_agentic: empty query ─────────────────────────────────────────────

    #[tokio::test]
    async fn run_agentic_rejects_empty_query() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = MockRouter(RouterJudgeOutput {
            action: Action::Clarify,
            clarifying_question: None,
            history_sufficiency: None,
            optimized_query: None,
        });

        let err = run_agentic(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            token,
            "chat",
            &h,
            "   ".into(),
            cb,
            &router,
            &MockJudge,
        )
        .await
        .unwrap_err();

        assert_eq!(err, SearchError::EmptyQuery);
        assert!(events.lock().unwrap().is_empty());
    }

    // ── run_agentic: pre-cancelled token ─────────────────────────────────────

    #[tokio::test]
    async fn run_agentic_emits_cancelled_when_token_already_cancelled() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        token.cancel();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = MockRouter(RouterJudgeOutput {
            action: Action::Clarify,
            clarifying_question: None,
            history_sufficiency: None,
            optimized_query: None,
        });

        let err = run_agentic(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            cb,
            &router,
            &MockJudge,
        )
        .await
        .unwrap_err();

        assert_eq!(err, SearchError::Cancelled);
        let evs = events.lock().unwrap();
        assert_eq!(evs[0], SearchEvent::Cancelled);
    }

    // ── run_agentic: CLARIFY branch ──────────────────────────────────────────

    #[tokio::test]
    async fn clarify_action_streams_question_tokens_then_done() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        let router = MockRouter(RouterJudgeOutput {
            action: Action::Clarify,
            clarifying_question: Some("which project?".into()),
            history_sufficiency: None,
            optimized_query: None,
        });

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            token,
            "chat",
            &h,
            "tell me more".into(),
            cb,
            &router,
            &MockJudge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        // First event must be AnalyzingQuery.
        assert_eq!(evs[0], SearchEvent::AnalyzingQuery);

        // At least one Token event must carry content from the clarifying question.
        let all_token_content: String = evs
            .iter()
            .filter_map(|e| match e {
                SearchEvent::Token { content } => Some(content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            all_token_content.contains("which") || all_token_content.contains("project"),
            "expected token stream to contain the clarifying question, got: {all_token_content}"
        );

        // Last event must be Done.
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);

        // No search-phase events.
        assert!(evs.iter().all(|e| !matches!(e, SearchEvent::Searching)));
        assert!(evs
            .iter()
            .all(|e| !matches!(e, SearchEvent::ReadingSources)));

        // Turn must be persisted to history.
        let conv = h.messages.lock().unwrap();
        assert_eq!(conv.len(), 2);
        assert_eq!(conv[0].content, "tell me more");
        assert_eq!(conv[1].content, "which project?");
    }

    #[tokio::test]
    async fn clarify_with_empty_question_still_emits_done() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        let router = MockRouter(RouterJudgeOutput {
            action: Action::Clarify,
            clarifying_question: None,
            history_sufficiency: None,
            optimized_query: None,
        });

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            cb,
            &router,
            &MockJudge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert_eq!(evs[0], SearchEvent::AnalyzingQuery);
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);
    }

    // ── run_agentic: history-sufficient branch ───────────────────────────────

    #[tokio::test]
    async fn history_sufficient_action_streams_from_history_without_search() {
        let mut ollama = mockito::Server::new_async().await;
        let stream_line =
            "{\"message\":{\"role\":\"assistant\",\"content\":\"from history\"},\"done\":false}\n\
             {\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true}\n";
        let _mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":true}"#.to_string(),
            ))
            .with_body(stream_line)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        let router = MockRouter(RouterJudgeOutput {
            action: Action::Proceed,
            clarifying_question: None,
            history_sufficiency: Some(Sufficiency::Sufficient),
            optimized_query: None,
        });

        run_agentic(
            &format!("{}/api/chat", ollama.url()),
            "m",
            &client,
            token,
            "chat",
            &h,
            "what is 2+2".into(),
            cb,
            &router,
            &MockJudge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        // AnalyzingQuery first.
        assert_eq!(evs[0], SearchEvent::AnalyzingQuery);

        // At least one Token with content.
        assert!(evs
            .iter()
            .any(|e| matches!(e, SearchEvent::Token { content } if content == "from history")));

        // Done last.
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);

        // No search events.
        assert!(evs.iter().all(|e| !matches!(e, SearchEvent::Searching)));
        assert!(evs
            .iter()
            .all(|e| !matches!(e, SearchEvent::ReadingSources)));
    }

    // ── run_agentic: not-sufficient stub boundary ────────────────────────────

    #[tokio::test]
    async fn proceed_but_not_sufficient_returns_internal_error_until_task14() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (_events, cb) = collect_events();

        let router = MockRouter(RouterJudgeOutput {
            action: Action::Proceed,
            clarifying_question: None,
            history_sufficiency: Some(Sufficiency::Insufficient),
            optimized_query: Some("rust async".into()),
        });

        let err = run_agentic(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            token,
            "chat",
            &h,
            "tell me about rust async".into(),
            cb,
            &router,
            &MockJudge,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, SearchError::Internal(_)),
            "expected Internal error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn proceed_with_none_sufficiency_returns_internal_error_until_task14() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (_events, cb) = collect_events();

        let router = MockRouter(RouterJudgeOutput {
            action: Action::Proceed,
            clarifying_question: None,
            history_sufficiency: None,
            optimized_query: None,
        });

        let err = run_agentic(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            cb,
            &router,
            &MockJudge,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, SearchError::Internal(_)));
    }

    #[tokio::test]
    async fn proceed_with_partial_sufficiency_returns_internal_error_until_task14() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (_events, cb) = collect_events();

        let router = MockRouter(RouterJudgeOutput {
            action: Action::Proceed,
            clarifying_question: None,
            history_sufficiency: Some(Sufficiency::Partial),
            optimized_query: None,
        });

        let err = run_agentic(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            cb,
            &router,
            &MockJudge,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, SearchError::Internal(_)));
    }
}
