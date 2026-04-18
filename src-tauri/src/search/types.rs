//! Shared types for the `/search` pipeline: streamed frontend events, internal
//! router decisions deserialised from the LLM, SearXNG response shapes, and
//! structured pipeline errors.
//!
//! All types that cross the Tauri IPC boundary (see [`SearchEvent`]) are
//! serialised with a `type` tag so the frontend can discriminate cleanly.
//! Internal types ([`RouterDecision`], [`SearxResult`]) never leave Rust.

use serde::{Deserialize, Serialize};

// ─── Streamed events ────────────────────────────────────────────────────────

/// A search result forwarded to the frontend for the sources footer. Only
/// `title` and `url` are included; the snippet content stays on the Rust side.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SearchResultPreview {
    pub title: String,
    pub url: String,
}

/// Structured event emitted to the frontend over the Tauri Channel during a
/// `search_pipeline` invocation. Matches the `SearchEvent` TypeScript union.
///
/// Lifecycle per pipeline run:
/// - `Classifying` -> `Clarifying` -> `Done`
/// - `Classifying` -> `Token`* -> `Done`  (answer-from-context branch)
/// - `Classifying` -> `Searching` -> `Token`* -> `Done`  (search branch)
/// - `Cancelled` may replace `Done` when the user cancels mid-stream.
/// - `Error` may replace any later event on a fatal backend failure.

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum SearchEvent {
    /// Router LLM call is in flight.
    Classifying,
    /// Router decided the query is ambiguous; pipeline terminates after
    /// emitting the clarifying question.
    Clarifying { question: String },
    /// SearXNG lookup is in flight.
    Searching,
    /// SearXNG results arrived; forwarded so the frontend can render the
    /// sources footer after synthesis completes.
    Sources { results: Vec<SearchResultPreview> },
    /// Streaming token from the answering LLM call.
    Token { content: String },
    /// Pipeline finished successfully. Emitted once, last.
    Done,
    /// User cancelled. Emitted instead of `Done` when cancellation wins.
    Cancelled,
    /// Fatal pipeline error with a user-facing message.
    Error { message: String },
}

// ─── Router output ──────────────────────────────────────────────────────────

/// Decoded router decision returned by the classifier LLM call.
///
/// Uses an externally tagged enum (`action` field) so the model never has to
/// emit conditionally-present fields. Each variant carries exactly the data
/// relevant to its branch, which matches what small instruction-tuned models
/// produce most reliably.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RouterDecision {
    /// Query is ambiguous; surface the clarifying question.
    Clarify { question: String },
    /// Prior conversation already contains the answer or the query is stable
    /// general knowledge; stream the answer using the chat system prompt.
    AnswerFromContext,
    /// Query is clear and requires fresh web results; run SearXNG with the
    /// LLM-optimised query, then synthesize.
    Search { optimized_query: String },
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
    /// Ollama returned content that could not be parsed as a RouterDecision.
    LlmBadJson,
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
            Self::SearxUnavailable => {
                "Search service unreachable\nRun `bun run search-box:start` and try again."
                    .to_string()
            }
            Self::SearxHttp(status) => format!("Search service failed\nHTTP {status}"),
            Self::NoResults => "No results found\nTry rephrasing your query.".to_string(),
            Self::EmptyQuery => "Empty query\nType a search query after /search.".to_string(),
            Self::Cancelled => "Cancelled".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_event_serialises_with_type_tag() {
        let classifying = serde_json::to_value(SearchEvent::Classifying).unwrap();
        assert_eq!(classifying["type"], "Classifying");

        let clarifying = serde_json::to_value(SearchEvent::Clarifying {
            question: "Which person?".into(),
        })
        .unwrap();
        assert_eq!(clarifying["type"], "Clarifying");
        assert_eq!(clarifying["question"], "Which person?");

        let token = serde_json::to_value(SearchEvent::Token {
            content: "hi".into(),
        })
        .unwrap();
        assert_eq!(token["type"], "Token");
        assert_eq!(token["content"], "hi");
    }

    #[test]
    fn search_event_variants_serialise_distinct_tags() {
        let searching = serde_json::to_value(SearchEvent::Searching).unwrap();
        assert_eq!(searching["type"], "Searching");

        let done = serde_json::to_value(SearchEvent::Done).unwrap();
        assert_eq!(done["type"], "Done");

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
    fn router_decision_parses_clarify() {
        let json = r#"{"action":"clarify","question":"Which?"}"#;
        let d: RouterDecision = serde_json::from_str(json).unwrap();
        assert_eq!(
            d,
            RouterDecision::Clarify {
                question: "Which?".into(),
            }
        );
    }

    #[test]
    fn router_decision_parses_answer_from_context() {
        let json = r#"{"action":"answer_from_context"}"#;
        let d: RouterDecision = serde_json::from_str(json).unwrap();
        assert_eq!(d, RouterDecision::AnswerFromContext);
    }

    #[test]
    fn router_decision_parses_search() {
        let json = r#"{"action":"search","optimized_query":"rust async"}"#;
        let d: RouterDecision = serde_json::from_str(json).unwrap();
        assert_eq!(
            d,
            RouterDecision::Search {
                optimized_query: "rust async".into(),
            }
        );
    }

    #[test]
    fn router_decision_rejects_unknown_action() {
        let json = r#"{"action":"explode"}"#;
        assert!(serde_json::from_str::<RouterDecision>(json).is_err());
    }

    #[test]
    fn router_decision_rejects_missing_action() {
        let json = r#"{"question":"what"}"#;
        assert!(serde_json::from_str::<RouterDecision>(json).is_err());
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
        assert!(SearchError::SearxUnavailable
            .user_message()
            .contains("search-box:start"));
        assert!(SearchError::SearxHttp(503).user_message().contains("503"));
        assert!(SearchError::NoResults.user_message().contains("No results"));
        assert!(SearchError::EmptyQuery
            .user_message()
            .contains("Empty query"));
        assert_eq!(SearchError::Cancelled.user_message(), "Cancelled");
    }
}
