//! Forensic per-conversation trace recorder shared by the chat path and
//! the `/search` pipeline.
//!
//! When [`crate::config::schema::DebugSection::trace_enabled`] is on,
//! every chat turn AND every search turn writes a forensic JSON-Lines
//! record into a per-conversation file under
//! `app_data_dir()/traces/<domain>/<conversation_id>.jsonl`. Files are
//! grouped by domain so an analysis agent can be pointed at exactly the
//! slice it cares about: `traces/chat/` for conversation patterns,
//! `traces/search/` for retrieval quality, or `traces/` for end-to-end
//! debugging.
//!
//! Off in shipped builds. Intended for local quality investigation: open
//! a `.jsonl` file and grep/jq across LLM prompts vs. judge verdicts vs.
//! user-visible assistant tokens to understand exactly what happened.
//!
//! # Architecture
//!
//! - [`TraceRecorder`] is the trait every callsite calls into. The chat
//!   layer and the search pipeline both thread `&Arc<dyn TraceRecorder>`
//!   through their execution contexts; no call site distinguishes the
//!   live recorder from the noop.
//! - [`NoopRecorder`] is the production default. Every method is a no-op.
//! - [`FileRecorder`] writes events for ONE `(domain, conversation_id)`
//!   pair into ONE file. Lazy directory + file creation on first record,
//!   `parking_lot::Mutex` around a buffered writer, per-line flush so
//!   partial files are still grep-friendly if the daemon crashes. I/O
//!   errors are warned once via `eprintln!` then become silent no-ops for
//!   the rest of the file's lifetime: trace failures must never affect
//!   the user-visible pipeline.
//! - [`crate::trace::registry::RegistryRecorder`] is the production
//!   composition. It owns one `Arc<FileRecorder>` per
//!   `(TraceDomain, ConversationId)`, lazily inserts on first event,
//!   evicts on `ConversationEnd`, and tolerates late-arriving events
//!   after eviction (file reopens in append mode).
//! - [`MockRecorder`] (test-only) collects every event into an in-memory
//!   `Vec<(ConversationId, RecorderEvent)>` for instrumentation-seam
//!   assertions.
//!
//! # Schema
//!
//! Each line is a self-describing JSON object:
//!
//! ```json
//! {"v":2,"seq":0,"ts_ms":1714762800000,"domain":"chat","conversation_id":"conv-abc","kind":"user_message",...payload}
//! ```
//!
//! `seq` is monotonic per `FileRecorder`; `ts_ms` is wall-clock UNIX
//! millis. The `v` field allows future schema evolution. Schema version
//! 2 added the `domain` and `conversation_id` top-level fields plus the
//! chat-domain variants. Consumers that hardcoded v1 must update.
//!
//! # Late-event tolerance
//!
//! `FileRecorder` opens its file in append mode. If a stray event arrives
//! after `ConversationEnd` (e.g. a cancelled stream's final
//! `AssistantTokens` arriving after the frontend's
//! `record_conversation_end` call), the registry lazily re-creates a
//! `FileRecorder` for the evicted key and writes a benign trailing line.
//! Consumers MUST tolerate post-end lines: the canonical end of a
//! conversation is the LAST line with `kind: "conversation_end"`, not
//! the first.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

use super::ids::ConversationId;

/// Schema version embedded in every record. Bump when the wire shape changes.
///
/// `2` adds the `domain` and `conversation_id` top-level fields plus the
/// chat-domain variants. Files produced by the v1 recorder (one file per
/// search turn, no chat events) and files produced by v2 (one file per
/// conversation, both chat and search events) are not interleaved on
/// disk because the v2 layout uses different directories.
pub const TRACE_SCHEMA_VERSION: u32 = 2;

/// Coarse-grained classification of every recorded event. Determines
/// which subdirectory under `traces/` the event lands in.
///
/// `Screenshot` events are routed to the `chat` domain because they are
/// always a side-effect of a user-visible chat turn (the `/screen` slash
/// command). They retain a distinct `domain()` value for ergonomic
/// filtering by analysis tooling but share a file with the parent chat
/// conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceDomain {
    /// Chat-layer events: user messages, assistant streaming, screen
    /// captures, conversation lifecycle. One file per conversation,
    /// rooted at `traces/chat/<conversation_id>.jsonl`.
    Chat,
    /// Search-pipeline events: router/judge LLM calls, SearXNG queries,
    /// reader batches, chunker output, rerank results, judge verdicts.
    /// One file per conversation, rooted at
    /// `traces/search/<conversation_id>.jsonl`.
    Search,
}

impl TraceDomain {
    /// Subdirectory name (under `traces/`) where files for this domain
    /// live. Used by [`FileRecorder::for_conversation`] when assembling
    /// the destination path.
    pub fn dir(self) -> &'static str {
        match self {
            TraceDomain::Chat => "chat",
            TraceDomain::Search => "search",
        }
    }
}

/// Trait every chat or search callsite uses to emit forensic events.
/// Implementors must be cheap when tracing is off (the [`NoopRecorder`]
/// case dominates production usage).
pub trait TraceRecorder: Send + Sync {
    /// Records a single event for the given conversation. Implementors
    /// MUST NOT panic, MUST NOT block for arbitrary durations, and MUST
    /// NOT propagate errors. Trace I/O failures must be swallowed (with
    /// a single rate-limited warning) so the chat path and the search
    /// pipeline are unaffected.
    fn record(&self, conversation_id: &ConversationId, event: RecorderEvent);
}

/// Production default: every method is a no-op. Constant time, zero
/// allocations.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopRecorder;

impl TraceRecorder for NoopRecorder {
    fn record(&self, _conversation_id: &ConversationId, _event: RecorderEvent) {}
}

/// Forensic trace events emitted by the chat layer or the search pipeline.
///
/// Each variant maps to one JSON-Lines record. Field shapes are
/// intentionally permissive (`Value`, `String`) for the search-domain
/// variants because the trace is a forensic dump, not a typed contract.
/// Chat-domain variants carry typed payloads because the chat layer's
/// surface is small and stable.
///
/// Every variant has a [`Self::domain`] method that returns its
/// [`TraceDomain`], used by the registry to pick the destination file.
#[derive(Debug, Clone)]
pub enum RecorderEvent {
    // ─── Search domain (existing variants from PR #126) ─────────────────
    /// Search-pipeline turn starts. Captures the user query, model slug,
    /// runtime search-config snapshot (so a trace is reproducible without
    /// the config file), and conversation-history length.
    TurnStart {
        turn_id: String,
        query: String,
        model: String,
        runtime_config: Value,
        history_len: usize,
    },
    /// Single Ollama `/api/chat` non-streaming JSON request issued by the
    /// search pipeline. Captures the stage label
    /// (router / snippet_judge / chunk_judge / synthesis /
    /// answer_from_context), the full request body the pipeline sent,
    /// the raw response string Ollama returned, and the request latency.
    /// `error` is `Some` when the call failed (timeout, transport, parse).
    LlmCall {
        stage: String,
        endpoint: String,
        request_body: Value,
        response_raw: Option<String>,
        latency_ms: u64,
        error: Option<String>,
    },
    /// Streaming Ollama `/api/chat` request used by the search synthesis
    /// path. `tokens` counts streamed pieces; `final_text` is the
    /// accumulated assistant message; per-token records are NOT emitted
    /// (would flood the file without adding signal).
    StreamingLlmCall {
        stage: String,
        endpoint: String,
        request_body: Value,
        final_text: Option<String>,
        tokens: u64,
        latency_ms: u64,
        error: Option<String>,
    },
    /// SearXNG search request: query string, full URL, raw HTTP response
    /// body, and the normalized result list the pipeline forwarded to the
    /// reranker.
    SearxngQuery {
        query: String,
        url: String,
        status: Option<u16>,
        response_raw: Option<String>,
        normalized_results: Value,
        latency_ms: u64,
        error: Option<String>,
    },
    /// Reader sidecar batch: per-URL fetch results including raw HTTP
    /// body and extracted text. Latency is per-URL plus the overall batch.
    ReaderBatch {
        urls: Vec<String>,
        per_url: Vec<ReaderUrlOutcome>,
        batch_latency_ms: u64,
        batch_error: Option<String>,
    },
    /// Page chunked into N pieces. `chunks` are the full chunk texts so
    /// a downstream consumer can reconstruct what the judge actually saw.
    ChunkerBatch {
        page_url: String,
        chunk_count: usize,
        chunks: Vec<String>,
    },
    /// Reranker output: top-k chunks with scores, in selection order.
    RerankResult {
        query: String,
        input_count: usize,
        top_k: Vec<RerankedChunk>,
    },
    /// Parsed judge verdict at a given stage (snippet or chunk). Captures
    /// the raw string the LLM returned plus the normalized verdict struct.
    JudgeVerdict {
        stage: String,
        raw: String,
        normalized: Value,
    },
    /// Warning surfaced by the search pipeline (e.g. JudgeFailed,
    /// BudgetExhausted).
    Warning { kind: String, payload: Value },
    /// Mirrors a [`crate::search::types::SearchEvent`] emission so the
    /// trace reflects the user-visible event stream alongside backend
    /// internals.
    SearchEventEmitted { event: Value },
    /// Search-pipeline turn ends. Captures the final action taken and
    /// total wall-clock latency. `error` is `Some` only on hard failure
    /// paths.
    TurnEnd {
        turn_id: String,
        final_action: String,
        final_source_urls: Vec<String>,
        total_latency_ms: u64,
        error: Option<String>,
    },

    // ─── Chat domain (new in v2) ────────────────────────────────────────
    /// First event in a chat-domain file. Captures the model slug and
    /// the resolved system prompt at the moment the conversation began,
    /// so the trace is reproducible without snapshotting the live
    /// `AppConfig`.
    ConversationStart {
        model: String,
        system_prompt: String,
    },
    /// User-authored message that triggered an assistant turn. Includes
    /// any attached image paths (paths only, not bytes) and the slash
    /// command that routed the message, if any.
    UserMessage {
        content: String,
        attached_images: Vec<String>,
        slash_command: Option<String>,
    },
    /// Streaming chunk of the assistant's hidden reasoning ("thinking")
    /// output. Emitted only when the active model returns thinking
    /// tokens AND the user has thinking mode enabled.
    AssistantThinking { chunk: String },
    /// Streaming chunk of the assistant's user-visible response.
    AssistantTokens { chunk: String },
    /// Assistant turn ends. Captures the total streamed token count and
    /// the wall-clock latency from `UserMessage` to stream-completion.
    AssistantComplete { total_tokens: u64, latency_ms: u64 },
    /// User invoked the `/screen` slash command and the backend wrote a
    /// JPEG snapshot to `image_path`. `displays` is the number of
    /// monitors captured (multi-monitor setups produce one merged image).
    ScreenCaptured { image_path: String, displays: u8 },
    /// Final event in a chat-domain file. Emitted by the frontend when
    /// the user resets the conversation or by the backend on app quit
    /// (reason = "quit"). Window-hide does NOT emit this event because
    /// Thuki's window-close intercept hides instead of quits and the
    /// same conversation can resume on next hotkey activation.
    ///
    /// Late events arriving after `ConversationEnd` are tolerated; see
    /// the module-level "Late-event tolerance" doc.
    ConversationEnd { reason: String },
}

impl RecorderEvent {
    /// Returns the [`TraceDomain`] this event belongs to. Used by the
    /// registry to pick the destination file.
    pub fn domain(&self) -> TraceDomain {
        match self {
            RecorderEvent::TurnStart { .. }
            | RecorderEvent::LlmCall { .. }
            | RecorderEvent::StreamingLlmCall { .. }
            | RecorderEvent::SearxngQuery { .. }
            | RecorderEvent::ReaderBatch { .. }
            | RecorderEvent::ChunkerBatch { .. }
            | RecorderEvent::RerankResult { .. }
            | RecorderEvent::JudgeVerdict { .. }
            | RecorderEvent::Warning { .. }
            | RecorderEvent::SearchEventEmitted { .. }
            | RecorderEvent::TurnEnd { .. } => TraceDomain::Search,
            RecorderEvent::ConversationStart { .. }
            | RecorderEvent::UserMessage { .. }
            | RecorderEvent::AssistantThinking { .. }
            | RecorderEvent::AssistantTokens { .. }
            | RecorderEvent::AssistantComplete { .. }
            | RecorderEvent::ScreenCaptured { .. }
            | RecorderEvent::ConversationEnd { .. } => TraceDomain::Chat,
        }
    }

    /// Returns true if this event terminates a conversation in the chat
    /// domain. Used by the registry to evict the file handle from its
    /// per-conversation cache.
    pub fn is_conversation_end(&self) -> bool {
        matches!(self, RecorderEvent::ConversationEnd { .. })
    }
}

/// Per-URL outcome inside a [`RecorderEvent::ReaderBatch`].
#[derive(Debug, Clone, Serialize)]
pub struct ReaderUrlOutcome {
    pub url: String,
    pub status: Option<u16>,
    pub latency_ms: u64,
    pub raw_body: Option<String>,
    pub extracted_text: Option<String>,
    pub error: Option<String>,
}

/// Single ranked chunk inside a [`RecorderEvent::RerankResult`].
#[derive(Debug, Clone, Serialize)]
pub struct RerankedChunk {
    pub source_url: String,
    pub score: f64,
    pub text: String,
}

/// Lazy file-backed recorder for ONE `(domain, conversation_id)` pair.
/// Constructor is cheap (stashes path); the directory and file are only
/// created on the first `record()` call.
///
/// Uses `parking_lot::Mutex` to match the rest of the codebase. The
/// mutex is held for the duration of one `serde_json::to_writer` +
/// `writeln` + `flush`, which is short enough to keep contention
/// negligible even under concurrent conversations writing through the
/// registry.
#[derive(Debug)]
pub struct FileRecorder {
    /// Resolved trace file path. Computed at construction so the path
    /// is stable for the lifetime of the recorder.
    path: PathBuf,
    /// Open writer, lazily initialized on first record. `None` after a
    /// previous I/O failure (we degrade to noop, see `failed`).
    state: Mutex<Option<BufWriter<File>>>,
    /// Monotonic record sequence. Survives re-entry across multiple
    /// records even though only one record is in flight at a time.
    seq: AtomicU64,
    /// Latched once a write or open error occurred. Subsequent records
    /// are silent no-ops; the warning is rate-limited to a single
    /// `eprintln!`.
    failed: AtomicBool,
    /// Set once the failure warning has been printed. Prevents log spam.
    warned: AtomicBool,
}

impl FileRecorder {
    /// Construct a recorder targeting
    /// `<traces_root>/<domain.dir()>/<conversation_id>.jsonl`. The
    /// directory is NOT created here; that happens on first record so
    /// we don't pay I/O cost (or surface errors) for conversations that
    /// never write.
    ///
    /// The recorder writes in append mode, so a recorder constructed
    /// for an existing path simply appends to the existing file. This
    /// is the mechanism that gives the registry late-event tolerance:
    /// after a `ConversationEnd` evicts the handle, a stray event for
    /// the same conversation rebuilds the recorder and appends a benign
    /// post-end line.
    pub fn for_conversation(
        traces_root: impl AsRef<Path>,
        domain: TraceDomain,
        conversation_id: &ConversationId,
    ) -> Self {
        let path = traces_root
            .as_ref()
            .join(domain.dir())
            .join(format!("{conversation_id}.jsonl"));
        Self {
            path,
            state: Mutex::new(None),
            seq: AtomicU64::new(0),
            failed: AtomicBool::new(false),
            warned: AtomicBool::new(false),
        }
    }

    /// Path the recorder writes to. Useful for tests and debug output.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Latches the recorder as failed and emits a single rate-limited
    /// stderr warning. Filesystem-I/O wrapper: this is the only call
    /// site that mutates the failed/warned flags, so the entire body is
    /// just I/O-failure plumbing. Logic is exercised through every
    /// error branch below; coverage is excluded because triggering each
    /// individual `eprintln!` reliably across CI would require contrived
    /// platform-specific filesystem behaviour.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn fail_with(&self, msg: String) {
        self.failed.store(true, Ordering::SeqCst);
        if self
            .warned
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            eprintln!("thuki: [trace] {msg}");
        }
    }

    /// Test-only entrypoint mirroring [`fail_with`] without the warn-once
    /// gate, exposed only so the rate-limiting unit test can verify the
    /// `warned` flag stays latched across repeated calls.
    #[cfg(test)]
    fn warn_once(&self, msg: &str) {
        self.fail_with(msg.to_string());
    }

    /// Ensures the file is open. On first call, creates the parent
    /// directory and opens the file in append mode. Returns `None` and
    /// marks the recorder failed on any I/O error.
    ///
    /// Filesystem-I/O wrapper: the body is just an open + mkdir thin
    /// shim around `std::fs`. Logic is exercised end-to-end through the
    /// `file_recorder_*` integration tests.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn ensure_open<'a>(
        &self,
        guard: &'a mut Option<BufWriter<File>>,
    ) -> Option<&'a mut BufWriter<File>> {
        if guard.is_some() {
            return guard.as_mut();
        }
        if let Some(parent) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                self.fail_with(format!(
                    "could not create trace dir {}: {e}",
                    parent.display()
                ));
                return None;
            }
        }
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(f) => {
                *guard = Some(BufWriter::new(f));
                guard.as_mut()
            }
            Err(e) => {
                self.fail_with(format!(
                    "could not open trace file {}: {e}",
                    self.path.display()
                ));
                None
            }
        }
    }

    /// Flushes any buffered writes to disk. Called by the registry on
    /// `ConversationEnd` so the post-end file is fully durable before
    /// the handle is evicted.
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub(crate) fn flush(&self) {
        let mut guard = self.state.lock();
        if let Some(writer) = guard.as_mut() {
            let _ = writer.flush();
        }
    }
}

impl TraceRecorder for FileRecorder {
    /// Filesystem-I/O wrapper: orchestrates [`serialize_event()`] (covered)
    /// and [`Self::ensure_open()`] (covered via integration tests). The body
    /// itself is plumbing around `std::io::Write` that cannot be
    /// exercised reliably for every error branch on every CI runner.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn record(&self, conversation_id: &ConversationId, event: RecorderEvent) {
        if self.failed.load(Ordering::SeqCst) {
            return;
        }
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let line = serialize_event(seq, ts_ms, conversation_id, &event)
            .unwrap_or_else(|e| serialize_failure_fallback(self, e));
        if line.is_empty() {
            return;
        }
        let mut guard = self.state.lock();
        let Some(writer) = self.ensure_open(&mut guard) else {
            return;
        };
        if let Err(e) = writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.flush())
        {
            self.fail_with(format!(
                "could not write trace line to {}: {e}",
                self.path.display()
            ));
        }
    }
}

/// Fallback used when `serialize_event` returns an error. Defense-in-depth
/// for a path that cannot fire under our typed events; coverage is
/// excluded because reaching it would require corrupting the in-memory
/// `RecorderEvent` representation.
#[cfg_attr(coverage_nightly, coverage(off))]
fn serialize_failure_fallback(recorder: &FileRecorder, err: serde_json::Error) -> String {
    recorder.fail_with(format!("could not serialize event: {err}"));
    String::new()
}

/// Serializes a single event into a newline-terminated JSON record.
///
/// Stamps the four standard top-level fields onto every line:
/// `v` (schema version), `seq` (per-recorder monotonic counter),
/// `ts_ms` (wall-clock UNIX millis), `domain` (chat or search), plus
/// `conversation_id` (the ID passed to `record`). The remaining fields
/// come from the event variant via the `Serialize` impl below.
fn serialize_event(
    seq: u64,
    ts_ms: u64,
    conversation_id: &ConversationId,
    event: &RecorderEvent,
) -> serde_json::Result<String> {
    let mut value = serde_json::to_value(event)?;
    if let Some(map) = value.as_object_mut() {
        map.insert("v".into(), Value::from(TRACE_SCHEMA_VERSION));
        map.insert("seq".into(), Value::from(seq));
        map.insert("ts_ms".into(), Value::from(ts_ms));
        map.insert(
            "domain".into(),
            serde_json::to_value(event.domain()).unwrap_or(Value::Null),
        );
        map.insert(
            "conversation_id".into(),
            Value::from(conversation_id.as_str()),
        );
    }
    let mut s = serde_json::to_string(&value)?;
    s.push('\n');
    Ok(s)
}

/// Manual `Serialize` for [`RecorderEvent`]: emits a `kind` discriminator
/// plus a flat payload. Avoids the default tagged-enum representation so
/// downstream consumers can grep `"kind":"llm_call"` directly.
impl Serialize for RecorderEvent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        match self {
            // ─── Search domain ────────────────────────────────────────
            RecorderEvent::TurnStart {
                turn_id,
                query,
                model,
                runtime_config,
                history_len,
            } => {
                map.serialize_entry("kind", "turn_start")?;
                map.serialize_entry("turn_id", turn_id)?;
                map.serialize_entry("query", query)?;
                map.serialize_entry("model", model)?;
                map.serialize_entry("runtime_config", runtime_config)?;
                map.serialize_entry("history_len", history_len)?;
            }
            RecorderEvent::LlmCall {
                stage,
                endpoint,
                request_body,
                response_raw,
                latency_ms,
                error,
            } => {
                map.serialize_entry("kind", "llm_call")?;
                map.serialize_entry("stage", stage)?;
                map.serialize_entry("endpoint", endpoint)?;
                map.serialize_entry("request_body", request_body)?;
                map.serialize_entry("response_raw", response_raw)?;
                map.serialize_entry("latency_ms", latency_ms)?;
                map.serialize_entry("error", error)?;
            }
            RecorderEvent::StreamingLlmCall {
                stage,
                endpoint,
                request_body,
                final_text,
                tokens,
                latency_ms,
                error,
            } => {
                map.serialize_entry("kind", "streaming_llm_call")?;
                map.serialize_entry("stage", stage)?;
                map.serialize_entry("endpoint", endpoint)?;
                map.serialize_entry("request_body", request_body)?;
                map.serialize_entry("final_text", final_text)?;
                map.serialize_entry("tokens", tokens)?;
                map.serialize_entry("latency_ms", latency_ms)?;
                map.serialize_entry("error", error)?;
            }
            RecorderEvent::SearxngQuery {
                query,
                url,
                status,
                response_raw,
                normalized_results,
                latency_ms,
                error,
            } => {
                map.serialize_entry("kind", "searxng_query")?;
                map.serialize_entry("query", query)?;
                map.serialize_entry("url", url)?;
                map.serialize_entry("status", status)?;
                map.serialize_entry("response_raw", response_raw)?;
                map.serialize_entry("normalized_results", normalized_results)?;
                map.serialize_entry("latency_ms", latency_ms)?;
                map.serialize_entry("error", error)?;
            }
            RecorderEvent::ReaderBatch {
                urls,
                per_url,
                batch_latency_ms,
                batch_error,
            } => {
                map.serialize_entry("kind", "reader_batch")?;
                map.serialize_entry("urls", urls)?;
                map.serialize_entry("per_url", per_url)?;
                map.serialize_entry("batch_latency_ms", batch_latency_ms)?;
                map.serialize_entry("batch_error", batch_error)?;
            }
            RecorderEvent::ChunkerBatch {
                page_url,
                chunk_count,
                chunks,
            } => {
                map.serialize_entry("kind", "chunker_batch")?;
                map.serialize_entry("page_url", page_url)?;
                map.serialize_entry("chunk_count", chunk_count)?;
                map.serialize_entry("chunks", chunks)?;
            }
            RecorderEvent::RerankResult {
                query,
                input_count,
                top_k,
            } => {
                map.serialize_entry("kind", "rerank_result")?;
                map.serialize_entry("query", query)?;
                map.serialize_entry("input_count", input_count)?;
                map.serialize_entry("top_k", top_k)?;
            }
            RecorderEvent::JudgeVerdict {
                stage,
                raw,
                normalized,
            } => {
                map.serialize_entry("kind", "judge_verdict")?;
                map.serialize_entry("stage", stage)?;
                map.serialize_entry("raw", raw)?;
                map.serialize_entry("normalized", normalized)?;
            }
            RecorderEvent::Warning { kind, payload } => {
                map.serialize_entry("kind", "warning")?;
                map.serialize_entry("warning_kind", kind)?;
                map.serialize_entry("payload", payload)?;
            }
            RecorderEvent::SearchEventEmitted { event } => {
                map.serialize_entry("kind", "search_event")?;
                map.serialize_entry("event", event)?;
            }
            RecorderEvent::TurnEnd {
                turn_id,
                final_action,
                final_source_urls,
                total_latency_ms,
                error,
            } => {
                map.serialize_entry("kind", "turn_end")?;
                map.serialize_entry("turn_id", turn_id)?;
                map.serialize_entry("final_action", final_action)?;
                map.serialize_entry("final_source_urls", final_source_urls)?;
                map.serialize_entry("total_latency_ms", total_latency_ms)?;
                map.serialize_entry("error", error)?;
            }

            // ─── Chat domain ──────────────────────────────────────────
            RecorderEvent::ConversationStart {
                model,
                system_prompt,
            } => {
                map.serialize_entry("kind", "conversation_start")?;
                map.serialize_entry("model", model)?;
                map.serialize_entry("system_prompt", system_prompt)?;
            }
            RecorderEvent::UserMessage {
                content,
                attached_images,
                slash_command,
            } => {
                map.serialize_entry("kind", "user_message")?;
                map.serialize_entry("content", content)?;
                map.serialize_entry("attached_images", attached_images)?;
                map.serialize_entry("slash_command", slash_command)?;
            }
            RecorderEvent::AssistantThinking { chunk } => {
                map.serialize_entry("kind", "assistant_thinking")?;
                map.serialize_entry("chunk", chunk)?;
            }
            RecorderEvent::AssistantTokens { chunk } => {
                map.serialize_entry("kind", "assistant_tokens")?;
                map.serialize_entry("chunk", chunk)?;
            }
            RecorderEvent::AssistantComplete {
                total_tokens,
                latency_ms,
            } => {
                map.serialize_entry("kind", "assistant_complete")?;
                map.serialize_entry("total_tokens", total_tokens)?;
                map.serialize_entry("latency_ms", latency_ms)?;
            }
            RecorderEvent::ScreenCaptured {
                image_path,
                displays,
            } => {
                map.serialize_entry("kind", "screen_captured")?;
                map.serialize_entry("image_path", image_path)?;
                map.serialize_entry("displays", displays)?;
            }
            RecorderEvent::ConversationEnd { reason } => {
                map.serialize_entry("kind", "conversation_end")?;
                map.serialize_entry("reason", reason)?;
            }
        }
        map.end()
    }
}

/// In-memory recorder used by tests to assert what the chat layer or
/// search pipeline emitted. Lives in a `cfg(test)` block so production
/// builds never link it.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct MockRecorder {
    events: Mutex<Vec<(ConversationId, RecorderEvent)>>,
}

#[cfg(test)]
impl MockRecorder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn snapshot(&self) -> Vec<(ConversationId, RecorderEvent)> {
        self.events.lock().clone()
    }

    pub(crate) fn len(&self) -> usize {
        self.events.lock().len()
    }
}

#[cfg(test)]
impl TraceRecorder for MockRecorder {
    fn record(&self, conversation_id: &ConversationId, event: RecorderEvent) {
        self.events.lock().push((conversation_id.clone(), event));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn fresh_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("thuki-trace-tests-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read_lines(path: &Path) -> Vec<Value> {
        let s = std::fs::read_to_string(path).expect("read trace file");
        s.lines()
            .map(|l| serde_json::from_str::<Value>(l).expect("valid json line"))
            .collect()
    }

    fn cid(s: &str) -> ConversationId {
        ConversationId::new(s)
    }

    #[test]
    fn trace_domain_dir_strings_match_layout() {
        assert_eq!(TraceDomain::Chat.dir(), "chat");
        assert_eq!(TraceDomain::Search.dir(), "search");
    }

    #[test]
    fn trace_domain_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&TraceDomain::Chat).unwrap(),
            "\"chat\""
        );
        assert_eq!(
            serde_json::to_string(&TraceDomain::Search).unwrap(),
            "\"search\""
        );
    }

    #[test]
    fn search_variants_belong_to_search_domain() {
        let cases = vec![
            RecorderEvent::TurnStart {
                turn_id: "t".into(),
                query: "q".into(),
                model: "m".into(),
                runtime_config: json!({}),
                history_len: 0,
            },
            RecorderEvent::LlmCall {
                stage: "router".into(),
                endpoint: "e".into(),
                request_body: json!({}),
                response_raw: None,
                latency_ms: 0,
                error: None,
            },
            RecorderEvent::StreamingLlmCall {
                stage: "synthesis".into(),
                endpoint: "e".into(),
                request_body: json!({}),
                final_text: None,
                tokens: 0,
                latency_ms: 0,
                error: None,
            },
            RecorderEvent::SearxngQuery {
                query: "q".into(),
                url: "u".into(),
                status: None,
                response_raw: None,
                normalized_results: json!([]),
                latency_ms: 0,
                error: None,
            },
            RecorderEvent::ReaderBatch {
                urls: vec![],
                per_url: vec![],
                batch_latency_ms: 0,
                batch_error: None,
            },
            RecorderEvent::ChunkerBatch {
                page_url: "u".into(),
                chunk_count: 0,
                chunks: vec![],
            },
            RecorderEvent::RerankResult {
                query: "q".into(),
                input_count: 0,
                top_k: vec![],
            },
            RecorderEvent::JudgeVerdict {
                stage: "snippet".into(),
                raw: "{}".into(),
                normalized: json!({}),
            },
            RecorderEvent::Warning {
                kind: "k".into(),
                payload: json!({}),
            },
            RecorderEvent::SearchEventEmitted { event: json!({}) },
            RecorderEvent::TurnEnd {
                turn_id: "t".into(),
                final_action: "answer".into(),
                final_source_urls: vec![],
                total_latency_ms: 0,
                error: None,
            },
        ];
        for e in cases {
            assert_eq!(e.domain(), TraceDomain::Search, "{:?} should be search", e);
        }
    }

    #[test]
    fn chat_variants_belong_to_chat_domain() {
        let cases = vec![
            RecorderEvent::ConversationStart {
                model: "m".into(),
                system_prompt: "s".into(),
            },
            RecorderEvent::UserMessage {
                content: "hi".into(),
                attached_images: vec![],
                slash_command: None,
            },
            RecorderEvent::AssistantThinking {
                chunk: "thinking".into(),
            },
            RecorderEvent::AssistantTokens {
                chunk: "answer".into(),
            },
            RecorderEvent::AssistantComplete {
                total_tokens: 1,
                latency_ms: 1,
            },
            RecorderEvent::ScreenCaptured {
                image_path: "p".into(),
                displays: 1,
            },
            RecorderEvent::ConversationEnd {
                reason: "quit".into(),
            },
        ];
        for e in cases {
            assert_eq!(e.domain(), TraceDomain::Chat, "{:?} should be chat", e);
        }
    }

    #[test]
    fn is_conversation_end_only_true_for_conversation_end_variant() {
        assert!(RecorderEvent::ConversationEnd { reason: "x".into() }.is_conversation_end());
        assert!(!RecorderEvent::AssistantTokens { chunk: "x".into() }.is_conversation_end());
        assert!(!RecorderEvent::Warning {
            kind: "x".into(),
            payload: json!({}),
        }
        .is_conversation_end());
    }

    #[test]
    fn noop_recorder_swallows_every_event() {
        let r = NoopRecorder;
        r.record(
            &cid("conv-x"),
            RecorderEvent::Warning {
                kind: "x".into(),
                payload: json!({}),
            },
        );
        // No panic, no I/O.
    }

    #[test]
    fn file_recorder_lazy_creates_dir_and_file_on_first_record() {
        let root = fresh_dir().join("nested").join("traces");
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-a"));
        assert!(!root.exists(), "constructor must be lazy");
        r.record(
            &cid("conv-a"),
            RecorderEvent::UserMessage {
                content: "hi".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        assert!(r.path().exists(), "file created on first record");
        assert!(r.path().parent().unwrap().exists(), "domain dir created");
        assert_eq!(
            r.path()
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "chat"
        );
    }

    #[test]
    fn file_recorder_writes_one_jsonl_line_per_record_with_schema_metadata() {
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-meta"));
        r.record(
            &cid("conv-meta"),
            RecorderEvent::ConversationStart {
                model: "qwen3:4b".into(),
                system_prompt: "You are Thuki...".into(),
            },
        );
        r.record(
            &cid("conv-meta"),
            RecorderEvent::UserMessage {
                content: "summarize".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        let lines = read_lines(r.path());
        assert_eq!(lines.len(), 2);
        for (i, line) in lines.iter().enumerate() {
            assert_eq!(line["v"], TRACE_SCHEMA_VERSION);
            assert_eq!(line["seq"], i as u64);
            assert_eq!(line["domain"], "chat");
            assert_eq!(line["conversation_id"], "conv-meta");
            assert!(line["ts_ms"].as_u64().unwrap() > 0);
        }
        assert_eq!(lines[0]["kind"], "conversation_start");
        assert_eq!(lines[0]["model"], "qwen3:4b");
        assert_eq!(lines[1]["kind"], "user_message");
        assert_eq!(lines[1]["content"], "summarize");
    }

    #[test]
    fn file_recorder_serializes_every_search_variant() {
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Search, &cid("conv-all"));
        r.record(
            &cid("conv-all"),
            RecorderEvent::TurnStart {
                turn_id: "t".into(),
                query: "q".into(),
                model: "m".into(),
                runtime_config: json!({}),
                history_len: 0,
            },
        );
        r.record(
            &cid("conv-all"),
            RecorderEvent::LlmCall {
                stage: "router".into(),
                endpoint: "http://x/api/chat".into(),
                request_body: json!({"messages": []}),
                response_raw: Some("{}".into()),
                latency_ms: 12,
                error: None,
            },
        );
        r.record(
            &cid("conv-all"),
            RecorderEvent::StreamingLlmCall {
                stage: "synthesis".into(),
                endpoint: "http://x/api/chat".into(),
                request_body: json!({}),
                final_text: Some("answer".into()),
                tokens: 7,
                latency_ms: 100,
                error: None,
            },
        );
        r.record(
            &cid("conv-all"),
            RecorderEvent::SearxngQuery {
                query: "rust".into(),
                url: "http://s/search".into(),
                status: Some(200),
                response_raw: Some("{}".into()),
                normalized_results: json!([]),
                latency_ms: 5,
                error: None,
            },
        );
        r.record(
            &cid("conv-all"),
            RecorderEvent::ReaderBatch {
                urls: vec!["http://a".into()],
                per_url: vec![ReaderUrlOutcome {
                    url: "http://a".into(),
                    status: Some(200),
                    latency_ms: 9,
                    raw_body: Some("<html/>".into()),
                    extracted_text: Some("hi".into()),
                    error: None,
                }],
                batch_latency_ms: 11,
                batch_error: None,
            },
        );
        r.record(
            &cid("conv-all"),
            RecorderEvent::ChunkerBatch {
                page_url: "http://a".into(),
                chunk_count: 1,
                chunks: vec!["chunk".into()],
            },
        );
        r.record(
            &cid("conv-all"),
            RecorderEvent::RerankResult {
                query: "rust".into(),
                input_count: 1,
                top_k: vec![RerankedChunk {
                    source_url: "http://a".into(),
                    score: 0.8,
                    text: "chunk".into(),
                }],
            },
        );
        r.record(
            &cid("conv-all"),
            RecorderEvent::JudgeVerdict {
                stage: "snippet".into(),
                raw: "{}".into(),
                normalized: json!({"sufficiency": "Sufficient"}),
            },
        );
        r.record(
            &cid("conv-all"),
            RecorderEvent::Warning {
                kind: "JudgeFailed".into(),
                payload: json!({"reason": "parse"}),
            },
        );
        r.record(
            &cid("conv-all"),
            RecorderEvent::SearchEventEmitted {
                event: json!({"type": "Done"}),
            },
        );
        r.record(
            &cid("conv-all"),
            RecorderEvent::TurnEnd {
                turn_id: "t".into(),
                final_action: "answer".into(),
                final_source_urls: vec!["http://a".into()],
                total_latency_ms: 1234,
                error: None,
            },
        );
        let lines = read_lines(r.path());
        let kinds: Vec<&str> = lines.iter().map(|l| l["kind"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            vec![
                "turn_start",
                "llm_call",
                "streaming_llm_call",
                "searxng_query",
                "reader_batch",
                "chunker_batch",
                "rerank_result",
                "judge_verdict",
                "warning",
                "search_event",
                "turn_end"
            ]
        );
        for line in &lines {
            assert_eq!(line["domain"], "search");
            assert_eq!(line["conversation_id"], "conv-all");
        }
    }

    #[test]
    fn file_recorder_serializes_every_chat_variant() {
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-chat"));
        r.record(
            &cid("conv-chat"),
            RecorderEvent::ConversationStart {
                model: "qwen3:4b".into(),
                system_prompt: "sys".into(),
            },
        );
        r.record(
            &cid("conv-chat"),
            RecorderEvent::UserMessage {
                content: "hello".into(),
                attached_images: vec!["/tmp/img.jpg".into()],
                slash_command: Some("/screen".into()),
            },
        );
        r.record(
            &cid("conv-chat"),
            RecorderEvent::AssistantThinking {
                chunk: "thinking...".into(),
            },
        );
        r.record(
            &cid("conv-chat"),
            RecorderEvent::AssistantTokens {
                chunk: "Hi there".into(),
            },
        );
        r.record(
            &cid("conv-chat"),
            RecorderEvent::AssistantComplete {
                total_tokens: 42,
                latency_ms: 500,
            },
        );
        r.record(
            &cid("conv-chat"),
            RecorderEvent::ScreenCaptured {
                image_path: "/tmp/snap.jpg".into(),
                displays: 2,
            },
        );
        r.record(
            &cid("conv-chat"),
            RecorderEvent::ConversationEnd {
                reason: "quit".into(),
            },
        );
        let lines = read_lines(r.path());
        let kinds: Vec<&str> = lines.iter().map(|l| l["kind"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            vec![
                "conversation_start",
                "user_message",
                "assistant_thinking",
                "assistant_tokens",
                "assistant_complete",
                "screen_captured",
                "conversation_end"
            ]
        );
        assert_eq!(lines[1]["attached_images"], json!(["/tmp/img.jpg"]));
        assert_eq!(lines[1]["slash_command"], "/screen");
        assert_eq!(lines[5]["displays"], 2);
        assert_eq!(lines[6]["reason"], "quit");
    }

    #[test]
    fn file_recorder_path_includes_domain_subfolder_and_conv_id_jsonl() {
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-x"));
        assert_eq!(r.path(), root.join("chat").join("conv-x.jsonl"));
        let r2 = FileRecorder::for_conversation(&root, TraceDomain::Search, &cid("conv-y"));
        assert_eq!(r2.path(), root.join("search").join("conv-y.jsonl"));
    }

    #[test]
    fn file_recorder_uses_arc_dyn_via_trait_object() {
        let root = fresh_dir();
        let r: Arc<dyn TraceRecorder> = Arc::new(FileRecorder::for_conversation(
            &root,
            TraceDomain::Chat,
            &cid("conv-arc"),
        ));
        r.record(
            &cid("conv-arc"),
            RecorderEvent::Warning {
                kind: "k".into(),
                payload: json!({}),
            },
        );
    }

    #[test]
    fn file_recorder_open_failure_is_silent_and_latches() {
        let root = fresh_dir();
        let blocking_file = root.join("blocker");
        std::fs::write(&blocking_file, b"x").unwrap();
        // Pass a root path that cannot be created (regular file in the way).
        let r = FileRecorder::for_conversation(
            blocking_file.join("inside"),
            TraceDomain::Chat,
            &cid("conv-fail"),
        );
        r.record(
            &cid("conv-fail"),
            RecorderEvent::Warning {
                kind: "k".into(),
                payload: json!({}),
            },
        );
        assert!(
            r.failed.load(Ordering::SeqCst),
            "open failure must latch the recorder"
        );
        // Second call exits at the latch check, exercising that branch.
        r.record(
            &cid("conv-fail"),
            RecorderEvent::Warning {
                kind: "k2".into(),
                payload: json!({}),
            },
        );
    }

    #[test]
    fn file_recorder_open_failure_when_target_path_is_a_directory_is_silent() {
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-dir"));
        // Pre-create the JSONL path AS A DIRECTORY so OpenOptions append
        // hits "Is a directory" when it tries to open it as a file.
        std::fs::create_dir_all(r.path()).unwrap();
        r.record(
            &cid("conv-dir"),
            RecorderEvent::Warning {
                kind: "k".into(),
                payload: json!({}),
            },
        );
        assert!(
            r.failed.load(Ordering::SeqCst),
            "open failure on directory-collision must latch the recorder"
        );
    }

    #[test]
    fn file_recorder_warn_once_only_prints_first_failure() {
        let root = fresh_dir();
        let blocking_file = root.join("blocker2");
        std::fs::write(&blocking_file, b"x").unwrap();
        let r = FileRecorder::for_conversation(
            blocking_file.join("nested"),
            TraceDomain::Chat,
            &cid("conv-warn"),
        );
        r.record(
            &cid("conv-warn"),
            RecorderEvent::Warning {
                kind: "k".into(),
                payload: json!({}),
            },
        );
        assert!(r.warned.load(Ordering::SeqCst));
        r.warn_once("ignored");
        r.warn_once("ignored2");
        assert!(r.warned.load(Ordering::SeqCst));
    }

    #[test]
    fn file_recorder_flush_is_safe_before_first_record() {
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-empty"));
        // No record calls yet; state is None. flush() must be a no-op.
        r.flush();
        assert!(
            !r.path().exists(),
            "flush before record must not create file"
        );
    }

    #[test]
    fn file_recorder_flush_after_record_is_safe() {
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-flush"));
        r.record(
            &cid("conv-flush"),
            RecorderEvent::AssistantTokens { chunk: "x".into() },
        );
        r.flush();
        // File was written and flushed; subsequent read sees the line.
        assert_eq!(read_lines(r.path()).len(), 1);
    }

    #[test]
    fn file_recorder_appends_when_constructed_for_existing_path() {
        let root = fresh_dir();
        let r1 = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-resume"));
        r1.record(
            &cid("conv-resume"),
            RecorderEvent::UserMessage {
                content: "first".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        r1.flush();
        // Second recorder for the same conv: simulates registry re-create
        // after eviction. Must append, not truncate.
        let r2 = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-resume"));
        r2.record(
            &cid("conv-resume"),
            RecorderEvent::UserMessage {
                content: "second".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        let lines = read_lines(r1.path());
        assert_eq!(lines.len(), 2, "second recorder must append, not truncate");
        assert_eq!(lines[0]["content"], "first");
        assert_eq!(lines[1]["content"], "second");
    }

    #[test]
    fn mock_recorder_collects_events_in_order_with_conv_id() {
        let m = MockRecorder::new();
        m.record(
            &cid("conv-a"),
            RecorderEvent::Warning {
                kind: "a".into(),
                payload: json!({}),
            },
        );
        m.record(
            &cid("conv-b"),
            RecorderEvent::Warning {
                kind: "b".into(),
                payload: json!({}),
            },
        );
        assert_eq!(m.len(), 2);
        let snap = m.snapshot();
        assert_eq!(snap[0].0, cid("conv-a"));
        assert_eq!(snap[1].0, cid("conv-b"));
        let dump: Vec<serde_json::Value> = snap
            .iter()
            .map(|(_, e)| serde_json::to_value(e).unwrap())
            .collect();
        assert_eq!(dump[0]["warning_kind"], "a");
        assert_eq!(dump[1]["warning_kind"], "b");
    }
}
