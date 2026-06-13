/*!
 * Installed-model manifest: CRUD over the `installed_models` SQLite table.
 *
 * Each row represents a GGUF model blob that has been downloaded and
 * content-addressed into the local blob store. The `id` field
 * (`"<repo>:<file_name>"`) is the stable public key; `sha256` and
 * `mmproj_sha256` are content addresses shared across rows (two models can
 * reference the same mmproj blob).
 *
 * All functions take a `&rusqlite::Connection` and are synchronous. Callers
 * inside async Tauri commands must use `spawn_blocking` or hold the
 * connection behind a `Mutex`.
 */

use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use serde::Serialize;

/// A GGUF model that has been downloaded and recorded in the manifest.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstalledModel {
    /// Stable key: `"<repo>:<file_name>"`. Uniquely identifies a model
    /// variant within the blob store.
    pub id: String,
    /// Human-readable label shown in the model picker (e.g. "Gemma 3 12B").
    pub display_name: String,
    /// Hugging Face repo slug (e.g. "google/gemma-3-12b-it-qat-gguf").
    pub repo: String,
    /// 40-hex git commit SHA pinned at download time; provenance anchor.
    pub revision: String,
    /// Filename within the repo (e.g. "gemma-3-12b-it-q4_k_m.gguf").
    pub file_name: String,
    /// SHA-256 hex digest of the weights blob.
    pub sha256: String,
    /// Compressed file size in bytes.
    pub size_bytes: u64,
    /// Quantisation label (e.g. "Q4_K_M").
    pub quant: String,
    /// Whether the model accepts image inputs.
    pub vision: bool,
    /// Whether the model exposes a thinking/scratchpad token stream.
    pub thinking: bool,
    /// Filename of the vision projection blob, if any.
    pub mmproj_file: Option<String>,
    /// SHA-256 hex digest of the mmproj blob, if any.
    pub mmproj_sha256: Option<String>,
}

/// Inserts or replaces a model row in the manifest. If a row with the same
/// `id` already exists it is replaced in full, so re-downloading a model
/// always produces an up-to-date entry. `created_at` is set to the current
/// Unix second timestamp inside this function.
///
/// # Errors
///
/// Returns a `rusqlite::Error` if the underlying SQL execution fails.
pub fn insert(conn: &Connection, model: &InstalledModel) -> SqlResult<()> {
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "INSERT OR REPLACE INTO installed_models \
         (id, display_name, repo, revision, file_name, sha256, size_bytes, \
          quant, vision, thinking, mmproj_file, mmproj_sha256, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            model.id,
            model.display_name,
            model.repo,
            model.revision,
            model.file_name,
            model.sha256,
            model.size_bytes as i64,
            model.quant,
            model.vision as i32,
            model.thinking as i32,
            model.mmproj_file,
            model.mmproj_sha256,
            created_at,
        ],
    )?;
    Ok(())
}

/// Returns all installed models ordered alphabetically by `display_name`.
///
/// # Errors
///
/// Returns a `rusqlite::Error` if the query fails.
pub fn list(conn: &Connection) -> SqlResult<Vec<InstalledModel>> {
    let mut stmt = conn.prepare(
        "SELECT id, display_name, repo, revision, file_name, sha256, \
                size_bytes, quant, vision, thinking, mmproj_file, mmproj_sha256 \
         FROM installed_models ORDER BY display_name",
    )?;
    let rows = stmt.query_map([], row_to_model)?;
    rows.collect()
}

/// Returns the model with the given `id`, or `None` if not present.
///
/// # Errors
///
/// Returns a `rusqlite::Error` if the query fails.
pub fn get(conn: &Connection, id: &str) -> SqlResult<Option<InstalledModel>> {
    conn.query_row(
        "SELECT id, display_name, repo, revision, file_name, sha256, \
                size_bytes, quant, vision, thinking, mmproj_file, mmproj_sha256 \
         FROM installed_models WHERE id = ?1",
        params![id],
        row_to_model,
    )
    .optional()
}

/// Deletes the model row identified by `id` and returns the SHA-256 values
/// (weights and mmproj) that are no longer referenced by any remaining row.
///
/// A blob SHA is included in the return value only when it is not referenced
/// by any other row in either the `sha256` or `mmproj_sha256` column. The
/// caller is responsible for removing the orphaned blobs from disk.
///
/// Returns an empty `Vec` if no row with the given `id` exists.
///
/// # Errors
///
/// Returns a `rusqlite::Error` if the delete or the reference-count query fails.
pub fn delete(conn: &Connection, id: &str) -> SqlResult<Vec<String>> {
    // Snapshot the SHA values of the row being deleted before it is gone.
    let target: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT sha256, mmproj_sha256 FROM installed_models WHERE id = ?1",
            params![id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;

    let Some((weights_sha, mmproj_sha)) = target else {
        return Ok(vec![]);
    };

    conn.execute("DELETE FROM installed_models WHERE id = ?1", params![id])?;

    // Collect candidate SHAs; deduplicate so a model that is its own mmproj
    // does not produce duplicate return entries.
    let mut candidates: Vec<String> = vec![weights_sha];
    if let Some(ref s) = mmproj_sha {
        if !candidates.contains(s) {
            candidates.push(s.clone());
        }
    }

    // Filter to those no longer referenced by any remaining row.
    let mut orphans = Vec::new();
    for sha in candidates {
        if sha_refcount(conn, &sha)? == 0 {
            orphans.push(sha);
        }
    }

    Ok(orphans)
}

/// Counts the number of `installed_models` rows that reference `sha` in
/// either the `sha256` or `mmproj_sha256` column.
fn sha_refcount(conn: &Connection, sha: &str) -> SqlResult<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM installed_models \
         WHERE sha256 = ?1 OR mmproj_sha256 = ?1",
        params![sha],
        |row| row.get(0),
    )
}

/// Maps a SQLite row to an [`InstalledModel`].
fn row_to_model(row: &rusqlite::Row<'_>) -> SqlResult<InstalledModel> {
    Ok(InstalledModel {
        id: row.get(0)?,
        display_name: row.get(1)?,
        repo: row.get(2)?,
        revision: row.get(3)?,
        file_name: row.get(4)?,
        sha256: row.get(5)?,
        size_bytes: row.get::<_, i64>(6)? as u64,
        quant: row.get(7)?,
        vision: row.get::<_, i32>(8)? != 0,
        thinking: row.get::<_, i32>(9)? != 0,
        mmproj_file: row.get(10)?,
        mmproj_sha256: row.get(11)?,
    })
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::open_in_memory;

    fn make_model(id: &str, sha256: &str) -> InstalledModel {
        InstalledModel {
            id: id.to_string(),
            display_name: format!("Model {id}"),
            repo: "org/repo".to_string(),
            revision: "a".repeat(40),
            file_name: format!("{id}.gguf"),
            sha256: sha256.to_string(),
            size_bytes: 1_000_000,
            quant: "Q4_K_M".to_string(),
            vision: false,
            thinking: false,
            mmproj_file: None,
            mmproj_sha256: None,
        }
    }

    fn make_model_with_mmproj(id: &str, sha256: &str, mmproj_sha: &str) -> InstalledModel {
        InstalledModel {
            mmproj_file: Some(format!("{id}-mmproj.gguf")),
            mmproj_sha256: Some(mmproj_sha.to_string()),
            ..make_model(id, sha256)
        }
    }

    #[test]
    fn insert_and_list_roundtrip() {
        let conn = open_in_memory().unwrap();
        let m = make_model("org/repo:model.gguf", "sha_weights_1");
        insert(&conn, &m).unwrap();

        let rows = list(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, m.id);
        assert_eq!(rows[0].sha256, m.sha256);
        assert_eq!(rows[0].mmproj_file, None);
        assert_eq!(rows[0].mmproj_sha256, None);
    }

    #[test]
    fn insert_and_list_roundtrip_with_mmproj() {
        let conn = open_in_memory().unwrap();
        let m = make_model_with_mmproj("org/repo:model.gguf", "sha_w", "sha_mm");
        insert(&conn, &m).unwrap();

        let rows = list(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].mmproj_file,
            Some("org/repo:model.gguf-mmproj.gguf".to_string())
        );
        assert_eq!(rows[0].mmproj_sha256, Some("sha_mm".to_string()));
    }

    #[test]
    fn list_is_ordered_by_display_name() {
        let conn = open_in_memory().unwrap();
        let mut b = make_model("id_b", "sha_b");
        b.display_name = "Zebra Model".to_string();
        let mut a = make_model("id_a", "sha_a");
        a.display_name = "Alpha Model".to_string();
        insert(&conn, &b).unwrap();
        insert(&conn, &a).unwrap();

        let rows = list(&conn).unwrap();
        assert_eq!(rows[0].display_name, "Alpha Model");
        assert_eq!(rows[1].display_name, "Zebra Model");
    }

    #[test]
    fn get_by_id_finds_row() {
        let conn = open_in_memory().unwrap();
        let m = make_model("org/repo:find.gguf", "sha_find");
        insert(&conn, &m).unwrap();

        let found = get(&conn, "org/repo:find.gguf").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().sha256, "sha_find");
    }

    #[test]
    fn get_by_id_returns_none_for_missing() {
        let conn = open_in_memory().unwrap();
        let result = get(&conn, "does/not:exist.gguf").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn delete_returns_orphaned_blobs() {
        let conn = open_in_memory().unwrap();
        let m = make_model_with_mmproj("org/repo:model.gguf", "sha_w", "sha_mm");
        insert(&conn, &m).unwrap();

        let orphans = delete(&conn, "org/repo:model.gguf").unwrap();
        // Both blobs are now unreferenced.
        assert_eq!(orphans.len(), 2);
        assert!(orphans.contains(&"sha_w".to_string()));
        assert!(orphans.contains(&"sha_mm".to_string()));

        // Row is gone.
        assert!(get(&conn, "org/repo:model.gguf").unwrap().is_none());
    }

    #[test]
    fn delete_returns_orphaned_blobs_no_mmproj() {
        let conn = open_in_memory().unwrap();
        let m = make_model("org/repo:simple.gguf", "sha_only");
        insert(&conn, &m).unwrap();

        let orphans = delete(&conn, "org/repo:simple.gguf").unwrap();
        assert_eq!(orphans, vec!["sha_only".to_string()]);
    }

    #[test]
    fn delete_keeps_shared_blob() {
        let conn = open_in_memory().unwrap();
        // Two models share the same mmproj SHA.
        let m1 = make_model_with_mmproj("org/repo:model1.gguf", "sha_w1", "sha_shared_mm");
        let m2 = make_model_with_mmproj("org/repo:model2.gguf", "sha_w2", "sha_shared_mm");
        insert(&conn, &m1).unwrap();
        insert(&conn, &m2).unwrap();

        // Delete model1; its weights blob is orphaned but the shared mmproj is not.
        let orphans = delete(&conn, "org/repo:model1.gguf").unwrap();
        assert_eq!(orphans, vec!["sha_w1".to_string()]);
        assert!(!orphans.contains(&"sha_shared_mm".to_string()));
    }

    #[test]
    fn delete_keeps_sha_referenced_as_mmproj_by_another_row() {
        let conn = open_in_memory().unwrap();
        // model1's weights SHA is also used as model2's mmproj SHA.
        let m1 = make_model("org/repo:model1.gguf", "sha_cross");
        let mut m2 = make_model("org/repo:model2.gguf", "sha_w2");
        m2.mmproj_sha256 = Some("sha_cross".to_string());
        m2.mmproj_file = Some("mmproj.gguf".to_string());
        insert(&conn, &m1).unwrap();
        insert(&conn, &m2).unwrap();

        // Deleting model1: sha_cross still referenced by model2's mmproj column.
        let orphans = delete(&conn, "org/repo:model1.gguf").unwrap();
        assert!(!orphans.contains(&"sha_cross".to_string()));
    }

    #[test]
    fn duplicate_install_upserts() {
        let conn = open_in_memory().unwrap();
        let m1 = make_model("org/repo:model.gguf", "sha_v1");
        insert(&conn, &m1).unwrap();

        // Re-insert with a different display_name and sha256.
        let mut m2 = make_model("org/repo:model.gguf", "sha_v2");
        m2.display_name = "Updated Name".to_string();
        insert(&conn, &m2).unwrap();

        let rows = list(&conn).unwrap();
        assert_eq!(rows.len(), 1, "upsert must not create a second row");
        assert_eq!(rows[0].sha256, "sha_v2");
        assert_eq!(rows[0].display_name, "Updated Name");
    }

    #[test]
    fn delete_nonexistent_returns_empty() {
        let conn = open_in_memory().unwrap();
        let orphans = delete(&conn, "does/not:exist.gguf").unwrap();
        assert!(orphans.is_empty());
    }

    #[test]
    fn vision_and_thinking_flags_roundtrip() {
        let conn = open_in_memory().unwrap();
        let m = InstalledModel {
            vision: true,
            thinking: true,
            ..make_model("org/repo:vt.gguf", "sha_vt")
        };
        insert(&conn, &m).unwrap();

        let found = get(&conn, "org/repo:vt.gguf").unwrap().unwrap();
        assert!(found.vision);
        assert!(found.thinking);
    }

    #[test]
    fn size_bytes_roundtrip_large_value() {
        let conn = open_in_memory().unwrap();
        let m = InstalledModel {
            size_bytes: u32::MAX as u64 + 1,
            ..make_model("org/repo:big.gguf", "sha_big")
        };
        insert(&conn, &m).unwrap();

        let found = get(&conn, "org/repo:big.gguf").unwrap().unwrap();
        assert_eq!(found.size_bytes, u32::MAX as u64 + 1);
    }

    // ── SQL-error paths (the `?` Err arms) ──────────────────────────────────
    // Each test drops the table so the next call hits a real SQL error, which
    // exercises the `?` propagation branches that cannot be reached against a
    // healthy schema.

    #[test]
    fn insert_propagates_sql_error_when_table_absent() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("DROP TABLE installed_models;").unwrap();
        let m = make_model("x:y.gguf", "sha");
        assert!(insert(&conn, &m).is_err());
    }

    #[test]
    fn list_propagates_sql_error_when_table_absent() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("DROP TABLE installed_models;").unwrap();
        assert!(list(&conn).is_err());
    }

    #[test]
    fn delete_propagates_sql_error_when_table_absent() {
        let conn = open_in_memory().unwrap();
        // Insert a row first so the SELECT snapshot finds it, then drop the
        // table so the DELETE statement hits a SQL error.
        let m = make_model("x:y.gguf", "sha_d");
        insert(&conn, &m).unwrap();
        conn.execute_batch("DROP TABLE installed_models;").unwrap();
        // The snapshot SELECT now fails because the table is gone.
        assert!(delete(&conn, "x:y.gguf").is_err());
    }

    #[test]
    fn delete_propagates_sql_error_on_delete_statement() {
        // Insert a row then replace the table with a view so the snapshot
        // SELECT still works (returning the row) but the DELETE statement
        // fails because the target is now a view, not a base table.
        // This exercises the `?` Err arm on the DELETE execute call.
        let conn = open_in_memory().unwrap();
        let m = make_model("x:y.gguf", "sha_rd");
        insert(&conn, &m).unwrap();
        // Rename the real table and create a non-updatable view in its place.
        conn.execute_batch(
            "ALTER TABLE installed_models RENAME TO installed_models_real; \
             CREATE VIEW installed_models AS SELECT * FROM installed_models_real;",
        )
        .unwrap();
        // snapshot SELECT works (reads through the view); DELETE on a view fails.
        assert!(delete(&conn, "x:y.gguf").is_err());
    }
}
