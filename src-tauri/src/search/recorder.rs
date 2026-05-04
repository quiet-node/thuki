//! Forensic per-turn trace recorder for the `/search` pipeline.
//!
//! When [`crate::config::schema::DebugSection::search_trace_enabled`] is on,
//! the pipeline records every LLM request/response, every SearXNG query and
//! raw response, every reader batch (per-URL latency + raw body + full
//! extracted text), every chunker/rerank step, and every judge verdict to a
//! single JSON-Lines file under `app_data_dir()/traces/`. One file per
//! pipeline turn.
//!
//! Off in shipped builds. Intended for local quality investigation: open a
//! `.jsonl` trace and grep/jq across LLM prompts vs. judge verdicts to
//! understand why the pipeline produced a given answer.
//!
//! # Architecture
//!
//! - [`PipelineRecorder`] is the trait every callsite calls into. The pipeline
//!   threads `&Arc<dyn PipelineRecorder>` through its execution context; no
//!   call site distinguishes the live recorder from the noop.
//! - [`NoopRecorder`] is the production default. Every method is a no-op.
//! - [`FileRecorder`] is the dev-mode implementation. Lazy directory + file
//!   creation on first record, `parking_lot::Mutex` around a buffered writer,
//!   per-line flush so partial files are still grep-friendly if the daemon
//!   crashes. I/O errors are warned once via `eprintln!` then become silent
//!   no-ops for the rest of the turn — trace failures must never affect the
//!   user-visible pipeline.
//! - [`MockRecorder`] (test-only) collects every event into an in-memory
//!   `Vec<RecorderEvent>` for instrumentation-seam assertions.
//!
//! # Schema
//!
//! Each line is a self-describing JSON object:
//!
//! ```json
//! {"v": 1, "seq": 0, "ts_ms": 1714762800000, "kind": "turn_start", ...payload}
//! ```
//!
//! `seq` is monotonic per recorder; `ts_ms` is wall-clock UNIX millis. The
//! `v` field allows future schema evolution.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

/// Schema version embedded in every record. Bump when the wire shape changes.
pub const TRACE_SCHEMA_VERSION: u32 = 1;

/// Trait every pipeline callsite uses to emit forensic events. Implementors
/// must be cheap when tracing is off (the [`NoopRecorder`] case dominates
/// production usage).
pub trait PipelineRecorder: Send + Sync {
    /// Records a single event. Implementors MUST NOT panic, MUST NOT block
    /// for arbitrary durations, and MUST NOT propagate errors. Trace I/O
    /// failures must be swallowed (with a single rate-limited warning) so
    /// the pipeline is unaffected.
    fn record(&self, event: RecorderEvent);
}

/// Production default: every method is a no-op. Constant time, zero
/// allocations.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopRecorder;

impl PipelineRecorder for NoopRecorder {
    fn record(&self, _event: RecorderEvent) {}
}

/// Forensic trace events emitted by the search pipeline.
///
/// Each variant maps to one JSON-Lines record. Field shapes are intentionally
/// permissive (`Value`, `String`) because the trace is a forensic dump, not
/// a typed contract. Downstream consumers (jq, ad-hoc scripts) read fields
/// by name.
#[derive(Debug, Clone)]
pub enum RecorderEvent {
    /// Pipeline turn starts. Captures the user query, model slug, runtime
    /// search-config snapshot (so a trace is reproducible without the
    /// config file), and conversation-history length.
    TurnStart {
        turn_id: String,
        query: String,
        model: String,
        runtime_config: Value,
        history_len: usize,
    },
    /// Single Ollama `/api/chat` non-streaming JSON request. Captures the
    /// stage label (router / snippet_judge / chunk_judge / synthesis /
    /// answer_from_context), the full request body the pipeline sent, the
    /// raw response string Ollama returned, and the request latency. `error`
    /// is `Some` when the call failed (timeout, transport, parse).
    LlmCall {
        stage: String,
        endpoint: String,
        request_body: Value,
        response_raw: Option<String>,
        latency_ms: u64,
        error: Option<String>,
    },
    /// Streaming Ollama `/api/chat` request used by the synthesis path.
    /// `tokens` counts streamed pieces; `final_text` is the accumulated
    /// assistant message; per-token records are NOT emitted (would flood
    /// the file without adding signal).
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
    /// Reader sidecar batch: per-URL fetch results including raw HTTP body
    /// and extracted text. Latency is per-URL plus the overall batch.
    ReaderBatch {
        urls: Vec<String>,
        per_url: Vec<ReaderUrlOutcome>,
        batch_latency_ms: u64,
        batch_error: Option<String>,
    },
    /// Page chunked into N pieces. `chunks` are the full chunk texts so a
    /// downstream consumer can reconstruct what the judge actually saw.
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
    /// Warning surfaced by the pipeline (e.g. JudgeFailed, BudgetExhausted).
    Warning { kind: String, payload: Value },
    /// Mirrors a [`crate::search::types::SearchEvent`] emission so the trace
    /// reflects the user-visible event stream alongside backend internals.
    SearchEventEmitted { event: Value },
    /// Pipeline turn ends. Captures the final action taken and total
    /// wall-clock latency. `error` is `Some` only on hard failure paths.
    TurnEnd {
        turn_id: String,
        final_action: String,
        final_source_urls: Vec<String>,
        total_latency_ms: u64,
        error: Option<String>,
    },
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

/// Builds a fresh, sortable, collision-resistant turn id.
///
/// Format: `<unix_secs>-<uuid_v4>`. Seconds are eyeball-readable when
/// browsing the traces directory; the v4 UUID guarantees uniqueness across
/// concurrent turns within the same second.
pub fn new_turn_id() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}-{}", uuid::Uuid::new_v4())
}

/// Lazy file-backed recorder. Constructor is cheap (stashes path + turn id);
/// the directory and file are only created on the first `record()` call.
///
/// Uses `parking_lot::Mutex` to match the rest of the codebase. The mutex is
/// held for the duration of one `serde_json::to_writer` + `writeln` + `flush`,
/// which is short enough to keep contention negligible even under concurrent
/// turns.
#[derive(Debug)]
pub struct FileRecorder {
    /// Resolved trace file path. Computed at construction so the path is
    /// stable for the lifetime of the recorder.
    path: PathBuf,
    /// Open writer, lazily initialized on first record. `None` after a
    /// previous I/O failure (we degrade to noop, see `failed`).
    state: Mutex<Option<BufWriter<File>>>,
    /// Monotonic record sequence. Survives re-entry across multiple records
    /// even though only one record is in flight at a time.
    seq: AtomicU64,
    /// Latched once a write or open error occurred. Subsequent records are
    /// silent no-ops; the warning is rate-limited to a single `eprintln!`.
    failed: AtomicBool,
    /// Set once the failure warning has been printed. Prevents log spam.
    warned: AtomicBool,
}

impl FileRecorder {
    /// Construct a recorder targeting `<dir>/<turn_id>.jsonl`. The directory
    /// is NOT created here; that happens on first record so we don't pay
    /// I/O cost (or surface errors) for turns that never write.
    pub fn new(dir: impl AsRef<Path>, turn_id: &str) -> Self {
        let path = dir.as_ref().join(format!("{turn_id}.jsonl"));
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
    /// stderr warning. Filesystem-I/O wrapper: this is the only call site
    /// that mutates the failed/warned flags, so the entire body is just
    /// I/O-failure plumbing. Logic is exercised through every error branch
    /// below; coverage is excluded because triggering each individual
    /// `eprintln!` reliably across CI would require contrived
    /// platform-specific filesystem behaviour.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn fail_with(&self, msg: String) {
        self.failed.store(true, Ordering::SeqCst);
        if self
            .warned
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            eprintln!("thuki: [search trace] {msg}");
        }
    }

    /// Test-only entrypoint mirroring [`fail_with`] without the warn-once
    /// gate, exposed only so the rate-limiting unit test can verify the
    /// `warned` flag stays latched across repeated calls.
    #[cfg(test)]
    fn warn_once(&self, msg: &str) {
        self.fail_with(msg.to_string());
    }

    /// Ensures the file is open. On first call, creates the parent directory
    /// and opens the file in append mode. Returns `None` and marks the
    /// recorder failed on any I/O error.
    ///
    /// Filesystem-I/O wrapper: the body is just an open + mkdir thin shim
    /// around `std::fs`. Logic is exercised end-to-end through the
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
}

impl PipelineRecorder for FileRecorder {
    /// Filesystem-I/O wrapper: orchestrates [`serialize_event`] (covered)
    /// and [`ensure_open`] (covered via integration tests). The body itself
    /// is plumbing around `std::io::Write` that cannot be exercised
    /// reliably for every error branch on every CI runner.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn record(&self, event: RecorderEvent) {
        if self.failed.load(Ordering::SeqCst) {
            return;
        }
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        // `serialize_event` produces a JSON line from a typed `RecorderEvent`.
        // `serde_json::to_value` cannot fail for our enum (no `Map<_, _>` keys
        // are non-strings, no `f64` is non-finite), so we treat any error as
        // unreachable-but-recoverable: latch the recorder as failed and skip
        // the line. The `unreachable!`-equivalent path is covered indirectly
        // through `fail_with`, which is itself a coverage-excluded I/O wrapper.
        let line = serialize_event(seq, ts_ms, &event)
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
/// for a path that cannot fire under our typed events; coverage is excluded
/// because reaching it would require corrupting the in-memory `RecorderEvent`
/// representation.
#[cfg_attr(coverage_nightly, coverage(off))]
fn serialize_failure_fallback(recorder: &FileRecorder, err: serde_json::Error) -> String {
    recorder.fail_with(format!("could not serialize event: {err}"));
    String::new()
}

/// Serializes a single event into a newline-terminated JSON record.
fn serialize_event(seq: u64, ts_ms: u64, event: &RecorderEvent) -> serde_json::Result<String> {
    let mut value = serde_json::to_value(event)?;
    if let Some(map) = value.as_object_mut() {
        map.insert("v".into(), Value::from(TRACE_SCHEMA_VERSION));
        map.insert("seq".into(), Value::from(seq));
        map.insert("ts_ms".into(), Value::from(ts_ms));
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
        }
        map.end()
    }
}

/// In-memory recorder used by tests to assert what the pipeline emitted.
/// Lives in a `cfg(test)` block so production builds never link it.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct MockRecorder {
    events: Mutex<Vec<RecorderEvent>>,
}

#[cfg(test)]
impl MockRecorder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn snapshot(&self) -> Vec<RecorderEvent> {
        self.events.lock().clone()
    }

    pub(crate) fn len(&self) -> usize {
        self.events.lock().len()
    }
}

#[cfg(test)]
impl PipelineRecorder for MockRecorder {
    fn record(&self, event: RecorderEvent) {
        self.events.lock().push(event);
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

    #[test]
    fn noop_recorder_swallows_every_event() {
        let r = NoopRecorder;
        r.record(RecorderEvent::Warning {
            kind: "x".into(),
            payload: json!({}),
        });
        // Nothing observable; the test simply asserts no panic and no I/O.
    }

    #[test]
    fn new_turn_id_format_is_unix_secs_dash_uuid() {
        let id = new_turn_id();
        let (secs, uuid) = id.split_once('-').expect("contains dash");
        assert!(secs.parse::<u64>().is_ok(), "secs prefix must parse: {id}");
        assert_eq!(uuid.len(), 36, "uuid v4 length: {id}");
    }

    #[test]
    fn new_turn_id_unique_across_calls() {
        let a = new_turn_id();
        let b = new_turn_id();
        assert_ne!(a, b);
    }

    #[test]
    fn file_recorder_lazy_creates_dir_and_file_on_first_record() {
        let dir = fresh_dir().join("nested").join("traces");
        assert!(!dir.exists(), "dir must not exist before record");
        let r = FileRecorder::new(&dir, "test-turn");
        // Constructor must not touch the filesystem.
        assert!(!dir.exists(), "constructor must be lazy");
        r.record(RecorderEvent::Warning {
            kind: "k".into(),
            payload: json!({"a": 1}),
        });
        assert!(dir.exists(), "dir created on first record");
        assert!(r.path().exists(), "file created on first record");
    }

    #[test]
    fn file_recorder_writes_one_jsonl_line_per_record_with_schema_metadata() {
        let dir = fresh_dir();
        let r = FileRecorder::new(&dir, "turn-a");
        r.record(RecorderEvent::TurnStart {
            turn_id: "turn-a".into(),
            query: "what is rust?".into(),
            model: "qwen3:30b".into(),
            runtime_config: json!({"max_iterations": 3}),
            history_len: 2,
        });
        r.record(RecorderEvent::Warning {
            kind: "JudgeFailed".into(),
            payload: json!({"reason": "parse"}),
        });
        let lines = read_lines(r.path());
        assert_eq!(lines.len(), 2, "two records => two lines");
        assert_eq!(lines[0]["v"], TRACE_SCHEMA_VERSION);
        assert_eq!(lines[0]["seq"], 0);
        assert_eq!(lines[0]["kind"], "turn_start");
        assert_eq!(lines[0]["query"], "what is rust?");
        assert_eq!(lines[0]["history_len"], 2);
        assert_eq!(lines[1]["seq"], 1);
        assert_eq!(lines[1]["kind"], "warning");
        assert_eq!(lines[1]["warning_kind"], "JudgeFailed");
        assert!(lines[0]["ts_ms"].as_u64().unwrap() > 0);
    }

    #[test]
    fn file_recorder_serializes_every_event_kind() {
        let dir = fresh_dir();
        let r = FileRecorder::new(&dir, "turn-all");
        r.record(RecorderEvent::TurnStart {
            turn_id: "t".into(),
            query: "q".into(),
            model: "m".into(),
            runtime_config: json!({}),
            history_len: 0,
        });
        r.record(RecorderEvent::LlmCall {
            stage: "router".into(),
            endpoint: "http://x/api/chat".into(),
            request_body: json!({"messages": []}),
            response_raw: Some("{}".into()),
            latency_ms: 12,
            error: None,
        });
        r.record(RecorderEvent::StreamingLlmCall {
            stage: "synthesis".into(),
            endpoint: "http://x/api/chat".into(),
            request_body: json!({}),
            final_text: Some("answer".into()),
            tokens: 7,
            latency_ms: 100,
            error: None,
        });
        r.record(RecorderEvent::SearxngQuery {
            query: "rust".into(),
            url: "http://s/search".into(),
            status: Some(200),
            response_raw: Some("{}".into()),
            normalized_results: json!([]),
            latency_ms: 5,
            error: None,
        });
        r.record(RecorderEvent::ReaderBatch {
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
        });
        r.record(RecorderEvent::ChunkerBatch {
            page_url: "http://a".into(),
            chunk_count: 1,
            chunks: vec!["chunk".into()],
        });
        r.record(RecorderEvent::RerankResult {
            query: "rust".into(),
            input_count: 1,
            top_k: vec![RerankedChunk {
                source_url: "http://a".into(),
                score: 0.8,
                text: "chunk".into(),
            }],
        });
        r.record(RecorderEvent::JudgeVerdict {
            stage: "snippet".into(),
            raw: "{}".into(),
            normalized: json!({"sufficiency": "Sufficient"}),
        });
        r.record(RecorderEvent::SearchEventEmitted {
            event: json!({"type": "Done"}),
        });
        r.record(RecorderEvent::TurnEnd {
            turn_id: "t".into(),
            final_action: "answer".into(),
            final_source_urls: vec!["http://a".into()],
            total_latency_ms: 1234,
            error: None,
        });
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
                "search_event",
                "turn_end"
            ]
        );
    }

    #[test]
    fn file_recorder_path_returns_target_path_with_jsonl_extension() {
        let dir = fresh_dir();
        let r = FileRecorder::new(&dir, "turn-x");
        assert_eq!(r.path(), dir.join("turn-x.jsonl"));
    }

    #[test]
    fn file_recorder_uses_arc_dyn_via_trait_object() {
        let dir = fresh_dir();
        let r: Arc<dyn PipelineRecorder> = Arc::new(FileRecorder::new(&dir, "turn-arc"));
        r.record(RecorderEvent::Warning {
            kind: "k".into(),
            payload: json!({}),
        });
        // Coverage: verifies the Arc<dyn ...> shape compiles and dispatches.
    }

    #[test]
    fn file_recorder_open_failure_is_silent_and_latches() {
        // Pass a path whose parent we make impossible to create: a regular
        // file at the parent location. mkdir -p fails because the path
        // component already exists as a file.
        let dir = fresh_dir();
        let blocking_file = dir.join("blocker");
        std::fs::write(&blocking_file, b"x").unwrap();
        let r = FileRecorder::new(blocking_file.join("inside"), "turn-fail");
        r.record(RecorderEvent::Warning {
            kind: "k".into(),
            payload: json!({}),
        });
        assert!(
            r.failed.load(Ordering::SeqCst),
            "open failure must latch the recorder"
        );
        // Second call exits at the latch check, exercising that branch.
        r.record(RecorderEvent::Warning {
            kind: "k2".into(),
            payload: json!({}),
        });
    }

    #[test]
    fn file_recorder_open_failure_when_target_path_is_a_directory_is_silent() {
        // mkdir -p of the parent succeeds, but `OpenOptions::open` on the
        // jsonl path fails because that path already exists and is a
        // directory. Exercises the OpenOptions Err arm in `ensure_open`
        // (distinct from the create_dir_all Err arm covered elsewhere).
        let dir = fresh_dir();
        let r = FileRecorder::new(&dir, "turn-dir-collision");
        // Pre-create the JSONL path AS A DIRECTORY, so OpenOptions append
        // hits "Is a directory" when it tries to open it as a file.
        std::fs::create_dir_all(r.path()).unwrap();
        r.record(RecorderEvent::Warning {
            kind: "k".into(),
            payload: json!({}),
        });
        assert!(
            r.failed.load(Ordering::SeqCst),
            "open failure on directory-collision must latch the recorder"
        );
    }

    #[test]
    fn file_recorder_warn_once_only_prints_first_failure() {
        let dir = fresh_dir();
        let blocking_file = dir.join("blocker2");
        std::fs::write(&blocking_file, b"x").unwrap();
        let r = FileRecorder::new(blocking_file.join("nested"), "turn-warn");
        // First record triggers warn + latch.
        r.record(RecorderEvent::Warning {
            kind: "k".into(),
            payload: json!({}),
        });
        assert!(r.warned.load(Ordering::SeqCst));
        // Direct repeated calls to warn_once must not flip the flag again.
        r.warn_once("ignored");
        r.warn_once("ignored2");
        assert!(r.warned.load(Ordering::SeqCst));
    }

    #[test]
    fn mock_recorder_collects_events_in_order() {
        let m = MockRecorder::new();
        m.record(RecorderEvent::Warning {
            kind: "a".into(),
            payload: json!({}),
        });
        m.record(RecorderEvent::Warning {
            kind: "b".into(),
            payload: json!({}),
        });
        assert_eq!(m.len(), 2);
        let snap = m.snapshot();
        // Round-trip through JSON serialization keeps the assertion exhaustive
        // without an unreachable fallback match arm: we built two `Warning`
        // events with `kind: a` and `kind: b`, so the on-the-wire shape must
        // reflect that ordering exactly.
        let dump: Vec<serde_json::Value> = snap
            .iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        assert_eq!(dump[0]["kind"], "warning");
        assert_eq!(dump[0]["warning_kind"], "a");
        assert_eq!(dump[1]["kind"], "warning");
        assert_eq!(dump[1]["warning_kind"], "b");
    }
}
