//! LLM operations for the `/search` pipeline.
//!
//! Two concerns live here:
//! 1. The **merged router+judge call** (`call_router_merged`) and the
//!    **universal sufficiency judge call** (`call_judge`) used by the agentic
//!    pipeline via the [`RouterJudgeCaller`] and [`JudgeCaller`] traits.
//! 2. Prompt-assembly helpers that produce the message array fed to the
//!    streaming answer stage (either `answer_from_context` or `search`).
//!
//! All functions are pure with respect to external state (no globals, no
//! hidden side effects) and accept their dependencies explicitly for
//! testability.

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::commands::ChatMessage;

use super::types::{
    Action, JudgeVerdict, RouterJudgeOutput, SearchError, SearxResult, Sufficiency,
};

/// Synthesis system prompt: instructs the answering LLM to cite sources and
/// avoid meta-commentary over the reference material.
pub const SYNTHESIS_SYSTEM_PROMPT: &str = include_str!("../../prompts/search_synthesis.txt");

/// Stage-specific judge prompts. The snippet stage sees short SearXNG
/// excerpts (small payload, fast triage rubric); the chunk stage sees full
/// reader-extracted passages (large payload, evidence-grading rubric with
/// worked examples). Splitting them improves verdict quality without growing
/// snippet-stage cost. Both prompts are written for reasoning-before-verdict
/// emission; the schema in `judge_output_schema` enforces the property order.
pub const SNIPPET_JUDGE_SYSTEM_PROMPT: &str =
    include_str!("../../prompts/search_snippet_judge.txt");
pub const CHUNK_JUDGE_SYSTEM_PROMPT: &str = include_str!("../../prompts/search_chunk_judge.txt");

/// Identifies which retrieval stage a judge call is judging. Selects the
/// stage-specific system prompt inside `call_judge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgeStage {
    /// SearXNG snippets (short). Uses `SNIPPET_JUDGE_SYSTEM_PROMPT`.
    Snippet,
    /// Reader-extracted chunks (long). Uses `CHUNK_JUDGE_SYSTEM_PROMPT`.
    Chunk,
}

impl JudgeStage {
    fn system_prompt(self) -> &'static str {
        match self {
            JudgeStage::Snippet => SNIPPET_JUDGE_SYSTEM_PROMPT,
            JudgeStage::Chunk => CHUNK_JUDGE_SYSTEM_PROMPT,
        }
    }
}

/// Merged router+judge prompt. Instructs the model to emit a single JSON
/// object covering both routing classification and history-sufficiency
/// assessment.
pub const SEARCH_PLAN_SYSTEM_PROMPT: &str = include_str!("../../prompts/search_plan.txt");

/// Extra guardrails for the history-only answer branch. Appended after the
/// normal chat system prompt so this branch cannot silently answer from model
/// priors when the router makes a bad sufficiency call.
const HISTORY_ONLY_SYSTEM_APPENDIX: &str = "\n\nYou are answering from the prior conversation only. Use only facts that already appear in earlier turns of this chat. Do not use your training knowledge, general world knowledge, the current date, or any external information. The latest user message is the question to answer, not evidence. If the prior conversation does not already contain the answer, reply exactly with: I can't answer that from this conversation alone.";

/// Hard timeout for the non-streaming router call. Passed by tests that call
/// call_router_merged directly.
#[allow(dead_code)]
pub const ROUTER_TIMEOUT_SECS: u64 = 45;

/// Cap on the router response length. Enough for a clarification question
/// with several suggestions; prevents runaway generation when the model
/// fails to produce valid JSON quickly.
pub const ROUTER_MAX_TOKENS: i32 = 512;

fn router_output_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["clarify", "proceed"]
            },
            "clarifying_question": {
                "anyOf": [
                    { "type": "string" },
                    { "type": "null" }
                ]
            },
            "history_sufficiency": {
                "anyOf": [
                    {
                        "type": "string",
                        "enum": ["sufficient", "partial", "insufficient"]
                    },
                    { "type": "null" }
                ]
            },
            "optimized_query": {
                "anyOf": [
                    { "type": "string" },
                    { "type": "null" }
                ]
            }
        },
        "required": [
            "action",
            "clarifying_question",
            "history_sufficiency",
            "optimized_query"
        ],
        "additionalProperties": false
    })
}

/// JSON schema for the sufficiency judge response. Passed to Ollama via the
/// `format` field so the model is constrained to emit a JSON object that
/// matches `JudgeVerdict` exactly. Without this, small local models often
/// emit shape variations (`partial` instead of `Partial`, missing
/// `gap_queries`, prose wrappers) that defeat the parser even with JSON
/// mode enabled.
fn judge_output_schema() -> serde_json::Value {
    // Property order is reasoning -> sufficiency -> gap_queries. Constrained
    // decoders that respect schema property order (Ollama with llama.cpp grammar
    // backend) emit fields in that sequence, which forces the model to write
    // its analysis BEFORE committing to a label. Empirical work on LLM-as-
    // judge consistently shows reasoning-first emission improves human-judge
    // agreement vs verdict-first emission. See Arize / MT-Bench / EMNLP 2025
    // design-choices studies. The `required` array is also reasoning-first so
    // serde_json deserialization order matches the wire-emitted order even on
    // backends that lexicographically reorder properties.
    serde_json::json!({
        "type": "object",
        "properties": {
            "reasoning": { "type": "string" },
            "sufficiency": {
                "type": "string",
                "enum": ["sufficient", "partial", "insufficient"]
            },
            "gap_queries": {
                "type": "array",
                "items": { "type": "string" }
            }
        },
        "required": ["reasoning", "sufficiency", "gap_queries"],
        "additionalProperties": false
    })
}

// ─── Shared input/output types ───────────────────────────────────────────────

/// A single evidence source passed to the universal sufficiency judge. Used by
/// [`call_judge`] to build the user-turn content from either SearXNG snippets
/// (initial round) or Trafilatura reader chunks (subsequent rounds).
///
/// Free-standing so the pipeline can construct instances from whichever source
/// stage is currently active without depending on internal snippet or chunk
/// types.
#[derive(Debug, Clone)]
pub struct JudgeSource {
    /// Display title of the source document.
    pub title: String,
    /// Canonical URL of the source document.
    pub url: String,
    /// Extracted text content: either a SearXNG snippet or a reader chunk.
    pub text: String,
}

// ─── Router request / response wire types ───────────────────────────────────

#[derive(Serialize)]
struct RouterOptions {
    /// Deterministic sampling so classification is reproducible.
    temperature: f64,
    top_p: f64,
    top_k: u32,
    num_predict: i32,
    /// Must match the value sent by warmup and the chat path so Ollama keeps
    /// the warmed-up model resident. Omitting it would cause Ollama to reload
    /// the model with its default 4096-token window.
    num_ctx: u32,
}

#[derive(Serialize)]
struct OllamaJsonRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    format: serde_json::Value,
    options: RouterOptions,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct OllamaResponseBody {
    message: OllamaResponseMessage,
}

// ─── Shared HTTP helper ──────────────────────────────────────────────────────

/// Sends a single non-streaming JSON-mode chat request to Ollama and returns
/// the raw `message.content` string from the response.
///
/// Used by [`call_router`], [`call_router_merged`], and [`call_judge`] so all
/// three share the same request/response wiring without duplication. Each
/// caller is responsible for deserializing the returned string into its own
/// output type.
///
/// `timeout_secs` is the per-call wall-clock limit. Production code passes
/// the router/judge timeout fields from
/// [`SearchRuntimeConfig`](super::config::SearchRuntimeConfig); tests pass
/// the corresponding `DEFAULT_*` constants from [`crate::config::defaults`].
#[allow(clippy::too_many_arguments)]
async fn request_json(
    endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    messages: Vec<ChatMessage>,
    format: serde_json::Value,
    cancel_token: &CancellationToken,
    timeout_secs: u64,
    num_ctx: u32,
) -> Result<String, SearchError> {
    let body = OllamaJsonRequest {
        model: model.to_string(),
        messages,
        stream: false,
        format,
        options: RouterOptions {
            temperature: 0.0,
            top_p: 1.0,
            top_k: 1,
            num_predict: ROUTER_MAX_TOKENS,
            num_ctx,
        },
    };

    let request = client
        .post(endpoint)
        .json(&body)
        .timeout(std::time::Duration::from_secs(timeout_secs));

    let response = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => return Err(SearchError::Cancelled),
        res = request.send() => res.map_err(|_| SearchError::LlmUnavailable)?,
    };

    if !response.status().is_success() {
        return Err(SearchError::LlmHttp(response.status().as_u16()));
    }

    let parsed: OllamaResponseBody = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => return Err(SearchError::Cancelled),
        body = response.json() => body.map_err(|_| SearchError::LlmBadJson)?,
    };

    Ok(parsed.message.content)
}

// ─── Merged router+judge call ────────────────────────────────────────────────

/// Merged router+judge call that returns [`RouterJudgeOutput`] in a single
/// Ollama roundtrip: routing classification plus, when proceeding, a
/// sufficiency verdict on conversation history and an optimized search query.
///
/// Uses [`SEARCH_PLAN_SYSTEM_PROMPT`] with `{{TODAY}}` replaced by the
/// supplied `today` string so the model is anchored to the real calendar date.
/// Pass the result of `pipeline::today_iso()` at the call site, or a fixed
/// string in tests.
///
/// Added alongside the existing [`call_router`] so the pipeline can migrate
/// incrementally. Task 13 swaps the call site; Task 16 retires the legacy path.
///
/// # Errors
/// - [`SearchError::Cancelled`] - token cancelled before or during the request.
/// - [`SearchError::LlmUnavailable`] - transport failure.
/// - [`SearchError::LlmHttp`] - non-2xx status from Ollama.
///
/// Note: this function retries once with a stricter user-message suffix when
/// the first router response cannot be parsed. If the schema still cannot be
/// recovered, it returns [`SearchError::Router`] instead of silently forcing a
/// web search, because malformed router output should fail closed.
#[allow(clippy::too_many_arguments)]
pub async fn call_router_merged(
    endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    history: &[ChatMessage],
    query: &str,
    today: &str,
    cancel_token: &CancellationToken,
    timeout_secs: u64,
    num_ctx: u32,
) -> Result<RouterJudgeOutput, SearchError> {
    if cancel_token.is_cancelled() {
        return Err(SearchError::Cancelled);
    }

    let system = SEARCH_PLAN_SYSTEM_PROMPT.replace("{{TODAY}}", today);

    // First attempt: standard prompt.
    let messages = build_router_messages(&system, history, query);
    let raw = request_json(
        endpoint,
        model,
        client,
        messages,
        router_output_schema(),
        cancel_token,
        timeout_secs,
        num_ctx,
    )
    .await?;
    if let Some(output) = try_parse_router_output(&raw) {
        return Ok(output);
    }

    // Retry with a stricter user message so the model is more likely to
    // emit a clean JSON object. Transport errors propagate; only JSON-shape
    // errors fall through to the default. No explicit cancel check needed
    // here: `request_json` races the token internally at its send site.
    let strict_query = format!(
        "{query}\n\nReply with ONLY the JSON object described by the system prompt. No prose, no markdown fences, no explanation."
    );
    let retry_messages = build_router_messages(&system, history, &strict_query);
    let retry_raw = request_json(
        endpoint,
        model,
        client,
        retry_messages,
        router_output_schema(),
        cancel_token,
        timeout_secs,
        num_ctx,
    )
    .await?;
    if let Some(output) = try_parse_router_output(&retry_raw) {
        return Ok(output);
    }

    Err(SearchError::Router(
        "router response could not be parsed after retry".to_string(),
    ))
}

/// Best-effort extraction of [`RouterJudgeOutput`] from raw LLM output.
/// Returns `None` when the output contains no balanced JSON object or the
/// shape does not match the expected schema.
fn try_parse_router_output(raw: &str) -> Option<RouterJudgeOutput> {
    let slice = crate::search::judge::extract_json_object_public(raw)?;
    normalize_router_output(slice).or_else(|| serde_json::from_str::<RouterJudgeOutput>(slice).ok())
}

fn normalize_router_output(raw_json: &str) -> Option<RouterJudgeOutput> {
    let value: serde_json::Value = serde_json::from_str(raw_json).ok()?;
    let object = value.as_object()?;

    let action = parse_router_action(read_json_string(object, &["action", "decision"])?)?;

    let clarifying_question = read_json_string(
        object,
        &[
            "clarifying_question",
            "clarifyingQuestion",
            "follow_up_question",
            "followUpQuestion",
            "question",
        ],
    )
    .map(str::to_string);

    let history_sufficiency = read_json_string(
        object,
        &["history_sufficiency", "historySufficiency", "sufficiency"],
    )
    .and_then(parse_router_sufficiency);

    let optimized_query = read_json_string(
        object,
        &[
            "optimized_query",
            "optimizedQuery",
            "search_query",
            "searchQuery",
            "query",
        ],
    )
    .map(str::to_string);

    Some(RouterJudgeOutput {
        action,
        clarifying_question,
        history_sufficiency,
        optimized_query,
    })
}

fn read_json_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| match value {
            serde_json::Value::String(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }
            _ => None,
        })
    })
}

fn parse_router_action(value: &str) -> Option<Action> {
    match value.trim().to_ascii_lowercase().as_str() {
        "clarify" => Some(Action::Clarify),
        "proceed" => Some(Action::Proceed),
        _ => None,
    }
}

fn parse_router_sufficiency(value: &str) -> Option<Sufficiency> {
    match value.trim().to_ascii_lowercase().as_str() {
        "sufficient" => Some(Sufficiency::Sufficient),
        "partial" => Some(Sufficiency::Partial),
        "insufficient" => Some(Sufficiency::Insufficient),
        _ => None,
    }
}

// ─── Universal sufficiency judge call ────────────────────────────────────────

/// Universal sufficiency judge. Called after each retrieval round with the
/// accumulated evidence to determine whether additional gap-filling rounds are
/// needed.
///
/// Sources can be either SearXNG snippets (initial round) or Trafilatura reader
/// chunks (subsequent rounds); the caller constructs [`JudgeSource`] slices
/// from whichever stage is active.
///
/// The returned verdict is normalized via [`judge::normalize_verdict`] so
/// downstream code can rely on invariants (e.g. `gap_queries` is empty when
/// `sufficiency` is `Sufficient`) even when the model returns malformed output.
///
/// # Errors
/// - [`SearchError::Cancelled`] - token cancelled before or during the request.
/// - [`SearchError::LlmUnavailable`] - transport failure.
/// - [`SearchError::LlmHttp`] - non-2xx status from Ollama.
///
/// Note: this function never returns [`SearchError::Judge`]. If the first
/// attempt produces output that does not parse as [`JudgeVerdict`], we retry
/// once with a stricter user-message suffix. If that also fails, we fall back
/// to a safe default (`Partial` + empty `gap_queries` + diagnostic reasoning)
/// so the pipeline always produces a result rather than surfacing a cryptic
/// parse error.
#[allow(clippy::too_many_arguments)]
pub async fn call_judge(
    endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    query: &str,
    sources: &[JudgeSource],
    cancel_token: &CancellationToken,
    timeout_secs: u64,
    num_ctx: u32,
    stage: JudgeStage,
) -> Result<JudgeVerdict, SearchError> {
    if cancel_token.is_cancelled() {
        return Err(SearchError::Cancelled);
    }

    let user_msg = build_judge_user_message(query, sources);
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: stage.system_prompt().to_string(),
            images: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_msg.clone(),
            images: None,
        },
    ];
    let raw = request_json(
        endpoint,
        model,
        client,
        messages,
        judge_output_schema(),
        cancel_token,
        timeout_secs,
        num_ctx,
    )
    .await?;
    if let Ok(mut verdict) = crate::search::judge::parse_verdict(&raw) {
        crate::search::judge::normalize_verdict(
            &mut verdict,
            crate::config::defaults::DEFAULT_GAP_QUERIES_PER_ROUND,
        );
        return Ok(verdict);
    }

    // Retry with a stricter user message so the model is more likely to
    // emit a clean JSON object. Transport errors propagate; only JSON-shape
    // errors fall through to the default. No explicit cancel check needed
    // here: `request_json` races the token internally at its send site.
    let strict_user_msg = format!(
        "{user_msg}\n\nReply with ONLY the JSON object described by the system prompt. No prose, no markdown fences, no explanation."
    );
    let retry_messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: stage.system_prompt().to_string(),
            images: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: strict_user_msg,
            images: None,
        },
    ];
    let retry_raw = request_json(
        endpoint,
        model,
        client,
        retry_messages,
        judge_output_schema(),
        cancel_token,
        timeout_secs,
        num_ctx,
    )
    .await?;
    if let Ok(mut verdict) = crate::search::judge::parse_verdict(&retry_raw) {
        crate::search::judge::normalize_verdict(
            &mut verdict,
            crate::config::defaults::DEFAULT_GAP_QUERIES_PER_ROUND,
        );
        return Ok(verdict);
    }

    // Both attempts produced unparseable output. Fall back to a safe default
    // so the pipeline still produces a result. Partial with no gap queries
    // lets the pipeline proceed to synthesis on whatever evidence it already
    // holds rather than aborting with a cryptic error. `parse_failure` flags
    // the verdict as synthetic so the pipeline can emit a `JudgeFailure`
    // warning and skip the empty `reasoning` from user-facing trace details.
    let mut verdict = JudgeVerdict {
        sufficiency: crate::search::types::Sufficiency::Partial,
        reasoning: String::new(),
        gap_queries: vec![],
        parse_failure: true,
    };
    crate::search::judge::normalize_verdict(
        &mut verdict,
        crate::config::defaults::DEFAULT_GAP_QUERIES_PER_ROUND,
    );
    Ok(verdict)
}

/// Builds the user-turn message for a judge call. Formats the question and
/// numbered source list so the model can assess coverage without seeing any
/// system metadata.
fn build_judge_user_message(query: &str, sources: &[JudgeSource]) -> String {
    let text_len: usize = sources.iter().map(|s| s.text.len()).sum();
    let mut s = String::with_capacity(256 + text_len);
    s.push_str("QUESTION:\n");
    s.push_str(query);
    s.push_str("\n\nSOURCES:\n");
    for (i, src) in sources.iter().enumerate() {
        s.push_str(&format!(
            "[{}] {} ({})\n{}\n\n",
            i + 1,
            src.title,
            src.url,
            src.text
        ));
    }
    s
}

fn format_router_history(history: &[ChatMessage]) -> String {
    if history.is_empty() {
        return "<empty>\n".to_string();
    }

    let mut out = String::with_capacity(history.len() * 96);
    for (index, message) in history.iter().enumerate() {
        let content = message.content.trim();
        let content = if content.is_empty() {
            "<empty>"
        } else {
            content
        };
        out.push_str(&format!("[{}] {}: {}\n", index + 1, message.role, content));
    }
    out
}

/// Builds the router request as `[system, user]` with an explicit transcript
/// block and a separately labeled latest user message. This makes the prior
/// history boundary visible even when the thread is empty.
fn build_router_messages(system: &str, history: &[ChatMessage], query: &str) -> Vec<ChatMessage> {
    let mut user_content = String::with_capacity(query.len() + history.len() * 96 + 256);
    user_content.push_str("PRIOR CONVERSATION TRANSCRIPT:\n");
    user_content.push_str(&format_router_history(history));
    user_content.push_str("\nLATEST USER MESSAGE:\n");
    user_content.push_str(query);
    user_content.push_str(
        "\n\nOnly the PRIOR CONVERSATION TRANSCRIPT counts toward history_sufficiency. The LATEST USER MESSAGE does not count as already-answered history.",
    );

    vec![
        ChatMessage {
            role: "system".to_string(),
            content: system.to_string(),
            images: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_content,
            images: None,
        },
    ]
}

// ─── Synthesis prompt assembly ──────────────────────────────────────────────

/// Builds the message array for the `search` synthesis stage: a dedicated
/// synthesis system prompt augmented with a plain-text sources block, then
/// the conversation history and the user's query. The sources block is
/// concatenated to the system prompt so it never appears as a user-authored
/// turn (which leads small models into "describe the document" mode).
///
/// Security note: source text is still untrusted input. This prompt structure
/// reduces role confusion, but it does not fully neutralize prompt injection
/// embedded inside fetched pages. Treat the synthesized answer as best-effort
/// and keep citation-backed provenance visible to the user.
///
/// `today` is a `YYYY-MM-DD` string injected at call time; it replaces the
/// `{{TODAY}}` placeholder in the prompt template so the model is always
/// anchored to the real calendar date rather than its training cutoff.
pub fn build_synthesis_messages(
    history: &[ChatMessage],
    query: &str,
    results: &[SearxResult],
    today: &str,
) -> Vec<ChatMessage> {
    let prompt = SYNTHESIS_SYSTEM_PROMPT.replace("{{TODAY}}", today);
    let mut system = String::with_capacity(prompt.len() + 1024);
    system.push_str(&prompt);
    system.push_str("\n\n# Sources\n\n");
    system.push_str(&format_sources(results));

    let mut msgs = Vec::with_capacity(history.len() + 2);
    msgs.push(ChatMessage {
        role: "system".to_string(),
        content: system,
        images: None,
    });
    msgs.extend(history.iter().cloned());
    msgs.push(ChatMessage {
        role: "user".to_string(),
        content: query.to_string(),
        images: None,
    });
    msgs
}

/// Builds the message array for the `answer_from_context` stage. Appends a
/// strict transcript-only guard to the supplied chat system prompt so the
/// answer is grounded in the conversation history alone.
pub fn build_answer_from_context_messages(
    chat_system_prompt: &str,
    history: &[ChatMessage],
    query: &str,
) -> Vec<ChatMessage> {
    let mut system =
        String::with_capacity(chat_system_prompt.len() + HISTORY_ONLY_SYSTEM_APPENDIX.len());
    system.push_str(chat_system_prompt.trim_end());
    system.push_str(HISTORY_ONLY_SYSTEM_APPENDIX);

    let mut msgs = Vec::with_capacity(history.len() + 2);
    msgs.push(ChatMessage {
        role: "system".to_string(),
        content: system,
        images: None,
    });
    msgs.extend(history.iter().cloned());
    msgs.push(ChatMessage {
        role: "user".to_string(),
        content: query.to_string(),
        images: None,
    });
    msgs
}

/// Renders a numbered plain-text block of sources. Titles and snippets have
/// already been HTML-entity-decoded and length-capped by the SearXNG client.
/// Deliberately no XML: the output is concatenated into a plain-text system
/// prompt, so XML escaping would corrupt ampersands, angle brackets, etc.
/// back into their entity forms.
fn format_sources(results: &[SearxResult]) -> String {
    let mut out = String::with_capacity(results.len() * 256);
    for (idx, r) in results.iter().enumerate() {
        let n = idx + 1;
        out.push_str(&format!("[{n}] {}\n", r.title.trim()));
        out.push_str(&format!("    URL: {}\n", r.url.trim()));
        if !r.content.trim().is_empty() {
            out.push_str(&format!("    {}\n", r.content.trim()));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            images: None,
        }
    }

    // ── build_synthesis_messages ────────────────────────────────────────────

    #[test]
    fn build_synthesis_messages_embeds_sources_in_system_prompt() {
        let results = vec![SearxResult {
            title: "T".into(),
            url: "https://u".into(),
            content: "C".into(),
        }];
        let msgs = build_synthesis_messages(&[], "q", &results, "2026-04-17");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.contains("# Sources"));
        assert!(msgs[0].content.contains("[1] T"));
        assert!(msgs[0].content.contains("https://u"));
        assert!(msgs[0].content.contains("C"));
        assert_eq!(msgs[1].role, "user");
        assert_eq!(msgs[1].content, "q");
    }

    #[test]
    fn build_synthesis_messages_interleaves_history() {
        let history = vec![mk_msg("user", "earlier"), mk_msg("assistant", "reply")];
        let results = vec![SearxResult {
            title: "T".into(),
            url: "https://u".into(),
            content: "C".into(),
        }];
        let msgs = build_synthesis_messages(&history, "now", &results, "2026-04-17");
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[1].role, "user");
        assert_eq!(msgs[1].content, "earlier");
        assert_eq!(msgs[3].role, "user");
        assert_eq!(msgs[3].content, "now");
    }

    #[test]
    fn build_synthesis_messages_injects_today_and_removes_placeholder() {
        let msgs = build_synthesis_messages(&[], "q", &[], "2026-04-17");
        let system = &msgs[0].content;
        assert!(
            system.contains("Today's date is 2026-04-17"),
            "system prompt must contain the injected date"
        );
        assert!(
            !system.contains("{{TODAY}}"),
            "placeholder must not appear in the final prompt"
        );
    }

    #[test]
    fn build_synthesis_messages_prompt_contains_date_grounding_rules() {
        let msgs = build_synthesis_messages(&[], "q", &[], "2026-04-17");
        let system = &msgs[0].content;
        // No-unsupported-dates rule.
        assert!(system.contains("NEVER state a date"));
        // Prefer-most-recent-date rule.
        assert!(system.contains("prefer the most recent date"));
        // Existing no-meta-commentary rule still present.
        assert!(system.contains("Do NOT describe, summarize, list, or meta-commentate"));
    }

    #[test]
    fn format_sources_numbers_entries_from_one() {
        let results = vec![
            SearxResult {
                title: "A".into(),
                url: "https://a".into(),
                content: "aa".into(),
            },
            SearxResult {
                title: "B".into(),
                url: "https://b".into(),
                content: "bb".into(),
            },
        ];
        let out = format_sources(&results);
        assert!(out.contains("[1] A"));
        assert!(out.contains("[2] B"));
    }

    #[test]
    fn format_sources_omits_blank_content_line() {
        let results = vec![SearxResult {
            title: "A".into(),
            url: "https://a".into(),
            content: "   ".into(),
        }];
        let out = format_sources(&results);
        assert!(out.contains("[1] A"));
        assert!(out.contains("https://a"));
        assert!(!out.contains("    \n"));
    }

    #[test]
    fn format_sources_empty_list_returns_empty_string() {
        assert_eq!(format_sources(&[]), "");
    }

    // ── build_answer_from_context_messages ──────────────────────────────────

    #[test]
    fn build_answer_from_context_messages_uses_supplied_system_prompt() {
        let msgs = build_answer_from_context_messages("base prompt", &[], "q");
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].content.starts_with("base prompt"));
        assert!(msgs[0]
            .content
            .contains("You are answering from the prior conversation only."));
        assert!(msgs[0]
            .content
            .contains("I can't answer that from this conversation alone."));
    }

    #[test]
    fn build_answer_from_context_messages_includes_history() {
        let history = vec![mk_msg("user", "prev"), mk_msg("assistant", "prev-reply")];
        let msgs = build_answer_from_context_messages("base", &history, "q");
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[1].content, "prev");
        assert_eq!(msgs[3].content, "q");
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn snippet_judge_prompt_declares_verdict_schema() {
        let p = SNIPPET_JUDGE_SYSTEM_PROMPT;
        assert!(p.contains("sufficiency"));
        assert!(p.contains("reasoning"));
        assert!(p.contains("gap_queries"));
        assert!(p.contains("sufficient"));
        assert!(p.contains("partial"));
        assert!(p.contains("insufficient"));
    }

    #[test]
    fn chunk_judge_prompt_declares_verdict_schema() {
        let p = CHUNK_JUDGE_SYSTEM_PROMPT;
        assert!(p.contains("sufficiency"));
        assert!(p.contains("reasoning"));
        assert!(p.contains("gap_queries"));
        assert!(p.contains("sufficient"));
        assert!(p.contains("partial"));
        assert!(p.contains("insufficient"));
    }

    #[test]
    fn judge_prompts_emit_reasoning_before_sufficiency() {
        // Reasoning-first emission is the whole point of the schema reorder.
        // Both prompts must explicitly tell the model to write reasoning then
        // sufficiency; otherwise small local models produce verdict-first.
        for p in [SNIPPET_JUDGE_SYSTEM_PROMPT, CHUNK_JUDGE_SYSTEM_PROMPT] {
            let r_idx = p
                .find("\"reasoning\"")
                .expect("prompt should reference reasoning property");
            let s_idx = p
                .find("\"sufficiency\"")
                .expect("prompt should reference sufficiency property");
            assert!(
                r_idx < s_idx,
                "prompt must demonstrate reasoning before sufficiency"
            );
        }
    }

    #[test]
    fn judge_stage_system_prompt_routes_correctly() {
        assert_eq!(
            JudgeStage::Snippet.system_prompt(),
            SNIPPET_JUDGE_SYSTEM_PROMPT
        );
        assert_eq!(JudgeStage::Chunk.system_prompt(), CHUNK_JUDGE_SYSTEM_PROMPT);
        assert_ne!(
            JudgeStage::Snippet.system_prompt(),
            JudgeStage::Chunk.system_prompt(),
            "snippet and chunk prompts must differ"
        );
    }

    #[test]
    fn synthesis_prompt_still_has_today_placeholder_and_citation_guidance() {
        let p = SYNTHESIS_SYSTEM_PROMPT;
        assert!(p.contains("{{TODAY}}"));
        assert!(p.contains("[1]"));
        assert!(p.contains("full-page chunk"));
    }

    #[test]
    fn search_plan_prompt_has_today_placeholder_and_required_fields() {
        let p = SEARCH_PLAN_SYSTEM_PROMPT;
        assert!(p.contains("{{TODAY}}"));
        assert!(p.contains("NOT part of the prior conversation transcript"));
        assert!(p.contains("\"action\""));
        assert!(p.contains("clarify"));
        assert!(p.contains("proceed"));
        assert!(p.contains("history_sufficiency"));
        assert!(p.contains("optimized_query"));
        // The router prompt MUST default to proceed and treat clarify as the
        // exception. Regression guard: previous versions defaulted the other
        // way and over-clarified on grounded named-entity queries.
        assert!(
            p.contains("Default to \"proceed\"") || p.contains("DEFAULT decision is \"proceed\""),
            "router prompt must declare proceed as the default action"
        );
        assert!(
            p.contains("Clarification is the exception"),
            "router prompt must call out clarification as the exception"
        );
    }
}

#[cfg(test)]
mod router_judge_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── build_judge_user_message ─────────────────────────────────────────────

    #[test]
    fn build_judge_user_message_formats_question_and_sources() {
        let sources = vec![
            JudgeSource {
                title: "T1".into(),
                url: "https://u1".into(),
                text: "body one".into(),
            },
            JudgeSource {
                title: "T2".into(),
                url: "https://u2".into(),
                text: "body two".into(),
            },
        ];
        let msg = build_judge_user_message("my question", &sources);
        assert!(msg.contains("QUESTION:\nmy question"));
        assert!(msg.contains("[1] T1 (https://u1)"));
        assert!(msg.contains("body one"));
        assert!(msg.contains("[2] T2 (https://u2)"));
        assert!(msg.contains("body two"));
    }

    #[test]
    fn build_judge_user_message_with_no_sources() {
        let msg = build_judge_user_message("q", &[]);
        assert!(msg.contains("QUESTION:\nq"));
        assert!(msg.contains("SOURCES:"));
        // No numbered entries.
        assert!(!msg.contains("[1]"));
    }

    // ── build_router_messages ────────────────────────────────────────────────

    #[test]
    fn build_router_messages_marks_empty_history_and_latest_query() {
        let msgs = build_router_messages("sys", &[], "what is today's date?");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].content, "sys");
        assert!(msgs[1]
            .content
            .contains("PRIOR CONVERSATION TRANSCRIPT:\n<empty>\n"));
        assert!(msgs[1]
            .content
            .contains("LATEST USER MESSAGE:\nwhat is today's date?"));
    }

    #[test]
    fn build_router_messages_flattens_history_with_roles() {
        let history = vec![
            ChatMessage {
                role: "user".into(),
                content: "prev".into(),
                images: None,
            },
            ChatMessage {
                role: "assistant".into(),
                content: "reply".into(),
                images: None,
            },
        ];
        let msgs = build_router_messages("sys", &history, "q");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].content, "sys");
        assert!(msgs[1].content.contains("[1] user: prev"));
        assert!(msgs[1].content.contains("[2] assistant: reply"));
        assert!(msgs[1].content.contains("LATEST USER MESSAGE:\nq"));
    }

    #[test]
    fn build_router_messages_replaces_blank_history_content_with_placeholder() {
        let history = vec![ChatMessage {
            role: "assistant".into(),
            content: "   ".into(),
            images: None,
        }];

        let msgs = build_router_messages("sys", &history, "q");

        assert!(msgs[1].content.contains("[1] assistant: <empty>"));
    }

    #[test]
    fn try_parse_router_output_normalizes_camel_case_clarify_shape() {
        let output = try_parse_router_output(
            r#"{"action":"clarify","clarifyingQuestion":"Who are you asking about?","historySufficiency":null,"optimizedQuery":null}"#,
        )
        .expect("expected normalized clarify output");

        assert_eq!(output.action, Action::Clarify);
        assert_eq!(
            output.clarifying_question.as_deref(),
            Some("Who are you asking about?")
        );
        assert!(output.history_sufficiency.is_none());
        assert!(output.optimized_query.is_none());
    }

    #[test]
    fn normalize_router_output_treats_blank_clarifying_question_as_none() {
        let output = normalize_router_output(
            r#"{"action":"clarify","clarifying_question":"   ","history_sufficiency":null,"optimized_query":null}"#,
        )
        .expect("expected normalized clarify output");

        assert_eq!(output.action, Action::Clarify);
        assert!(output.clarifying_question.is_none());
    }

    #[test]
    fn parse_router_action_rejects_unknown_values() {
        assert_eq!(parse_router_action("search"), None);
    }

    #[test]
    fn parse_router_sufficiency_rejects_unknown_values() {
        assert_eq!(parse_router_sufficiency("maybe"), None);
    }

    // ── call_router_merged ───────────────────────────────────────────────────

    #[tokio::test]
    async fn merged_router_requests_schema_constrained_format() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(wiremock::matchers::body_string_contains("\"format\":{"))
            .and(wiremock::matchers::body_string_contains("\"clarifying_question\""))
            .and(wiremock::matchers::body_string_contains("\"additionalProperties\":false"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "{\"action\":\"clarify\",\"clarifying_question\":\"Who are you asking about?\",\"history_sufficiency\":null,\"optimized_query\":null}"
                },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let output = call_router_merged(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            &[],
            "who is he?",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            crate::config::defaults::DEFAULT_NUM_CTX,
        )
        .await
        .expect("schema-constrained router call should parse");

        assert_eq!(output.action, Action::Clarify);
    }

    #[tokio::test]
    async fn merged_router_parses_proceed_with_sufficiency() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "{\"action\":\"proceed\",\"clarifying_question\":null,\"history_sufficiency\":\"insufficient\",\"optimized_query\":\"curl 8.10 CVE 2026\"}"
                },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let output = call_router_merged(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            &[],
            "tell me about curl CVE",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            crate::config::defaults::DEFAULT_NUM_CTX,
        )
        .await
        .unwrap();

        assert!(matches!(
            output.action,
            crate::search::types::Action::Proceed
        ));
        assert_eq!(
            output.optimized_query.as_deref(),
            Some("curl 8.10 CVE 2026")
        );
        assert_eq!(
            output.history_sufficiency,
            Some(crate::search::types::Sufficiency::Insufficient)
        );
        assert!(output.clarifying_question.is_none());
    }

    #[tokio::test]
    async fn merged_router_parses_clarify_with_question() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "{\"action\":\"clarify\",\"clarifying_question\":\"which project?\",\"history_sufficiency\":null,\"optimized_query\":null}"
                },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let output = call_router_merged(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            &[],
            "what is the status",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            crate::config::defaults::DEFAULT_NUM_CTX,
        )
        .await
        .unwrap();

        assert!(matches!(
            output.action,
            crate::search::types::Action::Clarify
        ));
        assert_eq!(
            output.clarifying_question.as_deref(),
            Some("which project?")
        );
        assert!(output.history_sufficiency.is_none());
        assert!(output.optimized_query.is_none());
    }

    #[tokio::test]
    async fn merged_router_injects_today_into_system_prompt() {
        let server = MockServer::start().await;
        // Capture the request body to verify TODAY injection.
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(wiremock::matchers::body_string_contains("2026-04-18"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "{\"action\":\"proceed\",\"clarifying_question\":null,\"history_sufficiency\":\"sufficient\",\"optimized_query\":\"q\"}"
                },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let output = call_router_merged(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            &[],
            "q",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            crate::config::defaults::DEFAULT_NUM_CTX,
        )
        .await
        .unwrap();
        assert!(matches!(
            output.action,
            crate::search::types::Action::Proceed
        ));
    }

    #[tokio::test]
    async fn merged_router_returns_cancelled_when_token_already_cancelled() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        token.cancel();
        let err = call_router_merged(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            &[],
            "q",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            crate::config::defaults::DEFAULT_NUM_CTX,
        )
        .await
        .unwrap_err();
        assert_eq!(err, SearchError::Cancelled);
    }

    #[tokio::test]
    async fn merged_router_returns_router_error_when_no_json_in_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": { "role": "assistant", "content": "Sorry, I cannot help." },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let err = call_router_merged(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            &[],
            "q",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            crate::config::defaults::DEFAULT_NUM_CTX,
        )
        .await
        .expect_err("router should fail closed when no valid JSON is recoverable");
        assert!(matches!(err, SearchError::Router(_)));
    }

    #[tokio::test]
    async fn merged_router_returns_router_error_when_json_does_not_match_schema() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": { "role": "assistant", "content": "{\"random\":\"shape\"}" },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let err = call_router_merged(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            &[],
            "q",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            crate::config::defaults::DEFAULT_NUM_CTX,
        )
        .await
        .expect_err("router should fail closed when the response shape stays invalid");
        assert!(matches!(err, SearchError::Router(_)));
    }

    #[tokio::test]
    async fn merged_router_returns_cancelled_if_token_fires_between_attempts() {
        use std::sync::Arc;
        use wiremock::Request;

        let server = MockServer::start().await;
        let token = Arc::new(CancellationToken::new());
        let token_clone = token.clone();
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(move |_req: &Request| {
                // Cancel after the first attempt finishes, before the retry
                // loop re-checks the token.
                token_clone.cancel();
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "message": { "role": "assistant", "content": "nope" },
                    "done": true
                }))
            })
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let err = call_router_merged(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            &[],
            "q",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            crate::config::defaults::DEFAULT_NUM_CTX,
        )
        .await
        .unwrap_err();
        assert_eq!(err, SearchError::Cancelled);
    }

    #[tokio::test]
    async fn merged_router_retry_recovers_when_second_attempt_parses() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use wiremock::Request;

        let server = MockServer::start().await;
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(move |_req: &Request| {
                let n = counter_clone.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "message": { "role": "assistant", "content": "I cannot." },
                        "done": true
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "message": { "role": "assistant", "content": "{\"action\":\"proceed\",\"clarifying_question\":null,\"history_sufficiency\":\"sufficient\",\"optimized_query\":\"cats\"}" },
                        "done": true
                    }))
                }
            })
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let output = call_router_merged(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            &[],
            "q",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            crate::config::defaults::DEFAULT_NUM_CTX,
        )
        .await
        .unwrap();
        assert!(matches!(
            output.action,
            crate::search::types::Action::Proceed
        ));
        assert_eq!(
            output.history_sufficiency,
            Some(crate::search::types::Sufficiency::Sufficient)
        );
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    // ── call_judge ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn judge_call_parses_partial_verdict() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "{\"sufficiency\":\"partial\",\"reasoning\":\"missing version\",\"gap_queries\":[\"q1\",\"q2\"]}"
                },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let sources = vec![JudgeSource {
            title: "t".into(),
            url: "u".into(),
            text: "s".into(),
        }];
        let verdict = call_judge(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            "q",
            &sources,
            &token,
            crate::config::defaults::DEFAULT_JUDGE_TIMEOUT_S,
            crate::config::defaults::DEFAULT_NUM_CTX,
            crate::search::llm::JudgeStage::Snippet,
        )
        .await
        .unwrap();

        assert!(matches!(
            verdict.sufficiency,
            crate::search::types::Sufficiency::Partial
        ));
        assert_eq!(verdict.gap_queries.len(), 2);
    }

    #[tokio::test]
    async fn judge_call_normalizes_gap_queries_when_sufficient() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "{\"sufficiency\":\"sufficient\",\"reasoning\":\"all here\",\"gap_queries\":[\"stale\"]}"
                },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let verdict = call_judge(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            "q",
            &[],
            &token,
            crate::config::defaults::DEFAULT_JUDGE_TIMEOUT_S,
            crate::config::defaults::DEFAULT_NUM_CTX,
            crate::search::llm::JudgeStage::Snippet,
        )
        .await
        .unwrap();

        assert!(matches!(
            verdict.sufficiency,
            crate::search::types::Sufficiency::Sufficient
        ));
        assert!(
            verdict.gap_queries.is_empty(),
            "sufficient verdict must drop gap_queries"
        );
    }

    #[tokio::test]
    async fn judge_call_returns_cancelled_when_token_already_cancelled() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        token.cancel();
        let err = call_judge(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            "q",
            &[],
            &token,
            crate::config::defaults::DEFAULT_JUDGE_TIMEOUT_S,
            crate::config::defaults::DEFAULT_NUM_CTX,
            crate::search::llm::JudgeStage::Snippet,
        )
        .await
        .unwrap_err();
        assert_eq!(err, SearchError::Cancelled);
    }

    #[tokio::test]
    async fn judge_call_falls_back_to_partial_when_no_json_in_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": { "role": "assistant", "content": "no json here" },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let verdict = call_judge(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            "q",
            &[],
            &token,
            crate::config::defaults::DEFAULT_JUDGE_TIMEOUT_S,
            crate::config::defaults::DEFAULT_NUM_CTX,
            crate::search::llm::JudgeStage::Snippet,
        )
        .await
        .expect("judge should fall back to safe defaults, not error");
        assert!(matches!(
            verdict.sufficiency,
            crate::search::types::Sufficiency::Partial
        ));
        assert!(verdict.gap_queries.is_empty());
        assert!(
            verdict.parse_failure,
            "fallback verdict must be flagged as a parse failure so the pipeline can emit JudgeFailure"
        );
        assert!(
            verdict.reasoning.is_empty(),
            "fallback reasoning must be empty so diagnostic strings do not leak into user-facing trace details"
        );
    }

    #[tokio::test]
    async fn judge_call_falls_back_when_json_does_not_match_schema() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": { "role": "assistant", "content": "{\"random\":\"shape\"}" },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let verdict = call_judge(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            "q",
            &[],
            &token,
            crate::config::defaults::DEFAULT_JUDGE_TIMEOUT_S,
            crate::config::defaults::DEFAULT_NUM_CTX,
            crate::search::llm::JudgeStage::Snippet,
        )
        .await
        .expect("judge should fall back to safe defaults, not error");
        assert!(matches!(
            verdict.sufficiency,
            crate::search::types::Sufficiency::Partial
        ));
        assert!(verdict.gap_queries.is_empty());
        assert!(verdict.parse_failure);
        assert!(verdict.reasoning.is_empty());
    }

    #[tokio::test]
    async fn judge_output_schema_constrains_verdict_shape() {
        // The schema must enumerate the exact sufficiency values the parser
        // accepts and require the three fields the verdict normalizer reads.
        // If a refactor drifts the schema, small local models start emitting
        // shape variations that defeat parsing.
        let schema = judge_output_schema();
        let suff_enum = &schema["properties"]["sufficiency"]["enum"];
        assert_eq!(
            suff_enum,
            &serde_json::json!(["sufficient", "partial", "insufficient"])
        );
        // Required order MUST be reasoning-first so constrained decoders that
        // honor schema property order force the model to write its analysis
        // before committing to a verdict.
        assert_eq!(
            schema["required"],
            serde_json::json!(["reasoning", "sufficiency", "gap_queries"])
        );
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn judge_call_requests_schema_constrained_format() {
        // Asserts the judge request sets `format` to the schema, not the bare
        // `"json"` string. Without the schema, Ollama JSON mode allows any
        // valid JSON shape and small models drift away from JudgeVerdict.
        let server = MockServer::start().await;
        let captured: std::sync::Arc<std::sync::Mutex<Option<serde_json::Value>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let captured_clone = captured.clone();
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(move |req: &wiremock::Request| {
                let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
                *captured_clone.lock().unwrap() = Some(body["format"].clone());
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "message": {
                        "role": "assistant",
                        "content": "{\"sufficiency\":\"sufficient\",\"reasoning\":\"r\",\"gap_queries\":[]}"
                    },
                    "done": true
                }))
            })
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        call_judge(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            "q",
            &[],
            &token,
            crate::config::defaults::DEFAULT_JUDGE_TIMEOUT_S,
            crate::config::defaults::DEFAULT_NUM_CTX,
            crate::search::llm::JudgeStage::Snippet,
        )
        .await
        .unwrap();

        let format = captured.lock().unwrap().clone().expect("format captured");
        assert_eq!(format, judge_output_schema());
    }

    #[tokio::test]
    async fn judge_call_retry_recovers_when_second_attempt_parses() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use wiremock::Request;

        let server = MockServer::start().await;
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(move |_req: &Request| {
                let n = counter_clone.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "message": { "role": "assistant", "content": "I cannot." },
                        "done": true
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "message": { "role": "assistant", "content": "{\"sufficiency\":\"sufficient\",\"reasoning\":\"all good\",\"gap_queries\":[]}" },
                        "done": true
                    }))
                }
            })
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let verdict = call_judge(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            "q",
            &[],
            &token,
            crate::config::defaults::DEFAULT_JUDGE_TIMEOUT_S,
            crate::config::defaults::DEFAULT_NUM_CTX,
            crate::search::llm::JudgeStage::Snippet,
        )
        .await
        .unwrap();
        assert!(matches!(
            verdict.sufficiency,
            crate::search::types::Sufficiency::Sufficient
        ));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn judge_call_returns_cancelled_if_token_fires_between_attempts() {
        use std::sync::Arc;
        use wiremock::Request;

        let server = MockServer::start().await;
        let token = Arc::new(CancellationToken::new());
        let token_clone = token.clone();
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(move |_req: &Request| {
                token_clone.cancel();
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "message": { "role": "assistant", "content": "nope" },
                    "done": true
                }))
            })
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let err = call_judge(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            "q",
            &[],
            &token,
            crate::config::defaults::DEFAULT_JUDGE_TIMEOUT_S,
            crate::config::defaults::DEFAULT_NUM_CTX,
            crate::search::llm::JudgeStage::Snippet,
        )
        .await
        .unwrap_err();
        assert_eq!(err, SearchError::Cancelled);
    }

    #[tokio::test]
    async fn request_json_returns_llm_http_error_on_non_success_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        // call_router_merged calls request_json internally; a 503 maps to
        // SearchError::LlmHttp(503).
        let err = call_router_merged(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            &[],
            "q",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            crate::config::defaults::DEFAULT_NUM_CTX,
        )
        .await
        .unwrap_err();
        assert_eq!(err, SearchError::LlmHttp(503));
    }

    #[tokio::test]
    async fn merged_router_sends_num_ctx_in_request_options() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(wiremock::matchers::body_string_contains(
                "\"num_ctx\":32768",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "{\"action\":\"proceed\",\"clarifying_question\":null,\"history_sufficiency\":\"sufficient\",\"optimized_query\":\"q\"}"
                },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let output = call_router_merged(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            &[],
            "q",
            "2026-04-18",
            &token,
            ROUTER_TIMEOUT_SECS,
            32768,
        )
        .await
        .unwrap();
        assert!(matches!(
            output.action,
            crate::search::types::Action::Proceed
        ));
    }

    #[tokio::test]
    async fn judge_call_sends_num_ctx_in_request_options() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(wiremock::matchers::body_string_contains(
                "\"num_ctx\":65536",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "role": "assistant",
                    "content": "{\"sufficiency\":\"sufficient\",\"reasoning\":\"r\",\"gap_queries\":[]}"
                },
                "done": true
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let _ = call_judge(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            "q",
            &[],
            &token,
            crate::config::defaults::DEFAULT_JUDGE_TIMEOUT_S,
            65536,
            JudgeStage::Snippet,
        )
        .await
        .unwrap();
    }
}
