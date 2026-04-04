use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, State};
use tokio_util::sync::CancellationToken;

/// Default configuration constants as the application currently lacks a Settings UI.
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_MODEL_NAME: &str = "gemma3:4b";
const DEFAULT_SYSTEM_PROMPT: &str = "You are Thuki (thư ký), a personal desktop secretary that \
lives as a floating overlay on macOS. You are fast, sharp, and helpful.\n\nResponse style:\n- Be \
concise. You appear in a small floating window — keep responses scannable.\n- Use short paragraphs, \
bullet points, and code blocks where appropriate.\n- Lead with the answer, then explain if needed. \
Never pad with filler.\n- Match the user's tone — casual if they're casual, precise if they're \
technical.\n\nWhen the user provides context (quoted text from another app):\n- Treat it as the \
subject of their question unless they say otherwise.\n- Summarize, explain, fix, or transform it as \
asked.\n- Don't repeat the context back unless specifically helpful.\n\nYou excel at: quick answers, \
summarizing text, explaining code, drafting messages, brainstorming ideas, and catching errors. You \
are the user's second brain — always ready, never in the way.";

/// Payload emitted back to the frontend per token chunk.
#[derive(Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum StreamChunk {
    /// A single token chunk string.
    Token(String),
    /// Indicates the stream has fully completed.
    Done,
    /// The user explicitly cancelled generation.
    Cancelled,
    /// An error occurred during processing.
    Error(String),
}

/// A single message in the Ollama `/api/chat` conversation format.
///
/// The optional `images` field carries base64-encoded image data for
/// multimodal models (e.g. `gemma3:4b`). When absent or empty, the
/// message is text-only.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
}

/// Request payload for Ollama `/api/chat` endpoint.
#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
}

/// Nested message object in Ollama `/api/chat` response chunks.
#[derive(Deserialize)]
struct OllamaChatResponseMessage {
    content: Option<String>,
}

/// Expected structured response chunk from Ollama `/api/chat`.
#[derive(Deserialize)]
struct OllamaChatResponse {
    message: Option<OllamaChatResponseMessage>,
    done: Option<bool>,
}

/// Holds the active cancellation token for the current generation request.
///
/// Only one generation runs at a time — starting a new request replaces the
/// previous token. `cancel_generation` cancels whatever is currently active.
#[derive(Default)]
pub struct GenerationState {
    token: Mutex<Option<CancellationToken>>,
}

impl GenerationState {
    /// Creates a new empty generation state with no active token.
    pub fn new() -> Self {
        Self {
            token: Mutex::new(None),
        }
    }

    /// Stores a new cancellation token, replacing any previous one.
    fn set(&self, token: CancellationToken) {
        *self.token.lock().unwrap() = Some(token);
    }

    /// Cancels the active generation, if any, and clears the stored token.
    fn cancel(&self) {
        if let Some(token) = self.token.lock().unwrap().take() {
            token.cancel();
        }
    }

    /// Clears the stored token without cancelling it (used on natural completion).
    fn clear(&self) {
        *self.token.lock().unwrap() = None;
    }
}

/// Backend-managed conversation history with an epoch counter to prevent
/// stale writes after a reset. The Rust side is the source of truth; the
/// frontend sends only new user messages and receives streamed tokens.
pub struct ConversationHistory {
    pub messages: Mutex<Vec<ChatMessage>>,
    pub epoch: AtomicU64,
}

impl Default for ConversationHistory {
    fn default() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            epoch: AtomicU64::new(0),
        }
    }
}

impl ConversationHistory {
    /// Creates a new empty conversation history at epoch 0.
    pub fn new() -> Self {
        Self::default()
    }
}

/// System prompt loaded once at startup from the `THUKI_SYSTEM_PROMPT`
/// environment variable, falling back to a built-in default.
pub struct SystemPrompt(pub String);

/// Reads `THUKI_SYSTEM_PROMPT` from the environment, falling back to the
/// built-in default when unset or empty.
pub fn load_system_prompt() -> String {
    std::env::var("THUKI_SYSTEM_PROMPT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string())
}

/// Core streaming logic for Ollama `/api/chat`, separated from the Tauri
/// command for testability. Uses `tokio::select!` to race each chunk read
/// against the cancellation token, ensuring the HTTP connection is dropped
/// immediately when the user cancels — which signals Ollama to stop inference.
/// Returns the accumulated assistant response so the caller can persist it.
pub async fn stream_ollama_chat(
    endpoint: &str,
    model: &str,
    messages: Vec<ChatMessage>,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    on_chunk: impl Fn(StreamChunk),
) -> String {
    let request_payload = OllamaChatRequest {
        model: model.to_string(),
        messages,
        stream: true,
    };

    let mut accumulated = String::new();

    let res = client.post(endpoint).json(&request_payload).send().await;

    match res {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status();
                let err_body = response.text().await.unwrap_or_default();
                let err_msg = if err_body.is_empty() {
                    format!("HTTP {}", status)
                } else {
                    format!("HTTP {}: {}", status, err_body)
                };
                on_chunk(StreamChunk::Error(err_msg));
                return accumulated;
            }

            let mut stream = response.bytes_stream();
            let mut buffer: Vec<u8> = Vec::new();

            loop {
                tokio::select! {
                    biased;
                    _ = cancel_token.cancelled() => {
                        // Drop the stream — closes the HTTP connection,
                        // which signals Ollama to stop inference.
                        drop(stream);
                        on_chunk(StreamChunk::Cancelled);
                        return accumulated;
                    }
                    chunk_opt = stream.next() => {
                        match chunk_opt {
                            Some(Ok(bytes)) => {
                                buffer.extend_from_slice(&bytes);

                                while let Some(idx) = buffer.iter().position(|&b| b == b'\n') {
                                    let line_bytes = buffer.drain(..=idx).collect::<Vec<u8>>();
                                    if let Ok(line_text) = String::from_utf8(line_bytes) {
                                        let trimmed = line_text.trim();
                                        if trimmed.is_empty() {
                                            continue;
                                        }

                                        if let Ok(json) =
                                            serde_json::from_str::<OllamaChatResponse>(trimmed)
                                        {
                                            if let Some(msg) = json.message {
                                                if let Some(token) = msg.content {
                                                    if !token.is_empty() {
                                                        accumulated.push_str(&token);
                                                        on_chunk(StreamChunk::Token(token));
                                                    }
                                                }
                                            }
                                            if let Some(true) = json.done {
                                                on_chunk(StreamChunk::Done);
                                            }
                                        }
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                on_chunk(StreamChunk::Error(e.to_string()));
                                return accumulated;
                            }
                            None => return accumulated,
                        }
                    }
                }
            }
        }
        Err(e) => {
            on_chunk(StreamChunk::Error(e.to_string()));
        }
    }

    accumulated
}

/// Streams a chat response from the local Ollama backend. Appends the user
/// message and assistant response to conversation history only after successful
/// completion. Uses an epoch counter to prevent stale writes after a reset.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn ask_ollama(
    message: String,
    quoted_text: Option<String>,
    on_event: Channel<StreamChunk>,
    client: State<'_, reqwest::Client>,
    generation: State<'_, GenerationState>,
    history: State<'_, ConversationHistory>,
    system_prompt: State<'_, SystemPrompt>,
) -> Result<(), String> {
    let endpoint = format!("{}/api/chat", DEFAULT_OLLAMA_URL.trim_end_matches('/'));
    let cancel_token = CancellationToken::new();
    generation.set(cancel_token.clone());

    // Build user message content, prepending quoted context when present.
    let content = match quoted_text {
        Some(ref qt) if !qt.trim().is_empty() => format!("Context: \"{}\"\n\n{}", qt, message),
        _ => message,
    };

    let user_msg = ChatMessage {
        role: "user".to_string(),
        content,
        images: None,
    };

    // Snapshot the current epoch and build the messages array for Ollama.
    // The user message is NOT yet committed to history — it is only added
    // after a successful response to prevent orphaned messages on errors.
    let (epoch_at_start, messages) = {
        let conv = history.messages.lock().unwrap();
        let epoch = history.epoch.load(Ordering::SeqCst);
        let mut msgs = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt.0.clone(),
            images: None,
        }];
        msgs.extend(conv.clone());
        msgs.push(user_msg.clone());
        (epoch, msgs)
    };

    let accumulated = stream_ollama_chat(
        &endpoint,
        DEFAULT_MODEL_NAME,
        messages,
        &client,
        cancel_token.clone(),
        |chunk| {
            let _ = on_event.send(chunk);
        },
    )
    .await;

    // Persist user + assistant messages to in-memory history when the epoch
    // has not changed (no reset during streaming) and we received content.
    // This includes cancelled generations so that subsequent requests retain
    // the conversational context (the user message and any partial response).
    let current_epoch = history.epoch.load(Ordering::SeqCst);
    if current_epoch == epoch_at_start && !accumulated.is_empty() {
        let mut conv = history.messages.lock().unwrap();
        conv.push(user_msg);
        conv.push(ChatMessage {
            role: "assistant".to_string(),
            content: accumulated,
            images: None,
        });
    }

    generation.clear();
    Ok(())
}

/// Cancels the currently active generation, if any.
///
/// Signals the `CancellationToken` stored in `GenerationState`, which causes the
/// `stream_ollama_chat` loop to exit immediately and drop the HTTP connection.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn cancel_generation(generation: State<'_, GenerationState>) -> Result<(), String> {
    generation.cancel();
    Ok(())
}

/// Clears the backend conversation history and increments the epoch counter.
/// The epoch increment prevents any in-flight `ask_ollama` from writing stale
/// messages into the freshly cleared history.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn reset_conversation(history: State<'_, ConversationHistory>) {
    history.epoch.fetch_add(1, Ordering::SeqCst);
    history.messages.lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    fn collect_chunks() -> (Arc<StdMutex<Vec<StreamChunk>>>, impl Fn(StreamChunk)) {
        let chunks: Arc<StdMutex<Vec<StreamChunk>>> = Arc::new(StdMutex::new(Vec::new()));
        let chunks_clone = chunks.clone();
        let callback = move |chunk: StreamChunk| {
            chunks_clone.lock().unwrap().push(chunk);
        };
        (chunks, callback)
    }

    /// Helper: builds a `/api/chat` response line from content + done flag.
    fn chat_line(content: &str, done: bool) -> String {
        format!(
            "{{\"message\":{{\"role\":\"assistant\",\"content\":\"{}\"}},\"done\":{}}}\n",
            content, done
        )
    }

    #[tokio::test]
    async fn streams_tokens_from_valid_response() {
        let mut server = mockito::Server::new_async().await;
        let body = format!(
            "{}{}{}",
            chat_line("Hello", false),
            chat_line(" world", false),
            chat_line("", true),
        );
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            images: None,
        }];

        let accumulated = stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            messages,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Token(t) if t == "Hello"));
        assert!(matches!(&chunks[1], StreamChunk::Token(t) if t == " world"));
        assert!(matches!(&chunks[2], StreamChunk::Done));
        assert_eq!(accumulated, "Hello world");
    }

    #[tokio::test]
    async fn handles_http_500() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Error(e) if e.contains("500")));
        assert!(accumulated.is_empty());
    }

    #[tokio::test]
    async fn handles_connection_refused() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            "http://127.0.0.1:1/api/chat",
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Error(_)));
        assert!(accumulated.is_empty());
    }

    #[tokio::test]
    async fn handles_malformed_json() {
        let mut server = mockito::Server::new_async().await;
        let body = format!("not json at all\n{}", chat_line("ok", true));
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[tokio::test]
    async fn handles_empty_response_body() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_body("")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.is_empty());
        assert!(accumulated.is_empty());
    }

    #[tokio::test]
    async fn tokens_arrive_in_order() {
        let mut server = mockito::Server::new_async().await;
        let body = format!(
            "{}{}{}{}",
            chat_line("A", false),
            chat_line("B", false),
            chat_line("C", false),
            chat_line("", true),
        );
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        let tokens: Vec<&str> = chunks
            .iter()
            .filter_map(|c| match c {
                StreamChunk::Token(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tokens, vec!["A", "B", "C"]);
        assert_eq!(accumulated, "ABC");
    }

    #[tokio::test]
    async fn handles_invalid_utf8_in_stream() {
        let mut server = mockito::Server::new_async().await;
        let mut body = b"\xFF\xFE\n".to_vec();
        body.extend_from_slice(chat_line("ok", true).as_bytes());
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[tokio::test]
    async fn handles_mid_stream_network_error() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\n\
                      Content-Type: application/x-ndjson\r\n\
                      Transfer-Encoding: chunked\r\n\r\n\
                      4\r\ntest",
                )
                .await;
        });

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            &format!("http://127.0.0.1:{}/api/chat", port),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        let has_no_tokens = chunks.iter().all(|c| !matches!(c, StreamChunk::Token(_)));
        assert!(has_no_tokens);
    }

    #[tokio::test]
    async fn http_500_with_empty_body() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(500)
            .with_body("")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Error(e) if e.contains("HTTP 500")));
    }

    #[tokio::test]
    async fn whitespace_only_lines_are_skipped() {
        let mut server = mockito::Server::new_async().await;
        let body = format!("   \n{}", chat_line("hi", true));
        let mock = server
            .mock("POST", "/api/chat")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[tokio::test]
    async fn message_field_absent_emits_only_done() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_body("{\"done\":true}\n")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().all(|c| !matches!(c, StreamChunk::Token(_))));
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[tokio::test]
    async fn cancellation_stops_stream_and_emits_cancelled() {
        use std::sync::Arc;
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_done = Arc::new(tokio::sync::Notify::new());
        let server_done_clone = server_done.clone();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let first_line = chat_line("A", false);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\r\n{}",
                first_line
            );
            let _ = stream.write_all(header.as_bytes()).await;
            server_done_clone.notified().await;
        });

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let (chunks, callback) = collect_chunks();

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            token_clone.cancel();
        });

        stream_ollama_chat(
            &format!("http://127.0.0.1:{}/api/chat", port),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::Token(t) if t == "A")));
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Cancelled)));
        assert!(chunks.iter().all(|c| !matches!(c, StreamChunk::Done)));

        server_done.notify_one();
        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn pre_cancelled_token_emits_cancelled_immediately() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/api/chat")
            .with_body(chat_line("Hello", true))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        token.cancel();

        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Cancelled)));
    }

    #[tokio::test]
    async fn sends_messages_array_in_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"messages":[{"role":"system","content":"Be helpful"},{"role":"user","content":"hi"}]}"#.to_string(),
            ))
            .with_body(chat_line("", true))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (_, callback) = collect_chunks();
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "Be helpful".to_string(),
                images: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
                images: None,
            },
        ];

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            messages,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn message_content_absent_emits_only_done() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_body("{\"message\":{\"role\":\"assistant\"},\"done\":true}\n")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().all(|c| !matches!(c, StreamChunk::Token(_))));
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[test]
    fn generation_state_set_and_cancel() {
        let state = GenerationState::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();

        state.set(token);
        assert!(!token_clone.is_cancelled());

        state.cancel();
        assert!(token_clone.is_cancelled());
    }

    #[test]
    fn generation_state_cancel_when_empty() {
        let state = GenerationState::new();
        state.cancel();
    }

    #[test]
    fn generation_state_clear_does_not_cancel() {
        let state = GenerationState::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();

        state.set(token);
        state.clear();
        assert!(!token_clone.is_cancelled());
    }

    #[test]
    fn generation_state_set_replaces_previous() {
        let state = GenerationState::new();
        let first = CancellationToken::new();
        let first_clone = first.clone();
        let second = CancellationToken::new();
        let second_clone = second.clone();

        state.set(first);
        state.set(second);

        state.cancel();
        assert!(!first_clone.is_cancelled());
        assert!(second_clone.is_cancelled());
    }

    #[test]
    fn load_system_prompt_returns_default_when_unset() {
        std::env::remove_var("THUKI_SYSTEM_PROMPT");

        let prompt = load_system_prompt();
        assert_eq!(prompt, DEFAULT_SYSTEM_PROMPT);
    }

    #[test]
    fn load_system_prompt_reads_env_var() {
        std::env::set_var("THUKI_SYSTEM_PROMPT", "Custom prompt");

        let prompt = load_system_prompt();
        assert_eq!(prompt, "Custom prompt");

        std::env::remove_var("THUKI_SYSTEM_PROMPT");
    }

    #[test]
    fn load_system_prompt_ignores_empty_env_var() {
        std::env::set_var("THUKI_SYSTEM_PROMPT", "   ");

        let prompt = load_system_prompt();
        assert_eq!(prompt, DEFAULT_SYSTEM_PROMPT);

        std::env::remove_var("THUKI_SYSTEM_PROMPT");
    }

    #[test]
    fn conversation_history_new_starts_at_epoch_zero() {
        let h = ConversationHistory::new();
        assert_eq!(h.epoch.load(Ordering::SeqCst), 0);
        assert!(h.messages.lock().unwrap().is_empty());
    }

    #[test]
    fn conversation_history_epoch_increments_on_clear() {
        let h = ConversationHistory::new();
        h.messages.lock().unwrap().push(ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            images: None,
        });

        h.epoch.fetch_add(1, Ordering::SeqCst);
        h.messages.lock().unwrap().clear();

        assert_eq!(h.epoch.load(Ordering::SeqCst), 1);
        assert!(h.messages.lock().unwrap().is_empty());
    }
}
