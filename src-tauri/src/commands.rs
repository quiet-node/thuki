use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, State};

/// Default configuration constants as the application currently lacks a Settings UI.
const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_MODEL_NAME: &str = "llama3.2:3b";

/// Payload emitted back to the frontend per token chunk
#[derive(Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum StreamChunk {
    /// A single token chunk string
    Token(String),
    /// Indicates the stream has fully completed
    Done,
    /// An error occurred during processing
    Error(String),
}

/// Request payload to Ollama server
#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
}

/// Expected structured response chunk from Ollama
#[derive(Deserialize)]
struct OllamaResponse {
    response: Option<String>,
    done: Option<bool>,
}

/// Core streaming logic, separated from the Tauri command for testability.
pub async fn stream_ollama(
    endpoint: &str,
    model: &str,
    prompt: String,
    client: &reqwest::Client,
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

            while let Some(chunk_res) = stream.next().await {
                match chunk_res {
                    Ok(bytes) => {
                        buffer.extend_from_slice(&bytes);

                        while let Some(idx) = buffer.iter().position(|&b| b == b'\n') {
                            let line_bytes = buffer.drain(..=idx).collect::<Vec<u8>>();
                            if let Ok(line_text) = String::from_utf8(line_bytes) {
                                let trimmed = line_text.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }

                                if let Ok(json) = serde_json::from_str::<OllamaResponse>(trimmed) {
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
                    Err(e) => {
                        on_chunk(StreamChunk::Error(e.to_string()));
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
/// Uses `State` to persist the HTTP Client's connection pool.
#[tauri::command]
pub async fn ask_ollama(
    prompt: String,
    on_event: Channel<StreamChunk>,
    client: State<'_, reqwest::Client>,
) -> Result<(), String> {
    let endpoint = format!("{}/api/generate", DEFAULT_OLLAMA_URL.trim_end_matches('/'));

    stream_ollama(&endpoint, DEFAULT_MODEL_NAME, prompt, &client, |chunk| {
        let _ = on_event.send(chunk);
    })
    .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn collect_chunks() -> (Arc<Mutex<Vec<StreamChunk>>>, impl Fn(StreamChunk)) {
        let chunks: Arc<Mutex<Vec<StreamChunk>>> = Arc::new(Mutex::new(Vec::new()));
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
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
            &client,
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
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
            &client,
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
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            "http://127.0.0.1:1/api/generate",
            "test-model",
            "hi".to_string(),
            &client,
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
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
            &client,
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
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
            &client,
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
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
            &client,
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
    async fn http_500_with_empty_body() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .with_status(500)
            .with_body("")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();

        stream_ollama(
            &format!("{}/api/generate", server.url()),
            "test-model",
            "hi".to_string(),
            &client,
            callback,
        )
        .await;

        mock.assert_async().await;
        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Error(e) if e.contains("HTTP 500")));
    }
}
