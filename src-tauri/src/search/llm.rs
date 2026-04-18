//! LLM operations for the `/search` pipeline.
//!
//! Two concerns live here:
//! 1. The non-streaming **router call** that classifies the user's query into
//!    a [`RouterDecision`]. Uses Ollama's `format: "json"` mode with a
//!    deterministic sampling profile so the output is strictly parseable.
//! 2. Prompt-assembly helpers that produce the message array fed to the
//!    streaming answer stage (either `answer_from_context` or `search`).
//!
//! All functions are pure with respect to external state (no globals, no
//! hidden side effects) and accept their dependencies explicitly for
//! testability.

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::commands::ChatMessage;

use super::types::{RouterDecision, SearchError, SearxResult};

/// Router system prompt: instructs the classifier LLM on the allowed actions
/// and their JSON output shapes.
pub const ROUTER_SYSTEM_PROMPT: &str = include_str!("../../prompts/search_router.txt");

/// Synthesis system prompt: instructs the answering LLM to cite sources and
/// avoid meta-commentary over the reference material.
pub const SYNTHESIS_SYSTEM_PROMPT: &str = include_str!("../../prompts/search_synthesis.txt");

/// System prompt for the universal sufficiency judge. Used by the pre-synthesis
/// judge call over snippets and over reader chunks.
// Task 11 wires this into the agentic loop; suppress the warning until then.
#[allow(dead_code)]
pub const JUDGE_SYSTEM_PROMPT: &str = include_str!("../../prompts/search_judge.txt");

/// Hard timeout for the non-streaming router call. Picked to accommodate cold
/// model starts on first pipeline invocation.
pub const ROUTER_TIMEOUT_SECS: u64 = 45;

/// Cap on the router response length. Enough for a clarification question
/// with several suggestions; prevents runaway generation when the model
/// fails to produce valid JSON quickly.
pub const ROUTER_MAX_TOKENS: i32 = 512;

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
struct RouterRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    format: String,
    options: RouterOptions,
}

#[derive(Deserialize)]
struct RouterResponseMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct RouterResponseBody {
    message: RouterResponseMessage,
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
    let body = RouterRequest {
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
        .timeout(std::time::Duration::from_secs(ROUTER_TIMEOUT_SECS));

    let response = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => return Err(SearchError::Cancelled),
        res = request.send() => res.map_err(|_| SearchError::LlmUnavailable)?,
    };

    if !response.status().is_success() {
        return Err(SearchError::LlmHttp(response.status().as_u16()));
    }

    let parsed: RouterResponseBody = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => return Err(SearchError::Cancelled),
        body = response.json() => body.map_err(|_| SearchError::LlmBadJson)?,
    };

    serde_json::from_str::<RouterDecision>(parsed.message.content.trim())
        .map_err(|_| SearchError::LlmBadJson)
}

/// Builds the router message array: `[system, ...history, user]`. The router
/// operates on the same conversation history the chat pipeline uses so it can
/// resolve pronouns ("him", "it") against earlier turns.
fn build_router_messages(history: &[ChatMessage], query: &str) -> Vec<ChatMessage> {
    let mut msgs = Vec::with_capacity(history.len() + 2);
    msgs.push(ChatMessage {
        role: "system".to_string(),
        content: ROUTER_SYSTEM_PROMPT.to_string(),
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
}
