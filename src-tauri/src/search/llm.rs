//! LLM operations for the `/search` pipeline.
//!
//! Three concerns live here:
//! 1. The non-streaming **router call** that classifies the user's query into
//!    a [`RouterDecision`]. Uses Ollama's `format: "json"` mode with a
//!    deterministic sampling profile so the output is strictly parseable.
//! 2. The **merged router+judge call** (`call_router_merged`) and the
//!    **universal sufficiency judge call** (`call_judge`), added for the v3
//!    agentic pipeline. Task 13 swaps the pipeline call site from
//!    `call_router` to `call_router_merged`; Task 16 retires the legacy path.
//! 3. Prompt-assembly helpers that produce the message array fed to the
//!    streaming answer stage (either `answer_from_context` or `search`).
//!
//! All functions are pure with respect to external state (no globals, no
//! hidden side effects) and accept their dependencies explicitly for
//! testability.

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::commands::ChatMessage;

use super::types::{JudgeVerdict, RouterDecision, RouterJudgeOutput, SearchError, SearxResult};

/// Router system prompt: instructs the classifier LLM on the allowed actions
/// and their JSON output shapes.
pub const ROUTER_SYSTEM_PROMPT: &str = include_str!("../../prompts/search_router.txt");

/// Synthesis system prompt: instructs the answering LLM to cite sources and
/// avoid meta-commentary over the reference material.
pub const SYNTHESIS_SYSTEM_PROMPT: &str = include_str!("../../prompts/search_synthesis.txt");

/// System prompt for the universal sufficiency judge. Used by the pre-synthesis
/// judge call over snippets and over reader chunks. Consumed by
/// [`call_judge`] introduced in Task 11.
// The constant is used inside `call_judge` which itself has no non-test call
// site until Task 13 wires it; suppress the transitive dead-code warning.
#[allow(dead_code)]
pub const JUDGE_SYSTEM_PROMPT: &str = include_str!("../../prompts/search_judge.txt");

/// New merged router+judge prompt. Instructs the model to emit a single JSON
/// object covering both routing classification and history-sufficiency
/// assessment. Added alongside `ROUTER_SYSTEM_PROMPT`; the pipeline swaps to
/// this one in Task 13.
// Consumed by `call_router_merged`; suppress the dead-code warning until
// Task 13 wires it into the pipeline call site.
#[allow(dead_code)]
pub const ROUTER_MERGED_SYSTEM_PROMPT: &str =
    include_str!("../../prompts/search_router_merged.txt");

/// Hard timeout for the non-streaming router call. Picked to accommodate cold
/// model starts on first pipeline invocation.
pub const ROUTER_TIMEOUT_SECS: u64 = 45;

/// Cap on the router response length. Enough for a clarification question
/// with several suggestions; prevents runaway generation when the model
/// fails to produce valid JSON quickly.
pub const ROUTER_MAX_TOKENS: i32 = 512;

// ─── Shared input/output types ───────────────────────────────────────────────

/// A single evidence source passed to the universal sufficiency judge. Used by
/// [`call_judge`] to build the user-turn content from either SearXNG snippets
/// (initial round) or Trafilatura reader chunks (subsequent rounds).
///
/// Free-standing so the pipeline can construct instances from whichever source
/// stage is currently active without depending on internal snippet or chunk
/// types.
// No non-test call site until Task 14 builds the initial retrieval round.
#[allow(dead_code)]
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
}

#[derive(Serialize)]
struct OllamaJsonRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    format: String,
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
/// `timeout_secs` is the per-call wall-clock limit; pass
/// [`ROUTER_TIMEOUT_SECS`] for router calls and
/// [`super::config::JUDGE_TIMEOUT_S`] for judge calls.
async fn request_json(
    endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    messages: Vec<ChatMessage>,
    cancel_token: &CancellationToken,
    timeout_secs: u64,
) -> Result<String, SearchError> {
    let body = OllamaJsonRequest {
        model: model.to_string(),
        messages,
        stream: false,
        format: "json".to_string(),
        options: RouterOptions {
            temperature: 0.0,
            top_p: 1.0,
            top_k: 1,
            num_predict: ROUTER_MAX_TOKENS,
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

// ─── Router call ────────────────────────────────────────────────────────────

/// Runs the classifier LLM call against Ollama's `/api/chat` endpoint with
/// `format: "json"` and deterministic sampling, returning a parsed
/// [`RouterDecision`].
///
/// Races the request against `cancel_token`; cancellation drops the HTTP
/// connection and returns [`SearchError::Cancelled`] without waiting for the
/// model to finish.
///
/// # Errors
/// - [`SearchError::Cancelled`] — token cancelled before or during the request.
/// - [`SearchError::LlmUnavailable`] — transport failure.
/// - [`SearchError::LlmHttp`] — non-2xx status from Ollama.
/// - [`SearchError::LlmBadJson`] — response body could not be decoded, or the
///   inner `message.content` was not a valid `RouterDecision`.
pub async fn call_router(
    endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    history: &[ChatMessage],
    query: &str,
    cancel_token: &CancellationToken,
) -> Result<RouterDecision, SearchError> {
    if cancel_token.is_cancelled() {
        return Err(SearchError::Cancelled);
    }

    let messages = build_router_messages(history, query);
    let raw = request_json(
        endpoint,
        model,
        client,
        messages,
        cancel_token,
        ROUTER_TIMEOUT_SECS,
    )
    .await?;

    serde_json::from_str::<RouterDecision>(raw.trim()).map_err(|_| SearchError::LlmBadJson)
}

// ─── Merged router+judge call ────────────────────────────────────────────────

/// Merged router+judge call that returns [`RouterJudgeOutput`] in a single
/// Ollama roundtrip: routing classification plus, when proceeding, a
/// sufficiency verdict on conversation history and an optimized search query.
///
/// Uses [`ROUTER_MERGED_SYSTEM_PROMPT`] with `{{TODAY}}` replaced by the
/// supplied `today` string so the model is anchored to the real calendar date.
/// Pass the result of `pipeline::today_iso()` at the call site, or a fixed
/// string in tests.
///
/// Added alongside the existing [`call_router`] so the pipeline can migrate
/// incrementally. Task 13 swaps the call site; Task 16 retires the legacy path.
///
/// # Errors
/// - [`SearchError::Cancelled`] — token cancelled before or during the request.
/// - [`SearchError::LlmUnavailable`] — transport failure.
/// - [`SearchError::LlmHttp`] — non-2xx status from Ollama.
/// - [`SearchError::Router`] — no JSON object in the response, or the JSON
///   did not match [`RouterJudgeOutput`].
// Suppress dead-code lint until Task 13 swaps the pipeline call site.
#[allow(dead_code)]
pub async fn call_router_merged(
    endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    history: &[ChatMessage],
    query: &str,
    today: &str,
    cancel_token: &CancellationToken,
) -> Result<RouterJudgeOutput, SearchError> {
    if cancel_token.is_cancelled() {
        return Err(SearchError::Cancelled);
    }

    let system = ROUTER_MERGED_SYSTEM_PROMPT.replace("{{TODAY}}", today);
    let messages = build_messages_with_system(&system, history, query);
    let raw = request_json(
        endpoint,
        model,
        client,
        messages,
        cancel_token,
        ROUTER_TIMEOUT_SECS,
    )
    .await?;

    let slice = crate::search::judge::extract_json_object_public(&raw)
        .ok_or_else(|| SearchError::Router("no JSON in router response".into()))?;
    serde_json::from_str::<RouterJudgeOutput>(slice)
        .map_err(|e| SearchError::Router(format!("router JSON: {e}")))
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
/// - [`SearchError::Cancelled`] — token cancelled before or during the request.
/// - [`SearchError::LlmUnavailable`] — transport failure.
/// - [`SearchError::LlmHttp`] — non-2xx status from Ollama.
/// - [`SearchError::Judge`] — no JSON in the response, or the JSON did not
///   match [`JudgeVerdict`].
// Suppress dead-code lint until Task 14 wires the initial retrieval round.
#[allow(dead_code)]
pub async fn call_judge(
    endpoint: &str,
    model: &str,
    client: &reqwest::Client,
    query: &str,
    sources: &[JudgeSource],
    cancel_token: &CancellationToken,
) -> Result<JudgeVerdict, SearchError> {
    if cancel_token.is_cancelled() {
        return Err(SearchError::Cancelled);
    }

    let user_msg = build_judge_user_message(query, sources);
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: JUDGE_SYSTEM_PROMPT.to_string(),
            images: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_msg,
            images: None,
        },
    ];
    let raw = request_json(
        endpoint,
        model,
        client,
        messages,
        cancel_token,
        super::config::JUDGE_TIMEOUT_S,
    )
    .await?;

    let mut verdict = crate::search::judge::parse_verdict(&raw)
        .map_err(|e| SearchError::Judge(format!("{e}")))?;
    crate::search::judge::normalize_verdict(
        &mut verdict,
        crate::search::config::GAP_QUERIES_PER_ROUND,
    );
    Ok(verdict)
}

/// Builds the user-turn message for a judge call. Formats the question and
/// numbered source list so the model can assess coverage without seeing any
/// system metadata.
// Private; used only by `call_judge` which is itself dead until Task 14.
#[allow(dead_code)]
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

/// Builds a message array of the form `[system, ...history, user]` using the
/// supplied `system` prompt string. Shared by [`build_router_messages`] and
/// [`call_router_merged`].
fn build_messages_with_system(
    system: &str,
    history: &[ChatMessage],
    query: &str,
) -> Vec<ChatMessage> {
    let mut msgs = Vec::with_capacity(history.len() + 2);
    msgs.push(ChatMessage {
        role: "system".to_string(),
        content: system.to_string(),
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

/// Builds the router message array: `[system, ...history, user]`. The router
/// operates on the same conversation history the chat pipeline uses so it can
/// resolve pronouns ("him", "it") against earlier turns.
fn build_router_messages(history: &[ChatMessage], query: &str) -> Vec<ChatMessage> {
    build_messages_with_system(ROUTER_SYSTEM_PROMPT, history, query)
}

// ─── Synthesis prompt assembly ──────────────────────────────────────────────

/// Builds the message array for the `search` synthesis stage: a dedicated
/// synthesis system prompt augmented with a plain-text sources block, then
/// the conversation history and the user's query. The sources block is
/// concatenated to the system prompt so it never appears as a user-authored
/// turn (which leads small models into "describe the document" mode).
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

/// Builds the message array for the `answer_from_context` stage. Uses the
/// supplied `chat_system_prompt` unchanged; the answer is grounded in the
/// conversation history alone (which already contains prior search results as
/// assistant turns).
pub fn build_answer_from_context_messages(
    chat_system_prompt: &str,
    history: &[ChatMessage],
    query: &str,
) -> Vec<ChatMessage> {
    let mut msgs = Vec::with_capacity(history.len() + 2);
    msgs.push(ChatMessage {
        role: "system".to_string(),
        content: chat_system_prompt.to_string(),
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

    // ── build_router_messages ───────────────────────────────────────────────

    #[test]
    fn build_router_messages_prepends_system_and_appends_user() {
        let history = vec![mk_msg("user", "hi"), mk_msg("assistant", "hello")];
        let msgs = build_router_messages(&history, "who is him?");
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.contains("search routing classifier"));
        assert_eq!(msgs[1].role, "user");
        assert_eq!(msgs[1].content, "hi");
        assert_eq!(msgs[2].role, "assistant");
        assert_eq!(msgs[3].role, "user");
        assert_eq!(msgs[3].content, "who is him?");
    }

    #[test]
    fn build_router_messages_with_empty_history() {
        let msgs = build_router_messages(&[], "q");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].role, "user");
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
        assert_eq!(msgs[0].content, "base prompt");
    }

    #[test]
    fn build_answer_from_context_messages_includes_history() {
        let history = vec![mk_msg("user", "prev"), mk_msg("assistant", "prev-reply")];
        let msgs = build_answer_from_context_messages("base", &history, "q");
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[1].content, "prev");
        assert_eq!(msgs[3].content, "q");
    }

    // ── call_router ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn call_router_parses_clarify_response() {
        let mut server = mockito::Server::new_async().await;
        let inner = r#"{"action":"clarify","question":"Who are you referring to? Give me a name or some context."}"#;
        let body = serde_json::json!({ "message": { "content": inner } }).to_string();
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"stream":false,"format":"json"}"#.to_string(),
            ))
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let decision = call_router(
            &format!("{}/api/chat", server.url()),
            "test-model",
            &client,
            &[],
            "q",
            &token,
        )
        .await
        .unwrap();
        mock.assert_async().await;
        assert_eq!(
            decision,
            RouterDecision::Clarify {
                question: "Who are you referring to? Give me a name or some context.".into(),
            }
        );
    }

    #[tokio::test]
    async fn call_router_parses_answer_from_context() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::json!({
            "message": { "content": r#"{"action":"answer_from_context"}"# }
        })
        .to_string();
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let decision = call_router(
            &format!("{}/api/chat", server.url()),
            "m",
            &client,
            &[],
            "q",
            &token,
        )
        .await
        .unwrap();
        mock.assert_async().await;
        assert_eq!(decision, RouterDecision::AnswerFromContext);
    }

    #[tokio::test]
    async fn call_router_parses_search() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::json!({
            "message": { "content": r#"{"action":"search","optimized_query":"rust"}"# }
        })
        .to_string();
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let decision = call_router(
            &format!("{}/api/chat", server.url()),
            "m",
            &client,
            &[],
            "q",
            &token,
        )
        .await
        .unwrap();
        mock.assert_async().await;
        assert_eq!(
            decision,
            RouterDecision::Search {
                optimized_query: "rust".into(),
            }
        );
    }

    #[tokio::test]
    async fn call_router_returns_cancelled_when_token_already_cancelled() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        token.cancel();
        let err = call_router(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            &[],
            "q",
            &token,
        )
        .await
        .unwrap_err();
        assert_eq!(err, SearchError::Cancelled);
    }

    #[tokio::test]
    async fn call_router_maps_transport_failure_to_unavailable() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let err = call_router(
            "http://127.0.0.1:1/api/chat",
            "m",
            &client,
            &[],
            "q",
            &token,
        )
        .await
        .unwrap_err();
        assert_eq!(err, SearchError::LlmUnavailable);
    }

    #[tokio::test]
    async fn call_router_maps_http_error_to_llm_http() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(500)
            .with_body("boom")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let err = call_router(
            &format!("{}/api/chat", server.url()),
            "m",
            &client,
            &[],
            "q",
            &token,
        )
        .await
        .unwrap_err();
        mock.assert_async().await;
        assert_eq!(err, SearchError::LlmHttp(500));
    }

    #[tokio::test]
    async fn call_router_maps_undecodable_body_to_bad_json() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_body("not json")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let err = call_router(
            &format!("{}/api/chat", server.url()),
            "m",
            &client,
            &[],
            "q",
            &token,
        )
        .await
        .unwrap_err();
        mock.assert_async().await;
        assert_eq!(err, SearchError::LlmBadJson);
    }

    #[tokio::test]
    async fn call_router_maps_inner_non_router_json_to_bad_json() {
        let mut server = mockito::Server::new_async().await;
        let body =
            serde_json::json!({ "message": { "content": r#"{"random":"shape"}"# } }).to_string();
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let err = call_router(
            &format!("{}/api/chat", server.url()),
            "m",
            &client,
            &[],
            "q",
            &token,
        )
        .await
        .unwrap_err();
        mock.assert_async().await;
        assert_eq!(err, SearchError::LlmBadJson);
    }

    #[tokio::test]
    async fn call_router_trims_whitespace_around_inner_json() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::json!({
            "message": { "content": "  \n{\"action\":\"answer_from_context\"}\n  " }
        })
        .to_string();
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let decision = call_router(
            &format!("{}/api/chat", server.url()),
            "m",
            &client,
            &[],
            "q",
            &token,
        )
        .await
        .unwrap();
        mock.assert_async().await;
        assert_eq!(decision, RouterDecision::AnswerFromContext);
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn judge_prompt_declares_verdict_schema() {
        let p = JUDGE_SYSTEM_PROMPT;
        assert!(p.contains("sufficiency"));
        assert!(p.contains("reasoning"));
        assert!(p.contains("gap_queries"));
        assert!(p.contains("sufficient"));
        assert!(p.contains("partial"));
        assert!(p.contains("insufficient"));
    }

    #[test]
    fn synthesis_prompt_still_has_today_placeholder_and_citation_guidance() {
        let p = SYNTHESIS_SYSTEM_PROMPT;
        assert!(p.contains("{{TODAY}}"));
        assert!(p.contains("[1]"));
        assert!(p.contains("full-page chunk"));
    }

    #[test]
    fn router_merged_prompt_has_today_placeholder_and_required_fields() {
        let p = ROUTER_MERGED_SYSTEM_PROMPT;
        assert!(p.contains("{{TODAY}}"));
        assert!(p.contains("\"action\""));
        assert!(p.contains("clarify"));
        assert!(p.contains("proceed"));
        assert!(p.contains("history_sufficiency"));
        assert!(p.contains("optimized_query"));
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

    // ── build_messages_with_system ───────────────────────────────────────────

    #[test]
    fn build_messages_with_system_interleaves_history() {
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
        let msgs = build_messages_with_system("sys", &history, "q");
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].content, "sys");
        assert_eq!(msgs[1].content, "prev");
        assert_eq!(msgs[2].content, "reply");
        assert_eq!(msgs[3].content, "q");
    }

    // ── call_router_merged ───────────────────────────────────────────────────

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
        )
        .await
        .unwrap_err();
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
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SearchError::Router(_)));
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
        )
        .await
        .unwrap_err();
        assert_eq!(err, SearchError::Cancelled);
    }

    #[tokio::test]
    async fn judge_call_returns_judge_error_when_no_json_in_response() {
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
        let err = call_judge(
            &format!("{}/api/chat", server.uri()),
            "m",
            &client,
            "q",
            &[],
            &token,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SearchError::Judge(_)));
    }
}
