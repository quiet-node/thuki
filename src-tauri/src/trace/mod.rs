//! Unified forensic trace recorder for the chat layer and the `/search`
//! pipeline.
//!
//! Three submodules:
//! - [`ids`]: `ConversationId` newtype and `new_turn_id` generator.
//! - [`recorder`]: `TraceRecorder` trait, `RecorderEvent` enum (chat +
//!   search variants), `TraceDomain`, `FileRecorder`, `NoopRecorder`.
//! - [`registry`]: `RegistryRecorder` production composition that owns
//!   one `FileRecorder` per `(domain, conversation_id)` pair.
//!
//! [`BoundRecorder`] is a thin wrapper that closes over a
//! `ConversationId` so call sites can emit events with a single-arg
//! `record(event)` instead of threading the id through every signature.
//! The chat layer and the search pipeline both hold an
//! `Arc<BoundRecorder>` for the conversation they belong to.
//!
//! See `recorder.rs` module-level docs for the JSONL schema and the
//! late-event tolerance contract.

use std::sync::Arc;

pub mod ids;
pub mod live;
pub mod recorder;
pub mod registry;

pub use ids::{new_turn_id, ConversationId};
pub use live::LiveTraceRecorder;
pub use recorder::{
    FileRecorder, NoopRecorder, ReaderUrlOutcome, RecorderEvent, RerankedChunk, TraceDomain,
    TraceRecorder, TRACE_SCHEMA_VERSION,
};
pub use registry::RegistryRecorder;

/// Recorder bound to a single `ConversationId`. Wraps an
/// `Arc<dyn TraceRecorder>` so call sites can emit events without
/// threading the conversation id through every function signature.
///
/// Constructed by `commands::ask_ollama` and `search::search_pipeline`
/// once at the start of each turn from managed state, then handed down
/// through the streaming or pipeline machinery as
/// `Arc<BoundRecorder>`. Cheap to clone (single `Arc`).
///
/// Manual `Debug` impl: `dyn TraceRecorder` does not require `Debug`
/// (and the noop / file recorder don't all implement it through a
/// trait-object boundary), so the inner recorder is rendered as an
/// opaque marker in debug output.
pub struct BoundRecorder {
    inner: Arc<dyn TraceRecorder>,
    conversation_id: ConversationId,
}

impl std::fmt::Debug for BoundRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundRecorder")
            .field("inner", &"<dyn TraceRecorder>")
            .field("conversation_id", &self.conversation_id)
            .finish()
    }
}

impl BoundRecorder {
    /// Wraps `inner` and binds it to `conversation_id`. Every
    /// subsequent `record(event)` call routes through `inner` with
    /// the bound id.
    pub fn new(inner: Arc<dyn TraceRecorder>, conversation_id: ConversationId) -> Self {
        Self {
            inner,
            conversation_id,
        }
    }

    /// Convenience constructor for tests and for the production path
    /// where `[debug] trace_enabled` is false: builds a `BoundRecorder`
    /// backed by a `NoopRecorder` so the rest of the code can hold an
    /// `Arc<BoundRecorder>` unconditionally.
    pub fn noop_for(conversation_id: ConversationId) -> Self {
        Self::new(Arc::new(NoopRecorder), conversation_id)
    }

    /// Records a single event for the bound conversation.
    pub fn record(&self, event: RecorderEvent) {
        self.inner.record(&self.conversation_id, event);
    }

    /// The conversation id this recorder was bound to. Useful for
    /// logging and for emitting downstream events that need to share
    /// the same id.
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn noop_for_constructs_bound_recorder_with_id() {
        let r = BoundRecorder::noop_for(ConversationId::new("conv-noop"));
        assert_eq!(r.conversation_id().as_str(), "conv-noop");
        // No panic, no I/O.
        r.record(RecorderEvent::Warning {
            kind: "k".into(),
            payload: json!({}),
        });
    }

    #[test]
    fn debug_impl_renders_opaque_inner_and_visible_conv_id() {
        // The manual Debug impl swaps the trait-object inner for an
        // opaque marker so the conversation id stays diagnostic-friendly
        // without leaking the recorder's internal fields. Exercises the
        // impl path that derive(Debug) cannot synthesize for the
        // dyn-trait field.
        let r = BoundRecorder::noop_for(ConversationId::new("conv-debug"));
        let rendered = format!("{r:?}");
        assert!(
            rendered.contains("BoundRecorder"),
            "debug output must label the type: {rendered}"
        );
        assert!(
            rendered.contains("<dyn TraceRecorder>"),
            "debug output must use the opaque inner marker: {rendered}"
        );
        assert!(
            rendered.contains("conv-debug"),
            "debug output must show the bound conversation id: {rendered}"
        );
    }

    #[test]
    fn bound_record_threads_conv_id_to_inner_recorder() {
        // Use a `MockRecorder` indirection by way of a small captor type.
        // The point is: the inner recorder receives the bound conv_id
        // even though the BoundRecorder caller only passed an event.
        struct Captor {
            seen: parking_lot::Mutex<Vec<(String, String)>>,
        }
        impl TraceRecorder for Captor {
            fn record(&self, conversation_id: &ConversationId, event: RecorderEvent) {
                let kind = match &event {
                    RecorderEvent::Warning { kind, .. } => kind.clone(),
                    _ => "other".into(),
                };
                self.seen
                    .lock()
                    .push((conversation_id.as_str().to_owned(), kind));
            }
        }

        let captor = Arc::new(Captor {
            seen: parking_lot::Mutex::new(Vec::new()),
        });
        let bound = BoundRecorder::new(captor.clone(), ConversationId::new("conv-thread"));
        bound.record(RecorderEvent::Warning {
            kind: "alpha".into(),
            payload: json!({}),
        });
        bound.record(RecorderEvent::Warning {
            kind: "beta".into(),
            payload: json!({}),
        });
        // Non-Warning event exercises the captor's `_ => "other"` fallback
        // arm so coverage hits both match arms in the test instrumentation.
        bound.record(RecorderEvent::AssistantTokens { chunk: "hi".into() });
        let seen = captor.seen.lock().clone();
        assert_eq!(
            seen,
            vec![
                ("conv-thread".to_owned(), "alpha".to_owned()),
                ("conv-thread".to_owned(), "beta".to_owned()),
                ("conv-thread".to_owned(), "other".to_owned()),
            ]
        );
    }
}
