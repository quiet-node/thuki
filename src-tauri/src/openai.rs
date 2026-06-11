//! Generic OpenAI-compatible `/v1` chat client.
//!
//! The twin of the native Ollama path in [`crate::commands`]: a streaming SSE
//! chat call ([`stream_openai_chat`]) that emits the exact same
//! [`StreamChunk`] channel contract as `stream_ollama_chat`, and a
//! non-streaming structured-output call ([`request_openai_json`]) that mirrors
//! the search pipeline's `request_json`. Used by the `builtin` (local
//! llama-server) and `openai` provider kinds.

use futures_util::StreamExt;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::commands::{ChatMessage, EngineError, EngineErrorKind, StreamChunk};
use crate::config::defaults::MAX_SSE_LINE_BYTES;

/// Groups the per-request parameters for [`stream_openai_chat`], mirroring
/// `OllamaChatParams` on the native path.
pub struct OpenAiChatParams {
    /// Server origin without a trailing slash; the client appends
    /// `/v1/chat/completions`.
    pub base_url: String,
    pub model: String,
    pub messages: Vec<ChatMessage>,
    /// Sent as a `Bearer` authorization header when `Some`.
    pub api_key: Option<String>,
}

/// Error returned by [`request_openai_json`]. Mirrors the classification the
/// search pipeline's `request_json` applies to its `SearchError` variants:
/// transport failures (including the per-request timeout) map to
/// `Unreachable`, non-2xx statuses to `Http`, unusable bodies to `BadBody`,
/// and token cancellation to `Cancelled`.
#[derive(Debug, PartialEq)]
pub enum OpenAiError {
    /// The server could not be reached (connect, transport, or timeout).
    Unreachable(String),
    /// The server answered with a non-2xx status; carries the response body.
    Http(u16, String),
    /// The response body could not be read or did not match the expected shape.
    BadBody(String),
    /// The caller's cancellation token fired before the response was read.
    Cancelled,
}

// ─── Wire types ──────────────────────────────────────────────────────────────

/// `choices[i].delta` object in a `/v1/chat/completions` SSE event. Unknown
/// fields are ignored so vendor extensions never break parsing.
#[derive(Deserialize, Default)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

/// A single entry of `choices` in an SSE event.
#[derive(Deserialize)]
struct SseChoice {
    #[serde(default)]
    delta: SseDelta,
}

/// The JSON payload of one `data:` SSE line.
#[derive(Deserialize)]
struct SseEvent {
    #[serde(default)]
    choices: Vec<SseChoice>,
}

/// `choices[i].message` object in a non-streaming `/v1/chat/completions`
/// response.
#[derive(Deserialize)]
struct JsonChoiceMessage {
    #[serde(default)]
    content: String,
}

/// A single entry of `choices` in a non-streaming response.
#[derive(Deserialize)]
struct JsonChoice {
    message: JsonChoiceMessage,
}

/// Top-level non-streaming `/v1/chat/completions` response body.
#[derive(Deserialize)]
struct JsonResponseBody {
    #[serde(default)]
    choices: Vec<JsonChoice>,
}

// ─── Message conversion ──────────────────────────────────────────────────────

/// Converts a [`ChatMessage`] into the OpenAI wire message shape. Text-only
/// messages keep `content` as a plain JSON string. Messages carrying images
/// switch `content` to the multipart form: a text part followed by one
/// `image_url` data-URI part per base64 image.
pub(crate) fn to_openai_message(msg: &ChatMessage) -> serde_json::Value {
    match &msg.images {
        Some(images) if !images.is_empty() => {
            let mut parts = vec![serde_json::json!({"type": "text", "text": msg.content})];
            for b64 in images {
                parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": {"url": format!("data:image/jpeg;base64,{b64}")},
                }));
            }
            serde_json::json!({"role": msg.role, "content": parts})
        }
        _ => serde_json::json!({"role": msg.role, "content": msg.content}),
    }
}

// ─── Error classification ────────────────────────────────────────────────────

/// Maps a reqwest connection/transport error to a provider-neutral
/// [`EngineError`], mirroring `classify_stream_error` on the native path:
/// connect/timeout failures are `EngineUnreachable`, everything else
/// (e.g. a connection reset mid-stream) is `Other`.
fn classify_v1_transport_error(e: &reqwest::Error) -> EngineError {
    if e.is_connect() || e.is_timeout() {
        EngineError {
            kind: EngineErrorKind::EngineUnreachable,
            message: format!("The inference server could not be reached.\n{e}"),
        }
    } else {
        EngineError {
            kind: EngineErrorKind::Other,
            message:
                "Something went wrong\nThe connection to the inference server was interrupted."
                    .to_string(),
        }
    }
}

/// Maps a non-2xx HTTP status from a `/v1` server to a provider-neutral
/// [`EngineError`], mirroring `classify_http_error` on the native path.
fn classify_v1_http_error(status: u16, model_name: &str) -> EngineError {
    match status {
        404 => EngineError {
            kind: EngineErrorKind::ModelNotFound,
            message: format!("Model not found\nThe server has no model named '{model_name}'."),
        },
        401 | 403 => EngineError {
            kind: EngineErrorKind::Other,
            message: format!(
                "Something went wrong\nAuthentication failed (HTTP {status}). Check the provider's API key."
            ),
        },
        _ => EngineError {
            kind: EngineErrorKind::Other,
            message: format!("Something went wrong\nHTTP {status}"),
        },
    }
}

/// Error emitted when the buffered unterminated SSE line exceeds
/// [`MAX_SSE_LINE_BYTES`]; the stream is aborted to bound memory.
fn oversize_sse_line_error() -> EngineError {
    EngineError {
        kind: EngineErrorKind::Other,
        message: "Something went wrong\nThe inference server sent an oversized stream line."
            .to_string(),
    }
}

// ─── Streaming chat ──────────────────────────────────────────────────────────

/// Streams a `/v1/chat/completions` request (`stream: true`) and emits the
/// same [`StreamChunk`] contract as `stream_ollama_chat`:
/// `choices[0].delta.content` becomes [`StreamChunk::Token`],
/// `choices[0].delta.reasoning_content` becomes
/// [`StreamChunk::ThinkingToken`], and `data: [DONE]` (or the stream ending
/// without it) becomes [`StreamChunk::Done`]. Exactly one terminal chunk
/// (`Done`, `Cancelled`, or `Error`) is emitted per call.
///
/// No sampling parameters are sent: the server and model defaults apply.
/// Returns the accumulated assistant content, mirroring `stream_ollama_chat`.
pub async fn stream_openai_chat(
    params: OpenAiChatParams,
    client: &reqwest::Client,
    cancel_token: CancellationToken,
    on_chunk: impl Fn(StreamChunk),
) -> String {
    let OpenAiChatParams {
        base_url,
        model,
        messages,
        api_key,
    } = params;
    let body = serde_json::json!({
        "model": model,
        "messages": messages.iter().map(to_openai_message).collect::<Vec<_>>(),
        "stream": true,
    });
    let mut request = client
        .post(format!("{base_url}/v1/chat/completions"))
        .json(&body);
    if let Some(ref key) = api_key {
        request = request.bearer_auth(key);
    }

    let mut accumulated = String::new();

    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => {
            on_chunk(StreamChunk::Error(classify_v1_transport_error(&e)));
            return accumulated;
        }
    };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        on_chunk(StreamChunk::Error(classify_v1_http_error(status, &model)));
        return accumulated;
    }

    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();

    loop {
        tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                // Drop the stream - closes the HTTP connection, which
                // signals the server to stop inference.
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
                            // Mirror the native path: a non-UTF-8 line is
                            // silently skipped.
                            let Ok(line_text) = String::from_utf8(line_bytes) else {
                                continue;
                            };
                            // trim handles the \r of \r\n line endings and
                            // collapses blank event-separator lines.
                            let trimmed = line_text.trim();
                            // SSE comments, `event:` lines, and anything else
                            // that is not a data line are ignored.
                            let Some(payload) = trimmed.strip_prefix("data: ") else {
                                continue;
                            };
                            if payload == "[DONE]" {
                                on_chunk(StreamChunk::Done);
                                return accumulated;
                            }
                            // Mirror the native path's tolerance: a data line
                            // that does not parse is silently skipped.
                            let Ok(event) = serde_json::from_str::<SseEvent>(payload) else {
                                continue;
                            };
                            if let Some(choice) = event.choices.first() {
                                if let Some(thinking) = choice
                                    .delta
                                    .reasoning_content
                                    .as_deref()
                                    .filter(|s| !s.is_empty())
                                {
                                    on_chunk(StreamChunk::ThinkingToken(thinking.to_string()));
                                }
                                if let Some(token) =
                                    choice.delta.content.as_deref().filter(|s| !s.is_empty())
                                {
                                    accumulated.push_str(token);
                                    on_chunk(StreamChunk::Token(token.to_string()));
                                }
                            }
                        }

                        // Bound the unterminated line a malicious or broken
                        // server can make us buffer.
                        if buffer.len() > MAX_SSE_LINE_BYTES {
                            on_chunk(StreamChunk::Error(oversize_sse_line_error()));
                            return accumulated;
                        }
                    }
                    Some(Err(e)) => {
                        on_chunk(StreamChunk::Error(classify_v1_transport_error(&e)));
                        return accumulated;
                    }
                    None => {
                        // The server closed the stream without a [DONE]
                        // marker. Emit a terminal Done so the frontend always
                        // leaves its streaming state (mirrors the native
                        // path's missing-done-marker handling).
                        on_chunk(StreamChunk::Done);
                        return accumulated;
                    }
                }
            }
        }
    }
}

// ─── Non-streaming structured output ─────────────────────────────────────────

/// Sends a single non-streaming `/v1/chat/completions` request with a strict
/// json-schema `response_format` and returns `choices[0].message.content`.
/// The structured-output twin of the search pipeline's `request_json`:
/// temperature 0 for deterministic classification, a per-call wall-clock
/// `timeout_secs`, and the same cancellation discipline.
#[allow(clippy::too_many_arguments)]
pub async fn request_openai_json(
    base_url: &str,
    model: &str,
    client: &reqwest::Client,
    messages: Vec<ChatMessage>,
    schema: serde_json::Value,
    api_key: Option<&str>,
    timeout_secs: u64,
    max_tokens: i32,
    cancel_token: &CancellationToken,
) -> Result<String, OpenAiError> {
    let body = serde_json::json!({
        "model": model,
        "messages": messages.iter().map(to_openai_message).collect::<Vec<_>>(),
        "stream": false,
        "temperature": 0,
        "max_tokens": max_tokens,
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": "out", "strict": true, "schema": schema},
        },
    });
    let mut request = client
        .post(format!("{base_url}/v1/chat/completions"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(timeout_secs));
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }

    let response = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => return Err(OpenAiError::Cancelled),
        res = request.send() => res.map_err(|e| OpenAiError::Unreachable(e.to_string()))?,
    };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body_text = response.text().await.unwrap_or_default();
        return Err(OpenAiError::Http(status, body_text));
    }

    let raw_body = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => return Err(OpenAiError::Cancelled),
        body = response.text() => body.map_err(|e| OpenAiError::BadBody(e.to_string()))?,
    };
    let parsed: JsonResponseBody =
        serde_json::from_str(&raw_body).map_err(|e| OpenAiError::BadBody(e.to_string()))?;
    parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .ok_or_else(|| OpenAiError::BadBody("response contained no choices".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn collect_chunks() -> (Arc<Mutex<Vec<StreamChunk>>>, impl Fn(StreamChunk)) {
        let chunks: Arc<Mutex<Vec<StreamChunk>>> = Arc::new(Mutex::new(Vec::new()));
        let chunks_clone = chunks.clone();
        let callback = move |chunk: StreamChunk| {
            chunks_clone.lock().unwrap().push(chunk);
        };
        (chunks, callback)
    }

    fn user_message(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
            images: None,
        }
    }

    fn chat_params(base_url: String) -> OpenAiChatParams {
        OpenAiChatParams {
            base_url,
            model: "test-model".to_string(),
            messages: vec![user_message("hi")],
            api_key: None,
        }
    }

    /// Helper: an SSE data line carrying a content delta.
    fn sse_content_line(token: &str) -> String {
        format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{token}\"}}}}]}}\n\n")
    }

    async fn mount_sse(server: &MockServer, body: impl Into<Vec<u8>>) {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body.into(), "text/event-stream"))
            .expect(1)
            .mount(server)
            .await;
    }

    // ── stream_openai_chat ──────────────────────────────────────────────────

    #[tokio::test]
    async fn streams_tokens_from_sse() {
        let server = MockServer::start().await;
        let body = format!(
            "{}{}data: {{\"choices\":[{{\"delta\":{{}}}}]}}\n\ndata: [DONE]\n",
            sse_content_line("Hello"),
            sse_content_line(" world"),
        );
        mount_sse(&server, body.into_bytes()).await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_openai_chat(
            chat_params(server.uri()),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Token(t) if t == "Hello"));
        assert!(matches!(&chunks[1], StreamChunk::Token(t) if t == " world"));
        assert!(matches!(&chunks[2], StreamChunk::Done));
        assert_eq!(chunks.len(), 3, "exactly one terminal Done");
        assert_eq!(accumulated, "Hello world");

        // Lock the wire contract: stream:true is sent and no sampling
        // parameters override the server/model defaults.
        let requests = server.received_requests().await.unwrap();
        let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(sent["stream"], serde_json::json!(true));
        assert!(sent.get("temperature").is_none());
        assert!(sent.get("top_p").is_none());
    }

    /// SSE lines arriving split across TCP segments must be reassembled
    /// through the line buffer before parsing.
    #[tokio::test]
    async fn streams_tokens_split_across_chunks() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 8192];
            let _ = stream.read(&mut req_buf).await;

            let sse = format!("{}data: [DONE]\n", sse_content_line("Hello"));
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                sse.len()
            );
            let _ = stream.write_all(header.as_bytes()).await;
            // Split the first data line mid-JSON across two writes.
            let (first, rest) = sse.split_at(20);
            let _ = stream.write_all(first.as_bytes()).await;
            let _ = stream.flush().await;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = stream.write_all(rest.as_bytes()).await;
            let _ = stream.shutdown().await;
        });

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_openai_chat(
            chat_params(format!("http://127.0.0.1:{port}")),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Token(t) if t == "Hello"));
        assert!(matches!(&chunks[1], StreamChunk::Done));
        assert_eq!(chunks.len(), 2);
        assert_eq!(accumulated, "Hello");
    }

    #[tokio::test]
    async fn reasoning_content_maps_to_thinking_token() {
        let server = MockServer::start().await;
        let body = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"reasoning_content\":\"hmm\"}}}}]}}\n\n{}data: [DONE]\n",
            sse_content_line("answer"),
        );
        mount_sse(&server, body.into_bytes()).await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_openai_chat(
            chat_params(server.uri()),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::ThinkingToken(t) if t == "hmm"));
        assert!(matches!(&chunks[1], StreamChunk::Token(t) if t == "answer"));
        assert!(matches!(&chunks[2], StreamChunk::Done));
        assert_eq!(
            accumulated, "answer",
            "thinking tokens must not be accumulated as content"
        );
    }

    /// `data: [DONE]` terminates the stream immediately: anything the server
    /// sends afterwards is never parsed and no second terminal chunk appears.
    #[tokio::test]
    async fn done_marker_ends_stream() {
        let server = MockServer::start().await;
        let body = format!(
            "{}data: [DONE]\n{}",
            sse_content_line("A"),
            sse_content_line("ignored"),
        );
        mount_sse(&server, body.into_bytes()).await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_openai_chat(
            chat_params(server.uri()),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Token(t) if t == "A"));
        assert!(matches!(&chunks[1], StreamChunk::Done));
        assert_eq!(chunks.len(), 2);
        assert_eq!(accumulated, "A");
    }

    /// A server that closes the stream without `[DONE]` must still produce a
    /// terminal Done (mirrors the native path's missing-done-marker fix).
    #[tokio::test]
    async fn stream_end_without_done_marker_emits_done() {
        let server = MockServer::start().await;
        let body = format!("{}{}", sse_content_line("A"), sse_content_line("B"));
        mount_sse(&server, body.into_bytes()).await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_openai_chat(
            chat_params(server.uri()),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Token(t) if t == "A"));
        assert!(matches!(&chunks[1], StreamChunk::Token(t) if t == "B"));
        assert!(matches!(&chunks[2], StreamChunk::Done));
        assert_eq!(chunks.len(), 3);
        assert_eq!(accumulated, "AB");
    }

    /// Mirrors the native path's policy for unparseable lines: malformed data
    /// lines, SSE comments, `event:` lines, and non-UTF-8 lines are all
    /// silently skipped; the stream continues.
    #[tokio::test]
    async fn malformed_data_line_policy() {
        let server = MockServer::start().await;
        let mut body = Vec::new();
        body.extend_from_slice(b"data: this is not json\n");
        body.extend_from_slice(b": sse comment\n");
        body.extend_from_slice(b"event: ping\n");
        body.extend_from_slice(b"\xFF\xFE\n");
        body.extend_from_slice(b"data: {\"choices\":[]}\n");
        body.extend_from_slice(b"data: {\"choices\":[{}]}\n");
        body.extend_from_slice(sse_content_line("ok").as_bytes());
        body.extend_from_slice(b"data: [DONE]\n");
        mount_sse(&server, body).await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_openai_chat(
            chat_params(server.uri()),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Token(t) if t == "ok"));
        assert!(matches!(&chunks[1], StreamChunk::Done));
        assert_eq!(chunks.len(), 2, "skipped lines must emit nothing");
        assert_eq!(accumulated, "ok");
    }

    #[tokio::test]
    async fn connect_refused_maps_engine_unreachable() {
        // Bind then drop a listener so the port is closed.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_openai_chat(
            chat_params(format!("http://127.0.0.1:{port}")),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(
            &chunks[0],
            StreamChunk::Error(e) if e.kind == EngineErrorKind::EngineUnreachable
                && e.message.starts_with("The inference server could not be reached.")
        ));
        assert_eq!(accumulated, "");
    }

    #[tokio::test]
    async fn http_404_maps_model_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        stream_openai_chat(
            chat_params(server.uri()),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(
            &chunks[0],
            StreamChunk::Error(e) if e.kind == EngineErrorKind::ModelNotFound
                && e.message.contains("test-model")
        ));
    }

    #[tokio::test]
    async fn http_401_maps_other_with_auth_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        stream_openai_chat(
            chat_params(server.uri()),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(
            &chunks[0],
            StreamChunk::Error(e) if e.kind == EngineErrorKind::Other
                && e.message.contains("Authentication failed (HTTP 401)")
        ));
    }

    #[tokio::test]
    async fn http_500_maps_other_with_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        stream_openai_chat(
            chat_params(server.uri()),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(
            &chunks[0],
            StreamChunk::Error(e) if e.kind == EngineErrorKind::Other
                && e.message.contains("HTTP 500")
        ));
    }

    /// 403 takes the same auth branch as 401.
    #[test]
    fn http_403_classifies_with_auth_message() {
        let error = classify_v1_http_error(403, "m");
        assert_eq!(error.kind, EngineErrorKind::Other);
        assert!(error.message.contains("Authentication failed (HTTP 403)"));
    }

    #[tokio::test]
    async fn cancel_emits_cancelled() {
        let server = MockServer::start().await;
        let body = format!("{}data: [DONE]\n", sse_content_line("never"));
        mount_sse(&server, body.into_bytes()).await;

        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        token.cancel();
        let (chunks, callback) = collect_chunks();
        let accumulated =
            stream_openai_chat(chat_params(server.uri()), &client, token, callback).await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Cancelled));
        assert_eq!(accumulated, "");
    }

    #[tokio::test]
    async fn oversize_sse_line_aborts_with_other() {
        let server = MockServer::start().await;
        // A single unterminated data line just over the cap; no newline ever
        // arrives, so the buffered length check must abort the stream.
        let mut body = b"data: ".to_vec();
        body.extend(std::iter::repeat_n(b'a', MAX_SSE_LINE_BYTES + 1));
        mount_sse(&server, body).await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_openai_chat(
            chat_params(server.uri()),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(matches!(
            &chunks[0],
            StreamChunk::Error(e) if e.kind == EngineErrorKind::Other
                && e.message.contains("oversized stream line")
        ));
        assert_eq!(accumulated, "");
    }

    /// A connection reset mid-stream surfaces as an Error chunk with kind
    /// Other (mirrors the native path: not a connect/timeout failure), and
    /// no Done is emitted after it.
    #[tokio::test]
    async fn mid_stream_error_maps_other() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 8192];
            let _ = stream.read(&mut req_buf).await;

            let first_line = sse_content_line("A");
            // Promise more bytes than are sent, then shut down: the client
            // sees a truncated body as a mid-stream transport error.
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
                first_line.len() + 64,
                first_line
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        });

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_openai_chat(
            chat_params(format!("http://127.0.0.1:{port}")),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let chunks = chunks.lock().unwrap();
        assert!(chunks
            .iter()
            .any(|chunk| matches!(chunk, StreamChunk::Token(t) if t == "A")));
        let error = chunks.iter().find_map(|chunk| match chunk {
            StreamChunk::Error(error) => Some(error),
            _ => None,
        });
        assert_eq!(error.unwrap().kind, EngineErrorKind::Other);
        assert!(chunks
            .iter()
            .all(|chunk| !matches!(chunk, StreamChunk::Done)));
        assert_eq!(accumulated, "A");
    }

    #[tokio::test]
    async fn api_key_sent_as_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("data: [DONE]\n", "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let mut params = chat_params(server.uri());
        params.api_key = Some("sk-test".to_string());
        stream_openai_chat(params, &client, CancellationToken::new(), callback).await;

        let chunks = chunks.lock().unwrap();
        assert!(matches!(&chunks[0], StreamChunk::Done));
    }

    #[tokio::test]
    async fn no_api_key_sends_no_authorization_header() {
        let server = MockServer::start().await;
        mount_sse(&server, b"data: [DONE]\n".to_vec()).await;

        let client = reqwest::Client::new();
        let (_, callback) = collect_chunks();
        stream_openai_chat(
            chat_params(server.uri()),
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].headers.contains_key("authorization"));
    }

    // ── to_openai_message ───────────────────────────────────────────────────

    #[test]
    fn text_only_message_keeps_plain_string_content() {
        let msg = user_message("hello");
        assert_eq!(
            to_openai_message(&msg),
            serde_json::json!({"role": "user", "content": "hello"})
        );
    }

    #[test]
    fn empty_images_vec_keeps_plain_string_content() {
        let msg = ChatMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
            images: Some(vec![]),
        };
        assert_eq!(
            to_openai_message(&msg),
            serde_json::json!({"role": "user", "content": "hello"})
        );
    }

    #[test]
    fn images_serialize_as_content_parts() {
        let msg = ChatMessage {
            role: "user".to_string(),
            content: "what is this?".to_string(),
            images: Some(vec!["QUJD".to_string(), "REVG".to_string()]),
        };
        assert_eq!(
            to_openai_message(&msg),
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "what is this?"},
                    {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,QUJD"}},
                    {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,REVG"}},
                ],
            })
        );
    }

    // ── request_openai_json ─────────────────────────────────────────────────

    #[tokio::test]
    async fn json_request_uses_response_format_and_extracts_content() {
        let server = MockServer::start().await;
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}},
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer sk-json"))
            .and(body_partial_json(serde_json::json!({
                "model": "test-model",
                "stream": false,
                "temperature": 0,
                "max_tokens": 256,
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {"name": "out", "strict": true, "schema": schema},
                },
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "{\"a\":1}"}}],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = request_openai_json(
            &server.uri(),
            "test-model",
            &client,
            vec![user_message("classify")],
            schema.clone(),
            Some("sk-json"),
            5,
            256,
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(result, Ok("{\"a\":1}".to_string()));
    }

    #[tokio::test]
    async fn json_request_http_error_maps() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = request_openai_json(
            &server.uri(),
            "m",
            &client,
            vec![user_message("q")],
            serde_json::json!({}),
            None,
            5,
            64,
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(result, Err(OpenAiError::Http(500, "boom".to_string())));
    }

    #[tokio::test]
    async fn json_request_cancel_maps_cancelled() {
        // No mock mounted: the pre-cancelled token must win the biased
        // select before any request is sent.
        let server = MockServer::start().await;

        let token = CancellationToken::new();
        token.cancel();
        let client = reqwest::Client::new();
        let result = request_openai_json(
            &server.uri(),
            "m",
            &client,
            vec![user_message("q")],
            serde_json::json!({}),
            None,
            5,
            64,
            &token,
        )
        .await;

        assert_eq!(result, Err(OpenAiError::Cancelled));
    }

    /// The per-call timeout surfaces through reqwest's send error, which maps
    /// to Unreachable (mirrors the native `request_json`, where a timeout is
    /// a transport error and maps to `LlmUnavailable`).
    #[tokio::test]
    async fn json_request_timeout_maps_unreachable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"choices": []}))
                    .set_delay(std::time::Duration::from_secs(5)),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = request_openai_json(
            &server.uri(),
            "m",
            &client,
            vec![user_message("q")],
            serde_json::json!({}),
            None,
            1,
            64,
            &CancellationToken::new(),
        )
        .await;

        assert!(matches!(result, Err(OpenAiError::Unreachable(_))));
    }

    /// A 2xx response whose body dies mid-read (connection closed before the
    /// promised Content-Length) maps to BadBody, mirroring the native
    /// `request_json` where a body-read failure is `LlmBadJson`.
    #[tokio::test]
    async fn json_request_body_read_failure_maps_bad_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 8192];
            let _ = stream.read(&mut req_buf).await;
            // Promise more bytes than are sent, then shut down.
            let response =
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{\"choices\"";
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        });

        let client = reqwest::Client::new();
        let result = request_openai_json(
            &format!("http://127.0.0.1:{port}"),
            "m",
            &client,
            vec![user_message("q")],
            serde_json::json!({}),
            None,
            5,
            64,
            &CancellationToken::new(),
        )
        .await;

        assert!(matches!(result, Err(OpenAiError::BadBody(_))));
    }

    #[tokio::test]
    async fn json_request_malformed_body_maps_bad_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = request_openai_json(
            &server.uri(),
            "m",
            &client,
            vec![user_message("q")],
            serde_json::json!({}),
            None,
            5,
            64,
            &CancellationToken::new(),
        )
        .await;

        assert!(matches!(result, Err(OpenAiError::BadBody(_))));
    }

    #[tokio::test]
    async fn json_request_empty_choices_maps_bad_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = request_openai_json(
            &server.uri(),
            "m",
            &client,
            vec![user_message("q")],
            serde_json::json!({}),
            None,
            5,
            64,
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(
            result,
            Err(OpenAiError::BadBody(
                "response contained no choices".to_string()
            ))
        );
    }

    #[tokio::test]
    async fn json_request_omits_authorization_without_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "ok"}}],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = request_openai_json(
            &server.uri(),
            "m",
            &client,
            vec![user_message("q")],
            serde_json::json!({}),
            None,
            5,
            64,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(result, Ok("ok".to_string()));

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].headers.contains_key("authorization"));
    }
}
