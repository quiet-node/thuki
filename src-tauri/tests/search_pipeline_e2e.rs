//! End-to-end integration tests for the `/search` agentic pipeline.
//!
//! Five canonical scenarios exercise `run_agentic` through the real trait seams
//! against mock SearXNG and mock reader servers. Each test names a distinct
//! observable behaviour so future contributors can identify coverage at a glance.
//!
//! Run with:
//!   cargo test --test search_pipeline_e2e

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use thuki_agent_lib::commands::ConversationHistory;
use thuki_agent_lib::config::defaults::DEFAULT_NUM_CTX;
use thuki_agent_lib::search::{
    run_agentic, Action, JudgeCaller, JudgeSource, JudgeVerdict, RouterJudgeCaller,
    RouterJudgeOutput, SearchError, SearchEvent, SearchMetadata, SearchWarning, Sufficiency,
};
use thuki_agent_lib::trace::{
    BoundRecorder, ConversationId, FileRecorder, NoopRecorder, TraceDomain, TraceRecorder,
};

/// Returns a recorder that writes `traces/search/<label>.jsonl` under
/// `THUKI_TRACE_DIR` when that env var is set, otherwise a noop. Lets
/// smoke tests dump full traces for judge-behaviour analysis without
/// changing the default test outcome.
///
/// Usage: `THUKI_TRACE_DIR=/tmp/thuki-traces cargo test --test search_pipeline_e2e`
fn opt_trace_recorder(label: &str) -> Arc<BoundRecorder> {
    let conv_id = ConversationId::new(label);
    let inner: Arc<dyn TraceRecorder> = if let Ok(dir) = std::env::var("THUKI_TRACE_DIR") {
        Arc::new(FileRecorder::for_conversation(
            dir,
            TraceDomain::Search,
            &conv_id,
        ))
    } else {
        Arc::new(NoopRecorder)
    };
    Arc::new(BoundRecorder::new(inner, conv_id))
}

// ── fixtures ──────────────────────────────────────────────────────────────────

/// Collects events emitted by the pipeline via a closure.
fn collect_events() -> (Arc<Mutex<Vec<SearchEvent>>>, impl Fn(SearchEvent)) {
    let events = Arc::new(Mutex::new(Vec::<SearchEvent>::new()));
    let clone = events.clone();
    let cb = move |e: SearchEvent| clone.lock().unwrap().push(e);
    (events, cb)
}

fn done_metadata(events: &[SearchEvent]) -> &SearchMetadata {
    match events.last().expect("expected final event") {
        SearchEvent::Done {
            metadata: Some(metadata),
        } => metadata,
        other => panic!("expected final Done event with metadata, got: {other:?}"),
    }
}

fn assert_done_iterations(events: &[SearchEvent], expected_iterations: usize) {
    let metadata = done_metadata(events);
    assert_eq!(
        metadata.iterations.len(),
        expected_iterations,
        "expected Done metadata with {expected_iterations} iterations, got: {metadata:?}"
    );
}

fn assert_refining_search(events: &[SearchEvent], attempt: u32) {
    assert!(
        events.iter().any(|event| matches!(
            event,
            SearchEvent::RefiningSearch {
                attempt: actual_attempt,
                ..
            } if *actual_attempt == attempt
        )),
        "expected RefiningSearch attempt={attempt} in: {events:?}"
    );
}

/// Minimal router mock: always returns the same `RouterJudgeOutput`.
struct MockRouter(RouterJudgeOutput);

#[async_trait]
impl RouterJudgeCaller for MockRouter {
    async fn call(
        &self,
        _history: &[thuki_agent_lib::commands::ChatMessage],
        _query: &str,
    ) -> Result<RouterJudgeOutput, SearchError> {
        Ok(self.0.clone())
    }
}

/// Builds a `MockRouter` configured for the PROCEED+Insufficient branch with
/// a named optimized query.
fn proceed_router(optimized_query: &str) -> MockRouter {
    MockRouter(RouterJudgeOutput {
        action: Action::Proceed,
        clarifying_question: None,
        history_sufficiency: Some(Sufficiency::Insufficient),
        optimized_query: Some(optimized_query.to_string()),
    })
}

/// Stateful judge mock that pops verdicts from a queue in order.
struct QueueJudge(Mutex<VecDeque<JudgeVerdict>>);

#[async_trait]
impl JudgeCaller for QueueJudge {
    async fn call(
        &self,
        _query: &str,
        _sources: &[JudgeSource],
        _stage: thuki_agent_lib::search::JudgeStage,
    ) -> Result<JudgeVerdict, SearchError> {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| SearchError::Internal("judge queue empty".into()))
    }
}

fn verdict_sufficient() -> JudgeVerdict {
    JudgeVerdict {
        sufficiency: Sufficiency::Sufficient,
        reasoning: "ok".into(),
        gap_queries: vec![],
        parse_failure: false,
    }
}

fn verdict_partial(gap_queries: Vec<String>) -> JudgeVerdict {
    JudgeVerdict {
        sufficiency: Sufficiency::Partial,
        reasoning: "partial".into(),
        gap_queries,
        parse_failure: false,
    }
}

fn verdict_insufficient(gap_queries: Vec<String>) -> JudgeVerdict {
    JudgeVerdict {
        sufficiency: Sufficiency::Insufficient,
        reasoning: "not enough".into(),
        gap_queries,
        parse_failure: false,
    }
}

/// Returns a JSON body for a SearXNG response with a single result.
fn searx_body(url: &str) -> serde_json::Value {
    serde_json::json!({
        "results": [{ "title": "result", "url": url, "content": "some relevant content" }]
    })
}

/// Returns a JSON body for a SearXNG response with multiple results.
fn searx_body_multi(urls: &[&str]) -> serde_json::Value {
    let results: Vec<serde_json::Value> = urls
        .iter()
        .map(|u| serde_json::json!({ "title": "result", "url": u, "content": "content" }))
        .collect();
    serde_json::json!({ "results": results })
}

/// Returns a newline-delimited Ollama streaming response emitting one token.
fn ollama_stream(token: &str) -> String {
    format!(
        "{{\"message\":{{\"role\":\"assistant\",\"content\":\"{token}\"}},\"done\":false}}\n\
         {{\"message\":{{\"role\":\"assistant\",\"content\":\"\"}},\"done\":true}}\n"
    )
}

/// Mounts a mock Ollama `/api/chat` endpoint on `server` that streams `token`.
async fn mount_ollama(server: &mut mockito::Server, token: &str) -> mockito::Mock {
    server
        .mock("POST", "/api/chat")
        .match_body(mockito::Matcher::PartialJsonString(
            r#"{"stream":true}"#.to_string(),
        ))
        .with_body(ollama_stream(token))
        .create_async()
        .await
}

/// Mounts a reader mock that returns a successful extraction for `url`.
async fn mount_reader_success(server: &MockServer, url: &str, content: &str) {
    Mock::given(method("POST"))
        .and(path("/extract"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "url": url,
            "title": "page title",
            "markdown": content,
            "status": "ok"
        })))
        .mount(server)
        .await;
}

// ── Scenario 1 ────────────────────────────────────────────────────────────────

/// Snippets are judged Sufficient on the first call; reader is never invoked.
///
/// Expected events: AnalyzingQuery, Searching, Sources, Composing, Token+, Done.
/// No ReadingSources, no Warning.
#[tokio::test]
async fn happy_path_snippets_sufficient_streams_answer() {
    let mut ollama = mockito::Server::new_async().await;
    let mut searx = mockito::Server::new_async().await;
    // Reader server started but must never be called.
    let reader_server = MockServer::start().await;

    let _searx_mock = searx
        .mock("GET", "/search")
        .match_query(mockito::Matcher::Any)
        .with_body(
            searx_body_multi(&["https://a.com/1", "https://a.com/2", "https://a.com/3"])
                .to_string(),
        )
        .create_async()
        .await;

    let _ollama_mock = mount_ollama(&mut ollama, "hello world").await;

    let client = reqwest::Client::new();
    let token = CancellationToken::new();
    let history = ConversationHistory::new();
    let (events, cb) = collect_events();
    let router = proceed_router("rust async");
    // Single judge call returns Sufficient immediately.
    let judge = QueueJudge(Mutex::new(vec![verdict_sufficient()].into_iter().collect()));

    run_agentic(
        &format!("{}/api/chat", ollama.url()),
        &format!("{}/search", searx.url()),
        &reader_server.uri(),
        "m",
        &client,
        token,
        "system",
        &history,
        "What is rust async?".into(),
        "2026-04-18",
        &cb,
        &router,
        &judge,
        &thuki_agent_lib::search::config::SearchRuntimeConfig::default(),
        DEFAULT_NUM_CTX,
        &opt_trace_recorder("happy_path_snippets_sufficient"),
    )
    .await
    .unwrap();

    let evs = events.lock().unwrap();

    assert_eq!(
        evs[0],
        SearchEvent::AnalyzingQuery,
        "first event must be AnalyzingQuery"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, SearchEvent::Searching { .. })),
        "Searching event missing"
    );
    assert!(
        evs.iter().any(|e| matches!(
            e,
            SearchEvent::Trace { step }
                if step.id == "analyze" || step.id == "round-1-search" || step.id == "compose"
        )),
        "Trace events missing"
    );
    assert!(
        evs.iter().any(|e| matches!(e, SearchEvent::Sources { .. })),
        "Sources event missing"
    );
    assert!(
        evs.iter().any(|e| matches!(e, SearchEvent::Composing)),
        "Composing event missing"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, SearchEvent::Token { content } if content == "hello world")),
        "expected token 'hello world'"
    );
    assert_done_iterations(&evs, 1);

    // IterationComplete event must be emitted when a search path runs.
    assert!(
        evs.iter()
            .any(|e| matches!(e, SearchEvent::IterationComplete { .. })),
        "expected IterationComplete event in happy path"
    );

    // Reader must not be called.
    assert!(
        evs.iter()
            .all(|e| !matches!(e, SearchEvent::ReadingSources)),
        "unexpected ReadingSources"
    );
    // No warnings.
    assert!(
        evs.iter()
            .all(|e| !matches!(e, SearchEvent::Warning { .. })),
        "unexpected Warning"
    );
}

// ── Scenario 2 ────────────────────────────────────────────────────────────────

/// Snippets are Partial; reader escalation fetches pages; chunks judge returns
/// Sufficient. One iteration trace in metadata (not directly observable but the
/// event sequence confirms the reader path was taken).
///
/// Expected events include ReadingSources. No Warning. Done last.
#[tokio::test]
async fn reader_escalation_with_chunks_sufficient() {
    let reader_server = MockServer::start().await;
    // Serve content for all fetched URLs.
    mount_reader_success(
        &reader_server,
        "https://b.com/1",
        "detailed content about rust",
    )
    .await;
    mount_reader_success(
        &reader_server,
        "https://b.com/2",
        "more about async runtimes",
    )
    .await;
    mount_reader_success(&reader_server, "https://b.com/3", "tokio internals").await;
    mount_reader_success(&reader_server, "https://b.com/4", "futures explained").await;
    mount_reader_success(&reader_server, "https://b.com/5", "pin and unpin").await;

    let mut ollama = mockito::Server::new_async().await;
    let mut searx = mockito::Server::new_async().await;

    let _searx_mock = searx
        .mock("GET", "/search")
        .match_query(mockito::Matcher::Any)
        .with_body(
            searx_body_multi(&[
                "https://b.com/1",
                "https://b.com/2",
                "https://b.com/3",
                "https://b.com/4",
                "https://b.com/5",
            ])
            .to_string(),
        )
        .create_async()
        .await;

    let _ollama_mock = mount_ollama(&mut ollama, "chunks answer").await;

    let client = reqwest::Client::new();
    let token = CancellationToken::new();
    let history = ConversationHistory::new();
    let (events, cb) = collect_events();
    let router = proceed_router("async rust");
    // First judge (snippets) = Partial; second judge (chunks) = Sufficient.
    let judge = QueueJudge(Mutex::new(
        vec![verdict_partial(vec!["gap1".into()]), verdict_sufficient()]
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
        "system",
        &history,
        "explain async rust".into(),
        "2026-04-18",
        &cb,
        &router,
        &judge,
        &thuki_agent_lib::search::config::SearchRuntimeConfig::default(),
        DEFAULT_NUM_CTX,
        &opt_trace_recorder("reader_escalation_chunks_sufficient"),
    )
    .await
    .unwrap();

    let evs = events.lock().unwrap();

    assert_eq!(evs[0], SearchEvent::AnalyzingQuery);
    assert!(evs
        .iter()
        .any(|e| matches!(e, SearchEvent::Searching { .. })));
    assert!(evs.iter().any(|e| matches!(e, SearchEvent::Sources { .. })));
    assert!(
        evs.iter().any(|e| matches!(e, SearchEvent::ReadingSources)),
        "ReadingSources event missing"
    );
    assert!(evs.iter().any(|e| matches!(e, SearchEvent::Composing)));
    assert!(evs
        .iter()
        .any(|e| matches!(e, SearchEvent::Token { content } if content == "chunks answer")),);
    assert_done_iterations(&evs, 1);

    // No warnings on clean reader escalation.
    assert!(
        evs.iter()
            .all(|e| !matches!(e, SearchEvent::Warning { .. })),
        "unexpected Warning in: {evs:?}"
    );
}

// ── Scenario 3 ────────────────────────────────────────────────────────────────

/// Reader is unavailable (connection refused). Pipeline degrades to snippets,
/// emits a ReaderUnavailable warning, and still reaches Done with tokens.
///
/// Expected events include Warning { reader_unavailable } and Done. No ReadingSources
/// event (reader returned an error, but ReadingSources was emitted before the attempt).
#[tokio::test]
async fn reader_unavailable_degrades_to_snippets_and_warns() {
    let mut ollama = mockito::Server::new_async().await;
    let mut searx = mockito::Server::new_async().await;

    let _searx_mock = searx
        .mock("GET", "/search")
        .match_query(mockito::Matcher::Any)
        .with_body(
            searx_body_multi(&["https://c.com/1", "https://c.com/2", "https://c.com/3"])
                .to_string(),
        )
        .create_async()
        .await;

    let _ollama_mock = mount_ollama(&mut ollama, "degraded answer").await;

    let client = reqwest::Client::new();
    let token = CancellationToken::new();
    let history = ConversationHistory::new();
    let (events, cb) = collect_events();
    let router = proceed_router("some query");
    // First judge (snippets) = Partial; triggers reader.
    // Reader is unavailable; falls back to snippets.
    // Second judge (snippet fallback, no chunks) = Sufficient.
    let judge = QueueJudge(Mutex::new(
        vec![verdict_partial(vec!["gap1".into()]), verdict_sufficient()]
            .into_iter()
            .collect(),
    ));

    // Deliberately pass a reader base URL that nothing is listening on.
    run_agentic(
        &format!("{}/api/chat", ollama.url()),
        &format!("{}/search", searx.url()),
        "http://127.0.0.1:1",
        "m",
        &client,
        token,
        "system",
        &history,
        "some query".into(),
        "2026-04-18",
        &cb,
        &router,
        &judge,
        &thuki_agent_lib::search::config::SearchRuntimeConfig::default(),
        DEFAULT_NUM_CTX,
        &opt_trace_recorder("reader_unavailable_degrades_to_snippets"),
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
    assert!(
        evs.iter().any(|e| matches!(e, SearchEvent::Token { .. })),
        "expected at least one Token event"
    );
}

// ── Scenario 4 ────────────────────────────────────────────────────────────────

/// Judge always returns Insufficient with fresh gap queries so the loop runs to
/// MAX_ITERATIONS (3 total rounds: initial + gap round 1 + gap round 2).
///
/// Expected: RefiningSearch events for gap-round attempts 1 and 2, a single
/// Warning { iteration_cap_exhausted }, and Done (fallback synthesis).
#[tokio::test]
async fn exhausted_gap_loop_warns_iteration_cap_and_streams_fallback() {
    use wiremock::matchers::query_param;

    let reader_server = MockServer::start().await;

    // Mount per-URL reader responses so each round has fresh content.
    mount_reader_success(&reader_server, "https://d.com/initial", "initial page").await;
    mount_reader_success(&reader_server, "https://d.com/gap1", "gap round 1 page").await;
    mount_reader_success(&reader_server, "https://d.com/gap2", "gap round 2 page").await;

    // wiremock SearXNG server to serve distinct URLs per gap query parameter.
    let searx_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "initial query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(searx_body("https://d.com/initial")))
        .mount(&searx_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "gap1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(searx_body("https://d.com/gap1")))
        .mount(&searx_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "gap2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(searx_body("https://d.com/gap2")))
        .mount(&searx_server)
        .await;

    let mut ollama = mockito::Server::new_async().await;
    let _ollama_mock = mount_ollama(&mut ollama, "fallback synthesis").await;

    let client = reqwest::Client::new();
    let token = CancellationToken::new();
    let history = ConversationHistory::new();
    let (events, cb) = collect_events();
    let router = proceed_router("initial query");
    // initial round (snippets): Insufficient -> gap1
    // initial round (chunks, after reader): Insufficient -> gap1
    // gap round 1 (chunks): Insufficient -> gap2
    // gap round 2 (chunks): Insufficient -> gap3 (final round still has more work)
    // Then IterationCapExhausted fallback synthesis.
    let judge = QueueJudge(Mutex::new(
        vec![
            verdict_insufficient(vec!["gap1".into()]),
            verdict_insufficient(vec!["gap1".into()]),
            verdict_insufficient(vec!["gap2".into()]),
            verdict_insufficient(vec!["gap3".into()]),
        ]
        .into_iter()
        .collect(),
    ));

    let searx_endpoint = format!("{}/search", searx_server.uri());
    run_agentic(
        &format!("{}/api/chat", ollama.url()),
        &searx_endpoint,
        &reader_server.uri(),
        "m",
        &client,
        token,
        "system",
        &history,
        "initial query".into(),
        "2026-04-18",
        &cb,
        &router,
        &judge,
        &thuki_agent_lib::search::config::SearchRuntimeConfig::default(),
        DEFAULT_NUM_CTX,
        &opt_trace_recorder("exhausted_gap_loop_warns_iteration_cap"),
    )
    .await
    .unwrap();

    let evs = events.lock().unwrap();

    assert_refining_search(&evs, 1);
    assert_refining_search(&evs, 2);
    // Exactly one IterationCapExhausted warning.
    let cap_warn_count = evs
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
        cap_warn_count, 1,
        "expected exactly one IterationCapExhausted warning"
    );
    // Fallback synthesis: Composing then tokens then Done.
    assert!(evs.iter().any(|e| matches!(e, SearchEvent::Composing)));
    assert_done_iterations(&evs, 3);
}

// ── Scenario 5 ────────────────────────────────────────────────────────────────

/// Cancellation fires during reader fetch. The last event must be Cancelled.
/// No Done event. The conversation history must remain empty (no persist).
#[tokio::test]
async fn cancel_midloop_does_not_persist_and_emits_cancelled() {
    let reader_server = MockServer::start().await;
    // Reader responds slowly so the cancel fires mid-fetch.
    Mock::given(method("POST"))
        .and(path("/extract"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(500))
                .set_body_json(serde_json::json!({
                    "url": "https://e.com/1",
                    "title": "slow",
                    "markdown": "content",
                    "status": "ok"
                })),
        )
        .mount(&reader_server)
        .await;

    let mut searx = mockito::Server::new_async().await;
    let _searx_mock = searx
        .mock("GET", "/search")
        .match_query(mockito::Matcher::Any)
        .with_body(searx_body("https://e.com/1").to_string())
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let token = CancellationToken::new();
    let token_clone = token.clone();
    let history = ConversationHistory::new();
    let (events, cb) = collect_events();
    let router = proceed_router("cancel test");
    // First judge (snippets) = Partial so reader escalation triggers.
    let judge = QueueJudge(Mutex::new(
        vec![verdict_partial(vec!["gap".into()])]
            .into_iter()
            .collect(),
    ));

    // Fire the cancellation token after a short delay, mid-reader-fetch.
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
        "system",
        &history,
        "cancel test".into(),
        "2026-04-18",
        &cb,
        &router,
        &judge,
        &thuki_agent_lib::search::config::SearchRuntimeConfig::default(),
        DEFAULT_NUM_CTX,
        &opt_trace_recorder("cancel_midloop_does_not_persist"),
    )
    .await
    .unwrap();

    let evs = events.lock().unwrap();

    assert!(
        evs.iter().any(|e| matches!(e, SearchEvent::Cancelled)),
        "expected Cancelled event in: {evs:?}"
    );
    assert!(
        !evs.iter().any(|e| matches!(e, SearchEvent::Done { .. })),
        "unexpected Done event after cancellation"
    );
    // No turn was persisted because no Ollama streaming completed.
    assert!(
        history.messages.lock().unwrap().is_empty(),
        "conversation history must be empty after cancel"
    );
}
