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

/// Streams text chunks from the local Ollama backend via `reqwest` to the frontend using `Channel`.
/// Uses `State` to persist the HTTP Client's connection pool.
#[tauri::command]
pub async fn ask_ollama(
    prompt: String,
    on_event: Channel<StreamChunk>,
    client: State<'_, reqwest::Client>,
) -> Result<(), String> {
    let endpoint = format!("{}/api/generate", DEFAULT_OLLAMA_URL.trim_end_matches('/'));

    let request_payload = OllamaRequest {
        model: DEFAULT_MODEL_NAME.to_string(),
        prompt,
        stream: true,
    };

    let res = client.post(&endpoint).json(&request_payload).send().await;

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
                let _ = on_event.send(StreamChunk::Error(err_msg));
                return Ok(());
            }

            let mut stream = response.bytes_stream();
            let mut buffer: Vec<u8> = Vec::new();

            // Handle network chunks buffering carefully to preserve complete JSON strings
            // and UTF-8 boundaries.
            while let Some(chunk_res) = stream.next().await {
                match chunk_res {
                    Ok(bytes) => {
                        buffer.extend_from_slice(&bytes);

                        while let Some(idx) = buffer.iter().position(|&b| b == b'\n') {
                            // Safely drain the buffer strictly up to the newline delimiter
                            let line_bytes = buffer.drain(..=idx).collect::<Vec<u8>>();
                            if let Ok(line_text) = String::from_utf8(line_bytes) {
                                let trimmed = line_text.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }

                                if let Ok(json) = serde_json::from_str::<OllamaResponse>(trimmed) {
                                    if let Some(token) = json.response {
                                        let _ = on_event.send(StreamChunk::Token(token));
                                    }
                                    if let Some(true) = json.done {
                                        let _ = on_event.send(StreamChunk::Done);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = on_event.send(StreamChunk::Error(e.to_string()));
                    }
                }
            }
        }
        Err(e) => {
            let _ = on_event.send(StreamChunk::Error(e.to_string()));
        }
    }

    Ok(())
}
