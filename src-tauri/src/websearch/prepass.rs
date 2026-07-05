//! The pre-pass: the invisible search trigger.
//!
//! One grammar-constrained LLM call per message decides whether the web is
//! needed and, if so, rewrites the (possibly context-dependent) message into a
//! standalone question plus 1-3 keyword queries. The model is constrained by a
//! strict `response_format` JSON schema, verified to coexist with the engine's
//! reasoning-control flow, so even small local models emit a parseable shape.
//!
//! ## Prompt shape (latency-critical)
//!
//! The bundled engine runs `--parallel 1` with prefix-based KV caching, so the
//! pre-pass prompt is built as the *chat prompt plus an appended decision
//! instruction*: same system prompt, same history, the decision instruction
//! appended to a copy of the latest user turn. The expensive system+history
//! prefix is therefore identical to the writer's prompt and is reused from
//! cache instead of being prefilled twice per message.
//!
//! ## Failure policy
//!
//! A malformed or unparseable response degrades to [`SearchDecision::No`]
//! (answer directly) rather than a spurious search: a false negative is cheap
//! and recoverable through the explicit `/search` force alias, whereas a
//! false positive spends latency and a third-party request on nothing.

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::commands::ChatMessage;

/// The three-way search decision emitted by the pre-pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchDecision {
    /// Answer directly from the model; no retrieval.
    No,
    /// Answer from source blocks already fetched earlier in this conversation.
    Cached,
    /// Run the retrieval pipeline.
    Web,
}

/// The pre-pass result: the decision plus the rewritten question and queries
/// used by the retrieval stages when the decision is `Cached` or `Web`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrePassDecision {
    pub decision: SearchDecision,
    pub standalone_question: String,
    pub queries: Vec<String>,
}

/// Why a pre-pass inference call failed at the transport level. Distinct from a
/// merely unparseable body, which is handled in-band by degrading to `No`.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum InferenceError {
    /// The engine request failed (connect, HTTP, read).
    #[error("inference request failed: {0}")]
    Request(String),
    /// The request was cancelled (new turn or user cancel).
    #[error("cancelled")]
    Cancelled,
}

/// Upper bound on keyword queries the pre-pass may emit, matching the schema
/// and the downstream fan-out cap.
const MAX_QUERIES: usize = 3;

/// Injectable pre-pass inference. The orchestrator depends on this trait so its
/// branch logic is tested with [`FakePrePass`]; the builtin engine backing lives
/// in the coverage-excluded [`BuiltinPrePass`].
#[async_trait]
pub trait PrePass: Send + Sync {
    /// Decides search intent for `latest_user_message` given the conversation
    /// so far. Never returns a bad-JSON error: an unparseable model response
    /// degrades in-band to [`SearchDecision::No`].
    async fn decide(
        &self,
        chat_system_prompt: &str,
        history: &[ChatMessage],
        latest_user_message: &str,
        today: &str,
        cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError>;
}

/// The production [`PrePass`], backed by the bundled `llama-server` engine over
/// its OpenAI-compatible `/v1` endpoint. Excluded from the coverage gate: it is
/// thin glue over [`crate::openai::request_openai_json`] and the pure helpers
/// ([`build_prepass_messages`], [`prepass_schema`], [`parse_prepass`],
/// [`prepass_or_no`]), which are all tested directly.
pub struct BuiltinPrePass {
    client: reqwest::Client,
    /// Engine base URL, e.g. `http://127.0.0.1:<port>`.
    base_url: String,
    /// Installed model id resolved from the manifest.
    model: String,
    /// Per-call wall-clock timeout (seconds).
    timeout_secs: u64,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl BuiltinPrePass {
    pub fn new(
        client: reqwest::Client,
        base_url: impl Into<String>,
        model: impl Into<String>,
        timeout_secs: u64,
    ) -> Self {
        Self {
            client,
            base_url: base_url.into(),
            model: model.into(),
            timeout_secs,
        }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[async_trait]
impl PrePass for BuiltinPrePass {
    async fn decide(
        &self,
        chat_system_prompt: &str,
        history: &[ChatMessage],
        latest_user_message: &str,
        today: &str,
        cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError> {
        let messages =
            build_prepass_messages(chat_system_prompt, history, latest_user_message, today);
        let raw = crate::openai::request_openai_json(
            &self.base_url,
            &self.model,
            &self.client,
            messages,
            prepass_schema(),
            None,
            self.timeout_secs,
            crate::config::defaults::PREPASS_MAX_TOKENS,
            crate::openai::V1Flavor::Builtin,
            cancel,
        )
        .await;
        match raw {
            // A 2xx response with unparseable JSON degrades to `No` in-band.
            Ok(content) => Ok(prepass_or_no(parse_prepass(&content), latest_user_message)),
            Err(crate::openai::OpenAiError::Cancelled) => Err(InferenceError::Cancelled),
            Err(other) => Err(InferenceError::Request(format!("{other:?}"))),
        }
    }
}

/// The instruction appended to a copy of the latest user turn. Kept as a strict
/// suffix so the system+history prefix stays identical to the writer prompt.
const DECISION_INSTRUCTION: &str = "\n\n---\nSilently decide whether answering the message above needs a web search. Output ONLY a JSON object with these fields:\n- \"search\": \"no\" if you can answer directly from your own knowledge or this conversation; \"cached\" if earlier turns already fetched the needed web sources; \"web\" if fresh web results are required (current events, prices, releases, anything after your knowledge cutoff, or facts you are unsure of).\n- \"standalone_question\": the message above rewritten as a single self-contained question that needs no conversation context.\n- \"queries\": 1 to 3 short keyword search queries (not full sentences). Always include today's date context where the question is time-sensitive.\nDo not answer the question itself.";

/// Builds the `response_format` JSON schema constraining the pre-pass output.
pub(crate) fn prepass_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "search": { "type": "string", "enum": ["no", "cached", "web"] },
            "standalone_question": { "type": "string" },
            "queries": {
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1,
                "maxItems": MAX_QUERIES
            }
        },
        "required": ["search", "standalone_question", "queries"],
        "additionalProperties": false
    })
}

/// Assembles the pre-pass message array: the chat system prompt, the history
/// verbatim, then the latest user turn with the decision instruction and date
/// appended. Sharing the system+history prefix with the writer prompt keeps the
/// KV cache warm across the two calls (see module docs).
pub(crate) fn build_prepass_messages(
    chat_system_prompt: &str,
    history: &[ChatMessage],
    latest_user_message: &str,
    today: &str,
) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(history.len() + 2);
    messages.push(ChatMessage {
        role: "system".to_string(),
        content: chat_system_prompt.to_string(),
        images: None,
    });
    messages.extend(history.iter().cloned());
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: format!("{latest_user_message}{DECISION_INSTRUCTION}\n\nToday's date is {today}."),
        images: None,
    });
    messages
}

/// The wire shape the grammar constrains the model to. Parsed leniently: the
/// `search` string is normalised to [`SearchDecision`] here rather than via
/// serde so an unexpected casing does not hard-fail the whole response.
#[derive(serde::Deserialize)]
struct PrePassWire {
    search: String,
    #[serde(default)]
    standalone_question: String,
    #[serde(default)]
    queries: Vec<String>,
}

/// Parses a raw pre-pass response into a normalised decision, or `None` when
/// the body is not the expected JSON shape or the `search` value is unknown.
/// Queries are trimmed, de-duplicated case-insensitively, emptied entries
/// dropped, and capped at [`MAX_QUERIES`]; `standalone_question` is trimmed.
pub(crate) fn parse_prepass(raw: &str) -> Option<PrePassDecision> {
    let wire: PrePassWire = serde_json::from_str(raw.trim()).ok()?;
    let decision = match wire.search.trim().to_ascii_lowercase().as_str() {
        "no" => SearchDecision::No,
        "cached" => SearchDecision::Cached,
        "web" => SearchDecision::Web,
        _ => return None,
    };
    Some(PrePassDecision {
        decision,
        standalone_question: wire.standalone_question.trim().to_string(),
        queries: normalize_queries(wire.queries),
    })
}

/// Resolves the final decision from a parse attempt, applying the failure
/// policy and backfilling required fields:
/// - a failed parse (`None`) becomes a `No` decision answering `latest`;
/// - a `Cached`/`Web` decision with no usable queries or an empty standalone
///   backfills from `latest` so the retrieval stages always have a query.
pub(crate) fn prepass_or_no(parsed: Option<PrePassDecision>, latest: &str) -> PrePassDecision {
    let mut decision = match parsed {
        Some(decision) => decision,
        None => {
            return PrePassDecision {
                decision: SearchDecision::No,
                standalone_question: latest.trim().to_string(),
                queries: Vec::new(),
            }
        }
    };
    if decision.standalone_question.trim().is_empty() {
        decision.standalone_question = latest.trim().to_string();
    }
    if matches!(
        decision.decision,
        SearchDecision::Web | SearchDecision::Cached
    ) && decision.queries.is_empty()
    {
        decision.queries = vec![decision.standalone_question.clone()];
    }
    decision
}

/// Normalises a raw query list: trim, drop empties, de-duplicate
/// case-insensitively preserving first-seen order, cap at [`MAX_QUERIES`].
fn normalize_queries(raw: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for query in raw {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_ascii_lowercase()) {
            out.push(trimmed.to_string());
            if out.len() == MAX_QUERIES {
                break;
            }
        }
    }
    out
}

/// Scriptable [`PrePass`] for unit tests: returns a fixed decision or error so
/// the orchestrator's branch logic is driven without a live engine.
#[cfg(test)]
pub(crate) struct FakePrePass {
    result: Result<PrePassDecision, InferenceError>,
}

#[cfg(test)]
impl FakePrePass {
    pub(crate) fn returning(result: Result<PrePassDecision, InferenceError>) -> Self {
        Self { result }
    }
}

#[cfg(test)]
#[async_trait]
impl PrePass for FakePrePass {
    async fn decide(
        &self,
        _chat_system_prompt: &str,
        _history: &[ChatMessage],
        _latest_user_message: &str,
        _today: &str,
        _cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError> {
        self.result.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
            images: None,
        }
    }

    // ── schema ──────────────────────────────────────────────────────────────

    #[test]
    fn schema_declares_enum_and_query_bounds() {
        let s = prepass_schema();
        assert_eq!(s["properties"]["search"]["enum"][0], "no");
        assert_eq!(s["properties"]["queries"]["maxItems"], MAX_QUERIES);
        assert_eq!(s["additionalProperties"], false);
    }

    // ── message assembly ────────────────────────────────────────────────────

    #[test]
    fn messages_share_system_and_history_prefix() {
        let history = vec![user("earlier question"), {
            let mut m = user("earlier answer");
            m.role = "assistant".into();
            m
        }];
        let msgs = build_prepass_messages("PERSONA", &history, "and now?", "2026-07-05");
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].content, "PERSONA");
        assert_eq!(msgs[1].content, "earlier question");
        assert_eq!(msgs[2].content, "earlier answer");
        assert_eq!(msgs[3].role, "user");
        assert!(msgs[3].content.starts_with("and now?"));
        assert!(msgs[3].content.contains("2026-07-05"));
        assert!(msgs[3].content.contains("\"search\""));
    }

    // ── parse ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_reads_web_decision() {
        let raw = r#"{"search":"web","standalone_question":"weather in Paris today","queries":["paris weather today"]}"#;
        let d = parse_prepass(raw).unwrap();
        assert_eq!(d.decision, SearchDecision::Web);
        assert_eq!(d.standalone_question, "weather in Paris today");
        assert_eq!(d.queries, vec!["paris weather today"]);
    }

    #[test]
    fn parse_reads_no_and_cached() {
        assert_eq!(
            parse_prepass(r#"{"search":"no","standalone_question":"hi","queries":["x"]}"#)
                .unwrap()
                .decision,
            SearchDecision::No
        );
        assert_eq!(
            parse_prepass(r#"{"search":"cached","standalone_question":"hi","queries":["x"]}"#)
                .unwrap()
                .decision,
            SearchDecision::Cached
        );
    }

    #[test]
    fn parse_is_case_insensitive_on_decision() {
        assert_eq!(
            parse_prepass(r#"{"search":"WEB","standalone_question":"q","queries":["a"]}"#)
                .unwrap()
                .decision,
            SearchDecision::Web
        );
    }

    #[test]
    fn parse_rejects_unknown_decision() {
        assert!(
            parse_prepass(r#"{"search":"maybe","standalone_question":"q","queries":["a"]}"#)
                .is_none()
        );
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(parse_prepass("not json at all").is_none());
    }

    #[test]
    fn parse_normalizes_queries() {
        let raw =
            r#"{"search":"web","standalone_question":"q","queries":["  A ","a","B","","C","D"]}"#;
        let d = parse_prepass(raw).unwrap();
        // trimmed, case-insensitive dedupe ("A"=="a"), empty dropped, capped at 3
        assert_eq!(d.queries, vec!["A", "B", "C"]);
    }

    // ── failure policy / backfill ───────────────────────────────────────────

    #[test]
    fn none_becomes_no_answering_latest() {
        let d = prepass_or_no(None, "what is 2 + 2");
        assert_eq!(d.decision, SearchDecision::No);
        assert_eq!(d.standalone_question, "what is 2 + 2");
        assert!(d.queries.is_empty());
    }

    #[test]
    fn web_with_empty_queries_backfills_from_standalone() {
        let parsed = Some(PrePassDecision {
            decision: SearchDecision::Web,
            standalone_question: "capital of France".into(),
            queries: vec![],
        });
        let d = prepass_or_no(parsed, "and there?");
        assert_eq!(d.decision, SearchDecision::Web);
        assert_eq!(d.queries, vec!["capital of France"]);
    }

    #[test]
    fn web_with_empty_standalone_backfills_from_latest() {
        let parsed = Some(PrePassDecision {
            decision: SearchDecision::Web,
            standalone_question: "   ".into(),
            queries: vec!["q".into()],
        });
        let d = prepass_or_no(parsed, "the real question");
        assert_eq!(d.standalone_question, "the real question");
        assert_eq!(d.queries, vec!["q"]);
    }

    #[test]
    fn no_decision_passes_through_without_query_backfill() {
        let parsed = Some(PrePassDecision {
            decision: SearchDecision::No,
            standalone_question: "hello".into(),
            queries: vec![],
        });
        let d = prepass_or_no(parsed, "hello");
        assert_eq!(d.decision, SearchDecision::No);
        assert!(d.queries.is_empty());
    }

    // ── trait / fake ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fake_prepass_returns_scripted_decision() {
        let want = PrePassDecision {
            decision: SearchDecision::Web,
            standalone_question: "q".into(),
            queries: vec!["q".into()],
        };
        let fake = FakePrePass::returning(Ok(want.clone()));
        let got = fake
            .decide("sys", &[], "q", "2026-07-05", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn fake_prepass_propagates_error() {
        let fake = FakePrePass::returning(Err(InferenceError::Cancelled));
        assert_eq!(
            fake.decide("sys", &[], "q", "2026-07-05", &CancellationToken::new())
                .await,
            Err(InferenceError::Cancelled)
        );
    }
}
