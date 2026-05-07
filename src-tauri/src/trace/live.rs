//! Hot-swappable trace recorder. The single managed-state instance
//! whose internal recorder can flip between `RegistryRecorder` and
//! `NoopRecorder` at runtime when the user toggles
//! `[debug] trace_enabled` from the Settings panel.
//!
//! # Why a wrapper
//!
//! `Arc<dyn TraceRecorder>` is locked in once at startup: the chat
//! command, the search pipeline, and the screenshot command all hold
//! their own `Arc` clones of the initial managed state, and Tauri's
//! managed-state surface has no way to swap a value out from under
//! existing State<'_> handles. `LiveTraceRecorder` solves this by
//! being the trait implementation itself; it forwards every
//! `record()` call to whatever inner recorder is currently installed.
//! Swap = write-lock + replace.
//!
//! # Concurrency
//!
//! `record()` takes a read-lock just long enough to clone the inner
//! `Arc`, then drops the lock before forwarding the call. This keeps
//! the read path lock-free in practice (the lock is uncontended for
//! the duration of an `Arc::clone`) and means an in-flight record
//! cannot deadlock against an in-flight swap. Streaming tasks that
//! cached an `Arc<BoundRecorder>` whose inner is this `LiveTraceRecorder`
//! pick up the new behaviour on the next `record()` call after the
//! swap.

use std::sync::Arc;

use parking_lot::RwLock;

use super::ids::ConversationId;
use super::recorder::{NoopRecorder, RecorderEvent, TraceRecorder};

/// Trace recorder whose backing implementation can be replaced at
/// runtime. Installed as Tauri managed state in `lib.rs::run()`; swapped
/// from `settings_commands::set_config_field` when the user flips
/// `[debug] trace_enabled` in the Settings panel.
///
/// Manual `Debug` impl: `dyn TraceRecorder` does not require `Debug`,
/// so the inner recorder is rendered as an opaque marker (mirrors the
/// pattern in `BoundRecorder`).
pub struct LiveTraceRecorder {
    inner: RwLock<Arc<dyn TraceRecorder>>,
}

impl std::fmt::Debug for LiveTraceRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveTraceRecorder")
            .field("inner", &"<dyn TraceRecorder>")
            .finish()
    }
}

impl LiveTraceRecorder {
    /// Wraps `initial` as the starting recorder. Production sites pass
    /// either a `RegistryRecorder` (when `trace_enabled = true` at
    /// startup) or a `NoopRecorder` (the default).
    pub fn new(initial: Arc<dyn TraceRecorder>) -> Self {
        Self {
            inner: RwLock::new(initial),
        }
    }

    /// Convenience constructor used at app startup when tracing is off.
    /// Equivalent to `LiveTraceRecorder::new(Arc::new(NoopRecorder))`.
    pub fn noop() -> Self {
        Self::new(Arc::new(NoopRecorder))
    }

    /// Replaces the inner recorder. Subsequent `record()` calls go to
    /// `new_inner`; in-flight records that had already cloned the prior
    /// inner finish writing through it (Arc semantics keep the previous
    /// recorder alive until the last clone drops).
    pub fn replace(&self, new_inner: Arc<dyn TraceRecorder>) {
        *self.inner.write() = new_inner;
    }
}

impl TraceRecorder for LiveTraceRecorder {
    fn record(&self, conversation_id: &ConversationId, event: RecorderEvent) {
        // Clone the Arc under read-lock, then drop the lock before
        // forwarding the call. Keeps swap and record from blocking
        // each other for the duration of the actual file I/O.
        let inner = {
            let guard = self.inner.read();
            Arc::clone(&*guard)
        };
        inner.record(conversation_id, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use serde_json::json;

    /// Test-local recorder that counts the events it sees, tagged with
    /// a label so the swap test can tell which inner was active for
    /// each emit.
    #[derive(Debug, Default)]
    struct LabeledCounter {
        label: &'static str,
        seen: Mutex<Vec<(String, &'static str)>>,
    }
    impl LabeledCounter {
        fn new(label: &'static str) -> Self {
            Self {
                label,
                seen: Mutex::new(Vec::new()),
            }
        }
    }
    impl TraceRecorder for LabeledCounter {
        fn record(&self, conversation_id: &ConversationId, _event: RecorderEvent) {
            self.seen
                .lock()
                .push((conversation_id.as_str().to_owned(), self.label));
        }
    }

    fn cid(s: &str) -> ConversationId {
        ConversationId::new(s)
    }

    fn warning() -> RecorderEvent {
        RecorderEvent::Warning {
            kind: "k".into(),
            payload: json!({}),
        }
    }

    #[test]
    fn record_routes_to_initial_inner() {
        let counter = Arc::new(LabeledCounter::new("initial"));
        let live = LiveTraceRecorder::new(counter.clone());
        live.record(&cid("conv-a"), warning());
        let seen = counter.seen.lock().clone();
        assert_eq!(seen, vec![("conv-a".to_owned(), "initial")]);
    }

    #[test]
    fn replace_swaps_inner_and_subsequent_records_use_the_new_recorder() {
        let first = Arc::new(LabeledCounter::new("before"));
        let second = Arc::new(LabeledCounter::new("after"));
        let live = LiveTraceRecorder::new(first.clone());
        live.record(&cid("conv-1"), warning());
        live.replace(second.clone());
        live.record(&cid("conv-1"), warning());
        live.record(&cid("conv-2"), warning());
        assert_eq!(
            first.seen.lock().clone(),
            vec![("conv-1".to_owned(), "before")],
            "pre-swap event must route to the original inner only"
        );
        assert_eq!(
            second.seen.lock().clone(),
            vec![
                ("conv-1".to_owned(), "after"),
                ("conv-2".to_owned(), "after"),
            ],
            "post-swap events must route to the new inner only"
        );
    }

    #[test]
    fn replace_can_be_invoked_multiple_times() {
        let a = Arc::new(LabeledCounter::new("a"));
        let b = Arc::new(LabeledCounter::new("b"));
        let c = Arc::new(LabeledCounter::new("c"));
        let live = LiveTraceRecorder::new(a.clone());
        live.record(&cid("x"), warning());
        live.replace(b.clone());
        live.record(&cid("x"), warning());
        live.replace(c.clone());
        live.record(&cid("x"), warning());
        live.replace(a.clone());
        live.record(&cid("x"), warning());
        assert_eq!(a.seen.lock().len(), 2, "first + final emits land on a");
        assert_eq!(b.seen.lock().len(), 1, "single emit lands on b");
        assert_eq!(c.seen.lock().len(), 1, "single emit lands on c");
    }

    #[test]
    fn noop_constructor_swallows_events_until_replaced() {
        let live = LiveTraceRecorder::noop();
        // No panic, no observable side effect.
        live.record(&cid("conv-noop"), warning());

        let counter = Arc::new(LabeledCounter::new("active"));
        live.replace(counter.clone());
        live.record(&cid("conv-noop"), warning());
        assert_eq!(
            counter.seen.lock().clone(),
            vec![("conv-noop".to_owned(), "active")],
            "post-replace events route to the new recorder; pre-replace was a true noop"
        );
    }

    #[test]
    fn live_recorder_implements_trace_recorder_trait_object() {
        // Compiles only if `LiveTraceRecorder: TraceRecorder`. Exists
        // so a future refactor cannot accidentally break the trait
        // impl that lib.rs depends on for `Arc<dyn TraceRecorder>`
        // coercion.
        let live: Arc<dyn TraceRecorder> = Arc::new(LiveTraceRecorder::noop());
        live.record(&cid("trait-obj"), warning());
    }

    #[test]
    fn debug_impl_renders_opaque_inner() {
        // The manual Debug impl swaps the trait-object inner for an
        // opaque marker so debug output stays diagnostic-friendly
        // without leaking the recorder's internal state. Mirrors the
        // pattern in `BoundRecorder::Debug`.
        let live = LiveTraceRecorder::noop();
        let rendered = format!("{live:?}");
        assert!(
            rendered.contains("LiveTraceRecorder"),
            "debug output must label the type: {rendered}"
        );
        assert!(
            rendered.contains("<dyn TraceRecorder>"),
            "debug output must use the opaque inner marker: {rendered}"
        );
    }
}
