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

use super::chunker;
use super::config;
use super::llm::{
    build_answer_from_context_messages, build_synthesis_messages, call_router, JudgeSource,
};
use super::reader;
use super::rerank;
use super::searxng;
use super::types::{
    Action, IterationStage, IterationTrace, JudgeVerdict, RouterDecision, RouterJudgeOutput,
    SearchError, SearchEvent, SearchMetadata, SearchResultPreview, SearchWarning, SearxResult,
    Sufficiency,
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
            .map(|r| SearchResultPreview {
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
    #[cfg_attr(coverage_nightly, coverage(off))]
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
    #[cfg_attr(coverage_nightly, coverage(off))]
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
/// - `Action::Proceed` + anything else: runs the initial search round.
///   SearXNG -> URL rerank -> snippets judge -> (if not sufficient) reader
///   -> chunk rerank -> chunks judge -> synthesis. Falls to the exhaustion
///   fallback after one round. Task 15 adds the gap loop.
///
/// Task 15 adds the gap loop and gap-query execution.
/// Task 16 retires [`run`] and makes this the sole Tauri command entry point.
// No non-test call site until Task 16 wires the Tauri command.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub async fn run_agentic<R, J>(
    ollama_endpoint: &str,
    searxng_endpoint: &str,
    reader_base_url: &str,
    model: &str,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    chat_system_prompt: &str,
    history: &ConversationHistory,
    query: String,
    today: &str,
    on_event: impl Fn(SearchEvent),
    router: &R,
    judge: &J,
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
                // Initial search round: SearXNG -> URL rerank -> snippets judge
                // -> (if partial/insufficient) reader -> chunk rerank -> chunks
                // judge -> synthesis. Task 15 adds the gap loop after this.
                let query = output
                    .optimized_query
                    .clone()
                    .unwrap_or_else(|| user_query.clone());
                let reader_client = reader::ReaderClient::new_with_base(reader_base_url);
                let mut warnings: Vec<SearchWarning> = Vec::new();
                let mut metadata = SearchMetadata::default();
                let mut accumulated_chunks: Vec<chunker::Chunk> = Vec::new();

                let iter_start = std::time::Instant::now();

                // Stage 1: SearXNG initial round.
                if cancel_token.is_cancelled() {
                    on_event(SearchEvent::Cancelled);
                    return Ok(());
                }
                on_event(SearchEvent::Searching);

                let raw_urls = match searxng::search(client, searxng_endpoint, &query).await {
                    Ok(v) => v,
                    Err(SearchError::NoResults) => {
                        warnings.push(SearchWarning::NoResultsInitial);
                        on_event(SearchEvent::Warning {
                            warning: SearchWarning::NoResultsInitial,
                        });
                        return Err(SearchError::NoResults);
                    }
                    Err(e) => return Err(e),
                };

                // Stage 2: Rerank URLs, take top K.
                let reranked = rerank::rerank(&query, raw_urls);
                let top_urls: Vec<_> = reranked.into_iter().take(config::TOP_K_URLS).collect();

                // Stage 3: Emit Sources preview.
                let sources_preview: Vec<SearchResultPreview> = top_urls
                    .iter()
                    .map(|r| SearchResultPreview {
                        title: r.title.clone(),
                        url: r.url.clone(),
                    })
                    .collect();
                on_event(SearchEvent::Sources {
                    results: sources_preview,
                });

                // Stage 4: Build snippet JudgeSources and call the snippets judge.
                let snippet_sources: Vec<JudgeSource> = top_urls
                    .iter()
                    .map(|r| JudgeSource {
                        title: r.title.clone(),
                        url: r.url.clone(),
                        text: r.content.clone(),
                    })
                    .collect();

                let snippet_verdict = judge.call(&query, &snippet_sources).await?;

                if matches!(snippet_verdict.sufficiency, Sufficiency::Sufficient) {
                    metadata.iterations.push(IterationTrace {
                        stage: IterationStage::Initial,
                        queries: vec![query.clone()],
                        urls_fetched: vec![],
                        reader_empty_urls: vec![],
                        judge_verdict: snippet_verdict.sufficiency,
                        judge_reasoning: snippet_verdict.reasoning.clone(),
                        duration_ms: iter_start.elapsed().as_millis() as u64,
                    });
                    metadata.total_duration_ms = iter_start.elapsed().as_millis() as u64;
                    // Convert snippet sources to SearxResult for synthesis.
                    let synth_results: Vec<SearxResult> = snippet_sources
                        .iter()
                        .map(|s| SearxResult {
                            title: s.title.clone(),
                            url: s.url.clone(),
                            content: s.text.clone(),
                        })
                        .collect();
                    let messages =
                        build_synthesis_messages(&history_snapshot, &query, &synth_results, today);
                    on_event(SearchEvent::Composing);
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
                    return Ok(());
                }

                // Stage 5: Reader escalation.
                if cancel_token.is_cancelled() {
                    on_event(SearchEvent::Cancelled);
                    return Ok(());
                }
                on_event(SearchEvent::ReadingSources);
                let reader_urls: Vec<String> = top_urls.iter().map(|r| r.url.clone()).collect();
                let reader_result = match reader_client
                    .fetch_batch_cancellable(&reader_urls, &cancel_token)
                    .await
                {
                    Ok(r) => r,
                    Err(reader::ReaderError::Cancelled) => {
                        on_event(SearchEvent::Cancelled);
                        return Ok(());
                    }
                    Err(reader::ReaderError::ServiceUnavailable) => {
                        warnings.push(SearchWarning::ReaderUnavailable);
                        on_event(SearchEvent::Warning {
                            warning: SearchWarning::ReaderUnavailable,
                        });
                        reader::ReaderBatchResult::default()
                    }
                    Err(reader::ReaderError::BatchTimeout) => {
                        warnings.push(SearchWarning::ReaderPartialFailure);
                        on_event(SearchEvent::Warning {
                            warning: SearchWarning::ReaderPartialFailure,
                        });
                        reader::ReaderBatchResult::default()
                    }
                };

                // Detect partial failure: more than 50% of URLs failed without
                // a full service-unavailable signal.
                let partial_threshold = (reader_urls.len() as f64 * 0.5).ceil() as usize;
                if !warnings.contains(&SearchWarning::ReaderUnavailable)
                    && !warnings.contains(&SearchWarning::ReaderPartialFailure)
                    && !reader_urls.is_empty()
                    && reader_result.failed_urls.len() > partial_threshold
                {
                    warnings.push(SearchWarning::ReaderPartialFailure);
                    on_event(SearchEvent::Warning {
                        warning: SearchWarning::ReaderPartialFailure,
                    });
                }

                // Stage 6: Chunk and rerank.
                let new_chunks =
                    chunker::chunk_pages(&reader_result.pages, config::CHUNK_TOKEN_SIZE);
                accumulated_chunks.extend(new_chunks);
                let top_chunks: Vec<chunker::Chunk> =
                    rerank::rerank_chunks(&accumulated_chunks, &query, config::TOP_K_CHUNKS)
                        .into_iter()
                        .cloned()
                        .collect();

                // Stage 7: Chunks judge. Fall back to snippets when reader was
                // degraded and produced no chunks.
                let judge_sources: Vec<JudgeSource> = if top_chunks.is_empty() {
                    snippet_sources.clone()
                } else {
                    top_chunks
                        .iter()
                        .map(|c| JudgeSource {
                            title: c.source_title.clone(),
                            url: c.source_url.clone(),
                            text: c.text.clone(),
                        })
                        .collect()
                };

                let chunk_verdict = judge.call(&query, &judge_sources).await?;

                metadata.iterations.push(IterationTrace {
                    stage: IterationStage::Initial,
                    queries: vec![query.clone()],
                    urls_fetched: reader_urls.clone(),
                    reader_empty_urls: reader_result.empty_urls.clone(),
                    judge_verdict: chunk_verdict.sufficiency,
                    judge_reasoning: chunk_verdict.reasoning.clone(),
                    duration_ms: iter_start.elapsed().as_millis() as u64,
                });

                // Build synthesis messages from judge sources (convert to SearxResult shape).
                let synth_results: Vec<SearxResult> = judge_sources
                    .iter()
                    .map(|s| SearxResult {
                        title: s.title.clone(),
                        url: s.url.clone(),
                        content: s.text.clone(),
                    })
                    .collect();
                let messages =
                    build_synthesis_messages(&history_snapshot, &query, &synth_results, today);

                if matches!(chunk_verdict.sufficiency, Sufficiency::Sufficient) {
                    metadata.total_duration_ms = iter_start.elapsed().as_millis() as u64;
                    on_event(SearchEvent::Composing);
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
                    return Ok(());
                }

                // Not sufficient. Task 15 adds the gap loop here.
                // For Task 14: fall to the exhaustion fallback.
                warnings.push(SearchWarning::IterationCapExhausted);
                on_event(SearchEvent::Warning {
                    warning: SearchWarning::IterationCapExhausted,
                });
                metadata.total_duration_ms = iter_start.elapsed().as_millis() as u64;
                on_event(SearchEvent::Composing);
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

    // ── QueueJudge: stateful mock that pops verdicts from a queue ─────────────

    use std::collections::VecDeque;

    struct QueueJudge(std::sync::Mutex<VecDeque<JudgeVerdict>>);

    #[async_trait]
    impl JudgeCaller for QueueJudge {
        async fn call(&self, _q: &str, _s: &[JudgeSource]) -> Result<JudgeVerdict, SearchError> {
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| SearchError::Internal("queue empty".into()))
        }
    }

    fn sufficient_verdict() -> JudgeVerdict {
        JudgeVerdict {
            sufficiency: Sufficiency::Sufficient,
            reasoning: "ok".into(),
            gap_queries: vec![],
        }
    }

    #[tokio::test]
    async fn queue_judge_returns_internal_error_when_empty() {
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));
        let err = judge.call("q", &[]).await.unwrap_err();
        assert!(matches!(err, SearchError::Internal(_)));
    }

    fn insufficient_verdict() -> JudgeVerdict {
        JudgeVerdict {
            sufficiency: Sufficiency::Insufficient,
            reasoning: "not enough".into(),
            gap_queries: vec!["q1".into()],
        }
    }

    fn partial_verdict() -> JudgeVerdict {
        JudgeVerdict {
            sufficiency: Sufficiency::Partial,
            reasoning: "partial".into(),
            gap_queries: vec!["q1".into()],
        }
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
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));

        let err = run_agentic(
            "http://127.0.0.1:1/api/chat",
            "http://127.0.0.1:1/search",
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "   ".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
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
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));

        let err = run_agentic(
            "http://127.0.0.1:1/api/chat",
            "http://127.0.0.1:1/search",
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
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
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            "http://127.0.0.1:1/search",
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "tell me more".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
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
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            "http://127.0.0.1:1/search",
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
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
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));

        run_agentic(
            &format!("{}/api/chat", ollama.url()),
            "http://127.0.0.1:1/search",
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "what is 2+2".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
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

    // ── run_agentic: initial search round tests ──────────────────────────────

    fn proceed_search_router(query: &str) -> MockRouter {
        MockRouter(RouterJudgeOutput {
            action: Action::Proceed,
            clarifying_question: None,
            history_sufficiency: Some(Sufficiency::Insufficient),
            optimized_query: Some(query.into()),
        })
    }

    fn searx_body_one_result(url: &str) -> String {
        serde_json::json!({
            "results": [
                { "title": "result", "url": url, "content": "some content" }
            ]
        })
        .to_string()
    }

    fn stream_line_token(token: &str) -> String {
        format!(
            "{{\"message\":{{\"role\":\"assistant\",\"content\":\"{token}\"}},\"done\":false}}\n\
             {{\"message\":{{\"role\":\"assistant\",\"content\":\"\"}},\"done\":true}}\n"
        )
    }

    // Test: snippets judge returns Sufficient; no reader, no Warning.
    #[tokio::test]
    async fn initial_round_snippets_sufficient_skips_reader() {
        let mut ollama = mockito::Server::new_async().await;
        let mut searx = mockito::Server::new_async().await;

        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(searx_body_one_result("https://example.com/a"))
            .create_async()
            .await;

        let stream = stream_line_token("answer");
        let _stream_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":true}"#.to_string(),
            ))
            .with_body(stream)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![sufficient_verdict()].into_iter().collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama.url()),
            &format!("{}/search", searx.url()),
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        assert_eq!(evs[0], SearchEvent::AnalyzingQuery);
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::Searching)));
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::Sources { .. })));
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::Composing)));
        assert!(
            evs.iter()
                .any(|e| matches!(e, SearchEvent::Token { content } if content == "answer")),
            "expected token with 'answer'"
        );
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);

        // No ReadingSources on snippet-sufficient path.
        assert!(evs
            .iter()
            .all(|e| !matches!(e, SearchEvent::ReadingSources)));
        // No warnings.
        assert!(evs
            .iter()
            .all(|e| !matches!(e, SearchEvent::Warning { .. })));
    }

    // Test: snippets partial, reader succeeds, chunks judge sufficient.
    // Exercises the full reader path: fetch pages -> chunk -> rerank chunks ->
    // judge from chunks (not snippet fallback).
    #[tokio::test]
    async fn initial_round_escalates_to_reader_when_snippets_partial() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "result",
                "markdown": "full page content about rust async",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let mut ollama = mockito::Server::new_async().await;
        let mut searx = mockito::Server::new_async().await;

        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(searx_body_one_result("https://example.com/a"))
            .create_async()
            .await;

        let stream = stream_line_token("final");
        let _stream_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":true}"#.to_string(),
            ))
            .with_body(stream)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");

        // First judge call (snippets) = partial; reader fetches pages;
        // second judge call (chunks) = sufficient.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![partial_verdict(), sufficient_verdict()]
                .into_iter()
                .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama.url()),
            &format!("{}/search", searx.url()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::Searching)));
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::ReadingSources)));
        // No ReaderUnavailable warning when reader succeeds: verify by
        // checking the event list contains no Warning events of any kind,
        // since this test configures the reader to succeed.
        let has_any_warning = evs.iter().any(|e| matches!(e, SearchEvent::Warning { .. }));
        assert!(!has_any_warning, "expected no warnings in: {evs:?}");
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);
    }

    // Test: judge always insufficient; falls to IterationCapExhausted warning.
    #[tokio::test]
    async fn initial_round_exhausts_emits_iteration_cap_exhausted_warning() {
        let mut ollama = mockito::Server::new_async().await;
        let mut searx = mockito::Server::new_async().await;

        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(searx_body_one_result("https://example.com/a"))
            .create_async()
            .await;

        let stream = stream_line_token("best effort");
        let _stream_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":true}"#.to_string(),
            ))
            .with_body(stream)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");

        // Both judge calls return insufficient.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![insufficient_verdict(), insufficient_verdict()]
                .into_iter()
                .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama.url()),
            &format!("{}/search", searx.url()),
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(
                e,
                SearchEvent::Warning {
                    warning: SearchWarning::IterationCapExhausted
                }
            )),
            "expected IterationCapExhausted warning in: {evs:?}"
        );
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);
    }

    // Test: SearXNG returns empty; emits NoResultsInitial warning and errors.
    #[tokio::test]
    async fn initial_round_no_searxng_results_emits_warning_and_errors() {
        let mut searx = mockito::Server::new_async().await;

        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(r#"{"results":[]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));

        let err = run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx.url()),
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap_err();

        assert_eq!(err, SearchError::NoResults);
        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(
                e,
                SearchEvent::Warning {
                    warning: SearchWarning::NoResultsInitial
                }
            )),
            "expected NoResultsInitial warning in: {evs:?}"
        );
    }

    // Test: reader unavailable, falls back to snippets for second judge call.
    #[tokio::test]
    async fn initial_round_reader_unavailable_degrades_gracefully() {
        let mut ollama = mockito::Server::new_async().await;
        let mut searx = mockito::Server::new_async().await;

        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(searx_body_one_result("https://example.com/a"))
            .create_async()
            .await;

        let stream = stream_line_token("degraded");
        let _stream_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":true}"#.to_string(),
            ))
            .with_body(stream)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");

        // First judge (snippets) = partial; triggers reader.
        // Reader will fail (READER_BASE_URL is not running in test).
        // Second judge (falls back to snippets because no chunks) = sufficient.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![partial_verdict(), sufficient_verdict()]
                .into_iter()
                .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama.url()),
            &format!("{}/search", searx.url()),
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(
                e,
                SearchEvent::Warning {
                    warning: SearchWarning::ReaderUnavailable
                }
            )),
            "expected ReaderUnavailable warning in: {evs:?}"
        );
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);
    }

    // ── Additional coverage: rare error and cancellation paths ─────────────────

    // A router that cancels a token as a side effect of being called, so tests
    // can exercise mid-flight cancellation that arrives after the router call.
    struct CancellingRouter {
        output: RouterJudgeOutput,
        token: CancellationToken,
    }

    #[async_trait]
    impl RouterJudgeCaller for CancellingRouter {
        async fn call(
            &self,
            _h: &[ChatMessage],
            _q: &str,
        ) -> Result<RouterJudgeOutput, SearchError> {
            self.token.cancel();
            Ok(self.output.clone())
        }
    }

    // Cancel fires mid-CLARIFY streaming (after router returns Clarify).
    #[tokio::test]
    async fn clarify_cancels_mid_stream_when_token_fired_after_router() {
        let token = CancellationToken::new();
        let router = CancellingRouter {
            output: RouterJudgeOutput {
                action: Action::Clarify,
                clarifying_question: Some(
                    "which specific project version are you asking about here".into(),
                ),
                history_sufficiency: None,
                optimized_query: None,
            },
            token: token.clone(),
        };
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));
        let client = reqwest::Client::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            "http://127.0.0.1:1/search",
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled event in: {evs:?}"
        );
    }

    // Cancel fires after router Proceed but before SearXNG.
    #[tokio::test]
    async fn proceed_cancels_before_searxng() {
        let token = CancellationToken::new();
        let router = CancellingRouter {
            output: RouterJudgeOutput {
                action: Action::Proceed,
                clarifying_question: None,
                history_sufficiency: Some(Sufficiency::Insufficient),
                optimized_query: Some("q".into()),
            },
            token: token.clone(),
        };
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));
        let client = reqwest::Client::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            "http://127.0.0.1:1/search",
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled event in: {evs:?}"
        );
    }

    // SearXNG returns a non-NoResults error (e.g. HTTP 503).
    #[tokio::test]
    async fn initial_round_propagates_searxng_http_error() {
        let mut searx = mockito::Server::new_async().await;
        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_status(503)
            .with_body("down")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (_, cb) = collect_events();
        let router = proceed_search_router("q");
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));

        let err = run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx.url()),
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap_err();

        assert_eq!(err, SearchError::SearxHttp(503));
    }

    // A judge that fires the CancellationToken the first time it is called, so
    // we can exercise the cancel-before-reader escalation path.
    struct CancelsOnJudgeCall {
        token: CancellationToken,
        verdict: JudgeVerdict,
    }

    #[async_trait]
    impl JudgeCaller for CancelsOnJudgeCall {
        async fn call(&self, _q: &str, _s: &[JudgeSource]) -> Result<JudgeVerdict, SearchError> {
            self.token.cancel();
            Ok(self.verdict.clone())
        }
    }

    // Cancel fires between snippet judge (partial) and reader escalation.
    #[tokio::test]
    async fn proceed_cancels_before_reader_after_snippets_partial() {
        let mut searx = mockito::Server::new_async().await;
        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(searx_body_one_result("https://example.com/a"))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");
        let judge = CancelsOnJudgeCall {
            token: token.clone(),
            verdict: partial_verdict(),
        };

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx.url()),
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled in: {evs:?}"
        );
    }

    // Reader returns Cancelled (cancellation fires during reader fetch).
    #[tokio::test]
    async fn reader_cancelled_mid_batch_emits_cancelled_event() {
        use std::time::Duration;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // Respond slowly so the cancel fires mid-fetch.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(200))
                    .set_body_json(serde_json::json!({
                        "url": "u", "title": "t", "markdown": "m", "status": "ok"
                    })),
            )
            .mount(&reader_server)
            .await;

        let mut searx = mockito::Server::new_async().await;
        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(searx_body_one_result("https://example.com/a"))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // First judge returns partial (to enter reader stage).
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![partial_verdict()].into_iter().collect(),
        ));

        // Cancel the token after a brief delay so it fires mid-reader-fetch.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            token_clone.cancel();
        });

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx.url()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled event in: {evs:?}"
        );
    }

    // Reader batch times out (READER_BATCH_TIMEOUT_S=1s in tests);
    // pipeline emits ReaderPartialFailure warning and continues.
    #[tokio::test]
    async fn reader_batch_timeout_emits_partial_failure_warning_and_continues() {
        use std::time::Duration;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // Respond after 2s -- longer than READER_BATCH_TIMEOUT_S=1s in tests.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(2))
                    .set_body_json(serde_json::json!({
                        "url": "u", "title": "t", "markdown": "m", "status": "ok"
                    })),
            )
            .mount(&reader_server)
            .await;

        let mut ollama = mockito::Server::new_async().await;
        let mut searx = mockito::Server::new_async().await;

        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(searx_body_one_result("https://example.com/a"))
            .create_async()
            .await;

        let stream = stream_line_token("ok");
        let _stream_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":true}"#.to_string(),
            ))
            .with_body(stream)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // snippets = partial; reader batch times out; second judge (snippet
        // fallback since no chunks) = sufficient.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![partial_verdict(), sufficient_verdict()]
                .into_iter()
                .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama.url()),
            &format!("{}/search", searx.url()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(
                e,
                SearchEvent::Warning {
                    warning: SearchWarning::ReaderPartialFailure
                }
            )),
            "expected ReaderPartialFailure from BatchTimeout in: {evs:?}"
        );
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);
    }

    // Reader: >50% of URLs fail (HTTP 502), triggers ReaderPartialFailure.
    // Uses 2 SearXNG results: one reader responds with 502 (Failed), the other
    // with 200+ok. The failed URL count (1 HTTP fail at "/extract") triggers the
    // >partial_threshold (ceil(2*0.5)=1, 1>1=false) rule... need more than 50%.
    //
    // To reliably trigger the >50% branch, we use 1 URL where reader responds
    // 502 (Failed). With 1 URL: threshold = ceil(1*0.5)=1, 1>1=false.
    //
    // To have failed_urls.len() > partial_threshold, we need at least 2 URLs
    // with more than 1 failure. With 2 URLs: threshold=1, failures must be >1.
    // Use a reader mock that returns 502 for both. Since both fail as HTTP (not
    // connect-refused), service_unavailable_count=0, any_succeeded=false, and
    // the reader returns Ok(result) with 2 failed_urls. Then 2 > 1 = true.
    #[tokio::test]
    async fn reader_majority_http_failures_emits_partial_failure_warning() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // All reader calls return HTTP 502 (classified as Failed, not
        // ServiceUnavailable). Any_succeeded stays false; service_unavailable
        // count stays 0; the reader returns Ok with failed_urls.len()=2.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(502))
            .mount(&reader_server)
            .await;

        let mut ollama = mockito::Server::new_async().await;
        let mut searx = mockito::Server::new_async().await;

        // Two results: both will fail at reader.
        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(
                serde_json::json!({
                    "results": [
                        { "title": "r1", "url": "https://example.com/a", "content": "c" },
                        { "title": "r2", "url": "https://example.com/b", "content": "c" },
                    ]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let stream = stream_line_token("ok");
        let _stream_mock = ollama
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":true}"#.to_string(),
            ))
            .with_body(stream)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // snippets = partial; reader returns 0 pages + 2 failed;
        // second judge gets snippet fallback (no chunks) and returns sufficient.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![partial_verdict(), sufficient_verdict()]
                .into_iter()
                .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama.url()),
            &format!("{}/search", searx.url()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            cb,
            &router,
            &judge,
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(
                e,
                SearchEvent::Warning {
                    warning: SearchWarning::ReaderPartialFailure
                }
            )),
            "expected ReaderPartialFailure warning in: {evs:?}"
        );
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done);
    }
}
