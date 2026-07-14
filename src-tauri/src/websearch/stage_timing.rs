//! Per-stage wall-clock timing for one built-in search turn.
//!
//! Captures classifier, SERP, page fetch, rank/assembly, judge, and related
//! stage durations without holding any lock across an await: each record takes
//! a start [`std::time::Instant`] that was snapped before the stage, then
//! stores only the finished `ms` under a short-held mutex.
//!
//! Emits:
//! - stderr lines: `[search] timing stage=<name> ms=<n>`
//! - one [`crate::trace::RecorderEvent::SearchTimings`] when [`TimingBag::flush`]
//!   runs (typically once the pipeline has produced an outcome).
//!
//! Writer TTFT (submit → first user-visible answer token) is recorded by the
//! chat layer once streaming begins; this module owns the pure helpers and the
//! in-pipeline stages the orchestrator can measure without the stream.

use std::sync::Mutex;
use std::time::Instant;

use crate::trace::{BoundRecorder, RecorderEvent, StageTiming};

/// Stable stage name for the classifier / pre-pass LLM call.
pub const STAGE_CLASSIFIER: &str = "classifier";
/// Stable stage name for keyless SERP races (all queries this tier).
pub const STAGE_SERP: &str = "serp";
/// Stable stage name for concurrent page fetch + extract.
pub const STAGE_FETCH: &str = "fetch";
/// Stable stage name for BM25 rank + context assembly.
pub const STAGE_RANK_ASSEMBLY: &str = "rank_assembly";
/// Stable stage name for the sufficiency-judge LLM call.
pub const STAGE_JUDGE: &str = "judge";
/// Stable stage name for writer prompt assembly (messages ready to stream).
pub const STAGE_WRITER_PREPARE: &str = "writer_prepare";
/// Stable stage name for submit → first writer answer token (stream TTFT).
pub const STAGE_WRITER_TTFT: &str = "writer_ttft";
/// Stable stage name for end-to-end pipeline wall time (submit → outcome).
pub const STAGE_PIPELINE: &str = "pipeline";
/// Stable stage name for a ForceWeb raw-query race SERP (when enabled).
pub const STAGE_RAW_RACE_SERP: &str = "raw_race_serp";

/// Formats one stderr timing line. Pure so tests assert the wire format without
/// capturing process stderr.
pub fn format_timing_line(stage: &str, ms: u64) -> String {
    format!("[search] timing stage={stage} ms={ms}")
}

/// Writes one timing line to stderr. Thin I/O over [`format_timing_line`].
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn emit_timing_stderr(stage: &str, ms: u64) {
    eprintln!("{}", format_timing_line(stage, ms));
}

/// Elapsed whole milliseconds from `start` to now, saturating at `u64::MAX`.
pub fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Whether two search queries are near-duplicates for ForceWeb race reuse.
///
/// Normalizes by lowercasing, trimming, collapsing interior whitespace, then
/// compares exact equality. Used so a raced raw-query SERP is kept when the
/// classifier rewrite is effectively the same string (common path stays one
/// DDG request).
pub fn queries_near_duplicate(a: &str, b: &str) -> bool {
    normalize_query_key(a) == normalize_query_key(b) && !normalize_query_key(a).is_empty()
}

/// Lowercases, trims, and collapses whitespace to a stable compare key.
fn normalize_query_key(q: &str) -> String {
    q.split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Thread-safe stage timing bag for one search turn.
///
/// Snap `Instant::now()` before each awaited stage; call [`Self::record`] after
/// the stage completes (no lock held across the await). Flush once at the end.
#[derive(Debug)]
pub struct TimingBag {
    /// Wall clock at bag construction (submit / pipeline start).
    submit: Instant,
    /// Finished stages in insertion order. Mutex only guards the short push.
    stages: Mutex<Vec<StageTiming>>,
}

impl TimingBag {
    /// Starts a bag at the current instant (search submit).
    pub fn new() -> Self {
        Self {
            submit: Instant::now(),
            stages: Mutex::new(Vec::new()),
        }
    }

    /// Starts a bag at a caller-supplied instant (injectable clock for tests).
    pub fn starting_at(submit: Instant) -> Self {
        Self {
            submit,
            stages: Mutex::new(Vec::new()),
        }
    }

    /// Instant this bag treats as submit / pipeline start.
    pub fn submit_instant(&self) -> Instant {
        self.submit
    }

    /// Records `stage` with wall time since `start`, emits stderr, stores the
    /// sample. Lock is taken only for the push (never across an await).
    pub fn record(&self, stage: &str, start: Instant) {
        self.record_ms(stage, elapsed_ms(start));
    }

    /// Records an already-measured `ms` for `stage` (tests + stream TTFT).
    pub fn record_ms(&self, stage: &str, ms: u64) {
        emit_timing_stderr(stage, ms);
        // Short critical section: clone stage name, push, drop. Never await while held.
        // Mutex poison is a programmer bug (panic while holding); never expected.
        let mut guard = self.stages.lock().expect("timing bag mutex");
        // Cap stage list so a buggy caller cannot grow unboundedly (DoS).
        const MAX_STAGES: usize = 32;
        if guard.len() >= MAX_STAGES {
            return;
        }
        guard.push(StageTiming {
            stage: stage.to_string(),
            ms,
        });
    }

    /// Snapshot of recorded stages (for unit tests).
    pub fn snapshot(&self) -> Vec<StageTiming> {
        self.stages.lock().expect("timing bag mutex").clone()
    }

    /// Adds pipeline total from submit, then emits [`RecorderEvent::SearchTimings`].
    pub fn flush(&self, recorder: &BoundRecorder) {
        let pipeline_ms = elapsed_ms(self.submit);
        // Only append pipeline if not already present (idempotent re-flush safe).
        let already = self.snapshot().iter().any(|s| s.stage == STAGE_PIPELINE);
        if !already {
            self.record_ms(STAGE_PIPELINE, pipeline_ms);
        }
        let stages = self.snapshot();
        recorder.record(RecorderEvent::SearchTimings { stages });
    }
}

impl Default for TimingBag {
    /// Same as [`TimingBag::new`].
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::recorder::MockRecorder;
    use crate::trace::{BoundRecorder, ConversationId, RecorderEvent};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn format_timing_line_matches_contract() {
        assert_eq!(
            format_timing_line(STAGE_CLASSIFIER, 42),
            "[search] timing stage=classifier ms=42"
        );
        assert_eq!(
            format_timing_line(STAGE_WRITER_TTFT, 0),
            "[search] timing stage=writer_ttft ms=0"
        );
    }

    #[test]
    fn elapsed_ms_non_negative() {
        let start = Instant::now();
        let ms = elapsed_ms(start);
        // Instant::now is monotonic; elapsed is always >= 0 and tiny here.
        assert!(ms < 60_000);
    }

    #[test]
    fn queries_near_duplicate_normalizes_case_and_space() {
        assert!(queries_near_duplicate(
            "  Latest  Figma  ownership ",
            "latest figma ownership"
        ));
        assert!(!queries_near_duplicate(
            "figma ownership",
            "adobe figma deal"
        ));
        assert!(!queries_near_duplicate("", ""));
        assert!(!queries_near_duplicate("   ", "\t"));
    }

    #[test]
    fn record_and_snapshot_preserve_order() {
        let bag = TimingBag::new();
        let t0 = Instant::now();
        bag.record_ms(STAGE_CLASSIFIER, 10);
        bag.record(STAGE_SERP, t0);
        let snap = bag.snapshot();
        assert_eq!(snap[0].stage, STAGE_CLASSIFIER);
        assert_eq!(snap[0].ms, 10);
        assert_eq!(snap[1].stage, STAGE_SERP);
    }

    #[test]
    fn flush_emits_search_timings_with_pipeline() {
        let mock = Arc::new(MockRecorder::default());
        let bound = BoundRecorder::new(mock.clone(), ConversationId::new("t"));
        let bag = TimingBag::starting_at(Instant::now());
        bag.record_ms(STAGE_CLASSIFIER, 100);
        bag.record_ms(STAGE_JUDGE, 50);
        bag.flush(&bound);
        let events = mock.snapshot();
        assert_eq!(events.len(), 1);
        let ok = matches!(
            &events[0].1,
            RecorderEvent::SearchTimings { stages }
                if stages.iter().any(|s| s.stage == STAGE_CLASSIFIER && s.ms == 100)
                    && stages.iter().any(|s| s.stage == STAGE_JUDGE && s.ms == 50)
                    && stages.iter().any(|s| s.stage == STAGE_PIPELINE)
        );
        assert!(ok);
    }

    #[test]
    fn default_constructs_empty_bag() {
        let bag = TimingBag::default();
        assert!(bag.snapshot().is_empty());
    }

    #[test]
    fn stage_list_is_capped() {
        let bag = TimingBag::new();
        for i in 0..40 {
            bag.record_ms(&format!("s{i}"), i as u64);
        }
        assert_eq!(bag.snapshot().len(), 32);
    }

    #[test]
    fn writer_ttft_helper_uses_submit_instant() {
        let bag = TimingBag::starting_at(Instant::now() - Duration::from_millis(25));
        let ttft = elapsed_ms(bag.submit_instant());
        assert!(ttft >= 20);
        bag.record_ms(STAGE_WRITER_TTFT, ttft);
        assert_eq!(bag.snapshot().last().unwrap().stage, STAGE_WRITER_TTFT);
    }
}
