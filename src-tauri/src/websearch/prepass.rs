//! The classifier: stage two of the search decision, for the ambiguous middle
//! the deterministic [`super::prefilter`] could not resolve.
//!
//! One grammar-constrained LLM call decides whether the web is needed and, if
//! so, rewrites the (possibly context-dependent) message into a standalone
//! question plus 1-3 keyword queries. The model is constrained by a strict
//! `response_format` JSON schema, verified to coexist with the engine's
//! reasoning-control flow, so even small local models emit a parseable shape.
//!
//! ## Prompt shape (persona-free by design)
//!
//! The classifier runs under its **own** short system prompt, NOT the chat
//! persona. This is deliberate: the chat persona instructs the model how to
//! behave toward the user (including, historically, to deflect current-info
//! questions), which biases a decision made inside that context. Decoupling the
//! decision from the persona is the correctness fix at the heart of this
//! module's redesign. A few-shot header and an explicit "when unsure of your own
//! freshness, choose web" rule bias the small local models toward searching
//! rather than answering from stale memory. Only the last few conversation turns
//! are embedded, as plain text for pronoun resolution, so a follow-up like "what
//! about there?" can still be rewritten into a standalone query.
//!
//! Dropping the persona prefix costs a small extra prefill on the ambiguous
//! turns that reach this stage (the engine runs `--parallel 1`), traded
//! knowingly for a correct decision. The pre-filter already resolves the obvious
//! turns with no model call at all, so this stage fires far less often than a
//! per-message pre-pass would.
//!
//! ## Failure policy
//!
//! A malformed or unparseable response degrades to [`SearchDecision::No`]
//! (answer directly) rather than a spurious search: a false negative is cheap
//! and recoverable, whereas a false positive spends latency and a third-party
//! request on nothing.

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

/// Which retrieval tier the classifier judged best for this turn. Advisory only:
/// the orchestrator combines it with deterministic gates (a vertical may still
/// run on its own signal, and the wiki tier is additionally volatility-guarded),
/// and an unknown or missing route parses to [`SearchRoute::Web`] (the general
/// engine tier), never a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchRoute {
    /// Current weather or forecast for a location → weather vertical.
    Weather,
    /// Current events, sports results/status, recent developments → news vertical.
    News,
    /// Stable definitional or historical facts → Wikipedia vertical.
    Wiki,
    /// Everything else (software versions, prices, niche live facts) → engines.
    Web,
}

impl SearchRoute {
    /// Normalises the raw `route` string from the model to a [`SearchRoute`].
    /// Any value that is not one of the four known tiers (including an empty or
    /// missing field) maps to [`SearchRoute::Web`], so a malformed route never
    /// fails the turn and only ever falls back to the general engine tier.
    pub(crate) fn from_wire(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "weather" => SearchRoute::Weather,
            "news" => SearchRoute::News,
            "wiki" => SearchRoute::Wiki,
            _ => SearchRoute::Web,
        }
    }
}

/// The pre-pass result: the decision, the routing hint, and the rewritten
/// question and queries used by the retrieval stages when the decision is
/// `Cached` or `Web`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrePassDecision {
    pub decision: SearchDecision,
    pub route: SearchRoute,
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
    /// so far, under the classifier's own persona-free prompt. Never returns a
    /// bad-JSON error: an unparseable model response degrades in-band to
    /// [`SearchDecision::No`].
    async fn decide(
        &self,
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
        history: &[ChatMessage],
        latest_user_message: &str,
        today: &str,
        cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError> {
        let messages = build_prepass_messages(history, latest_user_message, today);
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

/// The classifier's own system prompt: a short, persona-free routing role with a
/// few-shot header and an explicit bias toward searching when the model cannot
/// vouch for its own freshness. Kept separate from the chat persona on purpose
/// (see module docs): the persona must not colour the decision.
///
/// The leading `Reasoning: low` line is the gpt-oss (harmony) reasoning-effort
/// directive: without it the model spends 1000+ chain-of-thought tokens on a
/// three-way classification and can blow the call timeout (observed live at
/// ~63 tok/s decode). Inert plain text for every other model family.
const CLASSIFIER_SYSTEM: &str = "Reasoning: low\n\nYou are a retrieval-routing classifier inside a local AI assistant. Your only job is to decide whether answering the user's latest message needs a fresh web search, to pick which source best answers it, and if so to rewrite it into a standalone search query. You never answer the message itself.\n\nOutput ONLY a JSON object: {\"search\": \"no\"|\"cached\"|\"web\", \"route\": \"weather\"|\"news\"|\"wiki\"|\"web\", \"standalone_question\": \"...\", \"queries\": [\"...\"]}.\n\nChoose \"search\":\n- \"web\" when a good answer needs information that changes over time or is past your training cutoff: news, prices, weather, sports results, software versions, releases, schedules, who currently holds a role, or any live fact; OR when you are not confident your own knowledge is current and correct.\n- \"cached\" when the needed web sources were already fetched earlier in this same conversation.\n- \"no\" only when you can answer confidently and correctly from stable general knowledge or from the conversation alone.\nWhen you are unsure whether your knowledge is up to date, choose \"web\": a needless search is far cheaper than a confidently wrong answer.\n\nChoose \"route\" (which source best answers it):\n- \"weather\" for current weather or forecast for a place.\n- \"news\" for current events, sports results or status, elections, and anything asking the latest, current, or recent state of an evolving topic (a tournament, a race, a conflict, a company).\n- \"wiki\" for stable definitional or historical facts that do not change from month to month.\n- \"web\" for everything else (software versions, prices, product specs, niche live facts).\nWhen a question is about the present state of an ongoing event, route \"news\", never \"wiki\", even if it is phrased like \"what is ...\". Always set a route, even when search is \"no\".\n\n\"standalone_question\": the latest message rewritten as one self-contained question, resolving pronouns and references from the conversation.\n\"queries\": 1 to 3 short keyword search queries, not full sentences.\n\nExamples (message -> JSON):\n\"who is the CEO of OpenAI right now\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"who is the current CEO of OpenAI\",\"queries\":[\"openai ceo\"]}\n\"what is the boiling point of water\" -> {\"search\":\"no\",\"route\":\"wiki\",\"standalone_question\":\"what is the boiling point of water\",\"queries\":[\"boiling point of water\"]}\n\"what is photosynthesis\" -> {\"search\":\"web\",\"route\":\"wiki\",\"standalone_question\":\"what is photosynthesis\",\"queries\":[\"photosynthesis\"]}\n\"weather in Paris\" -> {\"search\":\"web\",\"route\":\"weather\",\"standalone_question\":\"what is the current weather in Paris\",\"queries\":[\"paris weather\"]}\n\"what's the latest status of the World Cup 2026\" -> {\"search\":\"web\",\"route\":\"news\",\"standalone_question\":\"what is the current status of the 2026 World Cup\",\"queries\":[\"world cup 2026 status\"]}\n\"who won the most recent F1 race\" -> {\"search\":\"web\",\"route\":\"news\",\"standalone_question\":\"who won the most recent Formula 1 race\",\"queries\":[\"latest f1 race winner\"]}\n\"write a short poem about autumn\" -> {\"search\":\"no\",\"route\":\"web\",\"standalone_question\":\"write a short poem about autumn\",\"queries\":[\"autumn poem\"]}\n(after discussing France) \"and its population?\" -> {\"search\":\"no\",\"route\":\"wiki\",\"standalone_question\":\"what is the population of France\",\"queries\":[\"france population\"]}\n(after discussing the US president) \"what about Argentina?\" -> {\"search\":\"web\",\"route\":\"web\",\"standalone_question\":\"who is the current president of Argentina\",\"queries\":[\"argentina president\"]}";

/// The trailing instruction on the classifier's user turn, after the optional
/// conversation block and the latest message.
const CLASSIFIER_INSTRUCTION: &str =
    "Decide for the latest message and output only the JSON object.";

/// Header introducing the embedded conversation context in the classifier's user
/// turn. The turns are context for pronoun resolution only, never instructions.
const CONVERSATION_HEADER: &str = "Conversation so far (context only):";

/// Builds the `response_format` JSON schema constraining the pre-pass output.
pub(crate) fn prepass_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "search": { "type": "string", "enum": ["no", "cached", "web"] },
            "route": { "type": "string", "enum": ["weather", "news", "wiki", "web"] },
            "standalone_question": { "type": "string" },
            "queries": {
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1,
                "maxItems": MAX_QUERIES
            }
        },
        "required": ["search", "route", "standalone_question", "queries"],
        "additionalProperties": false
    })
}

/// Assembles the classifier message array: the classifier's own system prompt,
/// then a single user turn that embeds the last few conversation turns (for
/// pronoun resolution), the latest message, today's date, and the output
/// instruction. The chat persona is intentionally absent (see module docs).
pub(crate) fn build_prepass_messages(
    history: &[ChatMessage],
    latest_user_message: &str,
    today: &str,
) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "system".to_string(),
            content: CLASSIFIER_SYSTEM.to_string(),
            images: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: build_classifier_user_turn(history, latest_user_message, today),
            images: None,
        },
    ]
}

/// Builds the classifier's single user turn: an optional conversation-context
/// block (last [`CLASSIFIER_HISTORY_TURNS`] turns as plain `Role: text` lines),
/// the latest message, today's date, and the trailing output instruction.
fn build_classifier_user_turn(
    history: &[ChatMessage],
    latest_user_message: &str,
    today: &str,
) -> String {
    let mut out = String::new();
    let context = recent_history_block(history);
    if !context.is_empty() {
        out.push_str(CONVERSATION_HEADER);
        out.push('\n');
        out.push_str(&context);
        out.push_str("\n\n");
    }
    out.push_str("Latest message: ");
    out.push_str(latest_user_message.trim());
    out.push_str("\n\nToday's date is ");
    out.push_str(today);
    out.push_str(".\n");
    out.push_str(CLASSIFIER_INSTRUCTION);
    out
}

/// Formats the last [`CLASSIFIER_HISTORY_TURNS`] conversation turns as plain
/// `Role: text` lines for context. Returns an empty string when there is no
/// history. Message images are ignored: the classifier reasons over text only.
fn recent_history_block(history: &[ChatMessage]) -> String {
    let start = history
        .len()
        .saturating_sub(crate::config::defaults::CLASSIFIER_HISTORY_TURNS);
    history[start..]
        .iter()
        .map(|m| {
            let role = if m.role == "assistant" {
                "Assistant"
            } else {
                "User"
            };
            format!("{role}: {}", m.content.trim())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The wire shape the grammar constrains the model to. Parsed leniently: the
/// `search` string is normalised to [`SearchDecision`] here rather than via
/// serde so an unexpected casing does not hard-fail the whole response.
#[derive(serde::Deserialize)]
struct PrePassWire {
    search: String,
    #[serde(default)]
    route: String,
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
        route: SearchRoute::from_wire(&wire.route),
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
                route: SearchRoute::Web,
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
        assert_eq!(s["properties"]["route"]["enum"][0], "weather");
        assert_eq!(s["properties"]["route"]["enum"][3], "web");
        assert_eq!(s["properties"]["queries"]["maxItems"], MAX_QUERIES);
        assert!(s["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "route"));
        assert_eq!(s["additionalProperties"], false);
    }

    // ── route parsing ─────────────────────────────────────────────────────────

    #[test]
    fn route_from_wire_maps_known_tiers() {
        assert_eq!(SearchRoute::from_wire("weather"), SearchRoute::Weather);
        assert_eq!(SearchRoute::from_wire("news"), SearchRoute::News);
        assert_eq!(SearchRoute::from_wire("wiki"), SearchRoute::Wiki);
        assert_eq!(SearchRoute::from_wire("web"), SearchRoute::Web);
        // Case-insensitive and whitespace-tolerant.
        assert_eq!(SearchRoute::from_wire("  NEWS "), SearchRoute::News);
    }

    #[test]
    fn route_from_wire_unknown_or_empty_falls_back_to_web() {
        assert_eq!(SearchRoute::from_wire("encyclopedia"), SearchRoute::Web);
        assert_eq!(SearchRoute::from_wire(""), SearchRoute::Web);
    }

    #[test]
    fn parse_reads_route_when_present() {
        let raw = r#"{"search":"web","route":"news","standalone_question":"q","queries":["a"]}"#;
        assert_eq!(parse_prepass(raw).unwrap().route, SearchRoute::News);
    }

    #[test]
    fn parse_defaults_route_to_web_when_missing_or_invalid() {
        // Missing route field entirely.
        let missing = r#"{"search":"web","standalone_question":"q","queries":["a"]}"#;
        assert_eq!(parse_prepass(missing).unwrap().route, SearchRoute::Web);
        // Present but not a known tier.
        let invalid =
            r#"{"search":"web","route":"maps","standalone_question":"q","queries":["a"]}"#;
        assert_eq!(parse_prepass(invalid).unwrap().route, SearchRoute::Web);
    }

    // ── message assembly ────────────────────────────────────────────────────

    #[test]
    fn messages_use_persona_free_classifier_system_prompt() {
        let msgs = build_prepass_messages(&[], "who won", "2026-07-05");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        // The classifier prompt, not the chat persona.
        assert_eq!(msgs[0].content, CLASSIFIER_SYSTEM);
        assert!(msgs[0].content.contains("retrieval-routing classifier"));
        assert!(msgs[0].content.contains("choose \"web\""));
        assert_eq!(msgs[1].role, "user");
        assert!(msgs[1].content.contains("Latest message: who won"));
        assert!(msgs[1].content.contains("2026-07-05"));
    }

    #[test]
    fn user_turn_embeds_recent_history_for_pronoun_resolution() {
        let history = vec![user("what is the capital of France"), {
            let mut m = user("Paris.");
            m.role = "assistant".into();
            m
        }];
        let msgs = build_prepass_messages(&history, "and its population?", "2026-07-05");
        let turn = &msgs[1].content;
        assert!(turn.contains("Conversation so far"));
        assert!(turn.contains("User: what is the capital of France"));
        assert!(turn.contains("Assistant: Paris."));
        assert!(turn.contains("Latest message: and its population?"));
    }

    #[test]
    fn user_turn_omits_conversation_block_when_no_history() {
        let msgs = build_prepass_messages(&[], "hello", "2026-07-05");
        assert!(!msgs[1].content.contains("Conversation so far"));
        assert!(msgs[1].content.starts_with("Latest message: hello"));
    }

    #[test]
    fn history_block_keeps_only_the_most_recent_turns() {
        // More turns than the cap: only the last CLASSIFIER_HISTORY_TURNS survive.
        let cap = crate::config::defaults::CLASSIFIER_HISTORY_TURNS;
        let history: Vec<ChatMessage> = (0..cap + 3).map(|i| user(&format!("turn {i}"))).collect();
        let block = recent_history_block(&history);
        assert!(!block.contains("turn 0"));
        assert!(block.contains(&format!("turn {}", cap + 2)));
        assert_eq!(block.lines().count(), cap);
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
            route: SearchRoute::Web,
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
            route: SearchRoute::Web,
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
            route: SearchRoute::Web,
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
            route: SearchRoute::Web,
            standalone_question: "q".into(),
            queries: vec!["q".into()],
        };
        let fake = FakePrePass::returning(Ok(want.clone()));
        let got = fake
            .decide(&[], "q", "2026-07-05", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn fake_prepass_propagates_error() {
        let fake = FakePrePass::returning(Err(InferenceError::Cancelled));
        assert_eq!(
            fake.decide(&[], "q", "2026-07-05", &CancellationToken::new())
                .await,
            Err(InferenceError::Cancelled)
        );
    }
}
