use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{ipc::Channel, State};
use tokio_util::sync::CancellationToken;

/// Default configuration constants as the application currently lacks a Settings UI.
const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_MODEL_NAME: &str = "llama3.2:3b";

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

/// Request payload to Ollama server.
#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
}

/// Expected structured response chunk from Ollama.
#[derive(Deserialize)]
struct OllamaResponse {
    response: Option<String>,
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
        Self::default()
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

/// Core streaming logic, separated from the Tauri command for testability.
///
/// Streams newline-delimited JSON from the Ollama `/api/generate` endpoint,
/// emitting `StreamChunk` variants via the provided callback. Uses `tokio::select!`
/// to race each chunk read against the cancellation token, ensuring the HTTP
/// connection is dropped immediately when the user cancels — which signals
/// Ollama to stop inference via Go's `context.Context` propagation.
pub async fn stream_ollama(
    endpoint: &str,
    model: &str,
    prompt: String,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    on_chunk: impl Fn(StreamChunk),
) {
    let request_payload = OllamaRequest {
        model: model.to_string(),
        prompt,
        stream: true,
    };

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
                return;
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
                        return;
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
                                            serde_json::from_str::<OllamaResponse>(trimmed)
                                        {
                                            if let Some(token) = json.response {
                                                if !token.is_empty() {
                                                    on_chunk(StreamChunk::Token(token));
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
                                return;
                            }
                            None => return,
                        }
                    }
                }
            }
        }
        Err(e) => {
            on_chunk(StreamChunk::Error(e.to_string()));
        }
    }
}

/// Streams text chunks from the local Ollama backend via `reqwest` to the frontend using `Channel`.
///
/// Creates a fresh `CancellationToken` for each request and stores it in `GenerationState`,
/// allowing `cancel_generation` to abort the stream at any time.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn ask_ollama(
    prompt: String,
    on_event: Channel<StreamChunk>,
    client: State<'_, reqwest::Client>,
    generation: State<'_, GenerationState>,
) -> Result<(), String> {
    let endpoint = format!("{}/api/generate", DEFAULT_OLLAMA_URL.trim_end_matches('/'));
    let cancel_token = CancellationToken::new();
    generation.set(cancel_token.clone());

    stream_ollama(
        &endpoint,
        DEFAULT_MODEL_NAME,
        prompt,
        &client,
        cancel_token,
        |chunk| {
            let _ = on_event.send(chunk);
        },
    )
    .await;

    generation.clear();
    Ok(())
}

/// Cancels the currently active generation, if any.
///
/// Signals the `CancellationToken` stored in `GenerationState`, which causes the
/// `stream_ollama` loop to exit immediately and drop the HTTP connection to Ollama.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn cancel_generation(generation: State<'_, GenerationState>) -> Result<(), String> {
    generation.cancel();
    Ok(())
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

    #[tokio::test]
    async fn streams_tokens_from_valid_response() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .with_body(
                "{\"response\":\"Hello\",\"done\":false}\n\
                 {\"response\":\" world\",\"done\":false}\n\
                 {\"response\":\"\",\"done\":true}\n",
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
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
    }

    #[tokio::test]
    async fn handles_http_500() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Error(e) if e.contains("500")));
    }

    #[tokio::test]
    async fn handles_connection_refused() {
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            "http://127.0.0.1:1/api/generate",
            "test-model",
            "hi".to_string(),
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Error(_)));
    }

    #[tokio::test]
    async fn handles_malformed_json() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .with_body("not json at all\n{\"response\":\"ok\",\"done\":true}\n")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
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
            .mock("POST", "/api/generate")
            .with_body("")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
            &client,
            token,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn tokens_arrive_in_order() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .with_body(
                "{\"response\":\"A\",\"done\":false}\n\
                 {\"response\":\"B\",\"done\":false}\n\
                 {\"response\":\"C\",\"done\":false}\n\
                 {\"response\":\"\",\"done\":true}\n",
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
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
    }

    #[tokio::test]
    async fn handles_invalid_utf8_in_stream() {
        let mut server = mockito::Server::new_async().await;
        let body = b"\xFF\xFE\n{\"response\":\"ok\",\"done\":true}\n".to_vec();
        let mock = server
            .mock("POST", "/api/generate")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
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

        stream_ollama(
            &format!("http://127.0.0.1:{}/api/generate", port),
            "test-model",
            "hi".to_string(),
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
            .mock("POST", "/api/generate")
            .with_status(500)
            .with_body("")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
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
        let mock = server
            .mock("POST", "/api/generate")
            .with_body("   \n{\"response\":\"hi\",\"done\":true}\n")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
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
    async fn response_field_absent_emits_only_done() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .with_body("{\"done\":true}\n")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
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
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\n\
                      Content-Type: application/x-ndjson\r\n\r\n\
                      {\"response\":\"A\",\"done\":false}\n",
                )
                .await;
            // Keep the connection open so cancellation can interrupt
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        });

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let (chunks, callback) = collect_chunks();

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            token_clone.cancel();
        });

        stream_ollama(
            &format!("http://127.0.0.1:{}/api/generate", port),
            "test-model",
            "hi".to_string(),
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
    }

    #[tokio::test]
    async fn pre_cancelled_token_emits_cancelled_immediately() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/api/generate")
            .with_body(
                "{\"response\":\"Hello\",\"done\":false}\n\
                 {\"response\":\"\",\"done\":true}\n",
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        token.cancel();

        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
            &client,
            token,
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Cancelled)));
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
}
