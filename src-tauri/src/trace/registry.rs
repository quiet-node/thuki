//! Multi-conversation registry that fans events out to per-conversation
//! `FileRecorder` instances.
//!
//! `RegistryRecorder` is the production composition installed in Tauri
//! managed state when [`crate::config::schema::DebugSection::trace_enabled`]
//! is true. It owns one `Arc<FileRecorder>` per `(TraceDomain,
//! ConversationId)` pair and routes every incoming event to the right
//! file based on the event's `domain()` and the `conversation_id`
//! passed to `record()`.
//!
//! # Concurrency
//!
//! The registry uses a `parking_lot::RwLock<HashMap>` so the fast path
//! (the recorder for this conversation already exists) is a cheap
//! read-lock + clone of an `Arc<FileRecorder>`. The slow path
//! (first event for a `(domain, conv_id)` pair) takes the write-lock,
//! double-checks the entry, and lazily inserts a new `FileRecorder`.
//!
//! The `Arc<FileRecorder>` returned from the registry can be cached by
//! callers in their per-conversation context to skip the registry
//! lookup entirely on hot paths (e.g., per-token `AssistantTokens`
//! emission). `commands::ask_ollama` does exactly this.
//!
//! # Eviction and late-event tolerance
//!
//! Two events trigger eviction:
//! - `RecorderEvent::ConversationEnd` evicts the chat-domain entry.
//! - `RecorderEvent::TurnEnd` evicts the search-domain entry, so a
//!   long-lived chat session with many `/search` turns does not
//!   accumulate one open file handle per turn until process exit.
//!
//! In both cases the registry flushes the file before dropping its
//! strong reference. In-flight `Arc<FileRecorder>` clones held by the
//! emitting task keep the file handle alive until they drop; `Arc`
//! semantics handle the ordering with no explicit synchronization.
//!
//! Late events arriving after the eviction-triggering event (e.g. a
//! cancelled stream's final `AssistantTokens` arriving after the
//! frontend's `record_conversation_end` call) lazily re-insert a new
//! `FileRecorder` for the evicted key. Because `FileRecorder` opens its
//! file in append mode, the late event lands as a benign trailing line
//! in the existing file. Consumers MUST tolerate post-end lines: the
//! canonical end of a conversation is the LAST line with
//! `kind: "conversation_end"` (chat) or the LAST `kind: "turn_end"`
//! for that turn id (search), not the first.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use super::ids::ConversationId;
use super::recorder::{FileRecorder, RecorderEvent, TraceDomain, TraceRecorder};

/// Production trace recorder. Wraps a per-`(domain, conversation_id)`
/// map of file handles rooted at `traces_root`.
#[derive(Debug)]
pub struct RegistryRecorder {
    /// Root path under which the per-domain subdirectories live. Each
    /// `FileRecorder` resolves its own path as
    /// `<traces_root>/<domain.dir()>/<conversation_id>.jsonl`.
    traces_root: PathBuf,
    /// Per-conversation file handles. `parking_lot::RwLock` matches the
    /// rest of the codebase and gives us a non-poisoning lock; the read
    /// path dominates because each conversation only inserts once.
    files: RwLock<HashMap<(TraceDomain, ConversationId), Arc<FileRecorder>>>,
}

impl RegistryRecorder {
    /// Constructs an empty registry rooted at `traces_root`. No
    /// directories are created here; each `FileRecorder` lazily
    /// creates its own parent directory on first record.
    pub fn new(traces_root: impl Into<PathBuf>) -> Self {
        Self {
            traces_root: traces_root.into(),
            files: RwLock::new(HashMap::new()),
        }
    }

    /// Returns the recorder for `(domain, conversation_id)`, creating
    /// it lazily if needed. Public for hot-path callers (e.g.
    /// `commands::ask_ollama`) that want to cache the `Arc` once and
    /// skip the registry lookup on every subsequent emit.
    ///
    /// Equivalent to a `record()` of `()`: read-locks the map, returns
    /// the existing handle on hit, and on miss takes the write-lock,
    /// double-checks the entry, and lazily inserts.
    pub fn recorder_for(
        &self,
        domain: TraceDomain,
        conversation_id: &ConversationId,
    ) -> Arc<FileRecorder> {
        let key = (domain, conversation_id.clone());
        if let Some(existing) = self.files.read().get(&key).cloned() {
            return existing;
        }
        let mut write = self.files.write();
        write
            .entry(key)
            .or_insert_with(|| {
                Arc::new(FileRecorder::for_conversation(
                    &self.traces_root,
                    domain,
                    conversation_id,
                ))
            })
            .clone()
    }

    /// Number of `(domain, conversation_id)` entries currently held in
    /// the map. Used by tests to assert eviction; not part of the
    /// production hot path.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.files.read().len()
    }

    /// Returns true if `(domain, conversation_id)` currently has a
    /// cached file handle in the registry. Tests use this to assert
    /// that `ConversationEnd` evicted the entry.
    #[cfg(test)]
    pub(crate) fn contains(&self, domain: TraceDomain, conversation_id: &ConversationId) -> bool {
        self.files
            .read()
            .contains_key(&(domain, conversation_id.clone()))
    }
}

impl TraceRecorder for RegistryRecorder {
    /// Routes the event to the right file and, on the per-domain
    /// terminal events, flushes + evicts the entry from the map.
    /// Chat domain evicts on `ConversationEnd`; search domain evicts on
    /// `TurnEnd`.
    fn record(&self, conversation_id: &ConversationId, event: RecorderEvent) {
        let domain = event.domain();
        let evict = event.is_conversation_end() || event.is_turn_end();
        let recorder = self.recorder_for(domain, conversation_id);
        recorder.record(conversation_id, event);
        if evict {
            // Flush BEFORE evicting so the post-end file is fully
            // durable on disk before the registry drops its strong
            // reference. In-flight `Arc` clones in streaming tasks
            // keep the file alive until they drop, but our flush gives
            // the post-end durability guarantee independent of those.
            recorder.flush();
            self.files
                .write()
                .remove(&(domain, conversation_id.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::recorder::ReaderUrlOutcome;
    use serde_json::{json, Value};
    use std::path::Path;

    fn fresh_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("thuki-trace-registry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cid(s: &str) -> ConversationId {
        ConversationId::new(s)
    }

    fn read_lines(path: &Path) -> Vec<Value> {
        let s = std::fs::read_to_string(path).expect("read trace file");
        s.lines()
            .map(|l| serde_json::from_str::<Value>(l).expect("valid json"))
            .collect()
    }

    #[test]
    fn lazy_insert_creates_handle_on_first_event() {
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        assert_eq!(reg.len(), 0);
        reg.record(
            &cid("conv-a"),
            RecorderEvent::UserMessage {
                content: "hi".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        assert_eq!(reg.len(), 1, "first event must lazy-insert");
        assert!(reg.contains(TraceDomain::Chat, &cid("conv-a")));
    }

    #[test]
    fn second_event_reuses_existing_handle() {
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        reg.record(
            &cid("conv-b"),
            RecorderEvent::UserMessage {
                content: "1".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        reg.record(
            &cid("conv-b"),
            RecorderEvent::UserMessage {
                content: "2".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        assert_eq!(reg.len(), 1, "second event must reuse handle, not insert");
        let path = root.join("chat").join("conv-b.jsonl");
        let lines = read_lines(&path);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn two_conversations_get_two_files() {
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        reg.record(
            &cid("conv-x"),
            RecorderEvent::UserMessage {
                content: "x1".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        reg.record(
            &cid("conv-y"),
            RecorderEvent::UserMessage {
                content: "y1".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        assert_eq!(reg.len(), 2);
        assert_eq!(read_lines(&root.join("chat").join("conv-x.jsonl")).len(), 1);
        assert_eq!(read_lines(&root.join("chat").join("conv-y.jsonl")).len(), 1);
    }

    #[test]
    fn chat_and_search_for_same_conv_get_separate_files_in_separate_folders() {
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        reg.record(
            &cid("conv-z"),
            RecorderEvent::UserMessage {
                content: "z".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        reg.record(
            &cid("conv-z"),
            RecorderEvent::SearxngQuery {
                query: "q".into(),
                url: "u".into(),
                status: Some(200),
                response_raw: None,
                normalized_results: json!([]),
                latency_ms: 1,
                error: None,
            },
        );
        assert_eq!(reg.len(), 2, "two domains × one conv → two entries");
        assert!(reg.contains(TraceDomain::Chat, &cid("conv-z")));
        assert!(reg.contains(TraceDomain::Search, &cid("conv-z")));
        assert_eq!(read_lines(&root.join("chat").join("conv-z.jsonl")).len(), 1);
        assert_eq!(
            read_lines(&root.join("search").join("conv-z.jsonl")).len(),
            1
        );
    }

    #[test]
    fn conversation_end_evicts_chat_entry() {
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        reg.record(
            &cid("conv-end"),
            RecorderEvent::UserMessage {
                content: "x".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        assert!(reg.contains(TraceDomain::Chat, &cid("conv-end")));
        reg.record(
            &cid("conv-end"),
            RecorderEvent::ConversationEnd {
                reason: "user_reset".into(),
            },
        );
        assert!(
            !reg.contains(TraceDomain::Chat, &cid("conv-end")),
            "ConversationEnd must evict the chat entry"
        );
    }

    #[test]
    fn late_event_after_end_appends_to_existing_file_via_lazy_recreate() {
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        reg.record(
            &cid("conv-late"),
            RecorderEvent::UserMessage {
                content: "first".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        reg.record(
            &cid("conv-late"),
            RecorderEvent::ConversationEnd {
                reason: "user_reset".into(),
            },
        );
        // The frontend signaled the end, but a stray AssistantTokens
        // from a cancelled stream arrives late. The registry must
        // tolerate this: re-insert + append, no panic, no duplicate
        // file.
        reg.record(
            &cid("conv-late"),
            RecorderEvent::AssistantTokens {
                chunk: "stray".into(),
            },
        );
        let path = root.join("chat").join("conv-late.jsonl");
        let lines = read_lines(&path);
        assert_eq!(
            lines.len(),
            3,
            "late event must append, not create a second file"
        );
        let kinds: Vec<&str> = lines.iter().map(|l| l["kind"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            vec!["user_message", "conversation_end", "assistant_tokens"],
            "consumers must tolerate post-end lines (canonical end is the LAST conversation_end)"
        );
    }

    #[test]
    fn recorder_for_returns_same_arc_on_repeated_lookup() {
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        let a = reg.recorder_for(TraceDomain::Chat, &cid("conv-arc"));
        let b = reg.recorder_for(TraceDomain::Chat, &cid("conv-arc"));
        // Pointer equality: same Arc, no second insert.
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn recorder_for_can_be_cached_for_hot_path() {
        // Simulates the per-streaming-task caching pattern that
        // `commands::ask_ollama` uses to bypass per-token registry
        // lookup.
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        let cached = reg.recorder_for(TraceDomain::Chat, &cid("conv-hot"));
        for i in 0..10 {
            cached.record(
                &cid("conv-hot"),
                RecorderEvent::AssistantTokens {
                    chunk: format!("tok-{i}"),
                },
            );
        }
        let path = root.join("chat").join("conv-hot.jsonl");
        let lines = read_lines(&path);
        assert_eq!(lines.len(), 10);
        for (i, line) in lines.iter().enumerate() {
            assert_eq!(line["chunk"], format!("tok-{i}"));
        }
    }

    #[test]
    fn turn_end_evicts_search_entry() {
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        reg.record(
            &cid("conv-search-end"),
            RecorderEvent::SearxngQuery {
                query: "q".into(),
                url: "u".into(),
                status: Some(200),
                response_raw: None,
                normalized_results: json!([]),
                latency_ms: 1,
                error: None,
            },
        );
        assert!(reg.contains(TraceDomain::Search, &cid("conv-search-end")));
        reg.record(
            &cid("conv-search-end"),
            RecorderEvent::TurnEnd {
                turn_id: "t-1".into(),
                final_action: "answered".into(),
                final_source_urls: vec![],
                total_latency_ms: 42,
                error: None,
            },
        );
        assert!(
            !reg.contains(TraceDomain::Search, &cid("conv-search-end")),
            "TurnEnd must evict the search-domain entry"
        );
    }

    #[test]
    fn turn_end_leaves_chat_entry_intact() {
        // Search-domain TurnEnd must not evict the chat-domain entry
        // for the same conversation id; the chat trace continues across
        // multiple `/search` turns.
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        reg.record(
            &cid("conv-mixed"),
            RecorderEvent::UserMessage {
                content: "before search".into(),
                attached_images: vec![],
                slash_command: None,
            },
        );
        reg.record(
            &cid("conv-mixed"),
            RecorderEvent::TurnEnd {
                turn_id: "t-1".into(),
                final_action: "answered".into(),
                final_source_urls: vec![],
                total_latency_ms: 10,
                error: None,
            },
        );
        assert!(
            reg.contains(TraceDomain::Chat, &cid("conv-mixed")),
            "TurnEnd is search-domain; the chat-domain entry must remain"
        );
        assert!(!reg.contains(TraceDomain::Search, &cid("conv-mixed")));
    }

    #[test]
    fn many_search_turns_do_not_accumulate_entries() {
        // Long-lived chat session that fires many `/search` turns: the
        // map size must not grow without bound. Each TurnEnd evicts.
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        for i in 0..20 {
            let conv = format!("conv-{i}");
            reg.record(
                &cid(&conv),
                RecorderEvent::SearxngQuery {
                    query: "q".into(),
                    url: "u".into(),
                    status: Some(200),
                    response_raw: None,
                    normalized_results: json!([]),
                    latency_ms: 1,
                    error: None,
                },
            );
            reg.record(
                &cid(&conv),
                RecorderEvent::TurnEnd {
                    turn_id: format!("t-{i}"),
                    final_action: "answered".into(),
                    final_source_urls: vec![],
                    total_latency_ms: 1,
                    error: None,
                },
            );
        }
        assert_eq!(
            reg.len(),
            0,
            "every search-only conversation must evict on TurnEnd"
        );
    }

    #[test]
    fn late_search_event_after_turn_end_appends_via_lazy_recreate() {
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        reg.record(
            &cid("conv-late-search"),
            RecorderEvent::SearxngQuery {
                query: "q".into(),
                url: "u".into(),
                status: Some(200),
                response_raw: None,
                normalized_results: json!([]),
                latency_ms: 1,
                error: None,
            },
        );
        reg.record(
            &cid("conv-late-search"),
            RecorderEvent::TurnEnd {
                turn_id: "t-late".into(),
                final_action: "answered".into(),
                final_source_urls: vec![],
                total_latency_ms: 1,
                error: None,
            },
        );
        // Stray late event from a delayed reader callback. Must
        // append, not panic, not duplicate the file.
        reg.record(
            &cid("conv-late-search"),
            RecorderEvent::Warning {
                kind: "reader_partial_failure".into(),
                payload: json!({}),
            },
        );
        let path = root.join("search").join("conv-late-search.jsonl");
        let lines = read_lines(&path);
        assert_eq!(lines.len(), 3);
        let kinds: Vec<&str> = lines.iter().map(|l| l["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["searxng_query", "turn_end", "warning"]);
    }

    #[test]
    fn search_event_with_reader_batch_payload_serializes_through_registry() {
        let root = fresh_dir();
        let reg = RegistryRecorder::new(&root);
        reg.record(
            &cid("conv-r"),
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
        let lines = read_lines(&root.join("search").join("conv-r.jsonl"));
        assert_eq!(lines[0]["kind"], "reader_batch");
        assert_eq!(lines[0]["domain"], "search");
    }
}
