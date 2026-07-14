//! Forensic per-conversation trace recorder for the chat path, including
//! built-in websearch turns.
//!
//! When [`crate::config::schema::DebugSection::trace_enabled`] is on,
//! every chat turn (and its websearch events) writes a forensic
//! JSON-Lines record into a per-conversation file under
//! `app_data_dir()/traces/chat/<conversation_id>.jsonl`.
//!
//! Off in shipped builds. Intended for local quality investigation: open
//! a `.jsonl` file and grep/jq across LLM prompts vs. retrieval events vs.
//! user-visible assistant tokens to understand exactly what happened.
//!
//! # Architecture
//!
//! - [`TraceRecorder`] is the trait every callsite calls into. Chat and
//!   websearch code paths both thread `&Arc<dyn TraceRecorder>` through
//!   their execution contexts; no call site distinguishes the live
//!   recorder from the noop.
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
/// chat-domain variants (including websearch events). Files produced by
/// the v1 recorder (one file per search turn, no chat events) and files
/// produced by v2 (one file per conversation under `traces/chat/`) are
/// not interleaved on disk because the v2 layout uses different paths.
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
    /// captures, conversation lifecycle, and the built-in web-search
    /// records (search decision, retrieval, escalation, requery, and
    /// citation audit). One file per conversation, rooted at
    /// `traces/chat/<conversation_id>.jsonl`.
    Chat,
}

impl TraceDomain {
    /// Subdirectory name (under `traces/`) where files for this domain
    /// live. Used by [`FileRecorder::for_conversation`] when assembling
    /// the destination path.
    pub fn dir(self) -> &'static str {
        match self {
            TraceDomain::Chat => "chat",
        }
    }
}

/// Trait every chat or websearch callsite uses to emit forensic events.
/// Implementors must be cheap when tracing is off (the [`NoopRecorder`]
/// case dominates production usage).
pub trait TraceRecorder: Send + Sync {
    /// Records a single event for the given conversation. Implementors
    /// MUST NOT panic, MUST NOT block for arbitrary durations, and MUST
    /// NOT propagate errors. Trace I/O failures must be swallowed (with
    /// a single rate-limited warning) so chat and websearch callsites
    /// are unaffected.
    fn record(&self, conversation_id: &ConversationId, event: RecorderEvent);
}

/// Production default: every method is a no-op. Constant time, zero
/// allocations.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopRecorder;

impl TraceRecorder for NoopRecorder {
    fn record(&self, _conversation_id: &ConversationId, _event: RecorderEvent) {}
}

/// One cited source recorded in a [`RecorderEvent::SearchRetrieved`] event: the
/// URL that grounded the answer and its human-readable title. The title is what
/// makes a vertical's generic homepage URL ("https://www.espn.com/",
/// "https://news.google.com/") legible in the forensic trace: those URLs alone
/// do not say which league or which headline set actually answered the turn, so
/// the title is recorded alongside every URL.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RetrievedSource {
    pub url: String,
    pub title: String,
}

/// One keyless engine's outcome for a single query, recorded in
/// [`RecorderEvent::SearchRetrieved`] for the general scraped-engine tier
/// (`tier == "engine"`). Deliberately lean: name, status, and hit count only,
/// matching the counts-not-payloads discipline the rest of the forensic
/// trace's summary fields already follow. Populated by
/// `crate::websearch::engine::web_search`, one entry per engine actually
/// consulted, so a trace shows a silently-empty or silently-blocked engine
/// even when the overall fused result looks healthy, instead of relying on
/// the stderr `[search]` log alone.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EngineStat {
    pub name: String,
    /// One of "ok", "empty", "blocked", "transport_error", "cache_hit", or
    /// "cooling" -- mirrors the `[search] engine=... <outcome>` stderr log
    /// line emitted alongside it.
    pub status: String,
    pub hit_count: usize,
}

/// One finished pipeline stage's wall time, recorded in
/// [`RecorderEvent::SearchTimings`]. `stage` is a stable snake_case label
/// (e.g. `classifier`, `serp`, `fetch`, `rank_assembly`, `judge`,
/// `writer_ttft`, `pipeline`); `ms` is whole milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StageTiming {
    pub stage: String,
    pub ms: u64,
}

/// Forensic trace events emitted by the chat layer, including built-in
/// websearch.
///
/// Each variant maps to one JSON-Lines record. Field shapes for
/// websearch events (`SearchDecided`, `SearchRetrieved`, etc.) are
/// intentionally permissive (`Value`, `String`) because the trace is a
/// forensic dump, not a typed contract. Lifecycle variants carry typed
/// payloads because the chat layer's surface is small and stable.
///
/// Every variant has a [`Self::domain`] method that returns its
/// [`TraceDomain`], used by the registry to pick the destination file.
#[derive(Debug, Clone)]
pub enum RecorderEvent {
    // ─── Chat domain ────────────────────────────────────────────────────
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
    /// Assistant turn ends. Captures streamed token count, wall-clock latency
    /// from `UserMessage` to stream-completion, and the **final** answer body
    /// after citation repair/`SetContent` (capped; never image bytes). Lets a
    /// trace match what history/UI stored without replaying the stream.
    AssistantComplete {
        total_tokens: u64,
        latency_ms: u64,
        final_content: String,
    },
    /// User invoked the `/screen` slash command and the backend wrote a
    /// JPEG snapshot to `image_path`. `displays` is the number of
    /// monitors captured (multi-monitor setups produce one merged image).
    ScreenCaptured { image_path: String, displays: u8 },
    /// Built-in search never entered the retrieval pipeline for this turn
    /// (gate-level skip). `reason` is a closed string enum:
    /// `non_vision_images` | `transform_slash` | `auto_off` | `engine_unavailable`.
    /// Pipeline decisions that *do* run the classifier still use
    /// [`Self::SearchDecided`] (with `decision` no/cached/web) instead.
    SearchSkipped { reason: String },
    /// Resolved auto-search decision for a chat turn that entered the
    /// orchestrator: pre-filter verdict, search decision (`no`/`cached`/`web`),
    /// whether `/search` forced the path, classifier route, standalone rewrite,
    /// and keyword queries. Emitted once per such turn so a trace shows why a
    /// turn did or did not retrieve and which source tier it aimed at.
    SearchDecided {
        prefilter: String,
        /// `no` | `cached` | `web` after prefilter + classifier resolve.
        decision: String,
        /// True when the `/search` slash command forced engines-only this turn.
        force: bool,
        route: String,
        standalone_question: String,
        queries: Vec<String>,
    },
    /// Which source tier answered a chat turn's auto-search, and the sources
    /// (URL + title) it cited. `tier` is one of "weather", "sports", "news",
    /// "wiki", "engine", or "cache". Emitted once when retrieval produces a
    /// grounded answer.
    ///
    /// `engine_stats` carries the general scraped-engine tier's per-query,
    /// per-engine outcome summary (see [`EngineStat`]): populated only when
    /// `tier == "engine"` (the direct engine-tier answer and the
    /// vertical-insufficient escalation path both set it), empty for every
    /// other tier, since those never race the keyless engines.
    ///
    /// `round` distinguishes the two records a requeried turn produces (see
    /// `crate::websearch::orchestrator::judge_and_requery`): `Some(1)` marks
    /// round one's sources, recorded just before the requery fires so they
    /// stay auditable in the trace even though they are about to be merged
    /// away and superseded. The terminal record for a turn (the only round,
    /// or the post-merge result that follows a round-one record and a
    /// `SearchRequeried` event) always carries `None`, the same shape this
    /// event had before `round` existed. `round` is omitted from the
    /// serialized JSON entirely when `None`, so no existing trace record's
    /// shape changes.
    SearchRetrieved {
        tier: String,
        sources: Vec<RetrievedSource>,
        engine_stats: Vec<EngineStat>,
        round: Option<u8>,
    },
    /// The sufficiency judge's verdict on a vertical's answer, and what the
    /// orchestrator did with it. Emitted once per turn a keyless vertical
    /// answered, so a trace shows why a vertical result was committed or
    /// escalated to the scraped engines. `from_tier` is the vertical judged
    /// ("weather", "sports", "news", "wiki"); `sufficient` is the verdict;
    /// `missing` is the judge's short phrase for what the block lacked (empty
    /// when sufficient); `escalated` is whether the scraped-engine tier was then
    /// run (only when the block was insufficient AND an engine was not cooling);
    /// `escalation_hit` is whether that escalation produced sources that
    /// replaced the vertical block.
    SearchEscalated {
        from_tier: String,
        sufficient: bool,
        missing: String,
        escalated: bool,
        escalation_hit: bool,
    },
    /// Post-generation citation audit for a source-grounded chat turn: a purely
    /// mechanical, zero-model check of how many of the writer's bracket
    /// citations were actually backed by the source text they point at.
    /// `cited` is the total citation references seen; `supported`, `weak`, and
    /// `unsupported` partition them by support strength; `unsupported_indices`
    /// lists the source numbers judged unsupported (including out-of-range
    /// numbers). `numeric_checked`, `numeric_matched`, and `numeric_missing`
    /// report the numeric-consistency guard's own counts (claim money
    /// figures, numbers, and dates checked against the cited source, and how
    /// many were absent), summed across every citation in the turn.
    /// `unverifiable` counts citations whose cited source had too little
    /// fetched text to check anything against (empty, or below
    /// `crate::config::defaults::CITE_UNVERIFIABLE_MIN_SOURCE_BYTES`); these
    /// are never double-counted in `unsupported` and never drive the
    /// answer-facing hedge note. A flagged `unsupported` citation now also
    /// may trigger repair / strip; total failure may surface
    /// `crate::websearch::cite_check::honest_failure_note`.
    CitationAudit {
        cited: usize,
        supported: usize,
        weak: usize,
        unsupported: usize,
        unsupported_indices: Vec<usize>,
        numeric_checked: usize,
        numeric_matched: usize,
        numeric_missing: usize,
        unverifiable: usize,
    },
    /// Final event in a chat-domain file. Emitted by the frontend when
    /// the user resets the conversation or by the backend on app quit
    /// (reason = "quit"). Window-hide does NOT emit this event because
    /// Thuki's window-close intercept hides instead of quits and the
    /// same conversation can resume on next hotkey activation.
    ///
    /// Late events arriving after `ConversationEnd` are tolerated; see
    /// the module-level "Late-event tolerance" doc.
    ConversationEnd { reason: String },
    /// The engine tier's own sufficiency judge (run after it assembles
    /// sources for the standalone question, see
    /// `crate::websearch::orchestrator::judge_and_requery`) found the
    /// result insufficient and fired its one bounded requery. `missing`
    /// is the judge's full, uncapped phrase for the gap; `requery` is the
    /// standalone question with that phrase appended and capped to
    /// `crate::config::defaults::REQUERY_MISSING_MAX_CHARS` at a word
    /// boundary, the exact string searched (so `requery` can carry less of
    /// `missing` than `missing` itself shows). Emitted at most once per turn
    /// (`crate::config::defaults::ENGINE_REQUERY_MAX`); a sufficient
    /// verdict, a judge failure, or an empty `missing` phrase never
    /// emits this event.
    SearchRequeried { missing: String, requery: String },
    /// Per-stage wall times for one built-in search turn (classifier, SERP,
    /// fetch, rank/assembly, judge, writer TTFT, pipeline total, and any
    /// other stages the orchestrator or stream layer recorded). Emitted once
    /// when the search pipeline flushes its [`crate::websearch::stage_timing::TimingBag`]
    /// (and again with `writer_ttft` only if the stream layer records that
    /// stage after the first answer token). Forensic only; not a typed SLA
    /// contract. Empty `stages` is valid but unused.
    SearchTimings { stages: Vec<StageTiming> },
}

impl RecorderEvent {
    /// Returns the [`TraceDomain`] this event belongs to. Used by the
    /// registry to pick the destination file.
    pub fn domain(&self) -> TraceDomain {
        // Every event belongs to the chat domain. The built-in web search
        // (both the `/search` command and the auto-search pre-pass) rides the
        // chat turn, so its decision/retrieval/escalation/requery and
        // citation-audit records are chat-domain events too.
        match self {
            RecorderEvent::ConversationStart { .. }
            | RecorderEvent::UserMessage { .. }
            | RecorderEvent::AssistantThinking { .. }
            | RecorderEvent::AssistantTokens { .. }
            | RecorderEvent::AssistantComplete { .. }
            | RecorderEvent::ScreenCaptured { .. }
            | RecorderEvent::SearchSkipped { .. }
            | RecorderEvent::SearchDecided { .. }
            | RecorderEvent::SearchRetrieved { .. }
            | RecorderEvent::SearchEscalated { .. }
            | RecorderEvent::CitationAudit { .. }
            | RecorderEvent::ConversationEnd { .. }
            | RecorderEvent::SearchRequeried { .. }
            | RecorderEvent::SearchTimings { .. } => TraceDomain::Chat,
        }
    }

    /// Returns true if this event terminates a conversation in the chat
    /// domain. Used by the registry to evict the file handle from its
    /// per-conversation cache.
    pub fn is_conversation_end(&self) -> bool {
        matches!(self, RecorderEvent::ConversationEnd { .. })
    }
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
/// `ts_ms` (wall-clock UNIX millis), `domain` (currently always chat),
/// plus `conversation_id` (the ID passed to `record`). The remaining
/// fields come from the event variant via the `Serialize` impl below.
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
                final_content,
            } => {
                map.serialize_entry("kind", "assistant_complete")?;
                map.serialize_entry("total_tokens", total_tokens)?;
                map.serialize_entry("latency_ms", latency_ms)?;
                map.serialize_entry("final_content", final_content)?;
            }
            RecorderEvent::ScreenCaptured {
                image_path,
                displays,
            } => {
                map.serialize_entry("kind", "screen_captured")?;
                map.serialize_entry("image_path", image_path)?;
                map.serialize_entry("displays", displays)?;
            }
            RecorderEvent::SearchSkipped { reason } => {
                map.serialize_entry("kind", "search_skipped")?;
                map.serialize_entry("reason", reason)?;
            }
            RecorderEvent::SearchDecided {
                prefilter,
                decision,
                force,
                route,
                standalone_question,
                queries,
            } => {
                map.serialize_entry("kind", "search_decided")?;
                map.serialize_entry("prefilter", prefilter)?;
                map.serialize_entry("decision", decision)?;
                map.serialize_entry("force", force)?;
                map.serialize_entry("route", route)?;
                map.serialize_entry("standalone_question", standalone_question)?;
                map.serialize_entry("queries", queries)?;
            }
            RecorderEvent::SearchRetrieved {
                tier,
                sources,
                engine_stats,
                round,
            } => {
                map.serialize_entry("kind", "search_retrieved")?;
                map.serialize_entry("tier", tier)?;
                map.serialize_entry("sources", sources)?;
                map.serialize_entry("engine_stats", engine_stats)?;
                // Omitted (not `null`) when `None`, so a non-requery turn's
                // record is byte-for-byte the same JSON it was before `round`
                // existed (see the variant's rustdoc).
                if let Some(round) = round {
                    map.serialize_entry("round", round)?;
                }
            }
            RecorderEvent::SearchEscalated {
                from_tier,
                sufficient,
                missing,
                escalated,
                escalation_hit,
            } => {
                map.serialize_entry("kind", "search_escalated")?;
                map.serialize_entry("from_tier", from_tier)?;
                map.serialize_entry("sufficient", sufficient)?;
                map.serialize_entry("missing", missing)?;
                map.serialize_entry("escalated", escalated)?;
                map.serialize_entry("escalation_hit", escalation_hit)?;
            }
            RecorderEvent::CitationAudit {
                cited,
                supported,
                weak,
                unsupported,
                unsupported_indices,
                numeric_checked,
                numeric_matched,
                numeric_missing,
                unverifiable,
            } => {
                map.serialize_entry("kind", "citation_audit")?;
                map.serialize_entry("cited", cited)?;
                map.serialize_entry("supported", supported)?;
                map.serialize_entry("weak", weak)?;
                map.serialize_entry("unsupported", unsupported)?;
                map.serialize_entry("unsupported_indices", unsupported_indices)?;
                map.serialize_entry("numeric_checked", numeric_checked)?;
                map.serialize_entry("numeric_matched", numeric_matched)?;
                map.serialize_entry("numeric_missing", numeric_missing)?;
                map.serialize_entry("unverifiable", unverifiable)?;
            }
            RecorderEvent::ConversationEnd { reason } => {
                map.serialize_entry("kind", "conversation_end")?;
                map.serialize_entry("reason", reason)?;
            }
            RecorderEvent::SearchRequeried { missing, requery } => {
                map.serialize_entry("kind", "search_requeried")?;
                map.serialize_entry("missing", missing)?;
                map.serialize_entry("requery", requery)?;
            }
            RecorderEvent::SearchTimings { stages } => {
                map.serialize_entry("kind", "search_timings")?;
                map.serialize_entry("stages", stages)?;
            }
        }
        map.end()
    }
}

/// In-memory recorder used by tests to assert what the chat layer
/// (including websearch) emitted. Lives in a `cfg(test)` block so
/// production builds never link it.
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
    }

    #[test]
    fn trace_domain_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&TraceDomain::Chat).unwrap(),
            "\"chat\""
        );
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
                final_content: "hi".into(),
            },
            RecorderEvent::ScreenCaptured {
                image_path: "p".into(),
                displays: 1,
            },
            RecorderEvent::SearchSkipped {
                reason: "auto_off".into(),
            },
            RecorderEvent::SearchDecided {
                prefilter: "force_web".into(),
                decision: "web".into(),
                force: false,
                route: "news".into(),
                standalone_question: "q".into(),
                queries: vec!["q".into()],
            },
            RecorderEvent::SearchRetrieved {
                tier: "news".into(),
                sources: vec![RetrievedSource {
                    url: "https://news.google.com/".into(),
                    title: "Google News headlines".into(),
                }],
                engine_stats: vec![],
                round: None,
            },
            RecorderEvent::SearchEscalated {
                from_tier: "sports".into(),
                sufficient: false,
                missing: "the full bracket".into(),
                escalated: true,
                escalation_hit: true,
            },
            RecorderEvent::CitationAudit {
                cited: 3,
                supported: 2,
                weak: 0,
                unsupported: 1,
                unsupported_indices: vec![9],
                numeric_checked: 1,
                numeric_matched: 0,
                numeric_missing: 1,
                unverifiable: 0,
            },
            RecorderEvent::ConversationEnd {
                reason: "quit".into(),
            },
            RecorderEvent::SearchRequeried {
                missing: "the treaty terms".into(),
                requery: "when signed the treaty terms".into(),
            },
            RecorderEvent::SearchTimings {
                stages: vec![StageTiming {
                    stage: "classifier".into(),
                    ms: 12,
                }],
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
        assert!(!RecorderEvent::SearchRequeried {
            missing: "x".into(),
            requery: "y".into(),
        }
        .is_conversation_end());
    }

    #[test]
    fn noop_recorder_swallows_every_event() {
        let r = NoopRecorder;
        r.record(
            &cid("conv-x"),
            RecorderEvent::AssistantTokens { chunk: "x".into() },
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
                final_content: "Hi there".into(),
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
            RecorderEvent::SearchSkipped {
                reason: "transform_slash".into(),
            },
        );
        r.record(
            &cid("conv-chat"),
            RecorderEvent::SearchDecided {
                prefilter: "ambiguous".into(),
                decision: "web".into(),
                force: false,
                route: "wiki".into(),
                standalone_question: "what is photosynthesis".into(),
                queries: vec!["photosynthesis".into()],
            },
        );
        r.record(
            &cid("conv-chat"),
            RecorderEvent::SearchRetrieved {
                tier: "wiki".into(),
                sources: vec![RetrievedSource {
                    url: "https://en.wikipedia.org/wiki/Photosynthesis".into(),
                    title: "Photosynthesis".into(),
                }],
                engine_stats: vec![],
                round: None,
            },
        );
        r.record(
            &cid("conv-chat"),
            RecorderEvent::SearchEscalated {
                from_tier: "sports".into(),
                sufficient: false,
                missing: "round-of-32 results".into(),
                escalated: true,
                escalation_hit: false,
            },
        );
        r.record(
            &cid("conv-chat"),
            RecorderEvent::CitationAudit {
                cited: 4,
                supported: 3,
                weak: 0,
                unsupported: 1,
                unsupported_indices: vec![9],
                numeric_checked: 2,
                numeric_matched: 1,
                numeric_missing: 1,
                unverifiable: 1,
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
                "search_skipped",
                "search_decided",
                "search_retrieved",
                "search_escalated",
                "citation_audit",
                "conversation_end"
            ]
        );
        assert_eq!(lines[1]["attached_images"], json!(["/tmp/img.jpg"]));
        assert_eq!(lines[1]["slash_command"], "/screen");
        assert_eq!(lines[4]["final_content"], "Hi there");
        assert_eq!(lines[5]["displays"], 2);
        assert_eq!(lines[6]["reason"], "transform_slash");
        assert_eq!(lines[7]["decision"], "web");
        assert_eq!(lines[7]["force"], false);
        assert_eq!(lines[7]["route"], "wiki");
        assert_eq!(lines[7]["queries"], json!(["photosynthesis"]));
        assert_eq!(lines[8]["tier"], "wiki");
        assert_eq!(
            lines[8]["sources"],
            json!([{"url": "https://en.wikipedia.org/wiki/Photosynthesis", "title": "Photosynthesis"}])
        );
        assert_eq!(lines[9]["from_tier"], "sports");
        assert_eq!(lines[9]["sufficient"], false);
        assert_eq!(lines[9]["missing"], "round-of-32 results");
        assert_eq!(lines[9]["escalated"], true);
        assert_eq!(lines[9]["escalation_hit"], false);
        assert_eq!(lines[10]["cited"], 4);
        assert_eq!(lines[10]["supported"], 3);
        assert_eq!(lines[10]["weak"], 0);
        assert_eq!(lines[10]["unsupported"], 1);
        assert_eq!(lines[10]["unsupported_indices"], json!([9]));
        assert_eq!(lines[10]["numeric_checked"], 2);
        assert_eq!(lines[10]["numeric_matched"], 1);
        assert_eq!(lines[10]["numeric_missing"], 1);
        assert_eq!(lines[10]["unverifiable"], 1);
        assert_eq!(lines[11]["reason"], "quit");
    }

    #[test]
    fn search_retrieved_serializes_engine_stats() {
        // The engine tier's per-query, per-engine outcome summary must reach
        // the JSONL record intact: one silently-blocked engine sitting right
        // next to a healthy one, exactly the case the trace previously had no
        // way to show.
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-engine"));
        r.record(
            &cid("conv-engine"),
            RecorderEvent::SearchRetrieved {
                tier: "engine".into(),
                sources: vec![RetrievedSource {
                    url: "https://example.com/a".into(),
                    title: "Example".into(),
                }],
                engine_stats: vec![
                    EngineStat {
                        name: "duckduckgo".into(),
                        status: "ok".into(),
                        hit_count: 10,
                    },
                    EngineStat {
                        name: "mojeek".into(),
                        status: "blocked".into(),
                        hit_count: 0,
                    },
                ],
                round: None,
            },
        );
        let lines = read_lines(r.path());
        assert_eq!(lines[0]["tier"], "engine");
        assert_eq!(
            lines[0]["engine_stats"],
            json!([
                {"name": "duckduckgo", "status": "ok", "hit_count": 10},
                {"name": "mojeek", "status": "blocked", "hit_count": 0},
            ])
        );
    }

    #[test]
    fn search_retrieved_round_one_serializes_the_round_field() {
        // A round-one pre-requery record must carry an explicit `round: 1` so
        // a trace consumer can tell it apart from the turn's terminal
        // (post-requery) record.
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-round1"));
        r.record(
            &cid("conv-round1"),
            RecorderEvent::SearchRetrieved {
                tier: "engine".into(),
                sources: vec![RetrievedSource {
                    url: "https://round-one.example/".into(),
                    title: "Round One".into(),
                }],
                engine_stats: vec![],
                round: Some(1),
            },
        );
        let lines = read_lines(r.path());
        assert_eq!(lines[0]["round"], json!(1));
    }

    #[test]
    fn search_retrieved_final_omits_the_round_key_entirely() {
        // A terminal (non-round-tagged) record must not carry a `round` key
        // at all, not even a `null` one: the exact same JSON shape this event
        // had before the field existed, so no existing trace consumer's
        // schema check needs updating.
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-final"));
        r.record(
            &cid("conv-final"),
            RecorderEvent::SearchRetrieved {
                tier: "engine".into(),
                sources: vec![],
                engine_stats: vec![],
                round: None,
            },
        );
        let lines = read_lines(r.path());
        assert!(lines[0].as_object().unwrap().get("round").is_none());
    }

    #[test]
    fn search_retrieved_engine_stats_empty_for_non_engine_tier() {
        // Verticals and the cache tier never race the keyless engines, so
        // their SearchRetrieved records must carry an empty engine_stats,
        // not a stale or default-filled list.
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-wiki"));
        r.record(
            &cid("conv-wiki"),
            RecorderEvent::SearchRetrieved {
                tier: "wiki".into(),
                sources: vec![],
                engine_stats: vec![],
                round: None,
            },
        );
        let lines = read_lines(r.path());
        assert_eq!(lines[0]["engine_stats"], json!([]));
    }

    #[test]
    fn file_recorder_serializes_search_requeried() {
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-requery"));
        r.record(
            &cid("conv-requery"),
            RecorderEvent::SearchRequeried {
                missing: "the treaty terms".into(),
                requery: "when signed the treaty terms".into(),
            },
        );
        let lines = read_lines(r.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["kind"], "search_requeried");
        assert_eq!(lines[0]["missing"], "the treaty terms");
        assert_eq!(lines[0]["requery"], "when signed the treaty terms");
        assert_eq!(lines[0]["domain"], "chat");
    }

    #[test]
    fn file_recorder_path_includes_domain_subfolder_and_conv_id_jsonl() {
        let root = fresh_dir();
        let r = FileRecorder::for_conversation(&root, TraceDomain::Chat, &cid("conv-x"));
        assert_eq!(r.path(), root.join("chat").join("conv-x.jsonl"));
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
            RecorderEvent::AssistantTokens { chunk: "k".into() },
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
            RecorderEvent::AssistantTokens { chunk: "k".into() },
        );
        assert!(
            r.failed.load(Ordering::SeqCst),
            "open failure must latch the recorder"
        );
        // Second call exits at the latch check, exercising that branch.
        r.record(
            &cid("conv-fail"),
            RecorderEvent::AssistantTokens { chunk: "k2".into() },
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
            RecorderEvent::AssistantTokens { chunk: "k".into() },
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
            RecorderEvent::AssistantTokens { chunk: "k".into() },
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
            RecorderEvent::AssistantTokens { chunk: "a".into() },
        );
        m.record(
            &cid("conv-b"),
            RecorderEvent::AssistantTokens { chunk: "b".into() },
        );
        assert_eq!(m.len(), 2);
        let snap = m.snapshot();
        assert_eq!(snap[0].0, cid("conv-a"));
        assert_eq!(snap[1].0, cid("conv-b"));
        let dump: Vec<serde_json::Value> = snap
            .iter()
            .map(|(_, e)| serde_json::to_value(e).unwrap())
            .collect();
        assert_eq!(dump[0]["chunk"], "a");
        assert_eq!(dump[1]["chunk"], "b");
    }
}
