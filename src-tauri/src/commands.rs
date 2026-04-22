use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, State};
use tokio_util::sync::CancellationToken;

/// Default configuration constants as the application currently lacks a Settings UI.
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_MODEL_NAME: &str = "gemma4:e2b";
const DEFAULT_SYSTEM_PROMPT: &str = include_str!("../prompts/system_prompt.txt");

/// Classifies the kind of error returned from the Ollama backend.
/// Used by the frontend to pick accent bar color and display copy.
#[derive(Clone, Serialize, PartialEq, Debug)]
#[serde(rename_all = "PascalCase")]
pub enum OllamaErrorKind {
    /// Ollama process is not running (connection refused / timeout).
    NotRunning,
    /// The requested model has not been pulled yet (HTTP 404).
    ModelNotFound,
    /// Any other unexpected error.
    Other,
}

/// Structured error emitted over the streaming channel.
/// Rust owns all user-facing copy; the frontend only uses `kind` for styling.
#[derive(Clone, Serialize, Debug)]
pub struct OllamaError {
    pub kind: OllamaErrorKind,
    /// Final user-facing string. First line is the title, remainder is the subtitle.
    pub message: String,
}

/// Maps an HTTP status code to a user-friendly `OllamaError`.
pub fn classify_http_error(status: u16) -> OllamaError {
    match status {
        404 => OllamaError {
            kind: OllamaErrorKind::ModelNotFound,
            message: format!(
                "Model not found\nRun: ollama pull {} in a terminal.",
                DEFAULT_MODEL_NAME
            ),
        },
        _ => OllamaError {
            kind: OllamaErrorKind::Other,
            message: format!("Something went wrong\nHTTP {status}"),
        },
    }
}

/// Maps a reqwest connection/transport error to a user-friendly `OllamaError`.
pub fn classify_stream_error(e: &reqwest::Error) -> OllamaError {
    if e.is_connect() || e.is_timeout() {
        OllamaError {
            kind: OllamaErrorKind::NotRunning,
            message: "Ollama isn't running\nStart Ollama and try again.".to_string(),
        }
    } else {
        OllamaError {
            kind: OllamaErrorKind::Other,
            message: "Something went wrong\nCould not reach Ollama.".to_string(),
        }
    }
}

/// Payload emitted back to the frontend per token chunk.
#[derive(Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum StreamChunk {
    /// A single token chunk string.
    Token(String),
    /// A single thinking/reasoning token chunk string.
    ThinkingToken(String),
    /// Indicates the stream has fully completed.
    Done,
    /// The user explicitly cancelled generation.
    Cancelled,
    /// A structured, user-friendly error occurred during processing.
    Error(OllamaError),
}

/// A single message in the Ollama `/api/chat` conversation format.
///
/// The optional `images` field carries base64-encoded image data for
/// multimodal models. When absent or empty, the message is text-only.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
}

/// Sampling parameters for Ollama `/api/chat`, following Google's recommended
/// configuration for Gemma4 models.
#[derive(Serialize)]
struct OllamaOptions {
    temperature: f64,
    top_p: f64,
    top_k: u32,
}

/// Request payload for Ollama `/api/chat` endpoint.
#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    think: bool,
    options: OllamaOptions,
}

/// Nested message object in Ollama `/api/chat` response chunks.
#[derive(Deserialize)]
struct OllamaChatResponseMessage {
    content: Option<String>,
    thinking: Option<String>,
}

/// Expected structured response chunk from Ollama `/api/chat`.
#[derive(Deserialize)]
struct OllamaChatResponse {
    message: Option<OllamaChatResponseMessage>,
    done: Option<bool>,
}

/// Holds the active cancellation token for the current generation request.
///
/// Only one generation runs at a time - starting a new request replaces the
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
    pub fn set_token(&self, token: CancellationToken) {
        *self.token.lock().unwrap() = Some(token);
    }

    /// Cancels the active generation, if any, and clears the stored token.
    pub fn cancel(&self) {
        if let Some(token) = self.token.lock().unwrap().take() {
            token.cancel();
        }
    }

    /// Clears the stored token without cancelling it (used on natural completion).
    pub fn clear_token(&self) {
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

/// Model configuration loaded once at startup from the `THUKI_SUPPORTED_AI_MODELS`
/// environment variable (comma-separated list). The first entry is the active model
/// used for inference. Falls back to `DEFAULT_MODEL_NAME` when unset or empty.
pub struct ModelConfig {
    pub active: String,
    pub all: Vec<String>,
}

/// Reads `THUKI_SUPPORTED_AI_MODELS` from the environment and returns a
/// `ModelConfig`. Trims whitespace around each entry and filters empty entries.
/// Defaults to `[DEFAULT_MODEL_NAME]` when the variable is unset or empty.
pub fn load_model_config() -> ModelConfig {
    let models: Vec<String> = std::env::var("THUKI_SUPPORTED_AI_MODELS")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            s.split(',')
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect()
        })
        .unwrap_or_else(|| vec![DEFAULT_MODEL_NAME.to_string()]);
    let active = models
        .first()
        .cloned()
        .unwrap_or_else(|| DEFAULT_MODEL_NAME.to_string());
    ModelConfig {
        active,
        all: models,
    }
}

/// Returns the active model and full supported list to the frontend.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn get_model_config(model_config: tauri::State<'_, ModelConfig>) -> serde_json::Value {
    serde_json::json!({ "active": model_config.active, "all": model_config.all })
}

/// Core streaming logic for Ollama `/api/chat`, separated from the Tauri
/// command for testability. Uses `tokio::select!` to race each chunk read
/// against the cancellation token, ensuring the HTTP connection is dropped
/// immediately when the user cancels - which signals Ollama to stop inference.
/// Returns the accumulated assistant response so the caller can persist it.
pub async fn stream_ollama_chat(
    endpoint: &str,
    model: &str,
    messages: Vec<ChatMessage>,
    think: bool,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    on_chunk: impl Fn(StreamChunk),
) -> String {
    let request_payload = OllamaChatRequest {
        model: model.to_string(),
        messages,
        stream: true,
        think,
        options: OllamaOptions {
            temperature: 1.0,
            top_p: 0.95,
            top_k: 64,
        },
    };

    let mut accumulated = String::new();

    let res = client.post(endpoint).json(&request_payload).send().await;

    match res {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status().as_u16();
                on_chunk(StreamChunk::Error(classify_http_error(status)));
                return accumulated;
            }

            let mut stream = response.bytes_stream();
            let mut buffer: Vec<u8> = Vec::new();

            loop {
                tokio::select! {
                    biased;
                    _ = cancel_token.cancelled() => {
                        // Drop the stream - closes the HTTP connection,
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
                                            if let Some(ref msg) = json.message {
                                                if let Some(ref thinking) = msg.thinking {
                                                    if !thinking.is_empty() {
                                                        on_chunk(StreamChunk::ThinkingToken(
                                                            thinking.clone(),
                                                        ));
                                                    }
                                                }
                                                if let Some(ref token) = msg.content {
                                                    if !token.is_empty() {
                                                        accumulated.push_str(token);
                                                        on_chunk(StreamChunk::Token(
                                                            token.clone(),
                                                        ));
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
                                on_chunk(StreamChunk::Error(classify_stream_error(&e)));
                                return accumulated;
                            }
                            None => return accumulated,
                        }
                    }
                }
            }
        }
        Err(e) => {
            on_chunk(StreamChunk::Error(classify_stream_error(&e)));
        }
    }

    accumulated
}

/// Streams a chat response from the local Ollama backend. Appends the user
/// message and assistant response to conversation history after completion
/// or cancellation (retaining context for follow-up requests). Uses an epoch
/// counter to prevent stale writes after a reset.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
#[allow(clippy::too_many_arguments)]
pub async fn ask_ollama(
    message: String,
    quoted_text: Option<String>,
    image_paths: Option<Vec<String>>,
    think: bool,
    on_event: Channel<StreamChunk>,
    client: State<'_, reqwest::Client>,
    generation: State<'_, GenerationState>,
    history: State<'_, ConversationHistory>,
    system_prompt: State<'_, SystemPrompt>,
    model_config: State<'_, ModelConfig>,
) -> Result<(), String> {
    let endpoint = format!("{}/api/chat", DEFAULT_OLLAMA_URL.trim_end_matches('/'));
    let cancel_token = CancellationToken::new();
    generation.set_token(cancel_token.clone());

    // Build user message content.  When quoted text is present, label it
    // explicitly so the model knows the highlighted text is the primary
    // subject and any attached images provide surrounding context.
    let content = match quoted_text {
        Some(ref qt) if !qt.trim().is_empty() => {
            format!("[Highlighted Text]\n\"{}\"\n\n[Request]\n{}", qt, message)
        }
        _ => message,
    };

    // Base64-encode attached images for the Ollama multimodal API.
    let images = match image_paths {
        Some(ref paths) if !paths.is_empty() => {
            Some(crate::images::encode_images_as_base64(paths)?)
        }
        _ => None,
    };

    let user_msg = ChatMessage {
        role: "user".to_string(),
        content,
        images,
    };

    // Snapshot the current epoch and build the messages array for Ollama.
    // The user message is NOT yet committed to history - it is only added
    // after a response (including partial/cancelled) to prevent orphaned
    // messages on errors.
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
        &model_config.active,
        messages,
        think,
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
        // Preserve images in history so that follow-up messages can still
        // reference earlier screenshots or attachments.  The full conversation
        // (including base64 blobs) is replayed to Ollama on every turn, which
        // is fine for a localhost-only setup.
        conv.push(user_msg);
        conv.push(ChatMessage {
            role: "assistant".to_string(),
            content: accumulated,
            images: None,
        });
    }

    generation.clear_token();
    Ok(())
}

/// Opens a URL in the system default browser (macOS `open` command).
///
/// Only `http://` and `https://` URLs are accepted; all other schemes are
/// rejected to prevent command injection and accidental protocol handler abuse.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("Only http/https URLs are supported".to_string());
    }
    std::process::Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|e| format!("Failed to open URL: {e}"))?;
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
            false,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Token(t) if t == "Hello"));
        assert!(matches!(&chunks[1], StreamChunk::Token(t) if t == " world"));
        assert_eq!(
            std::mem::discriminant(&chunks[2]),
            std::mem::discriminant(&StreamChunk::Done)
        );
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
            false,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            std::mem::discriminant(&chunks[0]),
            std::mem::discriminant(&StreamChunk::Error(OllamaError {
                kind: OllamaErrorKind::Other,
                message: String::new(),
            }))
        );
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
            false,
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            std::mem::discriminant(&chunks[0]),
            std::mem::discriminant(&StreamChunk::Error(OllamaError {
                kind: OllamaErrorKind::Other,
                message: String::new(),
            }))
        );
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
            false,
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
            false,
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
            false,
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
            false,
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
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 4096];
            let _ = stream.read(&mut req_buf).await;

            let first_line = chat_line("A", false);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\n\r\n{}",
                first_line.len() + 64,
                first_line
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        });

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        let accumulated = stream_ollama_chat(
            &format!("http://127.0.0.1:{}/api/chat", port),
            "test-model",
            vec![],
            false,
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(chunks
            .iter()
            .any(|chunk| matches!(chunk, StreamChunk::Token(token) if token == "A")));
        let error = chunks.iter().find_map(|chunk| match chunk {
            StreamChunk::Error(error) => Some(error),
            _ => None,
        });
        assert!(error.is_some());
        assert_eq!(error.unwrap().kind, OllamaErrorKind::Other);
        assert!(chunks
            .iter()
            .all(|chunk| !matches!(chunk, StreamChunk::Done)));
        assert_eq!(accumulated, "A");
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
            false,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(
            matches!(&chunks[0], StreamChunk::Error(e) if e.kind == OllamaErrorKind::Other && e.message.contains("500"))
        );
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
            false,
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
            false,
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
        use tokio::time::{timeout, Duration};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_done = Arc::new(tokio::sync::Notify::new());
        let server_done_clone = server_done.clone();

        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let (mut stream, _) = listener.accept().await.unwrap();
            // Consume the HTTP request so hyper doesn't see an UnexpectedMessage error
            // when it gets the response before its send is acknowledged.
            let mut req_buf = [0u8; 4096];
            let _ = stream.read(&mut req_buf).await;
            let first_line = chat_line("A", false);
            // Large Content-Length keeps the stream open after the first token so
            // the cancel fires mid-stream rather than at connection-close time.
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: 1048576\r\n\r\n{}",
                first_line
            );
            let _ = stream.write_all(header.as_bytes()).await;
            server_done_clone.notified().await;
        });

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let chunks: Arc<StdMutex<Vec<StreamChunk>>> = Arc::new(StdMutex::new(Vec::new()));
        let chunks_clone = chunks.clone();
        let first_token_seen = Arc::new(tokio::sync::Notify::new());
        let first_token_seen_clone = first_token_seen.clone();
        let callback = move |chunk: StreamChunk| {
            if matches!(&chunk, StreamChunk::Token(token) if token == "A") {
                first_token_seen_clone.notify_one();
            }
            chunks_clone.lock().unwrap().push(chunk);
        };

        let cancel_task = tokio::spawn(async move {
            timeout(Duration::from_secs(5), first_token_seen.notified())
                .await
                .expect("expected first token before cancellation");
            token_clone.cancel();
        });

        timeout(
            Duration::from_secs(5),
            stream_ollama_chat(
                &format!("http://127.0.0.1:{}/api/chat", port),
                "test-model",
                vec![],
                false,
                &client,
                token,
                callback,
            ),
        )
        .await
        .expect("expected stream cancellation path to complete");

        cancel_task.await.unwrap();

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
            false,
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
            false,
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
            false,
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

        state.set_token(token);
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

        state.set_token(token);
        state.clear_token();
        assert!(!token_clone.is_cancelled());
    }

    #[test]
    fn generation_state_set_replaces_previous() {
        let state = GenerationState::new();
        let first = CancellationToken::new();
        let first_clone = first.clone();
        let second = CancellationToken::new();
        let second_clone = second.clone();

        state.set_token(first);
        state.set_token(second);

        state.cancel();
        assert!(!first_clone.is_cancelled());
        assert!(second_clone.is_cancelled());
    }

    /// Guard to serialize tests that mutate environment variables.
    /// Rust runs tests in parallel by default; without serialization these
    /// tests race on shared environment variables.
    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    // ── load_model_config tests ──────────────────────────────────────────────

    #[test]
    fn load_model_config_returns_default_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("THUKI_SUPPORTED_AI_MODELS");
        let config = load_model_config();
        assert_eq!(config.active, DEFAULT_MODEL_NAME);
        assert_eq!(config.all, vec![DEFAULT_MODEL_NAME.to_string()]);
    }

    #[test]
    fn load_model_config_reads_single_model() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("THUKI_SUPPORTED_AI_MODELS", "gemma4:e4b");
        let config = load_model_config();
        assert_eq!(config.active, "gemma4:e4b");
        assert_eq!(config.all, vec!["gemma4:e4b".to_string()]);
        std::env::remove_var("THUKI_SUPPORTED_AI_MODELS");
    }

    #[test]
    fn load_model_config_reads_multiple_models_first_is_active() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("THUKI_SUPPORTED_AI_MODELS", "gemma4:e2b,gemma4:e4b");
        let config = load_model_config();
        assert_eq!(config.active, "gemma4:e2b");
        assert_eq!(
            config.all,
            vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()]
        );
        std::env::remove_var("THUKI_SUPPORTED_AI_MODELS");
    }

    #[test]
    fn load_model_config_trims_whitespace_around_entries() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("THUKI_SUPPORTED_AI_MODELS", " gemma4:e2b , gemma4:e4b ");
        let config = load_model_config();
        assert_eq!(config.active, "gemma4:e2b");
        assert_eq!(
            config.all,
            vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()]
        );
        std::env::remove_var("THUKI_SUPPORTED_AI_MODELS");
    }

    #[test]
    fn load_model_config_falls_back_to_default_when_whitespace_only() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("THUKI_SUPPORTED_AI_MODELS", "   ");
        let config = load_model_config();
        assert_eq!(config.active, DEFAULT_MODEL_NAME);
        assert_eq!(config.all, vec![DEFAULT_MODEL_NAME.to_string()]);
        std::env::remove_var("THUKI_SUPPORTED_AI_MODELS");
    }

    #[test]
    fn load_model_config_filters_empty_entries_from_list() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("THUKI_SUPPORTED_AI_MODELS", "gemma4:e2b,,gemma4:e4b");
        let config = load_model_config();
        assert_eq!(
            config.all,
            vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()]
        );
        std::env::remove_var("THUKI_SUPPORTED_AI_MODELS");
    }

    #[test]
    fn load_model_config_falls_back_when_all_entries_are_empty_commas() {
        let _guard = ENV_LOCK.lock().unwrap();
        // All entries filter to empty strings, leaving an empty list.
        // The active model must still fall back to DEFAULT_MODEL_NAME.
        std::env::set_var("THUKI_SUPPORTED_AI_MODELS", ",");
        let config = load_model_config();
        assert_eq!(config.active, DEFAULT_MODEL_NAME);
        assert_eq!(config.all, Vec::<String>::new());
        std::env::remove_var("THUKI_SUPPORTED_AI_MODELS");
    }

    // ── sampling options test ────────────────────────────────────────────────

    #[tokio::test]
    async fn sends_sampling_options_in_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"options":{"temperature":1.0,"top_p":0.95,"top_k":64}}"#.to_string(),
            ))
            .with_body(chat_line("", true))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (_, callback) = collect_chunks();

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            false,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
    }

    #[test]
    fn load_system_prompt_returns_default_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("THUKI_SYSTEM_PROMPT");

        let prompt = load_system_prompt();
        assert_eq!(prompt, DEFAULT_SYSTEM_PROMPT);
    }

    #[test]
    fn load_system_prompt_reads_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("THUKI_SYSTEM_PROMPT", "Custom prompt");

        let prompt = load_system_prompt();
        assert_eq!(prompt, "Custom prompt");

        std::env::remove_var("THUKI_SYSTEM_PROMPT");
    }

    #[test]
    fn load_system_prompt_ignores_empty_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
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

    // ─── OllamaError classification ───────────────────────────────────────────

    #[test]
    fn classify_http_404_returns_model_not_found() {
        let err = classify_http_error(404);
        assert_eq!(err.kind, OllamaErrorKind::ModelNotFound);
        assert!(err.message.contains("gemma4:e2b"));
    }

    #[test]
    fn classify_http_500_returns_other_with_status() {
        let err = classify_http_error(500);
        assert_eq!(err.kind, OllamaErrorKind::Other);
        assert!(err.message.contains("500"));
    }

    #[test]
    fn classify_http_401_returns_other_with_status() {
        let err = classify_http_error(401);
        assert_eq!(err.kind, OllamaErrorKind::Other);
        assert!(err.message.contains("401"));
    }

    #[tokio::test]
    async fn connection_refused_emits_not_running_error() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama_chat(
            "http://127.0.0.1:1/api/chat",
            "test-model",
            vec![],
            false,
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(
            matches!(&chunks[0], StreamChunk::Error(e) if e.kind == OllamaErrorKind::NotRunning)
        );
    }

    #[tokio::test]
    async fn http_404_emits_model_not_found_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(404)
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
            false,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(
            matches!(&chunks[0], StreamChunk::Error(e) if e.kind == OllamaErrorKind::ModelNotFound)
        );
    }

    #[test]
    fn thinking_token_serializes_correctly() {
        let chunk = StreamChunk::ThinkingToken("reasoning step".to_string());
        let json = serde_json::to_value(&chunk).unwrap();
        assert_eq!(json["type"], "ThinkingToken");
        assert_eq!(json["data"], "reasoning step");
    }

    #[test]
    fn ollama_chat_request_sends_think_false_explicitly() {
        let req = OllamaChatRequest {
            model: "test".to_string(),
            messages: vec![],
            stream: true,
            think: false,
            options: OllamaOptions {
                temperature: 1.0,
                top_p: 0.95,
                top_k: 64,
            },
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["think"], false);
    }

    #[test]
    fn ollama_chat_request_includes_think_when_true() {
        let req = OllamaChatRequest {
            model: "test".to_string(),
            messages: vec![],
            stream: true,
            think: true,
            options: OllamaOptions {
                temperature: 1.0,
                top_p: 0.95,
                top_k: 64,
            },
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["think"], true);
    }

    #[test]
    fn ollama_response_message_deserializes_thinking_field() {
        let json = r#"{"content":"hello","thinking":"let me think"}"#;
        let msg: OllamaChatResponseMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.content.unwrap(), "hello");
        assert_eq!(msg.thinking.unwrap(), "let me think");
    }

    #[test]
    fn ollama_response_message_thinking_absent() {
        let json = r#"{"content":"hello"}"#;
        let msg: OllamaChatResponseMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.content.unwrap(), "hello");
        assert!(msg.thinking.is_none());
    }

    #[tokio::test]
    async fn http_500_emits_other_error_with_status() {
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

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            false,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(
            matches!(&chunks[0], StreamChunk::Error(e) if e.kind == OllamaErrorKind::Other && e.message.contains("500"))
        );
    }

    /// Helper: builds a `/api/chat` response line with both thinking and content fields.
    fn chat_line_with_thinking(thinking: &str, content: &str, done: bool) -> String {
        format!(
            "{{\"message\":{{\"role\":\"assistant\",\"content\":\"{}\",\"thinking\":\"{}\"}},\"done\":{}}}\n",
            content, thinking, done
        )
    }

    #[tokio::test]
    async fn stream_ollama_chat_emits_thinking_tokens() {
        let mut server = mockito::Server::new_async().await;
        let body = format!(
            "{}{}{}",
            chat_line_with_thinking("step 1", "", false),
            chat_line_with_thinking("", "Hello", false),
            chat_line_with_thinking("", "", true),
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
            true,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();

        // ThinkingToken emitted for thinking field
        assert!(matches!(&chunks[0], StreamChunk::ThinkingToken(t) if t == "step 1"));
        // Token emitted for content field
        assert!(matches!(&chunks[1], StreamChunk::Token(t) if t == "Hello"));
        // Done emitted
        assert_eq!(
            std::mem::discriminant(&chunks[2]),
            std::mem::discriminant(&StreamChunk::Done)
        );

        // Accumulated return value contains only content, not thinking
        assert_eq!(accumulated, "Hello");
    }

    #[tokio::test]
    async fn stream_ollama_chat_sends_think_true_in_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"think":true}"#.to_string(),
            ))
            .with_body(chat_line("", true))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (_, callback) = collect_chunks();

        stream_ollama_chat(
            &format!("{}/api/chat", server.url()),
            "test-model",
            vec![],
            true,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn stream_ollama_chat_empty_thinking_not_emitted() {
        let mut server = mockito::Server::new_async().await;
        let body = format!(
            "{}{}",
            chat_line_with_thinking("", "Hello", false),
            chat_line_with_thinking("", "", true),
        );
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
            true,
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();

        // No ThinkingToken emitted for empty thinking field
        assert!(chunks
            .iter()
            .all(|c| !matches!(c, StreamChunk::ThinkingToken(_))));
        // Content token still emitted
        assert!(chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::Token(t) if t == "Hello")));
    }
}
