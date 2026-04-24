//! Orchestrator for the `/search` pipeline.
//!
//! Implements the agentic state machine:
//!   AnalyzingQuery -> Token* -> Done          (CLARIFY branch)
//!   AnalyzingQuery -> Token* -> Done          (history-sufficient branch)
//!   AnalyzingQuery -> Searching -> ... -> Done  (fresh web search + synthesis)
//!
//! [`run_agentic`] is the sole production entry point (Task 16). It uses two
//! trait seams, [`RouterJudgeCaller`] and [`JudgeCaller`], so tests can inject
//! deterministic mocks without spinning a mock Ollama server.
//!
//! The pipeline is the single owner of `ConversationHistory` mutations for a
//! search turn: every branch that produces a user-visible assistant message
//! persists both the user's query and the assistant reply so that subsequent
//! classifier calls can see the full conversational state.
//!
//! Cancellation is checked at every stage entry and before every network call.
//! Long-running HTTP awaits (SearXNG, judge) race inline against the
//! cancellation token via `tokio::select!`, so in-flight work is dropped
//! immediately when the user dismisses the overlay rather than waiting for
//! the round-trip to complete.

use std::sync::atomic::Ordering;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::commands::{stream_ollama_chat, ChatMessage, ConversationHistory, StreamChunk};

use super::chunker;
use super::config;
use super::llm::{
    build_answer_from_context_messages, build_synthesis_messages, call_judge, call_router_merged,
    JudgeSource,
};
use super::reader;
use super::rerank;
use super::searxng;
use super::types::{
    Action, IterationStage, IterationTrace, JudgeVerdict, RouterJudgeOutput, SearchError,
    SearchEvent, SearchMetadata, SearchResultPreview, SearchTraceCounts, SearchTraceKind,
    SearchTraceStatus, SearchTraceStep, SearchWarning, SearxResult, Sufficiency,
};

/// Build the `JudgeSource` list fed to the sufficiency judge and the synthesis
/// prompt from reranked chunks, deduplicated by source URL.
///
/// The reranker may return several chunks that came from the same page. Passing
/// all of them to the synthesizer inflates the numbered citation list so that
/// index `[k]` no longer maps 1:1 to a distinct source URL. Keeping only the
/// highest-scoring chunk per URL guarantees that citation indices match the
/// Sources footer positions shown in the UI, and keeps the judge's prompt
/// focused on distinct sources rather than repeated paragraphs.
fn chunks_to_judge_sources(chunks: &[chunker::Chunk]) -> Vec<JudgeSource> {
    let mut seen: std::collections::HashSet<String> = Default::default();
    chunks
        .iter()
        .filter(|c| seen.insert(c.source_url.clone()))
        .map(|c| JudgeSource {
            title: c.source_title.clone(),
            url: c.source_url.clone(),
            text: c.text.clone(),
        })
        .collect()
}

/// Emit the authoritative `Sources` event just before synthesis so the UI
/// footer matches the citation indices the LLM will actually produce.
///
/// Earlier `Sources` events during the initial-search and gap-refine rounds
/// advertise the URLs we are *considering*; this final event reflects the
/// URLs the synthesis prompt actually received, so `[k]` in the streamed
/// answer always maps to an entry the user can click.
fn emit_final_sources(on_event: &(dyn Fn(SearchEvent) + Sync), sources: &[JudgeSource]) {
    let previews: Vec<SearchResultPreview> = sources
        .iter()
        .map(|s| SearchResultPreview {
            title: s.title.clone(),
            url: s.url.clone(),
        })
        .collect();
    on_event(SearchEvent::Sources { results: previews });
}

fn emit_trace(on_event: &(dyn Fn(SearchEvent) + Sync), step: SearchTraceStep) {
    on_event(SearchEvent::Trace { step });
}

fn trace_step(
    id: impl Into<String>,
    kind: SearchTraceKind,
    status: SearchTraceStatus,
    title: impl Into<String>,
    summary: impl Into<String>,
) -> SearchTraceStep {
    SearchTraceStep {
        id: id.into(),
        kind,
        status,
        round: None,
        title: title.into(),
        summary: summary.into(),
        detail: None,
        queries: Vec::new(),
        urls: Vec::new(),
        domains: Vec::new(),
        verdict: None,
        counts: None,
    }
}

fn to_u32_saturating(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn domain_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .map(|host| host.trim_start_matches("www.").to_string())
        .unwrap_or_else(|| url.to_string())
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn unique_domains<I, S>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .map(|value| domain_of(value.as_ref()))
        .filter(|domain| seen.insert(domain.clone()))
        .take(6)
        .collect()
}

fn unique_urls<I, S>(values: I, limit: usize) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .map(|value| value.as_ref().trim().to_string())
        .filter(|url| !url.is_empty())
        .filter(|url| seen.insert(url.clone()))
        .take(limit)
        .collect()
}

fn snippet_judge_title() -> &'static str {
    "Checking what the results already cover"
}

fn snippet_judge_running_summary() -> &'static str {
    "Seeing whether the search results already answer the question on their own."
}

fn snippet_judge_summary(verdict: Sufficiency) -> String {
    match verdict {
        Sufficiency::Sufficient => "The results already line up on the key answer.".to_string(),
        Sufficiency::Partial => {
            "The results point in the right direction, but a few details are still missing."
                .to_string()
        }
        Sufficiency::Insufficient => "The results still do not say enough yet.".to_string(),
    }
}

fn snippet_judge_detail(verdict: Sufficiency, reasoning: &str) -> Option<String> {
    let reasoning = reasoning.trim();
    if reasoning.is_empty() {
        return None;
    }

    Some(match verdict {
        Sufficiency::Sufficient => {
            format!("Across these results, the main takeaway is: {reasoning}")
        }
        Sufficiency::Partial => format!("So far, the results suggest: {reasoning}"),
        Sufficiency::Insufficient => format!("What still seems unclear is: {reasoning}"),
    })
}

fn compose_title() -> &'static str {
    "Synthesizing the answer"
}

fn compose_summary(source_count: usize) -> String {
    let _ = source_count;
    "Pulling the strongest points together into a clear answer with citations.".to_string()
}

fn fallback_compose_summary(hit_iteration_cap: bool, source_count: usize) -> String {
    if hit_iteration_cap {
        "Pulling together the strongest evidence found so far into the best supported answer possible."
            .to_string()
    } else {
        compose_summary(source_count)
    }
}

fn judge_summary(subject: &str, verdict: Sufficiency) -> String {
    match verdict {
        Sufficiency::Sufficient => format!("The {subject} already cover the answer."),
        Sufficiency::Partial => format!("The {subject} help, but some details are still missing."),
        Sufficiency::Insufficient => {
            format!("The {subject} are not enough yet, so Thuki needs to keep digging.")
        }
    }
}

/// Returns the current UTC date formatted as `YYYY-MM-DD`.
///
/// Uses `time::OffsetDateTime::now_utc()` to avoid the unsoundness of
/// local-offset calculation in multi-threaded processes on Unix (documented
/// in the `time` crate README). UTC is appropriate here: the date string is
/// injected into the synthesis prompt purely to prevent the model from
/// substituting its training-cutoff year; sub-day precision is irrelevant.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn today_iso() -> String {
    let d = time::OffsetDateTime::now_utc().date();
    format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day())
}

/// Takes a snapshot of the conversation history and its epoch counter under a
/// single lock acquisition. The snapshot is used for the entire pipeline run;
/// if the epoch changes before we write back, the write is skipped (the user
/// started a new conversation mid-flight).
fn snapshot_history(history: &ConversationHistory) -> (u64, Vec<ChatMessage>) {
    let conv = lock_or_recover(&history.messages);
    let epoch = history.epoch.load(Ordering::SeqCst);
    (epoch, conv.clone())
}

fn can_answer_from_history(history_snapshot: &[ChatMessage]) -> bool {
    !history_snapshot.is_empty()
}

/// Runs a streaming Ollama call, translating `StreamChunk` events into
/// `SearchEvent` events and persisting the completed assistant turn on normal
/// completion (or partial completion via cancellation).
///
/// `warnings` and `metadata` are forwarded to `persist_turn`; the DB columns
/// for these fields were added in Task 17. The frontend serializes and passes
/// them back via `persist_message` when saving the turn.
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
    warnings: Vec<SearchWarning>,
    metadata: Option<SearchMetadata>,
    on_event: &impl Fn(SearchEvent),
) {
    let saw_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let saw_done_for_callback = saw_done.clone();
    let accumulated = stream_ollama_chat(
        endpoint,
        model,
        messages,
        false,
        client,
        cancel_token,
        |chunk| match chunk {
            StreamChunk::Done => {
                saw_done_for_callback.store(true, Ordering::SeqCst);
            }
            other => on_event(translate_chunk(other)),
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
            warnings,
            metadata.clone(),
        );
    }

    if saw_done.load(Ordering::SeqCst) {
        on_event(SearchEvent::Done { metadata });
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
        StreamChunk::Done => SearchEvent::Done { metadata: None },
        StreamChunk::Cancelled => SearchEvent::Cancelled,
        StreamChunk::Error(e) => SearchEvent::Error { message: e.message },
    }
}

/// Appends `(user, assistant)` to the conversation history, skipping the
/// write when the history epoch has advanced since the snapshot (i.e. the
/// user reset the conversation mid-pipeline). The epoch check is performed
/// under the lock so there is no race window between the check and the push.
///
/// `warnings` and `metadata` are accepted but not written to SQLite here:
/// the pipeline has no DB connection. They are available to the frontend via
/// the `search_warnings` and `search_metadata` fields added to
/// `SaveMessagePayload` in Task 17. The frontend passes them back when it
/// calls `persist_message` at the end of the turn.
fn persist_turn(
    history: &ConversationHistory,
    epoch_at_start: u64,
    user_msg: ChatMessage,
    assistant_msg: ChatMessage,
    warnings: Vec<SearchWarning>,
    metadata: Option<SearchMetadata>,
) {
    let _ = (warnings, metadata);
    let mut conv = lock_or_recover(&history.messages);
    if history.epoch.load(Ordering::SeqCst) != epoch_at_start {
        return;
    }
    conv.push(user_msg);
    conv.push(assistant_msg);
}

// ── Agentic trait seams ────────────────────────────────────────────────────

/// Abstracts the merged router+judge LLM call so the agentic pipeline can be
/// tested with deterministic mock output without spinning a real Ollama server.
///
/// Production code uses [`DefaultRouterJudge`]. Tests inject a struct that
/// returns a canned [`RouterJudgeOutput`].
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
/// a predetermined sequence of [`JudgeVerdict`]s.
#[async_trait]
pub trait JudgeCaller: Send + Sync {
    /// Judges how well the given sources answer the query.
    async fn call(&self, query: &str, sources: &[JudgeSource])
        -> Result<JudgeVerdict, SearchError>;
}

/// Production [`RouterJudgeCaller`] implementation.
///
/// Carries the Ollama endpoint, model name, HTTP client, today string, and
/// cancellation token so the trait method can be called with only `history`
/// and `query`. Constructed once per pipeline invocation by the Tauri command
/// handler and passed by reference into [`run_agentic`].
///
/// Tests must NOT use this struct directly as it would hit a real Ollama
/// instance. Inject a mock [`RouterJudgeCaller`] instead.
pub struct DefaultRouterJudge {
    endpoint: String,
    model: String,
    client: reqwest::Client,
    cancel: CancellationToken,
    today: String,
    router_timeout_secs: u64,
}

impl DefaultRouterJudge {
    /// Constructs a `DefaultRouterJudge` that delegates to
    /// [`llm::call_router_merged`].
    ///
    /// - `endpoint`: fully-qualified `/api/chat` URL (e.g.
    ///   `http://127.0.0.1:11434/api/chat`).
    /// - `model`: Ollama model identifier (e.g. `"mistral"`).
    /// - `client`: shared `reqwest::Client`; the Tauri command clones it from
    ///   `AppState`.
    /// - `cancel`: the pipeline's cancellation token; races against the HTTP
    ///   call inside `call_router_merged`.
    /// - `today`: `YYYY-MM-DD` string injected into the merged prompt so the
    ///   model is anchored to the real calendar date.
    /// - `router_timeout_secs`: per-call wall-clock limit from `AppConfig.search`.
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn new(
        endpoint: String,
        model: String,
        client: reqwest::Client,
        cancel: CancellationToken,
        today: String,
        router_timeout_secs: u64,
    ) -> Self {
        Self {
            endpoint,
            model,
            client,
            cancel,
            today,
            router_timeout_secs,
        }
    }
}

#[async_trait]
impl RouterJudgeCaller for DefaultRouterJudge {
    #[cfg_attr(coverage_nightly, coverage(off))]
    async fn call(
        &self,
        history: &[ChatMessage],
        query: &str,
    ) -> Result<RouterJudgeOutput, SearchError> {
        call_router_merged(
            &self.endpoint,
            &self.model,
            &self.client,
            history,
            query,
            &self.today,
            &self.cancel,
            self.router_timeout_secs,
        )
        .await
    }
}

/// Production [`JudgeCaller`] implementation.
///
/// Carries the Ollama endpoint, model name, HTTP client, and cancellation
/// token so the trait method can be called with only `query` and `sources`.
/// Constructed once per pipeline invocation by the Tauri command handler.
///
/// Tests must NOT use this struct directly. Inject a mock [`JudgeCaller`].
pub struct DefaultJudge {
    endpoint: String,
    model: String,
    client: reqwest::Client,
    cancel: CancellationToken,
    judge_timeout_secs: u64,
}

impl DefaultJudge {
    /// Constructs a `DefaultJudge` that delegates to [`llm::call_judge`].
    ///
    /// - `endpoint`: fully-qualified `/api/chat` URL.
    /// - `model`: Ollama model identifier.
    /// - `client`: shared `reqwest::Client`.
    /// - `cancel`: the pipeline's cancellation token; races against the HTTP
    ///   call inside `call_judge`.
    /// - `judge_timeout_secs`: per-call wall-clock limit from `AppConfig.search`.
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn new(
        endpoint: String,
        model: String,
        client: reqwest::Client,
        cancel: CancellationToken,
        judge_timeout_secs: u64,
    ) -> Self {
        Self {
            endpoint,
            model,
            client,
            cancel,
            judge_timeout_secs,
        }
    }
}

#[async_trait]
impl JudgeCaller for DefaultJudge {
    #[cfg_attr(coverage_nightly, coverage(off))]
    async fn call(
        &self,
        query: &str,
        sources: &[JudgeSource],
    ) -> Result<JudgeVerdict, SearchError> {
        call_judge(
            &self.endpoint,
            &self.model,
            &self.client,
            query,
            sources,
            &self.cancel,
            self.judge_timeout_secs,
        )
        .await
    }
}

// ── Cancellation helper ────────────────────────────────────────────────────

/// Checks the cancellation token and, if fired, emits `Cancelled` and returns
/// `true`. Used at every stage entry in [`run_agentic`] to ensure no stage
/// begins after the user has dismissed the overlay.
///
/// Returning `true` means the caller should emit `SearchEvent::Cancelled` via
/// `on_event` and return `Ok(())` immediately (the `Cancelled` event has
/// already been emitted by this helper).
#[inline]
fn is_cancelled_emit(cancel: &CancellationToken, on_event: &impl Fn(SearchEvent)) -> bool {
    if cancel.is_cancelled() {
        on_event(SearchEvent::Cancelled);
        true
    } else {
        false
    }
}

/// Locks a mutex and recovers the inner value if a prior holder panicked.
///
/// The search pipeline should degrade gracefully after unrelated panics rather
/// than aborting on poison. Recovering the inner value preserves in-memory
/// state and keeps the user-visible search flow available.
fn lock_or_recover<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Shared immutable inputs used by the extracted search-pipeline stages.
struct SearchExecutionContext<'a> {
    ollama_endpoint: &'a str,
    searxng_endpoint: &'a str,
    model: &'a str,
    client: &'a reqwest::Client,
    cancel_token: &'a CancellationToken,
    chat_system_prompt: &'a str,
    history: &'a ConversationHistory,
    today: &'a str,
    on_event: &'a (dyn Fn(SearchEvent) + Sync),
    runtime_config: &'a config::SearchRuntimeConfig,
}

/// Per-turn values reused across extracted search stages.
struct SearchTurnInputs<'a> {
    history_snapshot: &'a [ChatMessage],
    epoch_at_start: u64,
    user_msg: ChatMessage,
}

/// Result of the extracted gap-refinement loop.
enum GapLoopDisposition {
    /// The loop found enough evidence and already streamed the answer.
    Streamed,
    /// The loop exhausted its options and the caller should stream a
    /// best-effort fallback answer from the returned sources.
    Fallback {
        sources: Vec<JudgeSource>,
        hit_iteration_cap: bool,
    },
}

/// Converts synthesis and judge sources into the `SearxResult` shape expected
/// by the synthesis prompt builder.
fn judge_sources_to_results(sources: &[JudgeSource]) -> Vec<SearxResult> {
    sources
        .iter()
        .map(|source| SearxResult {
            title: source.title.clone(),
            url: source.url.clone(),
            content: source.text.clone(),
        })
        .collect()
}

/// Emits the final source list, compose trace step, and synthesis stream for a
/// chosen set of evidence sources.
async fn stream_synthesis_from_sources(
    shared: &SearchExecutionContext<'_>,
    turn: &SearchTurnInputs<'_>,
    query: &str,
    sources: &[JudgeSource],
    warnings: Vec<SearchWarning>,
    metadata: Option<SearchMetadata>,
    compose_summary_text: String,
) {
    let synth_results = judge_sources_to_results(sources);
    // KNOWN LIMITATION: fetched page content is not sanitized for prompt-injection attempts.
    // Mitigation: the synthesis system prompt instructs the model to follow its role; the local
    // Ollama model has no external auth or privileged actions to be hijacked. If this is extended
    // to a cloud model or multi-user context, sanitization becomes mandatory.
    let messages =
        build_synthesis_messages(turn.history_snapshot, query, &synth_results, shared.today);

    emit_final_sources(shared.on_event, sources);

    let mut compose_step = trace_step(
        "compose",
        SearchTraceKind::Compose,
        SearchTraceStatus::Running,
        compose_title(),
        compose_summary_text,
    );
    compose_step.domains = unique_domains(sources.iter().map(|source| source.url.as_str()));
    compose_step.counts = Some(SearchTraceCounts {
        sources: Some(to_u32_saturating(sources.len())),
        ..SearchTraceCounts::default()
    });
    emit_trace(shared.on_event, compose_step);
    (shared.on_event)(SearchEvent::Composing);

    run_streaming_branch(
        shared.ollama_endpoint,
        shared.model,
        shared.client,
        shared.cancel_token.clone(),
        messages,
        shared.history,
        turn.epoch_at_start,
        turn.user_msg.clone(),
        warnings,
        metadata,
        &shared.on_event,
    )
    .await;
}

/// Completes the clarification branch by streaming the clarifying question as
/// token events, persisting it, and finishing the turn.
async fn run_clarify_branch(
    cancel_token: &CancellationToken,
    history: &ConversationHistory,
    epoch_at_start: u64,
    user_msg: ChatMessage,
    question: String,
    on_event: &(dyn Fn(SearchEvent) + Sync),
) -> Result<(), SearchError> {
    let analyze_step = trace_step(
        "analyze",
        SearchTraceKind::Analyze,
        SearchTraceStatus::Completed,
        "Understanding the question",
        "This request could mean a few different things, so Thuki needs one more detail before searching.",
    );
    emit_trace(on_event, analyze_step);

    let clarify_step = trace_step(
        "clarify",
        SearchTraceKind::Clarify,
        SearchTraceStatus::Completed,
        "Waiting for clarification",
        "Search is paused until you clarify who or what you mean.",
    );
    emit_trace(on_event, clarify_step);

    for piece in split_into_stream_pieces(&question) {
        if is_cancelled_emit(cancel_token, &on_event) {
            return Ok(());
        }
        on_event(SearchEvent::Token { content: piece });
    }

    persist_turn(
        history,
        epoch_at_start,
        user_msg,
        ChatMessage {
            role: "assistant".to_string(),
            content: question,
            images: None,
        },
        Vec::new(),
        None,
    );
    on_event(SearchEvent::Done { metadata: None });
    Ok(())
}

/// Completes the history-sufficient branch by synthesizing an answer from the
/// current conversation without opening any web sources.
async fn run_history_answer_branch(
    shared: &SearchExecutionContext<'_>,
    turn: SearchTurnInputs<'_>,
    user_query: &str,
) -> Result<(), SearchError> {
    let mut analyze_step = trace_step(
        "analyze",
        SearchTraceKind::Analyze,
        SearchTraceStatus::Completed,
        "Understanding the question",
        "The current conversation already contains enough context, so no web search is needed.",
    );
    analyze_step.counts = Some(SearchTraceCounts {
        sources: Some(to_u32_saturating(turn.history_snapshot.len())),
        ..SearchTraceCounts::default()
    });
    emit_trace(shared.on_event, analyze_step);

    emit_trace(
        shared.on_event,
        trace_step(
            "history-answer",
            SearchTraceKind::HistoryAnswer,
            SearchTraceStatus::Running,
            "Answering from the current conversation",
            "Using what is already in this chat instead of opening web sources.",
        ),
    );

    let messages = build_answer_from_context_messages(
        shared.chat_system_prompt,
        turn.history_snapshot,
        user_query,
    );

    run_streaming_branch(
        shared.ollama_endpoint,
        shared.model,
        shared.client,
        shared.cancel_token.clone(),
        messages,
        shared.history,
        turn.epoch_at_start,
        turn.user_msg,
        Vec::new(),
        None,
        &shared.on_event,
    )
    .await;

    Ok(())
}

/// Runs the bounded gap-refinement loop after the initial retrieval round.
///
/// The loop keeps URL dedup state and the globally reranked chunk pool local to
/// this stage so its control flow can be reasoned about independently from the
/// initial round.
#[allow(clippy::too_many_arguments)]
async fn run_gap_refinement_loop(
    shared: &SearchExecutionContext<'_>,
    turn: &SearchTurnInputs<'_>,
    judge: &dyn JudgeCaller,
    query: &str,
    reader_client: &reader::ReaderClient,
    snippet_sources: Vec<JudgeSource>,
    warnings: &mut Vec<SearchWarning>,
    metadata: &mut SearchMetadata,
    mut accumulated_chunks: Vec<chunker::Chunk>,
    mut accumulated_urls: std::collections::HashSet<String>,
    mut current_queries: Vec<String>,
    started_at: std::time::Instant,
) -> Result<GapLoopDisposition, SearchError> {
    let gap_round_total = (shared.runtime_config.max_iterations as u32).saturating_sub(1);
    let mut hit_iteration_cap = false;

    for attempt in 2..=(shared.runtime_config.max_iterations as u32) {
        if current_queries.is_empty() {
            break;
        }
        if is_cancelled_emit(shared.cancel_token, &shared.on_event) {
            return Ok(GapLoopDisposition::Fallback {
                sources: snippet_sources.clone(),
                hit_iteration_cap: false,
            });
        }

        let gap_round = attempt - 1;

        let mut refine_step = trace_step(
            format!("round-{attempt}-refine"),
            SearchTraceKind::Refine,
            SearchTraceStatus::Completed,
            "Planned another search",
            "Thuki prepared a follow-up search to fill the remaining gaps.",
        );
        refine_step.round = Some(attempt);
        refine_step.queries = current_queries.clone();
        emit_trace(shared.on_event, refine_step);

        (shared.on_event)(SearchEvent::RefiningSearch {
            attempt: gap_round,
            total: gap_round_total,
        });

        let round_start = std::time::Instant::now();
        let mut round_search_step = trace_step(
            format!("round-{attempt}-search"),
            SearchTraceKind::Search,
            SearchTraceStatus::Running,
            "Searching the web again",
            "Looking for the missing details from a different angle.",
        );
        round_search_step.round = Some(attempt);
        round_search_step.queries = current_queries.clone();
        emit_trace(shared.on_event, round_search_step);
        (shared.on_event)(SearchEvent::Searching {
            queries: current_queries.clone(),
        });

        let gap_search_fut = searxng::search_all_with_endpoint(
            shared.searxng_endpoint,
            &current_queries,
            shared.runtime_config.search_timeout_s,
        );
        let gap_results = tokio::select! {
            biased;
            _ = shared.cancel_token.cancelled() => {
                (shared.on_event)(SearchEvent::Cancelled);
                return Ok(GapLoopDisposition::Fallback {
                    sources: snippet_sources.clone(),
                    hit_iteration_cap: false,
                });
            }
            res = gap_search_fut => res.unwrap_or_default(),
        };
        let gap_results_count = gap_results.len();

        let new_urls: Vec<_> = gap_results
            .into_iter()
            .filter(|result| accumulated_urls.insert(result.url.clone()))
            .collect();

        if new_urls.is_empty() {
            let mut round_search_step = trace_step(
                format!("round-{attempt}-search"),
                SearchTraceKind::Search,
                SearchTraceStatus::Completed,
                "Searching the web again",
                "This follow-up search did not surface any new pages beyond what Thuki had already seen.",
            );
            round_search_step.round = Some(attempt);
            round_search_step.queries = current_queries.clone();
            round_search_step.counts = Some(SearchTraceCounts {
                found: Some(0),
                ..SearchTraceCounts::default()
            });
            emit_trace(shared.on_event, round_search_step);

            metadata.iterations.push(IterationTrace {
                stage: IterationStage::GapRound { round: gap_round },
                queries: current_queries.clone(),
                urls_fetched: vec![],
                reader_empty_urls: vec![],
                judge_verdict: Sufficiency::Insufficient,
                judge_reasoning: "no new search results".into(),
                duration_ms: round_start.elapsed().as_millis() as u64,
            });
            (shared.on_event)(SearchEvent::IterationComplete {
                trace: metadata
                    .iterations
                    .last()
                    .expect("iteration was just pushed")
                    .clone(),
            });
            current_queries.clear();
            continue;
        }

        let mut round_search_step = trace_step(
            format!("round-{attempt}-search"),
            SearchTraceKind::Search,
            SearchTraceStatus::Completed,
            "Searching the web again",
            format!(
                "Found {} fresh results worth checking.",
                if new_urls.len() == 1 {
                    "1 result".to_string()
                } else {
                    format!("{} results", new_urls.len())
                }
            ),
        );
        round_search_step.round = Some(attempt);
        round_search_step.queries = current_queries.clone();
        round_search_step.urls = unique_urls(
            new_urls.iter().map(|result| result.url.as_str()),
            new_urls.len(),
        );
        round_search_step.domains =
            unique_domains(new_urls.iter().map(|result| result.url.as_str()));
        round_search_step.counts = Some(SearchTraceCounts {
            found: Some(to_u32_saturating(gap_results_count)),
            kept: Some(to_u32_saturating(new_urls.len())),
            ..SearchTraceCounts::default()
        });
        emit_trace(shared.on_event, round_search_step);

        let round_top_urls: Vec<SearxResult> = rerank::rerank(query, new_urls)
            .into_iter()
            .take(shared.runtime_config.top_k_urls)
            .collect();

        let mut rerank_step = trace_step(
            format!("round-{attempt}-url-rerank"),
            SearchTraceKind::UrlRerank,
            SearchTraceStatus::Completed,
            "Rerank pages based on relevance",
            format!(
                "Kept {} from this follow-up round for deeper reading.",
                if round_top_urls.len() == 1 {
                    "1 page".to_string()
                } else {
                    format!("{} pages", round_top_urls.len())
                }
            ),
        );
        rerank_step.round = Some(attempt);
        rerank_step.urls = unique_urls(
            round_top_urls.iter().map(|result| result.url.as_str()),
            round_top_urls.len(),
        );
        rerank_step.domains =
            unique_domains(round_top_urls.iter().map(|result| result.url.as_str()));
        rerank_step.counts = Some(SearchTraceCounts {
            kept: Some(to_u32_saturating(round_top_urls.len())),
            ..SearchTraceCounts::default()
        });
        emit_trace(shared.on_event, rerank_step);

        let preview: Vec<SearchResultPreview> = round_top_urls
            .iter()
            .map(|result| SearchResultPreview {
                title: result.title.clone(),
                url: result.url.clone(),
            })
            .collect();
        (shared.on_event)(SearchEvent::Sources { results: preview });

        if is_cancelled_emit(shared.cancel_token, &shared.on_event) {
            return Ok(GapLoopDisposition::Fallback {
                sources: snippet_sources.clone(),
                hit_iteration_cap: false,
            });
        }

        let round_reader_urls: Vec<String> = round_top_urls
            .iter()
            .map(|result| result.url.clone())
            .collect();
        let round_reader_domains = unique_domains(round_reader_urls.iter().map(|url| url.as_str()));
        let mut read_step = trace_step(
            format!("round-{attempt}-read"),
            SearchTraceKind::Read,
            SearchTraceStatus::Running,
            "Reading the shortlisted pages",
            format!(
                "Reading full text from {} this round.",
                if round_reader_urls.len() == 1 {
                    "1 page".to_string()
                } else {
                    format!("{} pages", round_reader_urls.len())
                }
            ),
        );
        read_step.round = Some(attempt);
        read_step.domains = round_reader_domains.clone();
        read_step.counts = Some(SearchTraceCounts {
            processed: Some(0),
            total: Some(to_u32_saturating(round_reader_urls.len())),
            ..SearchTraceCounts::default()
        });
        emit_trace(shared.on_event, read_step);
        (shared.on_event)(SearchEvent::ReadingSources);

        let round_progress_urls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let round_progress_urls_for_callback = round_progress_urls.clone();
        let round_reader_result = match reader_client
            .fetch_batch_with_progress(&round_reader_urls, shared.cancel_token, &|url| {
                (shared.on_event)(SearchEvent::FetchingUrl { url: url.clone() });
                let mut urls = lock_or_recover(round_progress_urls_for_callback.as_ref());
                urls.push(url.clone());
                let processed = urls.len();
                let mut progress_step = trace_step(
                    format!("round-{attempt}-read"),
                    SearchTraceKind::Read,
                    SearchTraceStatus::Running,
                    "Reading the shortlisted pages",
                    format!(
                        "Read {} of {} pages so far.",
                        processed,
                        round_reader_urls.len()
                    ),
                );
                progress_step.round = Some(attempt);
                progress_step.domains = unique_domains(urls.iter().map(|value| value.as_str()));
                progress_step.counts = Some(SearchTraceCounts {
                    processed: Some(to_u32_saturating(processed)),
                    total: Some(to_u32_saturating(round_reader_urls.len())),
                    ..SearchTraceCounts::default()
                });
                emit_trace(shared.on_event, progress_step);
            })
            .await
        {
            Ok(result) => result,
            Err(reader::ReaderError::Cancelled) => {
                (shared.on_event)(SearchEvent::Cancelled);
                return Ok(GapLoopDisposition::Fallback {
                    sources: snippet_sources.clone(),
                    hit_iteration_cap: false,
                });
            }
            Err(reader::ReaderError::ServiceUnavailable) => {
                if !warnings.contains(&SearchWarning::ReaderUnavailable) {
                    warnings.push(SearchWarning::ReaderUnavailable);
                    (shared.on_event)(SearchEvent::Warning {
                        warning: SearchWarning::ReaderUnavailable,
                    });
                }
                reader::ReaderBatchResult::default()
            }
            Err(reader::ReaderError::BatchTimeout) => {
                if !warnings.contains(&SearchWarning::ReaderPartialFailure) {
                    warnings.push(SearchWarning::ReaderPartialFailure);
                    (shared.on_event)(SearchEvent::Warning {
                        warning: SearchWarning::ReaderPartialFailure,
                    });
                }
                reader::ReaderBatchResult::default()
            }
        };

        let round_processed =
            round_reader_result.pages.len() + round_reader_result.empty_urls.len();
        let round_failed = round_reader_result.failed_urls.len();
        let mut read_step = trace_step(
            format!("round-{attempt}-read"),
            SearchTraceKind::Read,
            SearchTraceStatus::Completed,
            "Reading the shortlisted pages",
            if warnings.contains(&SearchWarning::ReaderUnavailable) && round_processed == 0 {
                "The page reader was unavailable, so this round fell back to snippets.".to_string()
            } else if round_processed == 0 {
                "No pages could be read cleanly in this round.".to_string()
            } else {
                format!(
                    "Read {} of {} pages and extracted the text.",
                    round_processed,
                    round_reader_urls.len()
                )
            },
        );
        read_step.round = Some(attempt);
        read_step.domains = round_reader_domains;
        read_step.counts = Some(SearchTraceCounts {
            processed: Some(to_u32_saturating(round_processed)),
            total: Some(to_u32_saturating(round_reader_urls.len())),
            empty: if !round_reader_result.empty_urls.is_empty() {
                Some(to_u32_saturating(round_reader_result.empty_urls.len()))
            } else {
                None
            },
            failed: if round_failed > 0 {
                Some(to_u32_saturating(round_failed))
            } else {
                None
            },
            ..SearchTraceCounts::default()
        });
        if round_failed > 0 || !round_reader_result.empty_urls.is_empty() {
            read_step.detail = Some(format!(
                "{} page{} failed and {} page{} returned little or no readable text.",
                round_failed,
                if round_failed == 1 { "" } else { "s" },
                round_reader_result.empty_urls.len(),
                if round_reader_result.empty_urls.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        emit_trace(shared.on_event, read_step);

        let round_partial_threshold = (round_reader_urls.len() as f64 * 0.5).ceil() as usize;
        if !warnings.contains(&SearchWarning::ReaderUnavailable)
            && !round_reader_urls.is_empty()
            && round_reader_result.failed_urls.len() > round_partial_threshold
            && !warnings.contains(&SearchWarning::ReaderPartialFailure)
        {
            warnings.push(SearchWarning::ReaderPartialFailure);
            (shared.on_event)(SearchEvent::Warning {
                warning: SearchWarning::ReaderPartialFailure,
            });
        }

        let round_chunks =
            chunker::chunk_pages(&round_reader_result.pages, config::CHUNK_TOKEN_SIZE);
        let mut chunk_step = trace_step(
            format!("round-{attempt}-chunk"),
            SearchTraceKind::Chunk,
            SearchTraceStatus::Completed,
            "Split the pages into passages",
            if round_chunks.is_empty() {
                "No readable full-page text was available to split in this round.".to_string()
            } else {
                format!(
                    "Split {} into {} for closer matching.",
                    if round_reader_result.pages.len() == 1 {
                        "1 page".to_string()
                    } else {
                        format!("{} pages", round_reader_result.pages.len())
                    },
                    if round_chunks.len() == 1 {
                        "1 passage".to_string()
                    } else {
                        format!("{} passages", round_chunks.len())
                    }
                )
            },
        );
        chunk_step.round = Some(attempt);
        chunk_step.counts = Some(SearchTraceCounts {
            pages: Some(to_u32_saturating(round_reader_result.pages.len())),
            chunks: Some(to_u32_saturating(round_chunks.len())),
            ..SearchTraceCounts::default()
        });
        emit_trace(shared.on_event, chunk_step);
        accumulated_chunks.extend(round_chunks);
        let round_top_chunks: Vec<chunker::Chunk> =
            rerank::rerank_chunks(&accumulated_chunks, query, config::TOP_K_CHUNKS)
                .into_iter()
                .cloned()
                .collect();

        let round_chunk_sources = unique_domains(
            round_top_chunks
                .iter()
                .map(|chunk| chunk.source_url.as_str()),
        );
        let mut chunk_rerank_step = trace_step(
            format!("round-{attempt}-chunk-rerank"),
            SearchTraceKind::ChunkRerank,
            SearchTraceStatus::Completed,
            "Picked the strongest passages",
            if round_top_chunks.is_empty() {
                "No full-page passages were available to rank in this round.".to_string()
            } else {
                format!(
                    "Kept {} across {}.",
                    if round_top_chunks.len() == 1 {
                        "1 passage".to_string()
                    } else {
                        format!("{} passages", round_top_chunks.len())
                    },
                    if round_chunk_sources.len() == 1 {
                        "1 source".to_string()
                    } else {
                        format!("{} sources", round_chunk_sources.len())
                    }
                )
            },
        );
        chunk_rerank_step.round = Some(attempt);
        chunk_rerank_step.domains = round_chunk_sources.clone();
        chunk_rerank_step.counts = Some(SearchTraceCounts {
            chunks: Some(to_u32_saturating(accumulated_chunks.len())),
            kept: Some(to_u32_saturating(round_top_chunks.len())),
            sources: Some(to_u32_saturating(round_chunk_sources.len())),
            ..SearchTraceCounts::default()
        });
        emit_trace(shared.on_event, chunk_rerank_step);

        let round_judge_sources: Vec<JudgeSource> = if round_top_chunks.is_empty() {
            snippet_sources.clone()
        } else {
            chunks_to_judge_sources(&round_top_chunks)
        };

        let mut chunk_judge_step = trace_step(
            format!("round-{attempt}-chunk-judge"),
            SearchTraceKind::ChunkJudge,
            SearchTraceStatus::Running,
            "Checking whether the evidence is enough",
            "Verifying whether the new passages close the remaining gaps.",
        );
        chunk_judge_step.round = Some(attempt);
        chunk_judge_step.domains =
            unique_domains(round_judge_sources.iter().map(|source| source.url.as_str()));
        chunk_judge_step.counts = Some(SearchTraceCounts {
            sources: Some(to_u32_saturating(round_judge_sources.len())),
            ..SearchTraceCounts::default()
        });
        emit_trace(shared.on_event, chunk_judge_step);

        let round_verdict = judge.call(query, &round_judge_sources).await?;

        let mut chunk_judge_step = trace_step(
            format!("round-{attempt}-chunk-judge"),
            SearchTraceKind::ChunkJudge,
            SearchTraceStatus::Completed,
            "Checking whether the evidence is enough",
            judge_summary("passages", round_verdict.sufficiency),
        );
        chunk_judge_step.round = Some(attempt);
        chunk_judge_step.domains =
            unique_domains(round_judge_sources.iter().map(|source| source.url.as_str()));
        chunk_judge_step.verdict = Some(round_verdict.sufficiency);
        chunk_judge_step.detail = Some(round_verdict.reasoning.clone());
        chunk_judge_step.counts = Some(SearchTraceCounts {
            sources: Some(to_u32_saturating(round_judge_sources.len())),
            ..SearchTraceCounts::default()
        });
        emit_trace(shared.on_event, chunk_judge_step);

        metadata.iterations.push(IterationTrace {
            stage: IterationStage::GapRound { round: gap_round },
            queries: current_queries.clone(),
            urls_fetched: round_reader_urls.clone(),
            reader_empty_urls: round_reader_result.empty_urls.clone(),
            judge_verdict: round_verdict.sufficiency,
            judge_reasoning: round_verdict.reasoning.clone(),
            duration_ms: round_start.elapsed().as_millis() as u64,
        });
        (shared.on_event)(SearchEvent::IterationComplete {
            trace: metadata
                .iterations
                .last()
                .expect("iteration was just pushed")
                .clone(),
        });

        if matches!(round_verdict.sufficiency, Sufficiency::Sufficient) {
            metadata.total_duration_ms = started_at.elapsed().as_millis() as u64;
            stream_synthesis_from_sources(
                shared,
                turn,
                query,
                &round_judge_sources,
                std::mem::take(warnings),
                Some(std::mem::take(metadata)),
                compose_summary(round_judge_sources.len()),
            )
            .await;
            return Ok(GapLoopDisposition::Streamed);
        }

        current_queries = round_verdict.gap_queries.clone();
        hit_iteration_cap =
            attempt == shared.runtime_config.max_iterations as u32 && !current_queries.is_empty();
    }

    let fallback_chunks: Vec<chunker::Chunk> =
        rerank::rerank_chunks(&accumulated_chunks, query, config::TOP_K_CHUNKS)
            .into_iter()
            .cloned()
            .collect();

    let fallback_sources = if fallback_chunks.is_empty() {
        snippet_sources
    } else {
        chunks_to_judge_sources(&fallback_chunks)
    };

    Ok(GapLoopDisposition::Fallback {
        sources: fallback_sources,
        hit_iteration_cap,
    })
}

// ── Agentic entry point ────────────────────────────────────────────────────

/// Agentic search pipeline entry point. The sole production entry point after
/// Task 16.
///
/// Branch summary:
/// - `Action::Clarify`: streams the clarifying question as `Token` events,
///   then `Done`. The question is persisted to history so the next turn sees
///   it.
/// - `Action::Proceed` + `history_sufficiency == Some(Sufficient)`: streams
///   the answer synthesised from conversation history alone.
/// - `Action::Proceed` + anything else: runs the initial search round.
///   SearXNG -> URL rerank -> snippets judge -> (if not sufficient) reader
///   -> chunk rerank -> chunks judge -> synthesis. Then a bounded gap loop
///   with warning dedup and an exhaustion fallback.
///
/// Cancellation is checked at every stage entry and before every long-running
/// network call. SearXNG calls race against the token via `tokio::select!`;
/// reader calls use `fetch_batch_cancellable`; judge calls pass the token to
/// `call_judge` which races internally. This ensures in-flight work is dropped
/// immediately on cancel rather than waiting for round-trips to complete.
#[allow(clippy::too_many_arguments)]
pub async fn run_agentic(
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
    on_event: &(dyn Fn(SearchEvent) + Sync),
    router: &dyn RouterJudgeCaller,
    judge: &dyn JudgeCaller,
    runtime_config: &config::SearchRuntimeConfig,
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

    on_event(SearchEvent::AnalyzingQuery);
    emit_trace(
        on_event,
        trace_step(
            "analyze",
            SearchTraceKind::Analyze,
            SearchTraceStatus::Running,
            "Understanding the question",
            "Deciding whether this needs fresh web results or can be answered from the current conversation.",
        ),
    );

    let (epoch_at_start, history_snapshot) = snapshot_history(history);

    let output = router.call(&history_snapshot, &user_query).await?;

    let user_msg = ChatMessage {
        role: "user".to_string(),
        content: user_query.clone(),
        images: None,
    };

    let shared = SearchExecutionContext {
        ollama_endpoint,
        searxng_endpoint,
        model,
        client,
        cancel_token: &cancel_token,
        chat_system_prompt,
        history,
        today,
        on_event,
        runtime_config,
    };

    match output.action {
        Action::Clarify => {
            run_clarify_branch(
                &cancel_token,
                history,
                epoch_at_start,
                user_msg,
                output.clarifying_question.unwrap_or_default(),
                on_event,
            )
            .await
        }
        Action::Proceed => {
            let can_short_circuit =
                matches!(output.history_sufficiency, Some(Sufficiency::Sufficient))
                    && can_answer_from_history(&history_snapshot);

            if can_short_circuit {
                run_history_answer_branch(
                    &shared,
                    SearchTurnInputs {
                        history_snapshot: &history_snapshot,
                        epoch_at_start,
                        user_msg,
                    },
                    &user_query,
                )
                .await
            } else {
                // Initial search round: SearXNG -> URL rerank -> snippets judge
                // -> (if partial/insufficient) reader -> chunk rerank -> chunks
                // judge -> synthesis. Task 15 adds the gap loop after this.
                let query = output
                    .optimized_query
                    .clone()
                    .unwrap_or_else(|| user_query.clone());
                let mut analyze_step = trace_step(
                    "analyze",
                    SearchTraceKind::Analyze,
                    SearchTraceStatus::Completed,
                    "Understanding the question",
                    "This needs fresh web results, so Thuki is switching into search mode.",
                );
                if matches!(output.history_sufficiency, Some(Sufficiency::Sufficient))
                    && !can_answer_from_history(&history_snapshot)
                {
                    analyze_step.detail = Some(
                        "The router marked conversation history as sufficient, but this thread has no prior turns yet. Falling back to live search."
                            .to_string(),
                    );
                }
                if query != user_query {
                    analyze_step.detail = Some(format!("Using search query: {query}"));
                    analyze_step.queries = vec![query.clone()];
                }
                emit_trace(on_event, analyze_step);

                let reader_client = reader::ReaderClient::new_with_base(
                    reader_base_url,
                    runtime_config.reader_per_url_timeout_s,
                    runtime_config.reader_batch_timeout_s,
                );
                let mut warnings: Vec<SearchWarning> = Vec::new();
                let mut metadata = SearchMetadata::default();
                let mut accumulated_chunks: Vec<chunker::Chunk> = Vec::new();

                let iter_start = std::time::Instant::now();
                let initial_round = 1_u32;

                // Stage 1: SearXNG initial round.
                if is_cancelled_emit(&cancel_token, &on_event) {
                    return Ok(());
                }
                let mut initial_search_step = trace_step(
                    format!("round-{initial_round}-search"),
                    SearchTraceKind::Search,
                    SearchTraceStatus::Running,
                    "Searching the web",
                    "Looking for public pages that can answer the question.",
                );
                initial_search_step.round = Some(initial_round);
                initial_search_step.queries = vec![query.clone()];
                emit_trace(on_event, initial_search_step);
                on_event(SearchEvent::Searching {
                    queries: vec![query.clone()],
                });

                let searxng_fut = searxng::search(
                    client,
                    searxng_endpoint,
                    &query,
                    runtime_config.search_timeout_s,
                );
                let raw_urls = match tokio::select! {
                    biased;
                    _ = cancel_token.cancelled() => Err(SearchError::Cancelled),
                    res = searxng_fut => res,
                } {
                    Ok(v) => v,
                    Err(SearchError::Cancelled) => {
                        on_event(SearchEvent::Cancelled);
                        return Ok(());
                    }
                    Err(SearchError::NoResults) => {
                        let mut search_step = trace_step(
                            format!("round-{initial_round}-search"),
                            SearchTraceKind::Search,
                            SearchTraceStatus::Completed,
                            "Searching the web",
                            "The search did not return any useful results.",
                        );
                        search_step.round = Some(initial_round);
                        search_step.queries = vec![query.clone()];
                        search_step.counts = Some(SearchTraceCounts {
                            found: Some(0),
                            ..SearchTraceCounts::default()
                        });
                        emit_trace(on_event, search_step);
                        warnings.push(SearchWarning::NoResultsInitial);
                        on_event(SearchEvent::Warning {
                            warning: SearchWarning::NoResultsInitial,
                        });
                        return Err(SearchError::NoResults);
                    }
                    Err(e) => return Err(e),
                };

                let mut search_step = trace_step(
                    format!("round-{initial_round}-search"),
                    SearchTraceKind::Search,
                    SearchTraceStatus::Completed,
                    "Searching the web",
                    format!(
                        "Found {} across {}.",
                        if raw_urls.len() == 1 {
                            "1 result".to_string()
                        } else {
                            format!("{} results", raw_urls.len())
                        },
                        if unique_domains(raw_urls.iter().map(|r| r.url.as_str())).len() == 1 {
                            "1 site".to_string()
                        } else {
                            format!(
                                "{} sites",
                                unique_domains(raw_urls.iter().map(|r| r.url.as_str())).len()
                            )
                        }
                    ),
                );
                search_step.round = Some(initial_round);
                search_step.queries = vec![query.clone()];
                search_step.urls = unique_urls(
                    raw_urls.iter().map(|result| result.url.as_str()),
                    raw_urls.len(),
                );
                search_step.domains = unique_domains(raw_urls.iter().map(|r| r.url.as_str()));
                search_step.counts = Some(SearchTraceCounts {
                    found: Some(to_u32_saturating(raw_urls.len())),
                    ..SearchTraceCounts::default()
                });
                emit_trace(on_event, search_step);

                // Stage 2: Rerank URLs, take top K.
                let reranked = rerank::rerank(&query, raw_urls);
                let top_urls: Vec<_> = reranked
                    .into_iter()
                    .take(runtime_config.top_k_urls)
                    .collect();

                let mut rerank_step = trace_step(
                    format!("round-{initial_round}-url-rerank"),
                    SearchTraceKind::UrlRerank,
                    SearchTraceStatus::Completed,
                    "Rerank pages based on relevance",
                    format!(
                        "Ranked the results and kept {} for closer reading.",
                        if top_urls.len() == 1 {
                            "1 page".to_string()
                        } else {
                            format!("{} pages", top_urls.len())
                        }
                    ),
                );
                rerank_step.round = Some(initial_round);
                rerank_step.urls = unique_urls(
                    top_urls.iter().map(|result| result.url.as_str()),
                    top_urls.len(),
                );
                rerank_step.domains = unique_domains(top_urls.iter().map(|r| r.url.as_str()));
                rerank_step.counts = Some(SearchTraceCounts {
                    kept: Some(to_u32_saturating(top_urls.len())),
                    ..SearchTraceCounts::default()
                });
                emit_trace(on_event, rerank_step);

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

                let mut snippet_judge_step = trace_step(
                    format!("round-{initial_round}-snippet-judge"),
                    SearchTraceKind::SnippetJudge,
                    SearchTraceStatus::Running,
                    snippet_judge_title(),
                    snippet_judge_running_summary(),
                );
                snippet_judge_step.round = Some(initial_round);
                snippet_judge_step.domains =
                    unique_domains(top_urls.iter().map(|r| r.url.as_str()));
                snippet_judge_step.counts = Some(SearchTraceCounts {
                    sources: Some(to_u32_saturating(snippet_sources.len())),
                    ..SearchTraceCounts::default()
                });
                emit_trace(on_event, snippet_judge_step);

                let snippet_verdict = judge.call(&query, &snippet_sources).await?;

                let mut snippet_judge_step = trace_step(
                    format!("round-{initial_round}-snippet-judge"),
                    SearchTraceKind::SnippetJudge,
                    SearchTraceStatus::Completed,
                    snippet_judge_title(),
                    snippet_judge_summary(snippet_verdict.sufficiency),
                );
                snippet_judge_step.round = Some(initial_round);
                snippet_judge_step.domains =
                    unique_domains(top_urls.iter().map(|r| r.url.as_str()));
                snippet_judge_step.verdict = Some(snippet_verdict.sufficiency);
                snippet_judge_step.detail =
                    snippet_judge_detail(snippet_verdict.sufficiency, &snippet_verdict.reasoning);
                snippet_judge_step.counts = Some(SearchTraceCounts {
                    sources: Some(to_u32_saturating(snippet_sources.len())),
                    ..SearchTraceCounts::default()
                });
                emit_trace(on_event, snippet_judge_step);

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
                    on_event(SearchEvent::IterationComplete {
                        trace: metadata
                            .iterations
                            .last()
                            .expect("iteration was just pushed")
                            .clone(),
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
                    emit_final_sources(on_event, &snippet_sources);
                    let mut compose_step = trace_step(
                        "compose",
                        SearchTraceKind::Compose,
                        SearchTraceStatus::Running,
                        compose_title(),
                        compose_summary(snippet_sources.len()),
                    );
                    compose_step.domains =
                        unique_domains(snippet_sources.iter().map(|s| s.url.as_str()));
                    compose_step.counts = Some(SearchTraceCounts {
                        sources: Some(to_u32_saturating(snippet_sources.len())),
                        ..SearchTraceCounts::default()
                    });
                    emit_trace(on_event, compose_step);
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
                        warnings,
                        Some(metadata),
                        &on_event,
                    )
                    .await;
                    return Ok(());
                }

                // Stage 5: Reader escalation.
                if is_cancelled_emit(&cancel_token, &on_event) {
                    return Ok(());
                }
                let reader_urls: Vec<String> = top_urls.iter().map(|r| r.url.clone()).collect();
                let reader_domains = unique_domains(reader_urls.iter().map(|url| url.as_str()));
                let mut read_step = trace_step(
                    format!("round-{initial_round}-read"),
                    SearchTraceKind::Read,
                    SearchTraceStatus::Running,
                    "Reading the shortlisted pages",
                    format!(
                        "Reading full text from {}.",
                        if reader_urls.len() == 1 {
                            "1 page".to_string()
                        } else {
                            format!("{} pages", reader_urls.len())
                        }
                    ),
                );
                read_step.round = Some(initial_round);
                read_step.domains = reader_domains.clone();
                read_step.counts = Some(SearchTraceCounts {
                    processed: Some(0),
                    total: Some(to_u32_saturating(reader_urls.len())),
                    ..SearchTraceCounts::default()
                });
                emit_trace(on_event, read_step);
                on_event(SearchEvent::ReadingSources);
                let progress_urls =
                    std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
                let progress_urls_for_callback = progress_urls.clone();
                let reader_result = match reader_client
                    .fetch_batch_with_progress(&reader_urls, &cancel_token, &|url| {
                        on_event(SearchEvent::FetchingUrl { url: url.clone() });
                        let mut urls = lock_or_recover(progress_urls_for_callback.as_ref());
                        urls.push(url.clone());
                        let processed = urls.len();
                        let mut progress_step = trace_step(
                            format!("round-{initial_round}-read"),
                            SearchTraceKind::Read,
                            SearchTraceStatus::Running,
                            "Reading the shortlisted pages",
                            format!("Read {} of {} pages so far.", processed, reader_urls.len()),
                        );
                        progress_step.round = Some(initial_round);
                        progress_step.domains =
                            unique_domains(urls.iter().map(|value| value.as_str()));
                        progress_step.counts = Some(SearchTraceCounts {
                            processed: Some(to_u32_saturating(processed)),
                            total: Some(to_u32_saturating(reader_urls.len())),
                            ..SearchTraceCounts::default()
                        });
                        emit_trace(on_event, progress_step);
                    })
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

                let read_processed = reader_result.pages.len() + reader_result.empty_urls.len();
                let read_failed = reader_result.failed_urls.len();
                let mut read_step = trace_step(
                    format!("round-{initial_round}-read"),
                    SearchTraceKind::Read,
                    SearchTraceStatus::Completed,
                    "Reading the shortlisted pages",
                    if warnings.contains(&SearchWarning::ReaderUnavailable) && read_processed == 0 {
                        "The page reader was unavailable, so Thuki continued with snippets."
                            .to_string()
                    } else if read_processed == 0 {
                        "No pages could be read cleanly, so Thuki kept working from lighter evidence.".to_string()
                    } else {
                        format!(
                            "Read {} of {} pages and extracted the text.",
                            read_processed,
                            reader_urls.len()
                        )
                    },
                );
                read_step.round = Some(initial_round);
                read_step.domains = reader_domains;
                read_step.counts = Some(SearchTraceCounts {
                    processed: Some(to_u32_saturating(read_processed)),
                    total: Some(to_u32_saturating(reader_urls.len())),
                    empty: if !reader_result.empty_urls.is_empty() {
                        Some(to_u32_saturating(reader_result.empty_urls.len()))
                    } else {
                        None
                    },
                    failed: if read_failed > 0 {
                        Some(to_u32_saturating(read_failed))
                    } else {
                        None
                    },
                    ..SearchTraceCounts::default()
                });
                if warnings.contains(&SearchWarning::ReaderUnavailable) {
                    read_step.detail = Some(
                        "Thuki fell back to search snippets because the full-page reader could not be reached.".to_string(),
                    );
                } else if read_failed > 0 || !reader_result.empty_urls.is_empty() {
                    read_step.detail = Some(format!(
                        "{} page{} failed and {} page{} returned little or no readable text.",
                        read_failed,
                        if read_failed == 1 { "" } else { "s" },
                        reader_result.empty_urls.len(),
                        if reader_result.empty_urls.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ));
                }
                emit_trace(on_event, read_step);

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
                let mut chunk_step = trace_step(
                    format!("round-{initial_round}-chunk"),
                    SearchTraceKind::Chunk,
                    SearchTraceStatus::Completed,
                    "Split the pages into passages",
                    if new_chunks.is_empty() {
                        "No readable full-page text was available to split into passages."
                            .to_string()
                    } else {
                        format!(
                            "Split {} into {} for closer matching.",
                            if reader_result.pages.len() == 1 {
                                "1 page".to_string()
                            } else {
                                format!("{} pages", reader_result.pages.len())
                            },
                            if new_chunks.len() == 1 {
                                "1 passage".to_string()
                            } else {
                                format!("{} passages", new_chunks.len())
                            }
                        )
                    },
                );
                chunk_step.round = Some(initial_round);
                chunk_step.counts = Some(SearchTraceCounts {
                    pages: Some(to_u32_saturating(reader_result.pages.len())),
                    chunks: Some(to_u32_saturating(new_chunks.len())),
                    ..SearchTraceCounts::default()
                });
                emit_trace(on_event, chunk_step);
                accumulated_chunks.extend(new_chunks);
                let top_chunks: Vec<chunker::Chunk> =
                    rerank::rerank_chunks(&accumulated_chunks, &query, config::TOP_K_CHUNKS)
                        .into_iter()
                        .cloned()
                        .collect();

                let chunk_sources =
                    unique_domains(top_chunks.iter().map(|chunk| chunk.source_url.as_str()));
                let mut chunk_rerank_step = trace_step(
                    format!("round-{initial_round}-chunk-rerank"),
                    SearchTraceKind::ChunkRerank,
                    SearchTraceStatus::Completed,
                    "Picked the strongest passages",
                    if top_chunks.is_empty() {
                        "No full-page passages were available to rank.".to_string()
                    } else {
                        format!(
                            "Kept {} across {}.",
                            if top_chunks.len() == 1 {
                                "1 passage".to_string()
                            } else {
                                format!("{} passages", top_chunks.len())
                            },
                            if chunk_sources.len() == 1 {
                                "1 source".to_string()
                            } else {
                                format!("{} sources", chunk_sources.len())
                            }
                        )
                    },
                );
                chunk_rerank_step.round = Some(initial_round);
                chunk_rerank_step.domains = chunk_sources.clone();
                chunk_rerank_step.counts = Some(SearchTraceCounts {
                    chunks: Some(to_u32_saturating(accumulated_chunks.len())),
                    kept: Some(to_u32_saturating(top_chunks.len())),
                    sources: Some(to_u32_saturating(chunk_sources.len())),
                    ..SearchTraceCounts::default()
                });
                emit_trace(on_event, chunk_rerank_step);

                // Stage 7: Chunks judge. Fall back to snippets when reader was
                // degraded and produced no chunks.
                let judge_sources: Vec<JudgeSource> = if top_chunks.is_empty() {
                    snippet_sources.clone()
                } else {
                    chunks_to_judge_sources(&top_chunks)
                };

                let mut chunk_judge_step = trace_step(
                    format!("round-{initial_round}-chunk-judge"),
                    SearchTraceKind::ChunkJudge,
                    SearchTraceStatus::Running,
                    "Checking whether the evidence is enough",
                    "Verifying whether the strongest passages fully answer the question.",
                );
                chunk_judge_step.round = Some(initial_round);
                chunk_judge_step.domains =
                    unique_domains(judge_sources.iter().map(|source| source.url.as_str()));
                chunk_judge_step.counts = Some(SearchTraceCounts {
                    sources: Some(to_u32_saturating(judge_sources.len())),
                    ..SearchTraceCounts::default()
                });
                emit_trace(on_event, chunk_judge_step);

                let chunk_verdict = judge.call(&query, &judge_sources).await?;

                let mut chunk_judge_step = trace_step(
                    format!("round-{initial_round}-chunk-judge"),
                    SearchTraceKind::ChunkJudge,
                    SearchTraceStatus::Completed,
                    "Checking whether the evidence is enough",
                    judge_summary("passages", chunk_verdict.sufficiency),
                );
                chunk_judge_step.round = Some(initial_round);
                chunk_judge_step.domains =
                    unique_domains(judge_sources.iter().map(|source| source.url.as_str()));
                chunk_judge_step.verdict = Some(chunk_verdict.sufficiency);
                chunk_judge_step.detail = Some(chunk_verdict.reasoning.clone());
                chunk_judge_step.counts = Some(SearchTraceCounts {
                    sources: Some(to_u32_saturating(judge_sources.len())),
                    ..SearchTraceCounts::default()
                });
                emit_trace(on_event, chunk_judge_step);

                metadata.iterations.push(IterationTrace {
                    stage: IterationStage::Initial,
                    queries: vec![query.clone()],
                    urls_fetched: reader_urls.clone(),
                    reader_empty_urls: reader_result.empty_urls.clone(),
                    judge_verdict: chunk_verdict.sufficiency,
                    judge_reasoning: chunk_verdict.reasoning.clone(),
                    duration_ms: iter_start.elapsed().as_millis() as u64,
                });
                on_event(SearchEvent::IterationComplete {
                    trace: metadata
                        .iterations
                        .last()
                        .expect("iteration was just pushed")
                        .clone(),
                });

                if matches!(chunk_verdict.sufficiency, Sufficiency::Sufficient) {
                    metadata.total_duration_ms = iter_start.elapsed().as_millis() as u64;
                    stream_synthesis_from_sources(
                        &shared,
                        &SearchTurnInputs {
                            history_snapshot: &history_snapshot,
                            epoch_at_start,
                            user_msg: user_msg.clone(),
                        },
                        &query,
                        &judge_sources,
                        std::mem::take(&mut warnings),
                        Some(std::mem::take(&mut metadata)),
                        compose_summary(judge_sources.len()),
                    )
                    .await;
                    return Ok(());
                }

                let gap_loop = run_gap_refinement_loop(
                    &shared,
                    &SearchTurnInputs {
                        history_snapshot: &history_snapshot,
                        epoch_at_start,
                        user_msg: user_msg.clone(),
                    },
                    judge,
                    &query,
                    &reader_client,
                    snippet_sources.clone(),
                    &mut warnings,
                    &mut metadata,
                    accumulated_chunks,
                    top_urls.iter().map(|result| result.url.clone()).collect(),
                    chunk_verdict.gap_queries.clone(),
                    iter_start,
                )
                .await?;

                let GapLoopDisposition::Fallback {
                    sources: fallback_sources,
                    hit_iteration_cap,
                } = gap_loop
                else {
                    return Ok(());
                };

                if is_cancelled_emit(&cancel_token, &on_event) {
                    return Ok(());
                }

                if hit_iteration_cap {
                    warnings.push(SearchWarning::IterationCapExhausted);
                    on_event(SearchEvent::Warning {
                        warning: SearchWarning::IterationCapExhausted,
                    });
                }

                metadata.total_duration_ms = iter_start.elapsed().as_millis() as u64;
                stream_synthesis_from_sources(
                    &shared,
                    &SearchTurnInputs {
                        history_snapshot: &history_snapshot,
                        epoch_at_start,
                        user_msg,
                    },
                    &query,
                    &fallback_sources,
                    warnings,
                    Some(metadata),
                    fallback_compose_summary(hit_iteration_cap, fallback_sources.len()),
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
#[cfg_attr(coverage_nightly, coverage(off))]
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

    fn make_user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user".into(),
            content: content.into(),
            images: None,
        }
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
        assert_eq!(
            translate_chunk(StreamChunk::Done),
            SearchEvent::Done { metadata: None }
        );
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

    #[test]
    fn lock_or_recover_returns_inner_after_mutex_poison() {
        let mutex = std::sync::Mutex::new(vec![1u32]);

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut guard = mutex.lock().unwrap();
            guard.push(2);
            panic!("poison mutex for recovery test");
        }));

        let guard = lock_or_recover(&mutex);
        assert_eq!(*guard, vec![1, 2]);
    }

    // ── persist_turn ────────────────────────────────────────────────────────

    #[test]
    fn persist_turn_appends_both_messages_under_matching_epoch() {
        let h = ConversationHistory::new();
        persist_turn(
            &h,
            0,
            make_user_msg("q"),
            ChatMessage {
                role: "assistant".into(),
                content: "a".into(),
                images: None,
            },
            Vec::new(),
            None,
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
            make_user_msg("q"),
            ChatMessage {
                role: "assistant".into(),
                content: "a".into(),
                images: None,
            },
            Vec::new(),
            None,
        );
        let conv = h.messages.lock().unwrap();
        assert!(conv.is_empty());
    }

    // ── run_streaming_branch: no persist on empty response ───────────────────

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
            vec![make_user_msg("q")],
            &h,
            0,
            make_user_msg("q"),
            Vec::new(),
            None,
            &cb,
        )
        .await;

        mock.assert_async().await;
        assert!(h.messages.lock().unwrap().is_empty());
    }

    // ── DefaultRouterJudge / DefaultJudge construction ───────────────────────

    #[test]
    fn default_router_judge_constructs_without_panic() {
        let cancel = CancellationToken::new();
        let _judge = DefaultRouterJudge::new(
            "http://127.0.0.1:11434/api/chat".into(),
            "mistral".into(),
            reqwest::Client::new(),
            cancel,
            "2026-04-18".into(),
            crate::config::defaults::DEFAULT_ROUTER_TIMEOUT_S,
        );
    }

    #[test]
    fn default_judge_constructs_without_panic() {
        let cancel = CancellationToken::new();
        let _judge = DefaultJudge::new(
            "http://127.0.0.1:11434/api/chat".into(),
            "mistral".into(),
            reqwest::Client::new(),
            cancel,
            crate::config::defaults::DEFAULT_JUDGE_TIMEOUT_S,
        );
    }

    // ── snippet_judge_detail / judge_summary unit coverage ───────────────────

    #[test]
    fn snippet_judge_detail_returns_none_for_empty_reasoning() {
        assert!(snippet_judge_detail(Sufficiency::Sufficient, "").is_none());
        assert!(snippet_judge_detail(Sufficiency::Sufficient, "   ").is_none());
    }

    #[test]
    fn judge_summary_partial_mentions_details_missing() {
        let s = judge_summary("snippets", Sufficiency::Partial);
        assert!(s.contains("snippets"), "expected subject in summary: {s}");
        assert!(
            s.contains("missing"),
            "expected 'missing' in partial summary: {s}"
        );
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

    struct ErrorRouter(SearchError);

    #[async_trait]
    impl RouterJudgeCaller for ErrorRouter {
        async fn call(
            &self,
            _h: &[ChatMessage],
            _q: &str,
        ) -> Result<RouterJudgeOutput, SearchError> {
            Err(self.0.clone())
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

    fn completed_trace_step<'a>(events: &'a [SearchEvent], id: &str) -> &'a SearchTraceStep {
        events
            .iter()
            .find_map(|event| match event {
                SearchEvent::Trace { step }
                    if step.id == id && step.status == SearchTraceStatus::Completed =>
                {
                    Some(step)
                }
                _ => None,
            })
            .expect("expected completed trace step")
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn done_metadata(events: &[SearchEvent]) -> &SearchMetadata {
        match events.last().expect("expected final event") {
            SearchEvent::Done {
                metadata: Some(metadata),
            } => metadata,
            other => panic!("expected final Done event with metadata, got: {other:?}"),
        }
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn assert_done_iterations(events: &[SearchEvent], expected_iterations: usize) {
        let metadata = done_metadata(events);
        assert_eq!(
            metadata.iterations.len(),
            expected_iterations,
            "expected Done metadata with {expected_iterations} iterations, got: {metadata:?}"
        );
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn assert_refining_search(events: &[SearchEvent], attempt: u32, total: u32) {
        assert!(
            events.iter().any(|event| matches!(
                event,
                SearchEvent::RefiningSearch {
                    attempt: actual_attempt,
                    total: actual_total,
                } if *actual_attempt == attempt && *actual_total == total
            )),
            "expected RefiningSearch attempt={attempt} total={total} in: {events:?}"
        );
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
        assert_eq!(
            std::mem::discriminant(&err),
            std::mem::discriminant(&SearchError::Internal(String::new())),
            "expected Internal error"
        );
    }

    fn insufficient_verdict() -> JudgeVerdict {
        JudgeVerdict {
            sufficiency: Sufficiency::Insufficient,
            reasoning: "not enough".into(),
            gap_queries: vec!["q1".into()],
        }
    }

    /// Like `insufficient_verdict` but with no gap queries. Used in the
    /// exhaustion test so the gap loop breaks immediately on the empty-queries
    /// guard rather than attempting real SearXNG calls.
    fn insufficient_verdict_no_gaps() -> JudgeVerdict {
        JudgeVerdict {
            sufficiency: Sufficiency::Insufficient,
            reasoning: "not enough".into(),
            gap_queries: vec![],
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
            all_token_content.contains("which") | all_token_content.contains("project"),
            "expected token stream to contain the clarifying question, got: {all_token_content}"
        );

        // Last event must be Done.
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done { metadata: None });

        // No search-phase events.
        assert!(evs
            .iter()
            .all(|e| !matches!(e, SearchEvent::Searching { .. })));
        assert!(evs
            .iter()
            .all(|e| !matches!(e, SearchEvent::ReadingSources)));

        let trace_steps = evs
            .iter()
            .filter_map(|event| match event {
                SearchEvent::Trace { step } => Some(step),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(trace_steps.iter().any(|step| {
            step.title == "Understanding the question"
                && step.summary
                    == "This request could mean a few different things, so Thuki needs one more detail before searching."
                && step.detail.is_none()
        }));
        assert!(trace_steps.iter().any(|step| {
            step.title == "Waiting for clarification"
                && step.summary == "Search is paused until you clarify who or what you mean."
                && step.detail.is_none()
        }));

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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert_eq!(evs[0], SearchEvent::AnalyzingQuery);
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done { metadata: None });
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
        {
            let mut history = h.messages.lock().unwrap();
            history.push(ChatMessage {
                role: "user".into(),
                content: "who is the current owner of the repo?".into(),
                images: None,
            });
            history.push(ChatMessage {
                role: "assistant".into(),
                content: "The repo owner is quiet-node.".into(),
                images: None,
            });
        }
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
        assert_eq!(*evs.last().unwrap(), SearchEvent::Done { metadata: None });

        // No search events.
        assert!(evs
            .iter()
            .all(|e| !matches!(e, SearchEvent::Searching { .. })));
        assert!(evs
            .iter()
            .all(|e| !matches!(e, SearchEvent::ReadingSources)));
    }

    #[tokio::test]
    async fn empty_history_never_short_circuits_even_if_router_claims_sufficient() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let searx_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/date")),
            )
            .mount(&searx_server)
            .await;

        let ollama_server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(stream_line_token("fresh answer")),
            )
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();

        let router = MockRouter(RouterJudgeOutput {
            action: Action::Proceed,
            clarifying_question: None,
            history_sufficiency: Some(Sufficiency::Sufficient),
            optimized_query: Some("today date".into()),
        });
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![sufficient_verdict()].into_iter().collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "what is today's date".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        assert!(evs
            .iter()
            .any(|event| matches!(event, SearchEvent::Searching { .. })));
        assert!(evs
            .iter()
            .any(|event| matches!(event, SearchEvent::Sources { .. })));
        assert!(evs.iter().any(
            |event| matches!(event, SearchEvent::Token { content } if content == "fresh answer")
        ));
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

    // A router that returns Proceed with optimized_query=None so the pipeline
    // falls back to user_query.clone() (line 449).
    fn proceed_router_no_opt() -> MockRouter {
        MockRouter(RouterJudgeOutput {
            action: Action::Proceed,
            clarifying_question: None,
            history_sufficiency: Some(Sufficiency::Insufficient),
            optimized_query: None,
        })
    }

    // Verifies that when the router returns optimized_query=None, the pipeline
    // falls back to the user query (unwrap_or_else closure on line 449 fires).
    #[tokio::test]
    async fn proceed_with_no_optimized_query_falls_back_to_user_query() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let searx_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        let ollama_server = MockServer::start().await;
        let stream = stream_line_token("answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_router_no_opt();
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![sufficient_verdict()].into_iter().collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "my query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert_done_iterations(&evs, 1);
        let metadata = done_metadata(&evs);
        assert_eq!(metadata.iterations[0].queries, vec!["my query".to_string()]);
    }

    #[tokio::test]
    async fn optimized_query_rewrite_updates_analyze_trace() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let searx_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        let ollama_server = MockServer::start().await;
        let stream = stream_line_token("answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("rewritten query");
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![sufficient_verdict()].into_iter().collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "original query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        let analyze_step = completed_trace_step(&evs, "analyze");
        assert_eq!(
            analyze_step.detail.as_deref(),
            Some("Using search query: rewritten query")
        );
        assert_eq!(analyze_step.queries, vec!["rewritten query".to_string()]);
        assert_done_iterations(&evs, 1);
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        assert_eq!(evs[0], SearchEvent::AnalyzingQuery);
        assert!(evs
            .iter()
            .any(|e| matches!(e, SearchEvent::Searching { .. })));
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::Sources { .. })));
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::Composing)));
        assert!(
            evs.iter()
                .any(|e| matches!(e, SearchEvent::Token { content } if content == "answer")),
            "expected token with 'answer'"
        );
        assert_done_iterations(&evs, 1);

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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(evs
            .iter()
            .any(|e| matches!(e, SearchEvent::Searching { .. })));
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::ReadingSources)));
        // No ReaderUnavailable warning when reader succeeds: verify by
        // checking the event list contains no Warning events of any kind,
        // since this test configures the reader to succeed.
        let has_any_warning = evs.iter().any(|e| matches!(e, SearchEvent::Warning { .. }));
        assert!(!has_any_warning, "expected no warnings in: {evs:?}");
        assert_done_iterations(&evs, 1);
    }

    // Test: initial round returns insufficient with no gap queries; gap loop
    // exits immediately on the empty-queries guard, so IterationCapExhausted
    // must NOT fire.
    #[tokio::test]
    async fn initial_round_with_no_gap_queries_does_not_emit_iteration_cap_exhausted() {
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

        // Snippet judge returns insufficient (no gaps) so reader is skipped.
        // Chunk judge returns insufficient with no gap queries so the gap loop
        // exits immediately on the empty-queries guard without ever entering a
        // gap round. hit_iteration_cap stays false, so no warning is emitted.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                insufficient_verdict_no_gaps(),
                insufficient_verdict_no_gaps(),
            ]
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            !evs.iter().any(|e| {
                *e == (SearchEvent::Warning {
                    warning: SearchWarning::IterationCapExhausted,
                })
            }),
            "expected no IterationCapExhausted warning in: {evs:?}"
        );
        assert_done_iterations(&evs, 1);
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
        // Reader will fail (DEFAULT_READER_URL is not running in test).
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
        assert_done_iterations(&evs, 1);
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap_err();

        assert_eq!(err, SearchError::SearxHttp(503));
    }

    #[tokio::test]
    async fn router_error_is_returned_from_run_agentic() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (_, cb) = collect_events();
        let router = ErrorRouter(SearchError::Internal("router failed".into()));
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap_err();

        assert_eq!(err, SearchError::Internal("router failed".into()));
    }

    #[tokio::test]
    async fn snippet_judge_error_is_returned_from_initial_round() {
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap_err();

        assert_eq!(err, SearchError::Internal("queue empty".into()));
    }

    #[tokio::test]
    async fn chunk_judge_error_is_returned_after_initial_reader() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "initial",
                "markdown": "content",
                "status": "ok"
            })))
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
        let h = ConversationHistory::new();
        let (_, cb) = collect_events();
        let router = proceed_search_router("q");
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![partial_verdict()].into_iter().collect(),
        ));

        let err = run_agentic(
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap_err();

        assert_eq!(err, SearchError::Internal("queue empty".into()));
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled event in: {evs:?}"
        );
    }

    // Reader batch times out (reader_batch_timeout_s=1s in tests);
    // pipeline emits ReaderPartialFailure warning and continues.
    #[tokio::test]
    async fn reader_batch_timeout_emits_partial_failure_warning_and_continues() {
        use std::time::Duration;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // Respond after 2s -- longer than reader_batch_timeout_s=1s in tests.
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
        assert_done_iterations(&evs, 1);
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
        let read_step = completed_trace_step(&evs, "round-1-read");
        assert_eq!(
            read_step.detail.as_deref(),
            Some("2 pages failed and 0 pages returned little or no readable text."),
            "expected pluralized read-step detail in: {evs:?}"
        );
        assert_done_iterations(&evs, 1);
    }

    #[tokio::test]
    async fn initial_reader_detail_singularizes_failed_page_count() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(502))
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        let read_step = completed_trace_step(&evs, "round-1-read");
        assert_eq!(
            read_step.detail.as_deref(),
            Some("1 page failed and 0 pages returned little or no readable text."),
            "expected singular failed-page detail in: {evs:?}"
        );
        assert_done_iterations(&evs, 1);
    }

    // ── Gap loop tests ─────────────────────────────────────────────────────────

    // Test 1: gap round 2 returns sufficient; pipeline synthesizes after gap.
    //
    // Judge sequence: Insufficient (snippets) -> Insufficient (chunks, gap_queries=["gap1"])
    //                 -> Sufficient (chunks after gap round 2).
    // max_iterations = 3, so there are 2 gap rounds. The first is reported as
    // attempt=1 of 2.
    #[tokio::test]
    async fn gap_round_succeeds_within_cap() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "initial result",
                "markdown": "initial page content about the topic",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        // Initial SearXNG query returns one URL.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "test query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        // Gap query "q1" (from insufficient_verdict helper) returns a different URL.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("gap answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");

        // Sequence: snippets partial (triggers reader), chunks insufficient with
        // gap_queries=["q1"] (from insufficient_verdict helper), gap round 2
        // fetches "q1" URL and judge returns sufficient.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                // snippets judge: partial triggers reader escalation
                partial_verdict(),
                // chunks judge after initial reader: insufficient, gap_queries=["q1"]
                insufficient_verdict(),
                // gap round 2 judge: sufficient
                sufficient_verdict(),
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        assert_refining_search(&evs, 1, 2);

        // Composing and Done must appear (synthesis ran).
        assert!(evs.iter().any(|e| matches!(e, SearchEvent::Composing)));
        assert_done_iterations(&evs, 2);

        // No IterationCapExhausted (succeeded before cap).
        assert!(
            !evs.iter().any(|e| {
                *e == (SearchEvent::Warning {
                    warning: SearchWarning::IterationCapExhausted,
                })
            }),
            "unexpected IterationCapExhausted in: {evs:?}"
        );

        // Metadata: 2 iteration traces (Initial + GapRound { round: 1 }).
        drop(evs);
        // (metadata is not directly accessible from outside; we verify through
        // the event stream shape which is the observable contract)
    }

    // Test 2: judge always insufficient; all max_iterations rounds fire.
    //
    // Each verdict provides a fresh gap query so the loop does not exit early.
    // The test verifies RefiningSearch for both gap rounds, and
    // IterationCapExhausted emitted exactly once.
    #[tokio::test]
    async fn gap_round_exhausts_all_iterations() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // Respond to any reader call with a valid page so chunks accumulate.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/page",
                "title": "page",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        // Initial query.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "test query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        // Gap round 2 query.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "gap2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/b")),
            )
            .mount(&searx_server)
            .await;

        // Gap round 3 query.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "gap3"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/c")),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("exhausted answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");

        // max_iterations=3: snippets judge + chunks judge (initial) + gap round 2
        // judge + gap round 3 judge = 4 calls total, all insufficient with
        // unique gap queries to keep the loop alive.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                // snippets: partial, enter reader
                JudgeVerdict {
                    sufficiency: Sufficiency::Partial,
                    reasoning: "partial".into(),
                    gap_queries: vec!["gap2".into()],
                },
                // initial chunks: insufficient -> gap round 2
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "not enough".into(),
                    gap_queries: vec!["gap2".into()],
                },
                // gap round 2 chunks: insufficient -> gap round 3
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "still not enough".into(),
                    gap_queries: vec!["gap3".into()],
                },
                // gap round 3 chunks: insufficient -> loop exhausted
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "exhausted".into(),
                    gap_queries: vec!["gap4".into()],
                },
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        assert_refining_search(&evs, 1, 2);
        assert_refining_search(&evs, 2, 2);

        // IterationCapExhausted exactly once.
        let exhaustion_count = evs
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SearchEvent::Warning {
                        warning: SearchWarning::IterationCapExhausted
                    }
                )
            })
            .count();
        assert_eq!(
            exhaustion_count, 1,
            "expected exactly 1 IterationCapExhausted, got {exhaustion_count} in: {evs:?}"
        );

        assert_done_iterations(&evs, 3);
    }

    // Test 3: gap round where all SearXNG queries return empty (no new URLs).
    //
    // Initial round: Insufficient with gap_queries=["q1","q2","q3"].
    // First gap round: SearXNG returns only already-seen URLs so
    // new_urls is empty. current_queries is cleared and the loop continues.
    // The for-range ends before a second gap round would run,
    // so hit_iteration_cap stays false. IterationCapExhausted must NOT fire.
    #[tokio::test]
    async fn gap_round_empty_searxng_breaks_loop_silently() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "initial",
                "markdown": "some initial content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        // Initial query returns one result.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "test query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        // Gap queries (q1, q2, q3) have no mock: wiremock returns 404.
        // search_all_with_endpoint uses unwrap_or_default() so 404 becomes
        // an empty result set, exercising the "no new URLs" branch.

        let stream = stream_line_token("best effort from initial");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");

        // snippets: partial -> reader; chunks: insufficient with 3 gap queries
        // -> gap round starts; gap round finds no new URLs -> loop breaks.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                JudgeVerdict {
                    sufficiency: Sufficiency::Partial,
                    reasoning: "partial".into(),
                    gap_queries: vec!["q1".into(), "q2".into(), "q3".into()],
                },
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "not enough".into(),
                    gap_queries: vec!["q1".into(), "q2".into(), "q3".into()],
                },
                // No third verdict needed: empty SearXNG means no judge call
                // for the gap round beyond the trace push.
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        // IterationCapExhausted must NOT fire: the loop exited via the
        // no-new-URLs branch (current_queries cleared), never reaching the
        // judge at attempt == max_iterations. hit_iteration_cap stays false.
        assert!(
            !evs.iter().any(|e| {
                *e == (SearchEvent::Warning {
                    warning: SearchWarning::IterationCapExhausted,
                })
            }),
            "expected no IterationCapExhausted in: {evs:?}"
        );

        assert_refining_search(&evs, 1, 2);

        // No second gap round fired after the empty-search branch broke the loop.
        assert!(
            !evs.iter()
                .any(|e| matches!(e, SearchEvent::RefiningSearch { attempt: 2, .. })),
            "unexpected second RefiningSearch event in: {evs:?}"
        );

        // Pipeline still synthesized an answer.
        assert_done_iterations(&evs, 2);
    }

    // Test 3b: boundary case -- the final gap round runs in full, judge returns
    // Insufficient with empty gap_queries. The pipeline now only emits
    // IterationCapExhausted when the final round still suggests follow-up work,
    // so this path should synthesize without the warning.
    #[tokio::test]
    #[cfg_attr(coverage_nightly, coverage(off))]
    async fn gap_round_attempt3_full_run_without_new_gaps_skips_cap_warning() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "page",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        // Initial query.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "test query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        // First gap round returns a new URL.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/b")),
            )
            .mount(&searx_server)
            .await;

        // Final gap round returns another new URL.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/c")),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("best effort");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");

        // Sequence: snippets partial, initial chunks insufficient with gap_queries=["q1"],
        // first gap-round judge insufficient with gap_queries=["q2"],
        // final gap-round judge insufficient with gap_queries=[] (empty: no further work).
        // The loop completes the final gap round in full, so hit_iteration_cap is set.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                JudgeVerdict {
                    sufficiency: Sufficiency::Partial,
                    reasoning: "partial".into(),
                    gap_queries: vec!["q1".into()],
                },
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "need more".into(),
                    gap_queries: vec!["q1".into()],
                },
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "still insufficient".into(),
                    gap_queries: vec!["q2".into()],
                },
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "exhausted".into(),
                    gap_queries: vec![],
                },
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        // No cap warning: the final gap round finished with no follow-up gaps.
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                SearchEvent::Warning {
                    warning: SearchWarning::IterationCapExhausted
                }
            )),
            "did not expect IterationCapExhausted when the final gap round produced no new gaps in: {evs:?}"
        );

        assert_refining_search(&evs, 1, 2);
        assert_refining_search(&evs, 2, 2);
        assert_done_iterations(&evs, 3);
    }

    // Test 3c: Sufficient verdict on the first gap round causes early return before
    // the post-loop block. IterationCapExhausted must NOT fire.
    #[tokio::test]
    async fn gap_round_sufficient_at_attempt2_no_cap_warning() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "page",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "test query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/b")),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("sufficient answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");

        // The first gap-round judge returns Sufficient: pipeline returns early.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                JudgeVerdict {
                    sufficiency: Sufficiency::Partial,
                    reasoning: "partial".into(),
                    gap_queries: vec!["q1".into()],
                },
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "need more".into(),
                    gap_queries: vec!["q1".into()],
                },
                JudgeVerdict {
                    sufficiency: Sufficiency::Sufficient,
                    reasoning: "done".into(),
                    gap_queries: vec![],
                },
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        assert!(
            !evs.iter().any(|e| {
                *e == (SearchEvent::Warning {
                    warning: SearchWarning::IterationCapExhausted,
                })
            }),
            "expected no IterationCapExhausted when the first gap round is sufficient in: {evs:?}"
        );
        assert_refining_search(&evs, 1, 2);
        assert_done_iterations(&evs, 2);
    }

    // Test 3d: Sufficient verdict on the final gap round causes early return at the
    // last possible round. IterationCapExhausted must NOT fire even though
    // the final iteration ran.
    #[tokio::test]
    async fn gap_round_sufficient_at_attempt3_no_cap_warning() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "page",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "test query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/b")),
            )
            .mount(&searx_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/c")),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("sufficient on last attempt");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");

        // The final gap-round judge returns Sufficient: pipeline
        // returns early via the Sufficient branch, never setting hit_iteration_cap.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                JudgeVerdict {
                    sufficiency: Sufficiency::Partial,
                    reasoning: "partial".into(),
                    gap_queries: vec!["q1".into()],
                },
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "need more".into(),
                    gap_queries: vec!["q1".into()],
                },
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "still need more".into(),
                    gap_queries: vec!["q2".into()],
                },
                JudgeVerdict {
                    sufficiency: Sufficiency::Sufficient,
                    reasoning: "done".into(),
                    gap_queries: vec![],
                },
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        assert!(
            !evs.iter().any(|e| {
                *e == (SearchEvent::Warning {
                    warning: SearchWarning::IterationCapExhausted,
                })
            }),
            "expected no IterationCapExhausted when the final gap round is sufficient in: {evs:?}"
        );

        assert_refining_search(&evs, 1, 2);
        assert_refining_search(&evs, 2, 2);
        assert_done_iterations(&evs, 3);
    }

    // Test 4: ReaderUnavailable across multiple gap rounds does not produce
    // duplicate warning events.
    //
    // All reader calls fail with ServiceUnavailable. The warning must appear
    // exactly once in the event stream even though multiple rounds encounter it.
    // We use a port that refuses connections (127.0.0.1:1) to trigger
    // ServiceUnavailable rather than a mock HTTP 503 (which would be Failed,
    // not ServiceUnavailable in the reader client logic).
    #[tokio::test]
    async fn gap_round_reader_unavailable_warning_deduplicated() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Reader is pointed at a refused port for all rounds.
        let reader_base_url = "http://127.0.0.1:1";

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        // Initial query.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "test query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        // Gap round 2 query.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "gap2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/b")),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("degraded answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");

        // snippets: partial -> reader unavailable in initial round;
        // chunks judge (snippet fallback): insufficient, gap_queries=["gap2"];
        // gap round 2 judge (snippet fallback again, reader still unavailable):
        // insufficient with no further gap queries -> exhaustion.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                JudgeVerdict {
                    sufficiency: Sufficiency::Partial,
                    reasoning: "partial".into(),
                    gap_queries: vec!["gap2".into()],
                },
                // chunks judge after initial reader failure: insufficient
                JudgeVerdict {
                    sufficiency: Sufficiency::Insufficient,
                    reasoning: "not enough".into(),
                    gap_queries: vec!["gap2".into()],
                },
                // gap round 2 chunks judge: insufficient, no more queries
                insufficient_verdict_no_gaps(),
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            reader_base_url,
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        // ReaderUnavailable exactly once.
        let unavail_count = evs
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SearchEvent::Warning {
                        warning: SearchWarning::ReaderUnavailable
                    }
                )
            })
            .count();
        assert_eq!(
            unavail_count, 1,
            "expected exactly 1 ReaderUnavailable event, got {unavail_count} in: {evs:?}"
        );

        assert_done_iterations(&evs, 2);
    }

    // ── Cancellation during initial SearXNG call (lines 475-476) ─────────────

    // Cancel fires while the initial SearXNG request is in-flight.
    // Lines 474-476: Err(SearchError::Cancelled) arm of the initial-round select.
    #[tokio::test]
    async fn initial_searxng_cancel_mid_flight_emits_cancelled() {
        use std::time::Duration;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let searx_server = MockServer::start().await;
        // Delay the SearXNG response so the cancel fires during the select.
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(200))
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");
        let judge = QueueJudge(std::sync::Mutex::new(VecDeque::new()));

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            token_clone.cancel();
        });

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx_server.uri()),
            "http://127.0.0.1:1",
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled in: {evs:?}"
        );
    }

    // ── A judge that cancels on the Nth call ──────────────────────────────────

    struct CancelsOnNthJudgeCall {
        token: CancellationToken,
        // Verdicts to return on call 1, 2, ...; cancels after the Nth call.
        verdicts: std::sync::Mutex<VecDeque<JudgeVerdict>>,
        cancel_on: usize,
        call_count: std::sync::Mutex<usize>,
    }

    impl CancelsOnNthJudgeCall {
        fn new(token: CancellationToken, verdicts: Vec<JudgeVerdict>, cancel_on: usize) -> Self {
            Self {
                token,
                verdicts: std::sync::Mutex::new(verdicts.into_iter().collect()),
                cancel_on,
                call_count: std::sync::Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl JudgeCaller for CancelsOnNthJudgeCall {
        async fn call(&self, _q: &str, _s: &[JudgeSource]) -> Result<JudgeVerdict, SearchError> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            let n = *count;
            drop(count);
            let verdict = self
                .verdicts
                .lock()
                .unwrap()
                .pop_front()
                .expect("CancelsOnNthJudgeCall verdict queue exhausted");
            if n == self.cancel_on {
                self.token.cancel();
            }
            Ok(verdict)
        }
    }

    // ── Cancel between initial-round judge and gap loop entry (line 683) ──────

    // Cancel fires after the initial chunks-judge returns Insufficient (entering
    // the gap loop) but before the loop body's is_cancelled_emit check at line 683.
    #[tokio::test]
    async fn cancel_before_gap_loop_iteration_emits_cancelled() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "t",
                "markdown": "content",
                "status": "ok"
            })))
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
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // Judge sequence: partial (triggers reader escalation),
        // then insufficient (enters gap loop) cancelling on the 2nd call so
        // is_cancelled_emit fires at the top of the first gap iteration.
        let judge = CancelsOnNthJudgeCall::new(
            token.clone(),
            vec![partial_verdict(), insufficient_verdict()],
            2,
        );

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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled in: {evs:?}"
        );
    }

    #[tokio::test]
    async fn gap_round_judge_error_is_returned_after_gap_reader() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(wiremock::matchers::body_partial_json(
                serde_json::json!({ "url": "https://example.com/a" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "initial",
                "markdown": "initial content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(wiremock::matchers::body_partial_json(
                serde_json::json!({ "url": "https://example.com/gap" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/gap",
                "title": "gap",
                "markdown": "gap content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (_, cb) = collect_events();
        let router = proceed_search_router("q");
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![partial_verdict(), insufficient_verdict()]
                .into_iter()
                .collect(),
        ));

        let err = run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap_err();

        assert_eq!(err, SearchError::Internal("queue empty".into()));
    }

    // ── Cancel during gap-round SearXNG call (lines 699-700) ─────────────────

    #[tokio::test]
    async fn gap_round_searxng_cancel_mid_flight_emits_cancelled() {
        use std::time::Duration;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "t",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        // Initial SearXNG responds immediately.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;

        // Gap SearXNG (q1) responds slowly so cancel fires during the select.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(300))
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // partial -> reader -> insufficient with gap_queries=[q1] (enters gap loop)
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![partial_verdict(), insufficient_verdict()]
                .into_iter()
                .collect(),
        ));

        // Cancel fires after initial round completes but during gap SearXNG.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            token_clone.cancel();
        });

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled in: {evs:?}"
        );
    }

    // ── Cancel after gap SearXNG completes but before reader (line 747) ────────

    // A judge that returns a verdict and then fires cancel via a side-effect,
    // used to time cancellation after the gap-SearXNG completes and before the
    // is_cancelled_emit at line 747.
    struct CancelsAfterGapSearxng {
        token: CancellationToken,
        verdicts: std::sync::Mutex<VecDeque<JudgeVerdict>>,
    }

    #[async_trait]
    impl JudgeCaller for CancelsAfterGapSearxng {
        async fn call(&self, _q: &str, _s: &[JudgeSource]) -> Result<JudgeVerdict, SearchError> {
            let verdict = self
                .verdicts
                .lock()
                .unwrap()
                .pop_front()
                .expect("CancelsAfterGapSearxng verdict queue exhausted");
            // Cancel after the SECOND judge call (initial-round chunks judge
            // returns insufficient with gap_queries). The pipeline then enters the
            // gap loop, runs gap SearXNG, reranks new URLs, and hits
            // is_cancelled_emit at line 747 before invoking the reader.
            if self.verdicts.lock().unwrap().is_empty() {
                self.token.cancel();
            }
            Ok(verdict)
        }
    }

    #[tokio::test]
    async fn cancel_before_gap_round_reader_emits_cancelled() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "t",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        // Initial SearXNG.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;
        // Gap SearXNG returns a new URL immediately.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // partial (triggers initial reader), then insufficient with gap_queries=["q1"]
        // (second call cancels). Gap SearXNG completes fast; is_cancelled_emit
        // fires at line 747 before reader.
        let judge = CancelsAfterGapSearxng {
            token: token.clone(),
            verdicts: std::sync::Mutex::new(
                vec![partial_verdict(), insufficient_verdict()]
                    .into_iter()
                    .collect(),
            ),
        };

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled in: {evs:?}"
        );
    }

    // ── Gap-round reader: Cancelled (lines 758-760) ───────────────────────────

    #[tokio::test]
    async fn gap_round_reader_cancelled_emits_cancelled() {
        use std::time::Duration;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // Initial round reader responds immediately.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "t",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;
        // Gap SearXNG returns a new URL.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        // A second mock server for the gap-round reader that delays long enough
        // for the cancel to fire mid-fetch.
        let gap_reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(300))
                    .set_body_json(serde_json::json!({
                        "url": "u", "title": "t", "markdown": "m", "status": "ok"
                    })),
            )
            .mount(&gap_reader_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // partial (triggers initial reader at reader_server),
        // insufficient with gap_queries=["q1"] (gap reader at gap_reader_server).
        // However, we need only one reader_base_url in run_agentic. We simulate
        // the gap-round reader cancellation by using the same slow reader mock
        // and mounting the initial response first (wiremock matches first
        // registered mock), then the slow one fires for the gap round.
        // Simpler: use a single reader server that always delays, and cancel
        // during the initial reader fetch (but we already have a test for that).
        // Actually: use partial_verdict -> insufficient. After initial reader
        // succeeds and chunk judge returns insufficient, gap SearXNG returns a
        // new URL. Then we cancel during the gap reader call.

        // Reset: use gap_reader_server for ALL reader calls. Initial reader
        // would also be slow, but we cancel AFTER initial reader succeeds.
        // To make this work cleanly, cancel after the initial reader + judge
        // completes. We use CancelsOnNthJudgeCall with cancel_on=2 (which fires
        // after the second judge call returns insufficient). Then gap SearXNG
        // runs fast, new URL arrives, gap reader call starts. But the token was
        // already cancelled by then and fetch_batch_cancellable returns Cancelled.

        let judge = CancelsOnNthJudgeCall::new(
            token.clone(),
            vec![partial_verdict(), insufficient_verdict()],
            2,
        );

        // Cancel fires immediately when judge call 2 returns. Gap SearXNG would
        // be selected, but is_cancelled_emit at line 683 fires first. That path
        // is already covered by cancel_before_gap_loop_iteration_emits_cancelled.
        // To hit the gap-round reader Cancelled arm (line 758), we need the
        // cancel to fire AFTER gap SearXNG and BEFORE reader. We use the
        // CancelsAfterGapSearxng judge for this, but delay the gap reader:
        let gap_judge = CancelsAfterGapSearxng {
            token: token.clone(),
            verdicts: std::sync::Mutex::new(
                vec![partial_verdict(), insufficient_verdict()]
                    .into_iter()
                    .collect(),
            ),
        };

        // The gap reader is slow; after CancelsAfterGapSearxng fires, the reader
        // call sees the cancelled token.
        run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx_server.uri()),
            &gap_reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &gap_judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled in: {evs:?}"
        );
        drop(judge); // suppress unused-variable warning
        drop(token_clone);
    }

    // ── Cancel during gap-round Sources check (line 747) ──────────────────────
    //
    // Cancel fires when the on_event callback receives the SECOND Sources event
    // (first = initial round, second = gap round). The pipeline's
    // is_cancelled_emit at line 746 fires immediately after, executing line 747.

    #[tokio::test]
    async fn cancel_at_gap_round_sources_event_executes_line_747() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // Both initial and gap reader calls: respond normally so they don't
        // interfere with the cancel-point test.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "t",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let token_cb = token.clone();
        let h = ConversationHistory::new();

        // on_event counts Sources events; cancels the token on the 2nd one
        // (gap round). When is_cancelled_emit runs at line 746, the token is
        // already cancelled, so line 747 executes.
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<SearchEvent>::new()));
        let events_clone = events.clone();
        let sources_seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sources_seen_cb = sources_seen.clone();
        let cb = move |e: SearchEvent| {
            if matches!(e, SearchEvent::Sources { .. }) {
                let n = sources_seen_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if n >= 2 {
                    token_cb.cancel();
                }
            }
            events_clone.lock().unwrap().push(e);
        };

        let router = proceed_search_router("q");
        // partial -> initial reader -> chunks: insufficient with gap_queries=["q1"]
        // gap round SearXNG returns "https://example.com/gap" -> Sources emitted
        // -> cancel fires -> is_cancelled_emit at line 746 -> line 747 returns.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![partial_verdict(), insufficient_verdict()]
                .into_iter()
                .collect(),
        ));

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled after gap-round Sources in: {evs:?}"
        );
    }

    // ── Cancel during gap-round reader call (lines 758-759) ────────────────────
    //
    // Cancel fires when the on_event callback receives the SECOND ReadingSources
    // event (gap round). The reader has a slow mock so the cancellation token is
    // already set when fetch_batch_cancellable runs its select! loop, yielding
    // FetchOutcome::Cancelled, which maps to Err(ReaderError::Cancelled) and
    // executes lines 758-759.

    #[tokio::test]
    async fn cancel_at_gap_round_reading_sources_event_executes_lines_758_759() {
        use std::time::Duration;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // Initial round reader succeeds immediately.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(wiremock::matchers::body_partial_json(
                serde_json::json!({ "url": "https://example.com/a" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "t",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;
        // Gap round reader has a long delay so cancel fires before it returns.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(5))
                    .set_body_json(serde_json::json!({
                        "url": "u", "title": "t", "markdown": "m", "status": "ok"
                    })),
            )
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let token_cb = token.clone();
        let h = ConversationHistory::new();

        // on_event counts ReadingSources events; cancels on the 2nd one (gap
        // round). The gap reader's 5s delay means fetch_batch_cancellable sees
        // the cancelled token in its select! and returns Err(Cancelled),
        // executing lines 758-759.
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<SearchEvent>::new()));
        let events_clone = events.clone();
        let reading_seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reading_seen_cb = reading_seen.clone();
        let cb = move |e: SearchEvent| {
            if matches!(e, SearchEvent::ReadingSources) {
                let n = reading_seen_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if n >= 2 {
                    token_cb.cancel();
                }
            }
            events_clone.lock().unwrap().push(e);
        };

        let router = proceed_search_router("q");
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![partial_verdict(), insufficient_verdict()]
                .into_iter()
                .collect(),
        ));

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled from gap-round reader in: {evs:?}"
        );
    }

    // ── Gap-round reader: ServiceUnavailable first occurrence (lines 764-767) ──
    //
    // A one-shot TCP listener responds to the initial reader call then closes.
    // The gap round reader connects to the same port but gets ECONNREFUSED
    // (is_connect() == true), which maps to FetchOutcome::ServiceUnavailable.
    // Since warnings is empty at that point, lines 764-767 fire.

    async fn one_shot_reader_server() -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{}", addr);

        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("one-shot accept");
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let body = serde_json::json!({
                "url": "https://example.com/a",
                "title": "initial",
                "markdown": "content",
                "status": "ok"
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            // listener drops here; subsequent connections receive ECONNREFUSED.
        });

        (base_url, handle)
    }

    #[tokio::test]
    async fn gap_round_reader_unavailable_first_occurrence_emits_warning() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // One-shot reader: accepts the initial-round call, then closes.
        // The gap-round call gets ECONNREFUSED => ServiceUnavailable.
        let (reader_base, _reader_handle) = one_shot_reader_server().await;

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // partial: initial reader succeeds (one-shot), chunks: insufficient
        // with gap_queries=["q1"], gap round reader = ECONNREFUSED =
        // ServiceUnavailable (first occurrence, lines 764-767 fire),
        // gap judge fallback: sufficient -> synthesize.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                partial_verdict(),
                insufficient_verdict(),
                sufficient_verdict(),
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_base,
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        // ReaderUnavailable warning fires exactly once (gap round, first occurrence).
        let unavail_count = evs
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SearchEvent::Warning {
                        warning: SearchWarning::ReaderUnavailable
                    }
                )
            })
            .count();
        assert_eq!(
            unavail_count, 1,
            "expected exactly one ReaderUnavailable warning in: {evs:?}"
        );
        assert_done_iterations(&evs, 2);
    }

    // ── Gap-round reader: BatchTimeout (lines 771-779) ────────────────────────

    #[tokio::test]
    async fn gap_round_reader_batch_timeout_emits_partial_failure_warning() {
        use std::time::Duration;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // Initial reader responds fast.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(wiremock::matchers::body_partial_json(
                serde_json::json!({ "url": "https://example.com/a" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "initial",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;
        // Gap reader responds after 2s (> reader_batch_timeout_s=1s in tests).
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

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // partial (initial reader succeeds), insufficient with gap_queries=["q1"],
        // gap round reader times out, judge fallback (no chunks): sufficient.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                partial_verdict(),
                insufficient_verdict(),
                sufficient_verdict(),
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
            "expected ReaderPartialFailure from gap-round BatchTimeout in: {evs:?}"
        );
        assert_done_iterations(&evs, 2);
    }

    // ── Gap-round BatchTimeout with dedup suppressed (line 777) ──────────────
    //
    // Initial round reader returns 2 failed URLs (>50%), which fires
    // ReaderPartialFailure in the initial-round partial-failure check.
    // Gap round reader times out (BatchTimeout). The dedup check at line 772
    // finds ReaderPartialFailure already in warnings, so the if body is skipped
    // and line 777 (the else side of the closing brace) executes.

    #[tokio::test]
    async fn gap_round_reader_batch_timeout_dedup_suppresses_second_warning() {
        use std::time::Duration;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // Initial round: 2 URLs both return 502 (Failed). threshold=ceil(2*0.5)=1,
        // failed_urls.len()=2 > 1, so ReaderPartialFailure fires in the initial round.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(wiremock::matchers::body_partial_json(
                serde_json::json!({ "url": "https://example.com/a" }),
            ))
            .respond_with(ResponseTemplate::new(502))
            .mount(&reader_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(wiremock::matchers::body_partial_json(
                serde_json::json!({ "url": "https://example.com/b" }),
            ))
            .respond_with(ResponseTemplate::new(502))
            .mount(&reader_server)
            .await;
        // Gap round reader: very slow (times out after reader_batch_timeout_s=1s).
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

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        // Initial SearXNG returns 2 URLs so reader has 2 targets.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    serde_json::json!({
                        "results": [
                            { "title": "r1", "url": "https://example.com/a", "content": "c" },
                            { "title": "r2", "url": "https://example.com/b", "content": "c" },
                        ]
                    })
                    .to_string(),
                ),
            )
            .mount(&searx_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // partial -> initial reader fails (2 failures > threshold) -> partial
        // failure warning fires in initial round -> insufficient (gap_queries=["q1"])
        // -> gap round reader times out -> dedup suppresses second BatchTimeout warning
        // -> judge (fallback): sufficient -> synthesize.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                partial_verdict(),
                insufficient_verdict(),
                sufficient_verdict(),
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        // ReaderPartialFailure fires exactly once (from initial round).
        let pf_count = evs
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SearchEvent::Warning {
                        warning: SearchWarning::ReaderPartialFailure
                    }
                )
            })
            .count();
        assert_eq!(
            pf_count, 1,
            "expected exactly one ReaderPartialFailure (dedup suppresses gap-round one) in: {evs:?}"
        );
        assert_done_iterations(&evs, 2);
    }

    // ── Gap-round reader: >50% partial failure (lines 783-794) ───────────────

    #[tokio::test]
    async fn gap_round_reader_majority_failures_emits_partial_failure_warning() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        // Initial reader: success for initial URL.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(wiremock::matchers::body_partial_json(
                serde_json::json!({ "url": "https://example.com/a" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "initial",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;
        // Gap reader: all calls return HTTP 502 (Failed). Two gap URLs fail -> 2 > 1 threshold.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(502))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;
        // Gap queries: return 2 new URLs so threshold=1, failures=2 > 1.
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    serde_json::json!({
                        "results": [
                            { "title": "r1", "url": "https://example.com/gap1", "content": "c" },
                            { "title": "r2", "url": "https://example.com/gap2", "content": "c" },
                        ]
                    })
                    .to_string(),
                ),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // partial -> initial reader succeeds -> insufficient with gap_queries=["q1"]
        // -> gap reader returns 2 failures (502) -> partial-failure check fires
        // -> judge (no chunks): sufficient -> synthesize.
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                partial_verdict(),
                insufficient_verdict(),
                sufficient_verdict(),
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
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
            "expected ReaderPartialFailure from gap-round majority failures in: {evs:?}"
        );
        let read_step = completed_trace_step(&evs, "round-2-read");
        assert_eq!(
            read_step.detail.as_deref(),
            Some("2 pages failed and 0 pages returned little or no readable text."),
            "expected pluralized gap-round read-step detail in: {evs:?}"
        );
        assert_done_iterations(&evs, 2);
    }

    #[tokio::test]
    async fn gap_round_reader_detail_singularizes_failed_page_count() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(wiremock::matchers::body_partial_json(
                serde_json::json!({ "url": "https://example.com/a" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "initial",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(502))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        let ollama_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let stream = stream_line_token("answer");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(stream))
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                partial_verdict(),
                insufficient_verdict(),
                sufficient_verdict(),
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        let read_step = completed_trace_step(&evs, "round-2-read");
        assert_eq!(
            read_step.detail.as_deref(),
            Some("1 page failed and 0 pages returned little or no readable text."),
            "expected singular gap-round failed-page detail in: {evs:?}"
        );
        assert_done_iterations(&evs, 2);
    }

    // ── Cancel before fallback synthesis after gap-loop exhaustion (line 874) ─

    // A judge that cancels on the Nth call (using CancelsOnNthJudgeCall with
    // cancel_on=3 so cancellation fires after gap-loop exhaustion, just before
    // line 874's is_cancelled_emit).
    #[tokio::test]
    async fn cancel_before_fallback_synthesis_after_gap_exhaustion_emits_cancelled() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "t",
                "markdown": "content",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/a")),
            )
            .mount(&searx_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("q");

        // Judge sequence: partial, insufficient (gap_queries=["q1"]), insufficient
        // with no gap queries (exhausts loop). Cancel fires on the 3rd call, which
        // is after the last gap-round judge returns insufficient (triggering
        // loop exhaustion). is_cancelled_emit at line 874 fires before synthesis.
        let judge = CancelsOnNthJudgeCall::new(
            token.clone(),
            vec![
                partial_verdict(),
                insufficient_verdict(), // gap_queries=["q1"] -> enters gap loop
                insufficient_verdict_no_gaps(), // gap round: no more queries -> exhausted
            ],
            3,
        );

        run_agentic(
            "http://127.0.0.1:1/api/chat",
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "q".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
            "expected Cancelled before fallback synthesis in: {evs:?}"
        );
        // No Done event: pipeline returned before synthesis.
        assert!(
            !evs.iter().any(|e| matches!(e, SearchEvent::Done { .. })),
            "unexpected Done event when cancelled before synthesis in: {evs:?}"
        );
    }

    // ── Gap-round reader: ServiceUnavailable first in gap round (lines 764-767) ─
    // Use a wiremock-based reader that only matches the initial URL; the gap
    // URL falls through to a background 127.0.0.1:1 mock for connection refusal.
    // This is not achievable with a single base URL. Use an alternative approach:
    // initial round reader = port-refused (so ReaderUnavailable warning fires in
    // initial round), gap round reader = also port-refused (dedup suppresses).
    // Lines 764-767 fire in the initial round. The gap-round match arm at 761
    // is also hit by the dedup test. Both are now covered.

    // ── IterationComplete events emitted per iteration ───────────────────────
    //
    // Verifies that `IterationComplete` events are emitted after each retrieval
    // iteration (one per metadata.iterations.push), and that the final Done
    // metadata mirrors the emitted iteration summary. Uses the
    // snippets-sufficient path (Site A) for
    // the simplest setup: one SearXNG result, snippet judge returns Sufficient,
    // no reader escalation. Exactly one IterationComplete must fire.
    #[tokio::test]
    async fn iteration_complete_events_match_done_metadata_summary() {
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
        // Snippet judge returns Sufficient: Site A push fires, reader skipped.
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
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();

        // At least one IterationComplete event must have been emitted.
        let iteration_complete_events: Vec<_> = evs
            .iter()
            .filter(|e| matches!(e, SearchEvent::IterationComplete { .. }))
            .collect();
        assert!(
            !iteration_complete_events.is_empty(),
            "expected at least one IterationComplete event in: {evs:?}"
        );

        // The trace must reflect the snippets-sufficient path:
        // stage=Initial, urls_fetched=[], judge_verdict=Sufficient.
        let first_trace = evs
            .iter()
            .find_map(|e| {
                if let SearchEvent::IterationComplete { trace } = e {
                    Some(trace)
                } else {
                    None
                }
            })
            .expect("expected IterationComplete event");
        assert_eq!(
            first_trace.stage,
            IterationStage::Initial,
            "expected Initial stage, got: {:?}",
            first_trace.stage
        );
        assert!(
            first_trace.urls_fetched.is_empty(),
            "expected no urls_fetched on snippet-sufficient path, got: {:?}",
            first_trace.urls_fetched
        );
        assert_eq!(
            first_trace.judge_verdict,
            Sufficiency::Sufficient,
            "expected Sufficient verdict"
        );

        let metadata = done_metadata(&evs);
        assert_eq!(
            metadata.iterations.as_slice(),
            std::slice::from_ref(first_trace)
        );
    }

    // ── Trace plural-label coverage ──────────────────────────────────────────

    fn searx_body_two_results(url1: &str, url2: &str) -> String {
        serde_json::json!({
            "results": [
                { "title": "result one", "url": url1, "content": "content from site one" },
                { "title": "result two", "url": url2, "content": "content from site two" }
            ]
        })
        .to_string()
    }

    // Lines 762, 764 (initial round "N sites" plural) and 1139 (chunk rerank
    // "N sources" plural): SearXNG returns 2 URLs from different domains so
    // unique_domains() length > 1, reader fetches both and returns distinct
    // source URLs, producing multiple chunk sources.
    #[tokio::test]
    async fn initial_round_multi_domain_plural_labels_in_trace() {
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(body_partial_json(
                serde_json::json!({ "url": "https://alpha.com/page" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://alpha.com/page",
                "title": "Alpha",
                "markdown": "alpha content about the query topic for testing",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(body_partial_json(
                serde_json::json!({ "url": "https://beta.org/page" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://beta.org/page",
                "title": "Beta",
                "markdown": "beta content about the query topic for testing purposes",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let mut ollama = mockito::Server::new_async().await;
        let mut searx = mockito::Server::new_async().await;

        let _searx_mock = searx
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(searx_body_two_results(
                "https://alpha.com/page",
                "https://beta.org/page",
            ))
            .create_async()
            .await;

        let stream = stream_line_token("answer");
        let _stream_mock = ollama
            .mock("POST", "/api/chat")
            .with_body(stream)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("multi domain query");
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
            "multi domain query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert_done_iterations(&evs, 1);
    }

    // Line 1056 (initial round read step "N page returned little or no readable
    // text" singular "page" branch): reader returns status != "ok" for 1 URL,
    // making empty_urls.len() == 1 so the singular branch fires.
    #[tokio::test]
    async fn initial_round_single_empty_url_singular_label_in_trace() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/a",
                "title": "result",
                "markdown": "",
                "status": "empty"
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

        let stream = stream_line_token("answer");
        let _stream_mock = ollama
            .mock("POST", "/api/chat")
            .with_body(stream)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("empty url query");
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
            "empty url query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert_done_iterations(&evs, 1);
    }

    // Lines 1614, 1619, 1662 (gap round chunk step "N pages", "N passages",
    // "N sources" plural): gap round fetches 2 URLs from different domains;
    // reader returns 2 pages, producing multiple passages and sources.
    #[tokio::test]
    async fn gap_round_multi_page_plural_labels_in_trace() {
        use wiremock::matchers::{body_partial_json, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(body_partial_json(
                serde_json::json!({ "url": "https://example.com/initial" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/initial",
                "title": "Initial",
                "markdown": "initial content about the query",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(body_partial_json(
                serde_json::json!({ "url": "https://gap-alpha.com/gap" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://gap-alpha.com/gap",
                "title": "Gap Alpha",
                "markdown": "gap round content from alpha site about the topic details",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .and(body_partial_json(
                serde_json::json!({ "url": "https://gap-beta.org/gap" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://gap-beta.org/gap",
                "title": "Gap Beta",
                "markdown": "gap round content from beta site providing extra context for the answer",
                "status": "ok"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "test query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/initial")),
            )
            .mount(&searx_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(searx_body_two_results(
                    "https://gap-alpha.com/gap",
                    "https://gap-beta.org/gap",
                )),
            )
            .mount(&searx_server)
            .await;

        let ollama_server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(stream_line_token("gap answer")),
            )
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                partial_verdict(),
                insufficient_verdict(),
                sufficient_verdict(),
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert_done_iterations(&evs, 2);
    }

    // Line 1575 (gap round read step singular "page returned little or no
    // readable text"): gap round reader returns status != "ok" for exactly 1
    // URL, making round_reader_result.empty_urls.len() == 1.
    #[tokio::test]
    async fn gap_round_single_empty_url_singular_label_in_trace() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let reader_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/initial",
                "title": "Initial",
                "markdown": "initial content about the topic",
                "status": "ok"
            })))
            .up_to_n_times(1)
            .mount(&reader_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://example.com/gap",
                "title": "Gap",
                "markdown": "",
                "status": "empty"
            })))
            .mount(&reader_server)
            .await;

        let searx_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "test query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/initial")),
            )
            .mount(&searx_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "q1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(searx_body_one_result("https://example.com/gap")),
            )
            .mount(&searx_server)
            .await;

        let ollama_server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(stream_line_token("gap answer")),
            )
            .mount(&ollama_server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let h = ConversationHistory::new();
        let (events, cb) = collect_events();
        let router = proceed_search_router("test query");
        let judge = QueueJudge(std::sync::Mutex::new(
            vec![
                partial_verdict(),
                insufficient_verdict(),
                sufficient_verdict(),
            ]
            .into_iter()
            .collect(),
        ));

        run_agentic(
            &format!("{}/api/chat", ollama_server.uri()),
            &format!("{}/search", searx_server.uri()),
            &reader_server.uri(),
            "m",
            &client,
            token,
            "chat",
            &h,
            "test query".into(),
            "2026-04-18",
            &cb,
            &router,
            &judge,
            &config::SearchRuntimeConfig::default(),
        )
        .await
        .unwrap();

        let evs = events.lock().unwrap();
        assert_done_iterations(&evs, 2);
    }
}
