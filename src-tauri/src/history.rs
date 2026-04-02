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
use tauri::State;

use crate::commands::{ChatMessage, ConversationHistory, SystemPrompt};
use crate::database;

/// Thread-safe wrapper around the SQLite connection.
pub struct Database(pub Mutex<Connection>);

/// Message payload sent from the frontend when saving a conversation.
#[derive(Deserialize)]
pub struct SaveMessagePayload {
    pub role: String,
    pub content: String,
    pub quoted_text: Option<String>,
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
    model: String,
    db: State<'_, Database>,
) -> Result<SaveConversationResponse, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

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
        database::create_conversation(&conn, placeholder_title.as_deref(), &model)
            .map_err(|e| e.to_string())?;

    let batch: Vec<(String, String, Option<String>)> = messages
        .into_iter()
        .map(|m| (m.role, m.content, m.quoted_text))
        .collect();

    database::insert_messages_batch(&conn, &conversation_id, &batch).map_err(|e| e.to_string())?;

    Ok(SaveConversationResponse { conversation_id })
}

/// Appends a single message to an already-saved conversation.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn persist_message(
    conversation_id: String,
    role: String,
    content: String,
    quoted_text: Option<String>,
    db: State<'_, Database>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    database::insert_message(
        &conn,
        &conversation_id,
        &role,
        &content,
        quoted_text.as_deref(),
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

    // Bump the epoch before replacing messages — same invariant as
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
        });
    }

    Ok(persisted)
}

/// Deletes a conversation and all its messages from SQLite.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn delete_conversation(conversation_id: String, db: State<'_, Database>) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    database::delete_conversation(&conn, &conversation_id).map_err(|e| e.to_string())
}

/// Generates a short AI title for a saved conversation by asking Ollama.
/// Runs as a fire-and-forget background task — the frontend polls or
/// refreshes the list to see the updated title.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn generate_title(
    conversation_id: String,
    messages: Vec<SaveMessagePayload>,
    db: State<'_, Database>,
    client: State<'_, reqwest::Client>,
    system_prompt: State<'_, SystemPrompt>,
) -> Result<(), String> {
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
            content: system_prompt.0.clone(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: title_prompt,
        },
    ];

    let endpoint = format!(
        "{}/api/chat",
        crate::commands::DEFAULT_OLLAMA_URL.trim_end_matches('/')
    );

    let cancel_token = tokio_util::sync::CancellationToken::new();
    let accumulated = crate::commands::stream_ollama_chat(
        &endpoint,
        crate::commands::DEFAULT_MODEL_NAME,
        title_messages,
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
            },
            SaveMessagePayload {
                role: "assistant".to_string(),
                content: "Rust is a systems programming language.".to_string(),
                quoted_text: None,
            },
        ];

        // Create conversation + insert messages.
        let placeholder_title = messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.content.trim().to_string());

        let conversation_id =
            database::create_conversation(&conn, placeholder_title.as_deref(), "llama3.2:3b")
                .unwrap();

        let batch: Vec<(String, String, Option<String>)> = messages
            .into_iter()
            .map(|m| (m.role, m.content, m.quoted_text))
            .collect();

        database::insert_messages_batch(&conn, &conversation_id, &batch).unwrap();

        // Load back.
        let loaded = database::load_messages(&conn, &conversation_id).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].role, "user");
        assert_eq!(loaded[0].content, "What is Rust?");
        assert_eq!(loaded[1].role, "assistant");
    }

    #[test]
    fn placeholder_title_truncation() {
        let long_message = "a".repeat(100);
        let trimmed = long_message.trim();
        let title = if trimmed.len() > 50 {
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
        };
        assert_eq!(title.len(), 53); // 50 chars + "..."
        assert!(title.ends_with("..."));
    }

    #[test]
    fn placeholder_title_short_message() {
        let short = "Hello";
        let title = if short.len() > 50 {
            format!("{}...", &short[..50])
        } else {
            short.to_string()
        };
        assert_eq!(title, "Hello");
    }

    #[test]
    fn placeholder_title_with_unicode() {
        let unicode_msg = "こんにちは世界、これはテストメッセージです。長いテキストを作成するために追加のテキストが必要です。";
        let trimmed = unicode_msg.trim();
        let title = if trimmed.len() > 50 {
            let end = trimmed
                .char_indices()
                .take(50)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(50);
            format!("{}...", &trimmed[..end])
        } else {
            trimmed.to_string()
        };
        // Should not panic on unicode boundary.
        assert!(title.ends_with("..."));
    }

    #[test]
    fn title_truncation_over_100_chars() {
        let mut title = "a".repeat(150);
        if title.len() > 100 {
            if let Some((i, c)) = title.char_indices().take(100).last() {
                title.truncate(i + c.len_utf8());
                title.push_str("...");
            }
        }
        assert_eq!(title.len(), 103); // 100 + "..."
    }
}
