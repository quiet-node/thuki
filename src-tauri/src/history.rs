/*!
 * Tauri commands for conversation history persistence.
 *
 * All commands interact with the SQLite database through `database.rs`.
 * The database connection is stored behind a `Mutex` in Tauri's managed
 * state so that it is safely shared across command invocations.
 */

use std::sync::Mutex;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri::State;

use crate::commands::{ChatMessage, ConversationHistory};
use crate::config::AppConfig;
use crate::database;
use crate::models::ActiveModelState;

/// Thread-safe wrapper around the SQLite connection.
pub struct Database(pub Mutex<Connection>);

/// A single search result source preview sent by the frontend when saving
/// an assistant message that was produced through the `/search` pipeline.
/// Matches the Rust `SearchResultPreview` and frontend `SearchResultPreview`
/// shape; kept as its own struct here to avoid a cross-module dependency.
#[derive(Clone, Deserialize, Serialize)]
pub struct SaveSearchSource {
    pub title: String,
    pub url: String,
}

/// Message payload sent from the frontend when saving a conversation.
#[derive(Deserialize)]
pub struct SaveMessagePayload {
    pub role: String,
    pub content: String,
    pub quoted_text: Option<String>,
    pub image_paths: Option<Vec<String>>,
    pub thinking_content: Option<String>,
    /// Sources footer for `/search` assistant messages. Serialised to JSON
    /// before hitting the `messages.search_sources` column.
    pub search_sources: Option<Vec<SaveSearchSource>>,
    /// Already-serialised `Vec<SearchWarning>` JSON string for search turns.
    /// Passed through verbatim to `messages.search_warnings`.
    pub search_warnings: Option<String>,
    /// Already-serialised `SearchMetadata` JSON string for search turns.
    /// Passed through verbatim to `messages.search_metadata`.
    pub search_metadata: Option<String>,
    /// Slug of the Ollama model that produced this response. Frontend stamps
    /// assistant payloads with the active model at generation time; `None`
    /// for user payloads. Accepted as missing via serde Option default.
    pub model_name: Option<String>,
}

/// Response returned when saving a conversation.
#[derive(Serialize)]
pub struct SaveConversationResponse {
    pub conversation_id: String,
}

/// Persists the current in-memory conversation to SQLite. Creates a new
/// conversation row and bulk-inserts all messages. Returns the conversation ID
/// so the frontend can track it for subsequent auto-persist calls.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn save_conversation(
    messages: Vec<SaveMessagePayload>,
    db: State<'_, Database>,
    active_model: State<'_, ActiveModelState>,
) -> Result<SaveConversationResponse, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let model_slug = {
        let guard = active_model.0.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };
    let model_slug =
        model_slug.ok_or_else(|| "No model selected; cannot save conversation.".to_string())?;

    // Use the first user message (truncated) as the initial title placeholder.
    let placeholder_title = messages.iter().find(|m| m.role == "user").map(|m| {
        let trimmed = m.content.trim();
        if trimmed.len() > 50 {
            format!(
                "{}...",
                &trimmed[..trimmed
                    .char_indices()
                    .take(50)
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(50)]
            )
        } else {
            trimmed.to_string()
        }
    });

    let conversation_id =
        database::create_conversation(&conn, placeholder_title.as_deref(), &model_slug)
            .map_err(|e| e.to_string())?;

    let batch: Vec<database::MessageBatchRow> = messages
        .into_iter()
        .map(|m| {
            let image_json = m.image_paths.filter(|v| !v.is_empty()).map(|v| {
                serde_json::to_string(&v).expect("Vec<String> serialization is infallible")
            });
            let sources_json = m.search_sources.filter(|v| !v.is_empty()).map(|v| {
                serde_json::to_string(&v)
                    .expect("Vec<SaveSearchSource> serialization is infallible")
            });
            (
                m.role,
                m.content,
                m.quoted_text,
                image_json,
                m.thinking_content,
                sources_json,
                m.search_warnings,
                m.search_metadata,
                m.model_name,
            )
        })
        .collect();

    database::insert_messages_batch(&conn, &conversation_id, &batch).map_err(|e| e.to_string())?;

    Ok(SaveConversationResponse { conversation_id })
}

/// Appends a single message to an already-saved conversation.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
#[allow(clippy::too_many_arguments)]
pub fn persist_message(
    conversation_id: String,
    role: String,
    content: String,
    quoted_text: Option<String>,
    image_paths: Option<Vec<String>>,
    thinking_content: Option<String>,
    search_sources: Option<Vec<SaveSearchSource>>,
    search_warnings: Option<String>,
    search_metadata: Option<String>,
    model_name: Option<String>,
    db: State<'_, Database>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let image_json = image_paths
        .filter(|v| !v.is_empty())
        .map(|v| serde_json::to_string(&v).expect("Vec<String> serialization is infallible"));
    let sources_json = search_sources.filter(|v| !v.is_empty()).map(|v| {
        serde_json::to_string(&v).expect("Vec<SaveSearchSource> serialization is infallible")
    });
    database::insert_message(
        &conn,
        &conversation_id,
        &role,
        &content,
        quoted_text.as_deref(),
        image_json.as_deref(),
        thinking_content.as_deref(),
        sources_json.as_deref(),
        search_warnings.as_deref(),
        search_metadata.as_deref(),
        model_name.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Lists saved conversations, optionally filtered by a title search query.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn list_conversations(
    search: Option<String>,
    db: State<'_, Database>,
) -> Result<Vec<database::ConversationSummary>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    database::list_conversations(&conn, search.as_deref()).map_err(|e| e.to_string())
}

/// Loads all messages for a conversation and syncs them into the backend
/// `ConversationHistory` so subsequent `ask_ollama` calls include context.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn load_conversation(
    conversation_id: String,
    db: State<'_, Database>,
    history: State<'_, ConversationHistory>,
) -> Result<Vec<database::PersistedMessage>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let persisted = database::load_messages(&conn, &conversation_id).map_err(|e| e.to_string())?;

    // Bump the epoch before replacing messages - same invariant as
    // `reset_conversation`. This prevents any in-flight `ask_ollama`
    // stream from appending stale tokens into the freshly loaded history.
    history
        .epoch
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let mut conv = history.messages.lock().map_err(|e| e.to_string())?;
    conv.clear();
    for msg in &persisted {
        conv.push(ChatMessage {
            role: msg.role.clone(),
            content: msg.content.clone(),
            images: None,
        });
    }

    Ok(persisted)
}

/// Deletes a conversation and all its messages from SQLite, and immediately
/// removes any image files referenced by those messages from disk.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn delete_conversation(
    app_handle: tauri::AppHandle,
    conversation_id: String,
    db: State<'_, Database>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    // Collect image paths before deleting messages (CASCADE will remove them).
    let messages = database::load_messages(&conn, &conversation_id).map_err(|e| e.to_string())?;
    let image_paths: Vec<String> = messages
        .iter()
        .filter_map(|m| m.image_paths.as_ref())
        .filter_map(|json| serde_json::from_str::<Vec<String>>(json).ok())
        .flatten()
        .collect();

    database::delete_conversation(&conn, &conversation_id).map_err(|e| e.to_string())?;

    // Best-effort file cleanup - don't fail the command if a file is missing.
    let base_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    for path in &image_paths {
        let _ = crate::images::remove_image(&base_dir, path);
    }

    Ok(())
}

/// Generates a short AI title for a saved conversation by asking Ollama.
/// Runs as a fire-and-forget background task - the frontend polls or
/// refreshes the list to see the updated title.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn generate_title(
    conversation_id: String,
    messages: Vec<SaveMessagePayload>,
    model: String,
    db: State<'_, Database>,
    client: State<'_, reqwest::Client>,
    app_config: State<'_, parking_lot::RwLock<AppConfig>>,
) -> Result<(), String> {
    let app_config = app_config.read().clone();
    // Build a condensed context for title generation.
    let mut context = String::new();
    for msg in &messages {
        let prefix = if msg.role == "user" {
            "User"
        } else {
            "Assistant"
        };
        context.push_str(&format!("{}: {}\n", prefix, msg.content));
        if context.len() > 1000 {
            break;
        }
    }

    let title_prompt = format!(
        "Summarize this conversation in 5 words or fewer as a title. \
         Return ONLY the title, no quotes, no punctuation at the end, no explanation.\n\n{}",
        context
    );

    let title_messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: app_config.prompt.resolved_system.clone(),
            images: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: title_prompt,
            images: None,
        },
    ];

    let endpoint = format!(
        "{}/api/chat",
        app_config.inference.ollama_url.trim_end_matches('/')
    );

    let cancel_token = tokio_util::sync::CancellationToken::new();
    let accumulated = crate::commands::stream_ollama_chat(
        &endpoint,
        &model,
        title_messages,
        false,
        &client,
        cancel_token,
        |_| {}, // No per-chunk side effects; we use the accumulated return value.
    )
    .await;

    let mut title = accumulated.trim().to_string();

    // Truncate overly long titles.
    if title.len() > 100 {
        if let Some((i, c)) = title.char_indices().take(100).last() {
            title.truncate(i + c.len_utf8());
            title.push_str("...");
        }
    }

    if !title.is_empty() {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        database::update_conversation_title(&conn, &conversation_id, &title)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database;

    /// Helper to create a Database state from an in-memory connection.
    fn test_db() -> Database {
        Database(Mutex::new(database::open_in_memory().unwrap()))
    }

    #[test]
    fn save_and_load_conversation_roundtrip() {
        let db = test_db();
        let conn = db.0.lock().unwrap();

        let messages = vec![
            SaveMessagePayload {
                role: "user".to_string(),
                content: "What is Rust?".to_string(),
                quoted_text: None,
                image_paths: Some(vec!["/tmp/img.jpg".to_string()]),
                thinking_content: None,
                search_sources: None,
                search_warnings: None,
                search_metadata: None,
                model_name: None,
            },
            SaveMessagePayload {
                role: "assistant".to_string(),
                content: "Rust is a systems programming language.".to_string(),
                quoted_text: None,
                image_paths: None,
                thinking_content: Some("Let me think about Rust...".to_string()),
                search_sources: Some(vec![
                    SaveSearchSource {
                        title: "Rust docs".into(),
                        url: "https://doc.rust-lang.org".into(),
                    },
                    SaveSearchSource {
                        title: "Tokio".into(),
                        url: "https://tokio.rs".into(),
                    },
                ]),
                search_warnings: Some(r#"["reader_unavailable"]"#.to_string()),
                search_metadata: Some(
                    r#"{"iterations":[],"total_duration_ms":10,"retries_performed":0}"#.to_string(),
                ),
                model_name: Some("gemma4:e2b".to_string()),
            },
        ];

        // Create conversation + insert messages.
        let placeholder_title = messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.content.trim().to_string());

        let conversation_id =
            database::create_conversation(&conn, placeholder_title.as_deref(), "gemma4:e2b")
                .unwrap();

        let batch: Vec<database::MessageBatchRow> = messages
            .into_iter()
            .map(|m| {
                let image_json = m.image_paths.filter(|v| !v.is_empty()).map(|v| {
                    serde_json::to_string(&v).expect("Vec<String> serialization is infallible")
                });
                let sources_json = m.search_sources.filter(|v| !v.is_empty()).map(|v| {
                    serde_json::to_string(&v)
                        .expect("Vec<SaveSearchSource> serialization is infallible")
                });
                (
                    m.role,
                    m.content,
                    m.quoted_text,
                    image_json,
                    m.thinking_content,
                    sources_json,
                    m.search_warnings,
                    m.search_metadata,
                    m.model_name,
                )
            })
            .collect();

        database::insert_messages_batch(&conn, &conversation_id, &batch).unwrap();

        // Load back.
        let loaded = database::load_messages(&conn, &conversation_id).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].role, "user");
        assert_eq!(loaded[0].content, "What is Rust?");
        assert_eq!(
            loaded[0].image_paths.as_deref(),
            Some(r#"["/tmp/img.jpg"]"#)
        );
        assert_eq!(loaded[0].thinking_content, None);
        assert!(loaded[0].search_sources.is_none());
        assert_eq!(loaded[1].role, "assistant");
        assert!(loaded[1].image_paths.is_none());
        assert_eq!(
            loaded[1].thinking_content.as_deref(),
            Some("Let me think about Rust...")
        );
        let sources_json = loaded[1].search_sources.as_deref().unwrap();
        assert!(sources_json.contains("Rust docs"));
        assert!(sources_json.contains("https://tokio.rs"));
        assert_eq!(
            loaded[1].search_warnings.as_deref(),
            Some(r#"["reader_unavailable"]"#)
        );
        assert!(loaded[1]
            .search_metadata
            .as_deref()
            .unwrap()
            .contains("total_duration_ms"));
        assert!(loaded[0].search_warnings.is_none());
        assert!(loaded[0].search_metadata.is_none());
        assert!(loaded[0].model_name.is_none());
        assert_eq!(loaded[1].model_name.as_deref(), Some("gemma4:e2b"));
    }

    #[test]
    fn placeholder_title_truncation() {
        let long_message = "a".repeat(100);
        // Long message (100 chars) should be truncated to 50 + "..."
        assert!(long_message.len() > 50);
        let end = long_message
            .char_indices()
            .take(50)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(50);
        let title = format!("{}...", &long_message[..end]);
        assert_eq!(title.len(), 53); // 50 chars + "..."
        assert!(title.ends_with("..."));
    }

    #[test]
    fn placeholder_title_short_message() {
        let short = "Hello";
        // Short message (5 chars) should not be truncated.
        assert!(short.len() <= 50);
        assert_eq!(short, "Hello");
    }

    #[test]
    fn placeholder_title_with_unicode() {
        let unicode_msg = "こんにちは世界、これはテストメッセージです。長いテキストを作成するために追加のテキストが必要です。";
        let trimmed = unicode_msg.trim();
        // Unicode message with byte length > 50 should truncate at char boundary.
        assert!(trimmed.len() > 50);
        let end = trimmed
            .char_indices()
            .take(50)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(50);
        let title = format!("{}...", &trimmed[..end]);
        // Should not panic on unicode boundary.
        assert!(title.ends_with("..."));
    }

    #[test]
    fn title_truncation_over_100_chars() {
        let mut title = "a".repeat(150);
        // Titles over 100 chars should be truncated to 100 + "..."
        assert!(title.len() > 100);
        let (i, c) = title.char_indices().take(100).last().unwrap();
        title.truncate(i + c.len_utf8());
        title.push_str("...");
        assert_eq!(title.len(), 103); // 100 + "..."
    }
}
