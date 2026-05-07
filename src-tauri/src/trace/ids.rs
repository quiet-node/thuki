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
// `into` + `from` make every (de)serialization detour through
// `From<String>` / `From<ConversationId> for String`, which forces
// sanitization on the deserialize path. Wire shape remains a bare string,
// matching the prior `serde(transparent)` contract.
#[serde(into = "String", from = "String")]
pub struct ConversationId(String);

/// Sentinel returned by [`ConversationId::new`] when sanitization strips the
/// input down to nothing (empty IPC payload, all-`/` traversal, etc.). All
/// such inputs collide on the same on-disk file, which is the safe outcome:
/// no traversal, no info leak, attacker-controlled input still gets a stable
/// per-process bucket.
pub const SANITIZED_FALLBACK: &str = "invalid-conversation-id";

/// Replaces every path-traversal vector with a safe character. Defense in
/// depth: production callers route `crypto.randomUUID()` through this
/// constructor, but the IPC boundary accepts arbitrary strings, so the
/// recorder must never let an attacker steer the path that `FileRecorder`
/// joins onto `app_data_dir/traces/{chat,search}/`.
fn sanitize(raw: String) -> String {
    // Step 1: collapse the path separators and NUL into a benign filler.
    let separator_safe: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            c => c,
        })
        .collect();
    // Step 2: rewrite `..` so `Path::join` cannot climb out of the trace
    // directory even if the caller stitched `..` segments around the
    // already-replaced separators.
    let traversal_safe = separator_safe.replace("..", "__");
    if traversal_safe.is_empty() {
        SANITIZED_FALLBACK.to_string()
    } else {
        traversal_safe
    }
}

impl ConversationId {
    /// Constructs a `ConversationId`, sanitizing the input so it can be
    /// joined onto a filesystem path without escaping the trace directory.
    /// See [`sanitize`] for the exact rewrite rules.
    pub fn new(s: impl Into<String>) -> Self {
        Self(sanitize(s.into()))
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
        Self::new(s)
    }
}

impl From<&str> for ConversationId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<ConversationId> for String {
    fn from(id: ConversationId) -> String {
        id.0
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
        // `serde(into = "String")` keeps the wire shape as a bare string,
        // so consumers reading JSONL never see a `{ "0": "..." }` wrapper.
        assert_eq!(json, "\"conv-y\"");
    }

    #[test]
    fn conversation_id_deserializes_from_bare_string() {
        let id: ConversationId = serde_json::from_str("\"conv-z\"").unwrap();
        assert_eq!(id.as_str(), "conv-z");
    }

    #[test]
    fn sanitize_strips_forward_slash_path_traversal() {
        // Joining `../etc/passwd` onto `traces/chat/` would resolve to
        // `traces/etc/passwd`, escaping the trace directory. Sanitize
        // collapses every separator into `_` so the join stays inside.
        let id = ConversationId::new("../etc/passwd");
        assert!(!id.as_str().contains('/'), "id leaked separator: {id}");
        assert!(!id.as_str().contains(".."), "id leaked traversal: {id}");
    }

    #[test]
    fn sanitize_strips_backslash_traversal() {
        let id = ConversationId::new("..\\..\\Windows\\System32");
        assert!(!id.as_str().contains('\\'));
        assert!(!id.as_str().contains(".."));
    }

    #[test]
    fn sanitize_strips_nul_byte() {
        let id = ConversationId::new("conv\0evil");
        assert!(!id.as_str().contains('\0'));
    }

    #[test]
    fn sanitize_collapses_chained_dot_dot() {
        // `....` must collapse fully so no `..` segment survives a single
        // sweep. `str::replace` is non-overlapping so we verify the output.
        let id = ConversationId::new("....");
        assert_eq!(id.as_str(), "____");
    }

    #[test]
    fn sanitize_empty_input_falls_back_to_sentinel() {
        let id = ConversationId::new("");
        assert_eq!(id.as_str(), SANITIZED_FALLBACK);
    }

    #[test]
    fn sanitize_preserves_uuid_shape() {
        // Production input is `crypto.randomUUID()`; this must round-trip
        // bit-for-bit so trace files keep their canonical filenames.
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let id = ConversationId::new(uuid);
        assert_eq!(id.as_str(), uuid);
    }

    #[test]
    fn from_str_and_from_string_route_through_sanitize() {
        let a = ConversationId::from("../evil".to_string());
        let b = ConversationId::from("../evil");
        assert_eq!(a, b);
        assert!(!a.as_str().contains(".."));
        assert!(!a.as_str().contains('/'));
    }

    #[test]
    fn deserialize_routes_through_sanitize() {
        // A trace file whose `conversation_id` somehow contained a
        // traversal payload must NOT be able to weaponize a re-read.
        let id: ConversationId = serde_json::from_str("\"../etc\"").unwrap();
        assert!(!id.as_str().contains(".."));
        assert!(!id.as_str().contains('/'));
    }

    #[test]
    fn into_string_returns_sanitized_payload() {
        // `serde(into = "String")` relies on this conversion; the
        // returned string must equal the in-memory storage.
        let id = ConversationId::new("../foo");
        let s: String = id.clone().into();
        assert_eq!(s, id.as_str());
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
