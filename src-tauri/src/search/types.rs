//! Shared types for the `/search` pipeline: streamed frontend events, internal
//! agentic output types, SearXNG response shapes, and structured pipeline errors.
//!
//! All types that cross the Tauri IPC boundary (see [`SearchEvent`]) are
//! serialised with a `type` tag so the frontend can discriminate cleanly.
//! Internal types ([`SearxResult`]) never leave Rust.

use serde::{Deserialize, Serialize};

// ─── Streamed events ────────────────────────────────────────────────────────

/// A search result forwarded to the frontend for the sources footer. Only
/// `title` and `url` are included; the snippet content stays on the Rust side.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SearchResultPreview {
    pub title: String,
    pub url: String,
}

/// Streaming, user-facing timeline step for the `/search` pipeline.
///
/// Unlike [`IterationTrace`], which is a coarse per-round diagnostic summary,
/// this type is designed for the live `SearchTraceBlock` UI. The backend may
/// re-emit the same `id` with updated `status`, `summary`, or `counts` as a
/// stage progresses.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchTraceKind {
    Analyze,
    Clarify,
    HistoryAnswer,
    Search,
    UrlRerank,
    SnippetJudge,
    Read,
    Chunk,
    ChunkRerank,
    ChunkJudge,
    Refine,
    Compose,
}

/// Lifecycle state for a live [`SearchTraceStep`].
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchTraceStatus {
    Running,
    Completed,
}

/// Optional compact metrics surfaced beside a [`SearchTraceStep`] in the UI.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchTraceCounts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<u32>,
}

/// A single live search-trace timeline step for the frontend.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchTraceStep {
    /// Stable identifier. Re-emitting the same id updates the same UI step.
    pub id: String,
    /// Semantic search-pipeline stage.
    pub kind: SearchTraceKind,
    /// Whether this stage is still running or has completed.
    pub status: SearchTraceStatus,
    /// 1-indexed retrieval round for looped search stages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
    /// Short stage title shown in the timeline header.
    pub title: String,
    /// Primary user-facing explanation for this stage.
    pub summary: String,
    /// Optional secondary explanation such as fallback context or missing info.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Queries used or planned in this stage.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<String>,
    /// Exact page URLs surfaced when users should see the concrete pages considered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
    /// Deduplicated source domains relevant to this stage.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
    /// Sufficiency verdict for judge stages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Sufficiency>,
    /// Compact counts displayed by the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<SearchTraceCounts>,
}

/// Structured event emitted to the frontend over the Tauri Channel during a
/// `search_pipeline` invocation. Matches the `SearchEvent` TypeScript union.
///
/// Lifecycle per pipeline run:
/// - `AnalyzingQuery` -> `Token`* -> `Done`  (clarification branch)
/// - `AnalyzingQuery` -> `Token`* -> `Done`  (answer-from-context branch)
/// - `AnalyzingQuery` -> `Searching` -> `Token`* -> `Done`  (search branch)
/// - `Cancelled` may replace `Done` when the user cancels mid-stream.
/// - `Error` may replace any later event on a fatal backend failure.
///
/// Agentic-loop variants extend the lifecycle with sufficiency judging,
/// reader stages, gap-refinement rounds, and warnings.

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum SearchEvent {
    /// User-facing timeline update for the live `SearchTraceBlock` UI.
    Trace { step: SearchTraceStep },
    /// SearXNG lookup is in flight. Carries the queries submitted to SearXNG so
    /// the frontend can display them in the live search trace.
    Searching { queries: Vec<String> },
    /// SearXNG results arrived; forwarded so the frontend can render the
    /// sources footer after synthesis completes.
    Sources { results: Vec<SearchResultPreview> },
    /// Streaming token from the answering LLM call.
    Token { content: String },
    /// Pipeline finished successfully. Emitted once, last. Search branches may
    /// attach a structured metadata summary for persistence and later analysis.
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<SearchMetadata>,
    },
    /// User cancelled. Emitted instead of `Done` when cancellation wins.
    Cancelled,
    /// Fatal pipeline error with a user-facing message.
    Error { message: String },
    /// Agentic router/judge LLM call is in flight.
    AnalyzingQuery,
    /// Reader is fetching and extracting text from the ranked source URLs.
    ReadingSources,
    /// A single URL is about to be fetched by the reader. Emitted once per URL
    /// before the HTTP request fires, so the frontend can stream in-progress URL
    /// analysis live during a retrieval round.
    FetchingUrl { url: String },
    /// Sufficiency judge returned `Partial` or `Insufficient`; starting
    /// another SearXNG round with gap-filling queries. `attempt` is the
    /// 1-indexed gap-round number; `total` is the configured maximum number
    /// of gap rounds.
    RefiningSearch { attempt: u32, total: u32 },
    /// Final synthesis LLM call is in flight (answer assembly phase).
    Composing,
    /// Non-fatal pipeline warning that the frontend may surface as a subtle
    /// indicator (e.g., a small icon in the sources footer).
    Warning { warning: SearchWarning },
    /// Pre-flight sandbox probe failed: the SearXNG or reader container is not
    /// running. The frontend renders a static setup-guidance card instead of a
    /// generic error bubble.
    SandboxUnavailable,
    /// Emitted after each retrieval iteration completes (after the judge
    /// verdict is received). Allows the frontend to stream trace rows live
    /// as the pipeline progresses rather than receiving all traces at once.
    /// Not emitted for clarification-only or history-sufficient turns.
    IterationComplete { trace: IterationTrace },
}

// ─── Router output ──────────────────────────────────────────────────────────

// ─── Agentic-loop types ─────────────────────────────────────────────────────

/// The action the router/judge should take for the current query. Used inside
/// [`RouterJudgeOutput`] to drive the agentic pipeline branch.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Query is ambiguous; pipeline should surface a clarifying question.
    Clarify,
    /// Query is clear and ready to proceed (search or answer from context).
    Proceed,
}

/// Degree to which the collected evidence answers the query. Returned by the
/// sufficiency judge LLM call and used to decide whether to run additional
/// gap-filling search rounds.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sufficiency {
    /// Collected evidence fully answers the query; no further rounds needed.
    Sufficient,
    /// Evidence partially answers the query; one or more gap rounds are
    /// worthwhile.
    Partial,
    /// Evidence does not answer the query; gap rounds are required.
    Insufficient,
}

/// Combined router and judge output for the agentic pipeline. The model emits
/// one JSON object that covers both routing classification and history-sufficiency
/// assessment.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RouterJudgeOutput {
    /// Whether the pipeline should clarify or proceed.
    pub action: Action,
    /// Present when `action` is `Clarify`; the question to surface to the user.
    #[serde(default)]
    pub clarifying_question: Option<String>,
    /// Optional early-exit signal: if the router judges conversation history
    /// as already sufficient, a search round can be skipped.
    #[serde(default)]
    pub history_sufficiency: Option<Sufficiency>,
    /// LLM-rewritten query optimised for SearXNG when `action` is `Proceed`.
    #[serde(default)]
    pub optimized_query: Option<String>,
}

/// Verdict returned by the sufficiency judge after each search round. Drives
/// the gap-refinement loop: `Partial` or `Insufficient` verdicts trigger
/// another round using `gap_queries`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JudgeVerdict {
    /// How well the current evidence answers the query.
    pub sufficiency: Sufficiency,
    /// Free-text explanation used for logging and debug traces.
    pub reasoning: String,
    /// Queries to run in the next gap round. Empty when `sufficiency` is
    /// `Sufficient`.
    #[serde(default)]
    pub gap_queries: Vec<String>,
}

/// Non-fatal conditions the pipeline can encounter. Emitted via
/// `SearchEvent::Warning` so the frontend can show subtle indicators without
/// aborting the response.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchWarning {
    /// The reader service (Jina/similar) was unreachable; snippets fell back
    /// to SearXNG content.
    ReaderUnavailable,
    /// Some URLs failed reader extraction; partial content was used.
    ReaderPartialFailure,
    /// The initial SearXNG round returned zero results.
    NoResultsInitial,
    /// The gap-refinement loop hit the configured maximum iteration count
    /// before the judge returned `Sufficient`.
    IterationCapExhausted,
    /// The router/judge LLM call failed or returned unparseable JSON; the
    /// pipeline fell back to a default branch.
    RouterFailure,
    /// The synthesis LLM stream was interrupted (e.g., timeout) before
    /// completion; the response may be truncated.
    SynthesisInterrupted,
}

/// Which phase of the multi-round retrieval loop an [`IterationTrace`] belongs
/// to.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum IterationStage {
    /// The first SearXNG round, issued with the original (or router-rewritten)
    /// query.
    Initial,
    /// A subsequent gap-filling round. `round` is 1-indexed.
    GapRound { round: u32 },
}

/// Diagnostic record for a single retrieval iteration, included in
/// [`SearchMetadata`]. Useful for debugging agentic-loop behaviour and for
/// future telemetry or trace UI.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct IterationTrace {
    /// Which phase this iteration covers.
    pub stage: IterationStage,
    /// Queries submitted to SearXNG in this iteration.
    pub queries: Vec<String>,
    /// URLs the reader attempted to fetch in this iteration.
    pub urls_fetched: Vec<String>,
    /// Subset of `urls_fetched` for which the reader returned empty content.
    pub reader_empty_urls: Vec<String>,
    /// Sufficiency verdict the judge returned after reviewing this iteration's
    /// evidence.
    pub judge_verdict: Sufficiency,
    /// Judge's free-text reasoning for the verdict.
    pub judge_reasoning: String,
    /// Wall-clock time spent on this iteration in milliseconds.
    pub duration_ms: u64,
}

/// End-of-pipeline summary attached to the `Done` event payload and used for
/// persistence and future telemetry. Aggregates all [`IterationTrace`] records
/// and top-level timing.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct SearchMetadata {
    /// Ordered list of retrieval iterations, one per search round.
    pub iterations: Vec<IterationTrace>,
    /// Total wall-clock time for the full pipeline in milliseconds.
    pub total_duration_ms: u64,
    /// Number of times an individual LLM or HTTP call was retried due to
    /// transient failures.
    pub retries_performed: u32,
}

// ─── SearXNG response ───────────────────────────────────────────────────────

/// Top-level SearXNG JSON response. Only the `results` array is consumed.
#[derive(Debug, Deserialize)]
pub struct SearxResponse {
    #[serde(default)]
    pub results: Vec<SearxResult>,
}

/// A single SearXNG result. The `content` field is the engine-provided snippet;
/// HTML entity decoding and length-capping happen in the client layer.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SearxResult {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub content: String,
}

// ─── Errors ─────────────────────────────────────────────────────────────────

/// Structured pipeline error. Each variant maps to a stable user-facing message
/// via [`SearchError::user_message`]; internal diagnostic detail is kept out of
/// user-visible output.
#[derive(Clone, Debug, PartialEq)]
pub enum SearchError {
    /// Ollama is not reachable (connection refused, timeout, DNS failure).
    LlmUnavailable,
    /// Ollama responded with a non-2xx status.
    LlmHttp(u16),
    /// Ollama returned content that could not be decoded as JSON.
    LlmBadJson,
    /// Merged router+judge call failed: either no JSON was found in the
    /// response, or the JSON could not be deserialized as RouterJudgeOutput.
    /// The inner string carries diagnostic detail for logging; do not surface
    /// it to the user.
    Router(String),
    /// Sufficiency-judge call failed: either no JSON was found in the response,
    /// or the JSON could not be deserialized as JudgeVerdict. The inner string
    /// carries diagnostic detail for logging.
    Judge(String),
    /// SearXNG is not reachable.
    SearxUnavailable,
    /// SearXNG responded with a non-2xx status.
    SearxHttp(u16),
    /// SearXNG returned zero usable results.
    NoResults,
    /// User-supplied query was empty after trimming.
    EmptyQuery,
    /// Pipeline cancelled via the shared CancellationToken.
    Cancelled,
    /// Internal pipeline invariant violated or a stub branch was reached. The
    /// inner string is for logging only and must not be shown to the user.
    /// Used as a placeholder in unfinished agentic branches until the
    /// implementing task fills them in.
    // Constructed by run_agentic which has no non-test call site until Task 16.
    #[allow(dead_code)]
    Internal(String),
    /// The search sandbox containers are not running. Raised when SearXNG
    /// reports a connection-refused error, indicating the sandbox was never
    /// started or died mid-pipeline.
    SandboxUnavailable,
}

impl SearchError {
    /// Returns the user-facing message for this error. Title on the first line,
    /// subtitle on the second; the frontend renders both.
    pub fn user_message(&self) -> String {
        match self {
            Self::LlmUnavailable => "Ollama isn't running\nStart Ollama and try again.".to_string(),
            Self::LlmHttp(status) => format!("Ollama request failed\nHTTP {status}"),
            Self::LlmBadJson => {
                "Search routing failed\nThe model returned an invalid response.".to_string()
            }
            Self::Router(_) => {
                "Search routing failed\nThe model returned an invalid response.".to_string()
            }
            Self::Judge(_) => {
                "Search analysis failed\nThe model returned an invalid response.".to_string()
            }
            Self::SearxUnavailable => {
                "Search service unreachable\nRun `bun run search-box:start` and try again."
                    .to_string()
            }
            Self::SearxHttp(status) => format!("Search service failed\nHTTP {status}"),
            Self::NoResults => "No results found\nTry rephrasing your query.".to_string(),
            Self::EmptyQuery => "Empty query\nType a search query after /search.".to_string(),
            Self::Cancelled => "Cancelled".to_string(),
            Self::Internal(_) => "Something went wrong.\nPlease try again.".to_string(),
            Self::SandboxUnavailable => {
                "Thuki's search sandbox isn't running.\n\
                The /search command needs two local containers (SearXNG and a Trafilatura reader) to be up. \
                See the setup steps in the repo README:\n\
                https://github.com/quiet-node/thuki/blob/main/README.md#setup-the-search-sandbox-optional-required-for-search"
                    .to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_event_serialises_with_type_tag() {
        let done = serde_json::to_value(SearchEvent::Done {
            metadata: Some(SearchMetadata {
                iterations: vec![IterationTrace {
                    stage: IterationStage::Initial,
                    queries: vec!["rust async".into()],
                    urls_fetched: vec!["https://rust-lang.org".into()],
                    reader_empty_urls: vec![],
                    judge_verdict: Sufficiency::Partial,
                    judge_reasoning: "still missing version details".into(),
                    duration_ms: 42,
                }],
                total_duration_ms: 42,
                retries_performed: 0,
            }),
        })
        .unwrap();
        assert_eq!(done["type"], "Done");
        assert_eq!(done["metadata"]["total_duration_ms"], 42);

        let token = serde_json::to_value(SearchEvent::Token {
            content: "hi".into(),
        })
        .unwrap();
        assert_eq!(token["type"], "Token");
        assert_eq!(token["content"], "hi");
    }

    #[test]
    fn search_event_variants_serialise_distinct_tags() {
        let trace = serde_json::to_value(SearchEvent::Trace {
            step: SearchTraceStep {
                id: "analyze".into(),
                kind: SearchTraceKind::Analyze,
                status: SearchTraceStatus::Running,
                round: None,
                title: "Understanding the question".into(),
                summary: "Deciding whether to search".into(),
                detail: None,
                queries: vec![],
                urls: vec![],
                domains: vec![],
                verdict: None,
                counts: None,
            },
        })
        .unwrap();
        assert_eq!(trace["type"], "Trace");
        assert_eq!(trace["step"]["kind"], "analyze");

        let searching = serde_json::to_value(SearchEvent::Searching {
            queries: vec!["rust async".into()],
        })
        .unwrap();
        assert_eq!(searching["type"], "Searching");
        assert_eq!(searching["queries"][0], "rust async");

        let done = serde_json::to_value(SearchEvent::Done { metadata: None }).unwrap();
        assert_eq!(done["type"], "Done");
        assert!(done.get("metadata").is_none());

        let cancelled = serde_json::to_value(SearchEvent::Cancelled).unwrap();
        assert_eq!(cancelled["type"], "Cancelled");

        let err = serde_json::to_value(SearchEvent::Error {
            message: "boom".into(),
        })
        .unwrap();
        assert_eq!(err["type"], "Error");
        assert_eq!(err["message"], "boom");
    }

    #[test]
    fn search_event_sandbox_unavailable_serialises_correct_tag() {
        let v = serde_json::to_value(SearchEvent::SandboxUnavailable).unwrap();
        assert_eq!(v["type"], "SandboxUnavailable");
    }

    #[test]
    fn searx_response_parses_results() {
        let json = r#"{"results":[{"title":"t","url":"https://x","content":"c"}]}"#;
        let r: SearxResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.results.len(), 1);
        assert_eq!(r.results[0].title, "t");
        assert_eq!(r.results[0].url, "https://x");
        assert_eq!(r.results[0].content, "c");
    }

    #[test]
    fn searx_response_parses_missing_fields_as_empty() {
        let json = r#"{"results":[{}]}"#;
        let r: SearxResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.results.len(), 1);
        assert_eq!(r.results[0].title, "");
        assert_eq!(r.results[0].url, "");
        assert_eq!(r.results[0].content, "");
    }

    #[test]
    fn searx_response_parses_absent_results_as_empty() {
        let json = r#"{}"#;
        let r: SearxResponse = serde_json::from_str(json).unwrap();
        assert!(r.results.is_empty());
    }

    #[test]
    fn error_messages_are_user_facing() {
        assert!(SearchError::LlmUnavailable
            .user_message()
            .contains("Ollama isn't running"));
        assert!(SearchError::LlmHttp(500).user_message().contains("500"));
        assert!(SearchError::LlmBadJson
            .user_message()
            .contains("invalid response"));
        assert!(SearchError::Router("diag".into())
            .user_message()
            .contains("invalid response"));
        assert!(SearchError::Judge("diag".into())
            .user_message()
            .contains("analysis failed"));
        assert!(SearchError::SearxUnavailable
            .user_message()
            .contains("search-box:start"));
        assert!(SearchError::SearxHttp(503).user_message().contains("503"));
        assert!(SearchError::NoResults.user_message().contains("No results"));
        assert!(SearchError::EmptyQuery
            .user_message()
            .contains("Empty query"));
        assert_eq!(SearchError::Cancelled.user_message(), "Cancelled");
        assert!(SearchError::Internal("diag".into())
            .user_message()
            .contains("Something went wrong"));
        assert!(SearchError::SandboxUnavailable
            .user_message()
            .contains("search sandbox isn't running"));
        assert!(SearchError::SandboxUnavailable
            .user_message()
            .contains("github.com/quiet-node/thuki"));
    }

    #[test]
    fn sandbox_unavailable_message_differs_from_searx_unavailable() {
        // SandboxUnavailable and SearxUnavailable must produce distinct user
        // messages so the frontend can distinguish sandbox-down from other failures.
        let sandbox_msg = SearchError::SandboxUnavailable.user_message();
        let searx_msg = SearchError::SearxUnavailable.user_message();
        assert_ne!(sandbox_msg, searx_msg);
    }
}

#[cfg(test)]
mod new_type_tests {
    use super::*;

    #[test]
    fn sufficiency_round_trips_snake_case() {
        assert_eq!(
            serde_json::to_value(Sufficiency::Sufficient).unwrap(),
            serde_json::json!("sufficient")
        );
        assert_eq!(
            serde_json::to_value(Sufficiency::Partial).unwrap(),
            serde_json::json!("partial")
        );
        assert_eq!(
            serde_json::to_value(Sufficiency::Insufficient).unwrap(),
            serde_json::json!("insufficient")
        );
        let back: Sufficiency = serde_json::from_str(r#""sufficient""#).unwrap();
        assert_eq!(back, Sufficiency::Sufficient);
        let back: Sufficiency = serde_json::from_str(r#""partial""#).unwrap();
        assert_eq!(back, Sufficiency::Partial);
        let back: Sufficiency = serde_json::from_str(r#""insufficient""#).unwrap();
        assert_eq!(back, Sufficiency::Insufficient);
    }

    #[test]
    fn judge_verdict_deserializes_with_gap_queries() {
        let json =
            r#"{"sufficiency":"partial","reasoning":"missing version","gap_queries":["q1"]}"#;
        let v: JudgeVerdict = serde_json::from_str(json).unwrap();
        assert_eq!(v.sufficiency, Sufficiency::Partial);
        assert_eq!(v.reasoning, "missing version");
        assert_eq!(v.gap_queries, vec!["q1"]);
    }

    #[test]
    fn router_judge_output_deserializes_clarify_only() {
        let json = r#"{"action":"clarify","clarifying_question":"Which framework?","history_sufficiency":null,"optimized_query":null}"#;
        let o: RouterJudgeOutput = serde_json::from_str(json).unwrap();
        assert_eq!(o.action, Action::Clarify);
        assert_eq!(o.clarifying_question, Some("Which framework?".to_string()));
        assert_eq!(o.history_sufficiency, None);
        assert_eq!(o.optimized_query, None);
    }

    #[test]
    fn search_warning_serializes_snake_case() {
        let v = serde_json::to_value(SearchWarning::ReaderUnavailable).unwrap();
        assert_eq!(v, serde_json::json!("reader_unavailable"));
    }

    #[test]
    fn search_event_refining_search_serializes_camel_case_tag() {
        let event = SearchEvent::RefiningSearch {
            attempt: 2,
            total: 3,
        };
        let v = serde_json::to_value(event).unwrap();
        assert_eq!(v["type"], "RefiningSearch");
        assert_eq!(v["attempt"], 2);
        assert_eq!(v["total"], 3);
    }

    #[test]
    fn search_event_analyzing_query_serializes_camel_case_tag() {
        let event = SearchEvent::AnalyzingQuery;
        let v = serde_json::to_value(event).unwrap();
        assert_eq!(v["type"], "AnalyzingQuery");
    }

    #[test]
    fn iteration_stage_gap_round_serializes_snake_case_kind() {
        let stage = IterationStage::GapRound { round: 2 };
        let v = serde_json::to_value(stage).unwrap();
        assert_eq!(v["kind"], "gap_round");
        assert_eq!(v["round"], 2);
    }

    #[test]
    fn search_event_fetching_url_serializes_correct_tag() {
        let event = SearchEvent::FetchingUrl {
            url: "https://example.com".into(),
        };
        let v = serde_json::to_value(event).unwrap();
        assert_eq!(v["type"], "FetchingUrl");
        assert_eq!(v["url"], "https://example.com");
    }

    #[test]
    fn search_event_iteration_complete_serializes_correct_tag_and_payload() {
        let trace = IterationTrace {
            stage: IterationStage::Initial,
            queries: vec!["q1".into()],
            urls_fetched: vec![],
            reader_empty_urls: vec![],
            judge_verdict: Sufficiency::Sufficient,
            judge_reasoning: "ok".into(),
            duration_ms: 42,
        };
        let event = SearchEvent::IterationComplete {
            trace: trace.clone(),
        };
        let v = serde_json::to_value(event).unwrap();
        assert_eq!(v["type"], "IterationComplete");
        assert_eq!(v["trace"]["duration_ms"], 42);
        assert_eq!(v["trace"]["judge_reasoning"], "ok");
        assert!(v["trace"]["queries"].is_array());
    }

    #[test]
    fn search_trace_step_serializes_optional_fields_cleanly() {
        let step = SearchTraceStep {
            id: "round-1-search".into(),
            kind: SearchTraceKind::Search,
            status: SearchTraceStatus::Completed,
            round: Some(1),
            title: "Searching the web".into(),
            summary: "Found 5 results across 3 sites.".into(),
            detail: Some("Using the rewritten query.".into()),
            queries: vec!["rust async runtime".into()],
            urls: vec!["https://rust-lang.org".into()],
            domains: vec!["rust-lang.org".into(), "docs.rs".into()],
            verdict: Some(Sufficiency::Partial),
            counts: Some(SearchTraceCounts {
                found: Some(5),
                kept: Some(3),
                ..SearchTraceCounts::default()
            }),
        };

        let value = serde_json::to_value(step).unwrap();
        assert_eq!(value["kind"], "search");
        assert_eq!(value["status"], "completed");
        assert_eq!(value["round"], 1);
        assert_eq!(value["queries"][0], "rust async runtime");
        assert_eq!(value["urls"][0], "https://rust-lang.org");
        assert_eq!(value["counts"]["found"], 5);
        assert_eq!(value["verdict"], "partial");
    }
}
