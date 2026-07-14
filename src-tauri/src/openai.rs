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

/// Which flavor of `/v1` server a request targets. Decided at the route
/// dispatch (where the provider kind is known) and carried into the error
/// classifiers so user-facing copy matches the provider: the bundled engine
/// speaks about "Thuki's engine" and points at Settings, while any other
/// OpenAI-compatible server keeps provider-neutral wording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum V1Flavor {
    /// The bundled llama-server sidecar at a loopback port.
    Builtin,
    /// Any other OpenAI-compatible server (an `openai`-kind provider).
    Remote,
}

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
    /// Picks the user-facing error copy for this request.
    pub flavor: V1Flavor,
    /// Whether the model should run a reasoning pass before answering.
    /// Reasoning is opt-in (the `/think` command); a plain message answers
    /// directly. Honored only on the built-in engine via
    /// [`reasoning_template_kwargs`]; remote `/v1` servers ignore it.
    pub enable_thinking: bool,
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

/// Maps a reqwest connection/transport error to an [`EngineError`],
/// mirroring `classify_stream_error` on the native path:
/// connect/timeout failures are `EngineUnreachable`, everything else
/// (e.g. a connection reset mid-stream) is `Other`. The unreachable copy
/// branches on `flavor`: the bundled engine is Thuki's own process (the
/// next message re-ensures it), while a remote server keeps neutral wording.
fn classify_v1_transport_error(e: &reqwest::Error, flavor: V1Flavor) -> EngineError {
    if e.is_connect() || e.is_timeout() {
        v1_unreachable_error(&e.to_string(), flavor)
    } else {
        EngineError {
            kind: EngineErrorKind::Other,
            message:
                "Something went wrong\nThe connection to the inference server was interrupted."
                    .to_string(),
        }
    }
}

/// Copy for an unreachable `/v1` server, keyed by flavor. Shared by the
/// streaming classifier above and the search pipeline's structured-output
/// error mapping so each flavor's unreachable copy lives in exactly one
/// place. The bundled engine is Thuki's own process (the next message
/// re-ensures it); a remote server keeps neutral wording plus the transport
/// detail.
pub(crate) fn v1_unreachable_error(detail: &str, flavor: V1Flavor) -> EngineError {
    EngineError {
        kind: EngineErrorKind::EngineUnreachable,
        message: match flavor {
            V1Flavor::Builtin => {
                "Thuki's engine isn't running\nSend your message again to restart it.".to_string()
            }
            V1Flavor::Remote => format!("The inference server could not be reached.\n{detail}"),
        },
    }
}

/// Maps a non-2xx HTTP status from a `/v1` server to an [`EngineError`],
/// mirroring `classify_http_error` on the native path. The 404 copy branches
/// on `flavor`: the bundled engine steers the user to the Settings download
/// flow, a remote server names the model it is missing. Shared with the
/// search pipeline's structured-output error mapping.
pub(crate) fn classify_v1_http_error(
    status: u16,
    model_name: &str,
    flavor: V1Flavor,
) -> EngineError {
    match status {
        404 => EngineError {
            kind: EngineErrorKind::ModelNotFound,
            message: match flavor {
                V1Flavor::Builtin => {
                    "Model not found\nPick or download a model in Settings.".to_string()
                }
                V1Flavor::Remote => {
                    format!("Model not found\nThe server has no model named '{model_name}'.")
                }
            },
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

// ─── Reasoning control ───────────────────────────────────────────────────────

/// The per-request reasoning switch to merge into a `/v1` body as
/// `chat_template_kwargs`, or `None` when the request must carry no such field.
///
/// llama.cpp injects these into the model's chat template and a template
/// silently ignores any kwarg it does not read (verified on the `b9590`
/// sidecar with Qwen3.5: the full set below suppresses reasoning with
/// no error). So one harmless "blast" covers every reasoning family that
/// exposes a template-level switch, with no per-family detection:
/// `enable_thinking` (Qwen3/3.5, GLM, Hunyuan, Gemma), `thinking` (IBM Granite,
/// DeepSeek-V3.x), and `thinking_budget` (`0` = off / `-1` = unrestricted, for
/// ByteDance Seed-OSS). `false`/`0` answers directly; `true`/`-1` reasons.
///
/// Families with no template switch (DeepSeek-R1 + distills, QwQ, gpt-oss
/// Harmony, MiniMax, EXAONE, Phi-4-reasoning, ...) reason regardless of this
/// switch: the compute cannot be stopped on this engine. Their reasoning is
/// not suppressed; [`stream_openai_chat`] surfaces any `reasoning_content` in
/// the thinking block (always shown, never hidden), so the chain of thought is
/// presented cleanly rather than running invisibly.
///
/// Only the bundled engine ([`V1Flavor::Builtin`]) receives the kwargs; the
/// fields are llama.cpp-specific and an arbitrary OpenAI-compatible server may
/// reject an unknown body key, so remote providers get nothing.
fn reasoning_template_kwargs(flavor: V1Flavor, enable_thinking: bool) -> Option<serde_json::Value> {
    match flavor {
        V1Flavor::Builtin => Some(serde_json::json!({
            "enable_thinking": enable_thinking,
            "thinking": enable_thinking,
            "thinking_budget": if enable_thinking { -1 } else { 0 },
        })),
        V1Flavor::Remote => None,
    }
}

/// Chat-template kwargs for structured (classifier / judge) builtin calls.
///
/// Extends the thinking-off blast with `reasoning_effort: "low"` for models
/// whose Harmony / Jinja templates honor that knob (gpt-oss). Templates that
/// ignore unknown kwargs (gemma and others) keep emitting JSON with no
/// reasoning tokens, so this is a no-op on those families rather than a
/// gemma-only false green. Does not require a spawn-arg change: b9946 enables
/// Jinja by default. Remote providers still get no kwargs.
fn structured_reasoning_kwargs(flavor: V1Flavor) -> Option<serde_json::Value> {
    match flavor {
        V1Flavor::Builtin => {
            let mut kwargs = reasoning_template_kwargs(flavor, false)?;
            // Low effort steers gpt-oss away from thousand-token reasoning
            // blocks that burn PREPASS/JUDGE max_tokens before any JSON.
            kwargs["reasoning_effort"] = serde_json::json!("low");
            Some(kwargs)
        }
        V1Flavor::Remote => None,
    }
}

/// Builds the streaming `/v1/chat/completions` request body. Pulled out of
/// [`stream_openai_chat`] so the reasoning-control wiring is unit-tested
/// without a live server. No sampling parameters are sent: the server and
/// model defaults apply.
pub(crate) fn chat_request_body(
    model: &str,
    messages: &[ChatMessage],
    flavor: V1Flavor,
    enable_thinking: bool,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages.iter().map(to_openai_message).collect::<Vec<_>>(),
        "stream": true,
    });
    if let Some(kwargs) = reasoning_template_kwargs(flavor, enable_thinking) {
        body["chat_template_kwargs"] = kwargs;
    }
    body
}

// ─── Streaming chat ──────────────────────────────────────────────────────────

/// Formats the diagnostic stderr line emitted when an SSE chat stream ends
/// without a `data: [DONE]` marker (the `None` arm of [`stream_openai_chat`]'s
/// loop). This is a diagnostic hook for an unreproduced bug (J6): some
/// first-turn submissions complete instantly with zero tokens, and the prime
/// suspect is this EOF-without-`[DONE]` arm silently turning a premature stream
/// close into a normal `Done`. Recording the accumulated byte count and
/// token-event count lets a live dogfood session tell an empty premature close
/// (0 bytes, 0 events) from a full one. Pure so the wire path's diagnostic
/// string stays unit-tested without a live server.
fn eof_without_done_diagnostic(accumulated_bytes: usize, token_events: usize) -> String {
    format!(
        "openai: SSE stream closed without [DONE]; accumulated {accumulated_bytes} bytes across {token_events} token events"
    )
}

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
        flavor,
        enable_thinking,
    } = params;
    let body = chat_request_body(&model, &messages, flavor, enable_thinking);
    let mut request = client
        .post(format!("{base_url}/v1/chat/completions"))
        .json(&body);
    if let Some(ref key) = api_key {
        request = request.bearer_auth(key);
    }

    let mut accumulated = String::new();
    // Diagnostic-only (J6 unreproduced zero-token bug): counts content token
    // events so the EOF-without-`[DONE]` arm can report how many actually
    // arrived. Observationally invisible; thinking tokens are excluded so the
    // count stays paired to `accumulated`.
    let mut token_events: usize = 0;

    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => {
            on_chunk(StreamChunk::Error(classify_v1_transport_error(&e, flavor)));
            return accumulated;
        }
    };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        on_chunk(StreamChunk::Error(classify_v1_http_error(
            status, &model, flavor,
        )));
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
                                // Whatever reasoning the model emits is always
                                // shown (never hidden): an `Optional` model that
                                // honored the OFF blast emits none, while a
                                // model that always reasons gets its thinking
                                // surfaced cleanly in the thinking block rather
                                // than running invisibly.
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
                                    token_events += 1;
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
                        on_chunk(StreamChunk::Error(classify_v1_transport_error(&e, flavor)));
                        return accumulated;
                    }
                    None => {
                        // The server closed the stream without a [DONE]
                        // marker. Emit a terminal Done so the frontend always
                        // leaves its streaming state (mirrors the native
                        // path's missing-done-marker handling).
                        //
                        // Diagnostic hook for the unreproduced J6 zero-token
                        // bug: this arm is the prime suspect for silently
                        // converting a premature/empty stream close into a
                        // normal completion. Log to stderr (never the frontend)
                        // how much content actually arrived before the EOF so a
                        // live occurrence separates an empty close from a full
                        // one. Zero-cost on the normal [DONE] path, which
                        // returns before ever reaching this arm.
                        eprintln!(
                            "{}",
                            eof_without_done_diagnostic(accumulated.len(), token_events)
                        );
                        on_chunk(StreamChunk::Done);
                        return accumulated;
                    }
                }
            }
        }
    }
}

// ─── Non-streaming structured output ─────────────────────────────────────────

/// Builds the `/v1/chat/completions` request body for a structured-output
/// (non-streaming, temperature 0) call. Used by both [`request_openai_json`]
/// (the live wire call) and the search pipeline's trace helper so the logged
/// body always mirrors the wire exactly.
pub(crate) fn json_request_body(
    model: &str,
    messages: &[ChatMessage],
    schema: serde_json::Value,
    max_tokens: i32,
    flavor: V1Flavor,
) -> serde_json::Value {
    let mut body = serde_json::json!({
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
    // Structured output must never reason on the built-in engine: a thinking
    // pass would consume the `max_tokens` budget before any JSON is emitted,
    // yielding empty content. Force thinking off and (for Harmony/Jinja
    // models) set reasoning_effort low; remote servers get nothing.
    if let Some(kwargs) = structured_reasoning_kwargs(flavor) {
        body["chat_template_kwargs"] = kwargs;
    }
    body
}

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
    flavor: V1Flavor,
    cancel_token: &CancellationToken,
) -> Result<String, OpenAiError> {
    let body = json_request_body(model, &messages, schema, max_tokens, flavor);
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
            flavor: V1Flavor::Remote,
            enable_thinking: false,
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

    /// Locks the exact tagged string of the J6 EOF-without-`[DONE]` diagnostic
    /// so the byte/token-event contract a live dogfood session greps for cannot
    /// drift silently.
    #[test]
    fn eof_without_done_diagnostic_format() {
        assert_eq!(
            eof_without_done_diagnostic(0, 0),
            "openai: SSE stream closed without [DONE]; accumulated 0 bytes across 0 token events"
        );
        assert_eq!(
            eof_without_done_diagnostic(42, 7),
            "openai: SSE stream closed without [DONE]; accumulated 42 bytes across 7 token events"
        );
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

    /// Builtin flavor: an unreachable sidecar reads as Thuki's own engine
    /// being down, not as a generic "inference server". The full string is
    /// pinned: it is rendered verbatim by ErrorCard.
    #[tokio::test]
    async fn connect_refused_builtin_names_thukis_engine() {
        // Bind then drop a listener so the port is closed.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };

        let client = reqwest::Client::new();
        let (chunks, callback) = collect_chunks();
        let accumulated = stream_openai_chat(
            OpenAiChatParams {
                flavor: V1Flavor::Builtin,
                ..chat_params(format!("http://127.0.0.1:{port}"))
            },
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
                && e.message == "Thuki's engine isn't running\nSend your message again to restart it."
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
        let error = classify_v1_http_error(403, "m", V1Flavor::Remote);
        assert_eq!(error.kind, EngineErrorKind::Other);
        assert!(error.message.contains("Authentication failed (HTTP 403)"));
    }

    /// Builtin flavor: a 404 steers the user to the Settings download flow
    /// (the bundled engine has no server-side model listing to consult).
    /// The full string is pinned: it is rendered verbatim by ErrorCard.
    #[test]
    fn http_404_builtin_points_at_settings() {
        let error = classify_v1_http_error(404, "org/repo:m.gguf", V1Flavor::Builtin);
        assert_eq!(error.kind, EngineErrorKind::ModelNotFound);
        assert_eq!(
            error.message,
            "Model not found\nPick or download a model in Settings."
        );
    }

    /// Remote flavor: the 404 copy names the model the server is missing.
    /// Pinned byte-for-byte so builtin copy work never drifts it.
    #[test]
    fn http_404_remote_names_the_missing_model() {
        let error = classify_v1_http_error(404, "test-model", V1Flavor::Remote);
        assert_eq!(error.kind, EngineErrorKind::ModelNotFound);
        assert_eq!(
            error.message,
            "Model not found\nThe server has no model named 'test-model'."
        );
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
            V1Flavor::Remote,
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
            V1Flavor::Remote,
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
            V1Flavor::Remote,
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
            V1Flavor::Remote,
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
            V1Flavor::Remote,
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
            V1Flavor::Remote,
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
            V1Flavor::Remote,
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
            V1Flavor::Remote,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(result, Ok("ok".to_string()));

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].headers.contains_key("authorization"));
    }

    // ── reasoning control (chat_template_kwargs.enable_thinking) ─────────────

    /// Built-in chat carries the llama.cpp per-request reasoning switch. With
    /// reasoning opted out (the default), the body sets
    /// `chat_template_kwargs.enable_thinking = false` so the model answers
    /// directly instead of running a thinking pass.
    #[test]
    fn builtin_chat_body_disables_thinking_by_default() {
        let body = chat_request_body("m", &[user_message("hi")], V1Flavor::Builtin, false);
        // The OFF blast covers every reasoning family that honors a template
        // kwarg, in one harmless payload: `enable_thinking` (Qwen/GLM/Hunyuan/
        // Gemma), `thinking` (Granite/DeepSeek-V3.x), `thinking_budget` 0
        // (Seed-OSS). Templates ignore the kwargs they do not read.
        let kwargs = &body["chat_template_kwargs"];
        assert_eq!(kwargs["enable_thinking"], serde_json::json!(false));
        assert_eq!(kwargs["thinking"], serde_json::json!(false));
        assert_eq!(kwargs["thinking_budget"], serde_json::json!(0));
        assert_eq!(body["stream"], serde_json::json!(true));
    }

    /// Built-in chat with `/think` opts in: the ON blast sets every kwarg to
    /// the reasoning-enabled value (`thinking_budget` -1 = unrestricted).
    #[test]
    fn builtin_chat_body_enables_thinking_when_opted_in() {
        let body = chat_request_body("m", &[user_message("hi")], V1Flavor::Builtin, true);
        let kwargs = &body["chat_template_kwargs"];
        assert_eq!(kwargs["enable_thinking"], serde_json::json!(true));
        assert_eq!(kwargs["thinking"], serde_json::json!(true));
        assert_eq!(kwargs["thinking_budget"], serde_json::json!(-1));
    }

    /// Remote `/v1` servers never receive the llama.cpp-specific
    /// `chat_template_kwargs` field: an arbitrary OpenAI-compatible server may
    /// reject an unknown body key, and the `/think` opt-in is built-in only.
    #[test]
    fn remote_chat_body_omits_thinking_kwargs() {
        let body = chat_request_body("m", &[user_message("hi")], V1Flavor::Remote, true);
        assert!(body.get("chat_template_kwargs").is_none());
    }

    /// Structured-output calls (search judges, title generation) must never
    /// reason on the built-in engine: a thinking pass would consume the
    /// `max_tokens` budget before any JSON is emitted. The builtin structured
    /// body forces thinking off and sets `reasoning_effort: low` for Harmony
    /// models; templates that ignore the knobs (gemma) still emit JSON only.
    #[test]
    fn builtin_structured_body_disables_thinking() {
        let body = json_request_body(
            "m",
            &[user_message("q")],
            serde_json::json!({}),
            64,
            V1Flavor::Builtin,
        );
        let kwargs = &body["chat_template_kwargs"];
        assert_eq!(kwargs["enable_thinking"], serde_json::json!(false));
        assert_eq!(kwargs["thinking"], serde_json::json!(false));
        assert_eq!(kwargs["thinking_budget"], serde_json::json!(0));
        assert_eq!(kwargs["reasoning_effort"], serde_json::json!("low"));
        assert_eq!(body["stream"], serde_json::json!(false));
    }

    /// Streaming chat (user-facing writer) does NOT set `reasoning_effort`:
    /// that knob is reserved for structured critical-path calls so a user
    /// `/think` opt-in is not fighting a forced-low effort.
    #[test]
    fn builtin_chat_body_omits_reasoning_effort() {
        let off = chat_request_body("m", &[user_message("hi")], V1Flavor::Builtin, false);
        assert!(off["chat_template_kwargs"].get("reasoning_effort").is_none());
        let on = chat_request_body("m", &[user_message("hi")], V1Flavor::Builtin, true);
        assert!(on["chat_template_kwargs"].get("reasoning_effort").is_none());
    }

    /// Remote structured-output bodies stay clean of the llama.cpp kwarg.
    #[test]
    fn remote_structured_body_omits_thinking_kwargs() {
        let body = json_request_body(
            "m",
            &[user_message("q")],
            serde_json::json!({}),
            64,
            V1Flavor::Remote,
        );
        assert!(body.get("chat_template_kwargs").is_none());
    }

    /// End to end: a built-in streaming chat actually sends the reasoning
    /// switch on the wire, locking `stream_openai_chat` to `chat_request_body`.
    #[tokio::test]
    async fn builtin_stream_sends_enable_thinking_on_the_wire() {
        let server = MockServer::start().await;
        mount_sse(&server, b"data: [DONE]\n".to_vec()).await;

        let client = reqwest::Client::new();
        let (_, callback) = collect_chunks();
        stream_openai_chat(
            OpenAiChatParams {
                flavor: V1Flavor::Builtin,
                enable_thinking: false,
                ..chat_params(server.uri())
            },
            &client,
            CancellationToken::new(),
            callback,
        )
        .await;

        let requests = server.received_requests().await.unwrap();
        let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            sent["chat_template_kwargs"]["enable_thinking"],
            serde_json::json!(false)
        );
    }
}
