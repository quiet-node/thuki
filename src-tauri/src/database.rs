/*!
 * SQLite persistence layer for conversation history.
 *
 * Stores conversations and messages in `~/.thuki/thuki.db` using rusqlite
 * with WAL journal mode for concurrent read access during streaming writes.
 *
 * All public functions accept a `&Connection` and are synchronous — callers
 * in async Tauri commands should use `spawn_blocking` or hold the connection
 * behind a `Mutex`.
 */

use rusqlite::{params, Connection, Result as SqlResult};
use serde::Serialize;

/// Summary of a conversation for the history dropdown list.
#[derive(Clone, Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: Option<String>,
    pub model: String,
    pub updated_at: i64,
    pub message_count: i64,
}

/// A persisted message read back from the database.
#[derive(Clone, Serialize)]
pub struct PersistedMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub quoted_text: Option<String>,
    pub created_at: i64,
}

/// Opens (or creates) the SQLite database at `~/.thuki/thuki.db` and runs
/// migrations. Returns the ready-to-use connection.
///
/// # Errors
///
/// Returns an error if the home directory cannot be determined, the
/// `~/.thuki/` directory cannot be created, or SQLite initialisation fails.
pub fn open_database() -> SqlResult<Connection> {
    let db_path =
        resolve_db_path().map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

    let conn = Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    run_migrations(&conn)?;
    Ok(conn)
}

/// Opens an in-memory database for testing. Runs the same migrations as
/// the file-backed database.
#[cfg(test)]
pub fn open_in_memory() -> SqlResult<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    run_migrations(&conn)?;
    Ok(conn)
}

/// Resolves the database file path, creating `~/.thuki/` if it does not exist.
fn resolve_db_path() -> std::io::Result<std::path::PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "home directory not found")
    })?;
    let dir = home.join(".thuki");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("thuki.db"))
}

/// Creates the schema tables if they do not already exist.
fn run_migrations(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversations (
            id          TEXT PRIMARY KEY,
            title       TEXT,
            model       TEXT NOT NULL,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL,
            meta        TEXT
        );

        CREATE TABLE IF NOT EXISTS messages (
            id              TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL
                REFERENCES conversations(id) ON DELETE CASCADE,
            role            TEXT NOT NULL,
            content         TEXT NOT NULL,
            quoted_text     TEXT,
            created_at      INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_messages_conversation
            ON messages(conversation_id, created_at);

        CREATE INDEX IF NOT EXISTS idx_conversations_updated
            ON conversations(updated_at DESC);",
    )
}

// ─── Conversation CRUD ──────────────────────────────────────────────────────

/// Inserts a new conversation row and returns its UUID.
pub fn create_conversation(
    conn: &Connection,
    title: Option<&str>,
    model: &str,
) -> SqlResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_millis();
    conn.execute(
        "INSERT INTO conversations (id, title, model, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, title, model, now, now],
    )?;
    Ok(id)
}

/// Lists conversations ordered by most recently updated, with an optional
/// case-insensitive title substring filter.
pub fn list_conversations(
    conn: &Connection,
    search: Option<&str>,
) -> SqlResult<Vec<ConversationSummary>> {
    let mut stmt;
    let mut rows_iter;

    match search {
        Some(q) if !q.trim().is_empty() => {
            let pattern = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
            stmt = conn.prepare(
                "SELECT c.id, c.title, c.model, c.updated_at,
                        (SELECT COUNT(*) FROM messages m WHERE m.conversation_id = c.id)
                 FROM conversations c
                 WHERE c.title LIKE ?1 ESCAPE '\\'
                 ORDER BY c.updated_at DESC",
            )?;
            rows_iter = stmt.query_map(params![pattern], map_summary)?;
        }
        _ => {
            stmt = conn.prepare(
                "SELECT c.id, c.title, c.model, c.updated_at,
                        (SELECT COUNT(*) FROM messages m WHERE m.conversation_id = c.id)
                 FROM conversations c
                 ORDER BY c.updated_at DESC",
            )?;
            rows_iter = stmt.query_map([], map_summary)?;
        }
    }

    rows_iter.by_ref().collect()
}

/// Updates the title of an existing conversation.
pub fn update_conversation_title(
    conn: &Connection,
    conversation_id: &str,
    title: &str,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3",
        params![title, now_millis(), conversation_id],
    )?;
    Ok(())
}

/// Deletes a conversation and its messages (via ON DELETE CASCADE).
pub fn delete_conversation(conn: &Connection, conversation_id: &str) -> SqlResult<()> {
    conn.execute(
        "DELETE FROM conversations WHERE id = ?1",
        params![conversation_id],
    )?;
    Ok(())
}

// ─── Message CRUD ───────────────────────────────────────────────────────────

/// Inserts a single message and touches the conversation's `updated_at`.
pub fn insert_message(
    conn: &Connection,
    conversation_id: &str,
    role: &str,
    content: &str,
    quoted_text: Option<&str>,
) -> SqlResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_millis();
    conn.execute(
        "INSERT INTO messages (id, conversation_id, role, content, quoted_text, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, conversation_id, role, content, quoted_text, now],
    )?;
    conn.execute(
        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
        params![now, conversation_id],
    )?;
    Ok(id)
}

/// Bulk-inserts messages for the initial save. Runs inside a transaction.
pub fn insert_messages_batch(
    conn: &Connection,
    conversation_id: &str,
    messages: &[(String, String, Option<String>)], // (role, content, quoted_text)
) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;
    let now = now_millis();
    {
        let mut stmt = tx.prepare(
            "INSERT INTO messages (id, conversation_id, role, content, quoted_text, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for (role, content, quoted_text) in messages {
            let id = uuid::Uuid::new_v4().to_string();
            stmt.execute(params![
                id,
                conversation_id,
                role,
                content,
                quoted_text.as_deref(),
                now
            ])?;
        }
    }
    tx.execute(
        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
        params![now, conversation_id],
    )?;
    tx.commit()
}

/// Loads all messages for a conversation in chronological order.
pub fn load_messages(conn: &Connection, conversation_id: &str) -> SqlResult<Vec<PersistedMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, content, quoted_text, created_at
         FROM messages
         WHERE conversation_id = ?1
         ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![conversation_id], |row| {
        Ok(PersistedMessage {
            id: row.get(0)?,
            role: row.get(1)?,
            content: row.get(2)?,
            quoted_text: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Maps a row from the conversations query to a `ConversationSummary`.
fn map_summary(row: &rusqlite::Row) -> SqlResult<ConversationSummary> {
    Ok(ConversationSummary {
        id: row.get(0)?,
        title: row.get(1)?,
        model: row.get(2)?,
        updated_at: row.get(3)?,
        message_count: row.get(4)?,
    })
}

/// Current UTC time in milliseconds since the Unix epoch.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_millis() as i64
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_create_tables() {
        let conn = open_in_memory().unwrap();
        // Verify both tables exist by querying sqlite_master.
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(tables.contains(&"conversations".to_string()));
        assert!(tables.contains(&"messages".to_string()));
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = open_in_memory().unwrap();
        // Running migrations again should not error.
        run_migrations(&conn).unwrap();
    }

    #[test]
    fn create_and_list_conversations() {
        let conn = open_in_memory().unwrap();
        let id = create_conversation(&conn, Some("Test Chat"), "llama3.2:3b").unwrap();
        assert!(!id.is_empty());

        let convos = list_conversations(&conn, None).unwrap();
        assert_eq!(convos.len(), 1);
        assert_eq!(convos[0].title.as_deref(), Some("Test Chat"));
        assert_eq!(convos[0].model, "llama3.2:3b");
        assert_eq!(convos[0].message_count, 0);
    }

    #[test]
    fn list_conversations_with_search_filter() {
        let conn = open_in_memory().unwrap();
        create_conversation(&conn, Some("Rust Code Help"), "llama3.2:3b").unwrap();
        create_conversation(&conn, Some("Draft Email"), "llama3.2:3b").unwrap();

        let results = list_conversations(&conn, Some("rust")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title.as_deref(), Some("Rust Code Help"));

        // Empty search returns all.
        let all = list_conversations(&conn, Some("")).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn search_escapes_sql_wildcards() {
        let conn = open_in_memory().unwrap();
        create_conversation(&conn, Some("100% done"), "llama3.2:3b").unwrap();
        create_conversation(&conn, Some("something else"), "llama3.2:3b").unwrap();

        let results = list_conversations(&conn, Some("100%")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title.as_deref(), Some("100% done"));
    }

    #[test]
    fn update_conversation_title() {
        let conn = open_in_memory().unwrap();
        let id = create_conversation(&conn, Some("Old Title"), "llama3.2:3b").unwrap();

        super::update_conversation_title(&conn, &id, "New Title").unwrap();

        let convos = list_conversations(&conn, None).unwrap();
        assert_eq!(convos[0].title.as_deref(), Some("New Title"));
    }

    #[test]
    fn delete_conversation_cascades_messages() {
        let conn = open_in_memory().unwrap();
        let id = create_conversation(&conn, Some("To Delete"), "llama3.2:3b").unwrap();
        insert_message(&conn, &id, "user", "hello", None).unwrap();
        insert_message(&conn, &id, "assistant", "hi there", None).unwrap();

        delete_conversation(&conn, &id).unwrap();

        let convos = list_conversations(&conn, None).unwrap();
        assert!(convos.is_empty());

        let msgs = load_messages(&conn, &id).unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn insert_and_load_messages() {
        let conn = open_in_memory().unwrap();
        let id = create_conversation(&conn, None, "llama3.2:3b").unwrap();

        insert_message(&conn, &id, "user", "What is Rust?", Some("quoted context")).unwrap();
        insert_message(&conn, &id, "assistant", "Rust is a systems language.", None).unwrap();

        let msgs = load_messages(&conn, &id).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "What is Rust?");
        assert_eq!(msgs[0].quoted_text.as_deref(), Some("quoted context"));
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "Rust is a systems language.");
        assert!(msgs[1].quoted_text.is_none());
    }

    #[test]
    fn insert_messages_batch_is_atomic() {
        let conn = open_in_memory().unwrap();
        let id = create_conversation(&conn, None, "llama3.2:3b").unwrap();

        let batch = vec![
            ("user".to_string(), "hello".to_string(), None),
            ("assistant".to_string(), "hi".to_string(), None),
            (
                "user".to_string(),
                "how are you?".to_string(),
                Some("context".to_string()),
            ),
        ];
        insert_messages_batch(&conn, &id, &batch).unwrap();

        let msgs = load_messages(&conn, &id).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].quoted_text.as_deref(), Some("context"));

        // Message count reflected in listing.
        let convos = list_conversations(&conn, None).unwrap();
        assert_eq!(convos[0].message_count, 3);
    }

    #[test]
    fn insert_message_touches_updated_at() {
        let conn = open_in_memory().unwrap();
        let id = create_conversation(&conn, None, "llama3.2:3b").unwrap();
        let before = list_conversations(&conn, None).unwrap()[0].updated_at;

        // Small delay to ensure timestamp changes.
        std::thread::sleep(std::time::Duration::from_millis(5));

        insert_message(&conn, &id, "user", "test", None).unwrap();
        let after = list_conversations(&conn, None).unwrap()[0].updated_at;

        assert!(after >= before);
    }

    #[test]
    fn conversations_ordered_by_most_recent() {
        let conn = open_in_memory().unwrap();
        let id1 = create_conversation(&conn, Some("First"), "llama3.2:3b").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        create_conversation(&conn, Some("Second"), "llama3.2:3b").unwrap();

        let convos = list_conversations(&conn, None).unwrap();
        assert_eq!(convos[0].title.as_deref(), Some("Second"));
        assert_eq!(convos[1].title.as_deref(), Some("First"));

        // Updating a message in the first conversation bumps it to the top.
        std::thread::sleep(std::time::Duration::from_millis(5));
        insert_message(&conn, &id1, "user", "bump", None).unwrap();

        let convos = list_conversations(&conn, None).unwrap();
        assert_eq!(convos[0].title.as_deref(), Some("First"));
    }

    #[test]
    fn create_conversation_with_no_title() {
        let conn = open_in_memory().unwrap();
        let id = create_conversation(&conn, None, "llama3.2:3b").unwrap();
        let convos = list_conversations(&conn, None).unwrap();
        assert_eq!(convos.len(), 1);
        assert!(convos[0].title.is_none());
        assert!(!id.is_empty());
    }

    #[test]
    fn delete_nonexistent_conversation_is_noop() {
        let conn = open_in_memory().unwrap();
        // Should not error — DELETE with no matching rows is valid SQL.
        delete_conversation(&conn, "nonexistent-id").unwrap();
    }

    #[test]
    fn load_messages_empty_conversation() {
        let conn = open_in_memory().unwrap();
        let id = create_conversation(&conn, None, "llama3.2:3b").unwrap();
        let msgs = load_messages(&conn, &id).unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn now_millis_returns_reasonable_value() {
        let ms = now_millis();
        // Should be after 2024-01-01 in milliseconds.
        assert!(ms > 1_704_067_200_000);
    }

    #[test]
    fn resolve_db_path_creates_directory() {
        // This test verifies the path resolution logic — it creates ~/.thuki/
        // which is acceptable in test environments.
        let path = resolve_db_path().unwrap();
        assert!(path.ends_with("thuki.db"));
        assert!(path.parent().unwrap().exists());
    }
}
