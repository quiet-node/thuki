//! Identity types used by the unified trace recorder.
//!
//! `ConversationId` is a newtype around `String` so the type system
//! distinguishes a conversation id from a turn id at every API boundary
//! that touches both. The chat layer is the only producer; SQLite stores
//! the same value for the user-facing conversation history UI.
//!
//! `new_turn_id()` is the canonical search-pipeline turn-id generator,
//! kept here so both the chat-domain and search-domain code paths source
//! their identifiers from a single module.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Stable identifier for a single user-facing conversation. The frontend
/// generates the value (matching the SQLite schema's `conversation_id`
/// column) and threads it through every Tauri command that records trace
/// events for the conversation.
///
/// Newtype over `String` so a function expecting a conversation id cannot
/// silently accept a turn id, message id, or arbitrary user input. The
/// conversion to/from `String` is explicit at every boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConversationId(String);

impl ConversationId {
    /// Constructs a `ConversationId` from any owned-or-borrowed string.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Returns the underlying string slice. Useful for logging, display,
    /// and the rare callsite that needs to feed the id back into a Tauri
    /// IPC call without taking ownership.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConversationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ConversationId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ConversationId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Builds a fresh, sortable, collision-resistant turn id for the search
/// pipeline.
///
/// Format: `<unix_secs>-<uuid_v4>`. Seconds prefix is eyeball-readable when
/// browsing the traces directory; the v4 UUID guarantees uniqueness across
/// concurrent turns within the same second.
pub fn new_turn_id() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}-{}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_id_roundtrips_through_string() {
        let id = ConversationId::new("conv-abc123");
        assert_eq!(id.as_str(), "conv-abc123");
        assert_eq!(id.to_string(), "conv-abc123");
    }

    #[test]
    fn conversation_id_from_string_matches_from_str() {
        let a = ConversationId::from("conv-x".to_string());
        let b = ConversationId::from("conv-x");
        assert_eq!(a, b);
    }

    #[test]
    fn conversation_id_serializes_as_bare_string() {
        let id = ConversationId::new("conv-y");
        let json = serde_json::to_string(&id).unwrap();
        // `serde(transparent)` keeps the wire shape as a bare string,
        // so consumers reading JSONL never see a `{ "0": "..." }` wrapper.
        assert_eq!(json, "\"conv-y\"");
    }

    #[test]
    fn conversation_id_deserializes_from_bare_string() {
        let id: ConversationId = serde_json::from_str("\"conv-z\"").unwrap();
        assert_eq!(id.as_str(), "conv-z");
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
}
