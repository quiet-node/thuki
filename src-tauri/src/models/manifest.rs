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

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use serde::Serialize;

use super::HfGgufPart;

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
    /// Whether the model's reasoning cannot be turned off (it always reasons).
    /// Set by the reasoning classifier at install (and corrected by the runtime
    /// backstop). For rows written before the column existed the stored value
    /// is `NULL`, read here as `false` and re-classified by the startup heal.
    pub reasoning_always: bool,
    /// Filename of the vision projection blob, if any.
    pub mmproj_file: Option<String>,
    /// SHA-256 hex digest of the mmproj blob, if any.
    pub mmproj_sha256: Option<String>,
    /// Ordered shards of a multi-part (split) GGUF model, empty for an ordinary
    /// single-file model. `file_name`/`sha256`/`size_bytes` above stay the first
    /// shard's (the representative); the full set is needed both to download every
    /// shard and to rebuild the split through the load-time symlink shim.
    pub parts: Vec<HfGgufPart>,
}

/// Inserts or replaces a model row in the manifest. If a row with the same
/// `id` already exists it is replaced in full, so re-downloading a model
/// always produces an up-to-date entry. `created_at` is set to the current
/// Unix second timestamp inside this function.
///
/// Returns the SHA-256 values of the replaced row (weights, mmproj, and every
/// shard of a multi-part model) that are no longer referenced by any row after
/// the replace, mirroring [`delete`]: a re-download whose upstream content
/// changed would otherwise strand the old multi-GB blob forever. The caller is
/// responsible for removing the orphaned blobs from disk. Empty when no row was
/// replaced or every old SHA is still referenced (same content, or shared with
/// another row).
///
/// # Errors
///
/// Returns a `rusqlite::Error` if the underlying SQL execution fails.
pub fn insert(conn: &Connection, model: &InstalledModel) -> SqlResult<Vec<String>> {
    // Snapshot the full SHA set of the row being replaced before it is gone:
    // weights, mmproj, and every shard. Missing any shard here would strand
    // the old shard blobs when a multi-part model's content changes.
    let replaced = snapshot_row_shas(conn, &model.id)?;

    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "INSERT OR REPLACE INTO installed_models \
         (id, display_name, repo, revision, file_name, sha256, size_bytes, \
          quant, vision, thinking, reasoning_always, mmproj_file, mmproj_sha256, parts, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
            model.reasoning_always as i32,
            model.mmproj_file,
            model.mmproj_sha256,
            encode_parts(&model.parts),
            created_at,
        ],
    )?;

    // Of the replaced row's shas, return those no longer referenced anywhere
    // after the replace (the new row's shas count as referenced, so an unchanged
    // sha is never reported).
    orphans_after_mutation(conn, replaced)
}

/// Returns all installed models ordered alphabetically by `display_name`.
///
/// # Errors
///
/// Returns a `rusqlite::Error` if the query fails.
pub fn list(conn: &Connection) -> SqlResult<Vec<InstalledModel>> {
    let mut stmt = conn.prepare(
        "SELECT id, display_name, repo, revision, file_name, sha256, \
                size_bytes, quant, vision, thinking, mmproj_file, mmproj_sha256, \
                reasoning_always, parts \
         FROM installed_models ORDER BY display_name",
    )?;
    let rows = stmt.query_map([], row_to_model)?;
    rows.collect()
}

/// Returns the installed models whose `reasoning_always` is `NULL`: rows
/// written before the column existed, never touched by the classifier. The
/// startup heal re-classifies each from its local blob (or the registry for a
/// curated row) and persists the result via [`update_classification`], so a
/// subsequent call returns an empty list.
pub fn list_unclassified(conn: &Connection) -> SqlResult<Vec<InstalledModel>> {
    let mut stmt = conn.prepare(
        "SELECT id, display_name, repo, revision, file_name, sha256, \
                size_bytes, quant, vision, thinking, mmproj_file, mmproj_sha256, \
                reasoning_always, parts \
         FROM installed_models WHERE reasoning_always IS NULL ORDER BY display_name",
    )?;
    let rows = stmt.query_map([], row_to_model)?;
    rows.collect()
}

/// Persists a reasoning classification onto an existing row: sets both
/// `thinking` and `reasoning_always`. Used by the startup heal to populate a
/// previously-`NULL` row. A no-op (zero rows changed) when `id` is absent.
pub fn update_classification(
    conn: &Connection,
    id: &str,
    thinking: bool,
    reasoning_always: bool,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE installed_models SET thinking = ?2, reasoning_always = ?3 WHERE id = ?1",
        params![id, thinking as i32, reasoning_always as i32],
    )?;
    Ok(())
}

/// Marks a model as always-reasoning from observed runtime behavior (the
/// backstop saw reasoning stream while reasoning was requested off). Forces
/// both `reasoning_always` and `thinking` true, since a model that always
/// reasons necessarily emits thinking tokens. Idempotent.
pub fn mark_reasoning_always(conn: &Connection, id: &str) -> SqlResult<()> {
    conn.execute(
        "UPDATE installed_models SET reasoning_always = 1, thinking = 1 WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Returns the model with the given `id`, or `None` if not present.
///
/// # Errors
///
/// Returns a `rusqlite::Error` if the query fails.
pub fn get(conn: &Connection, id: &str) -> SqlResult<Option<InstalledModel>> {
    conn.query_row(
        "SELECT id, display_name, repo, revision, file_name, sha256, \
                size_bytes, quant, vision, thinking, mmproj_file, mmproj_sha256, \
                reasoning_always, parts \
         FROM installed_models WHERE id = ?1",
        params![id],
        row_to_model,
    )
    .optional()
}

/// Deletes the model row identified by `id` and returns the SHA-256 values
/// (weights, mmproj, and every shard of a multi-part model) that are no longer
/// referenced by any remaining row.
///
/// A blob SHA is included in the return value only when it is not referenced
/// by any remaining row's `sha256`, `mmproj_sha256`, or `parts` shards. The
/// caller is responsible for removing the orphaned blobs from disk.
///
/// Returns an empty `Vec` if no row with the given `id` exists.
///
/// # Errors
///
/// Returns a `rusqlite::Error` if the delete or the reference-count query fails.
pub fn delete(conn: &Connection, id: &str) -> SqlResult<Vec<String>> {
    // Snapshot the full SHA set of the row being deleted before it is gone.
    let Some(target) = snapshot_row_shas(conn, id)? else {
        return Ok(vec![]);
    };

    conn.execute("DELETE FROM installed_models WHERE id = ?1", params![id])?;

    // Of the deleted row's shas, return those no longer referenced by any
    // remaining row.
    orphans_after_mutation(conn, Some(target))
}

/// Reads the complete set of blob shas a single row references: its weights
/// `sha256`, its `mmproj_sha256` (when present), and every shard sha inside its
/// `parts` JSON. `None` when no row with `id` exists. Deduplicated, so a row
/// whose weights and mmproj (or a shard) share a sha never lists it twice.
fn snapshot_row_shas(conn: &Connection, id: &str) -> SqlResult<Option<Vec<String>>> {
    let row: Option<(String, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT sha256, mmproj_sha256, parts FROM installed_models WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(row.map(|(weights, mmproj, parts)| {
        let mut shas: Vec<String> = vec![weights];
        if let Some(s) = mmproj {
            if !shas.contains(&s) {
                shas.push(s);
            }
        }
        for part in decode_parts(parts.as_deref()) {
            if !shas.contains(&part.sha256) {
                shas.push(part.sha256);
            }
        }
        shas
    }))
}

/// Given the full sha set of a just-removed-or-replaced row, returns those
/// shas no longer referenced by any current row. Computed against the table
/// AFTER the mutation, so a sha the new (replacing) row still carries is never
/// reported. `None` candidates (no row was there) yield an empty `Vec`.
fn orphans_after_mutation(
    conn: &Connection,
    candidates: Option<Vec<String>>,
) -> SqlResult<Vec<String>> {
    let Some(candidates) = candidates else {
        return Ok(vec![]);
    };
    let referenced = referenced_shas(conn)?;
    Ok(candidates
        .into_iter()
        .filter(|sha| !referenced.contains(sha))
        .collect())
}

/// Returns every blob sha referenced by any current `installed_models` row:
/// every `sha256`, every non-NULL `mmproj_sha256`, and every shard sha of a
/// multi-part model. This is the authoritative live-reference set the blob
/// garbage collector checks a removal candidate against. Counting only the two
/// scalar columns (the prior `sha_refcount`) would miss shards and let deleting
/// one multi-part model orphan-delete another model's shard blobs.
///
/// Built from [`list`] so the shard decoding flows through the same
/// `row_to_model` path every read uses, rather than a parallel query.
fn referenced_shas(conn: &Connection) -> SqlResult<HashSet<String>> {
    let mut referenced = HashSet::new();
    for model in list(conn)? {
        referenced.insert(model.sha256);
        if let Some(s) = model.mmproj_sha256 {
            referenced.insert(s);
        }
        for part in model.parts {
            referenced.insert(part.sha256);
        }
    }
    Ok(referenced)
}

/// Encodes a model's shard list for the `parts` column: `None` for a single-file
/// model (stored as SQL NULL), otherwise the JSON array. A serialization failure
/// (not reachable for this plain struct) also collapses to `None` rather than
/// panicking, degrading to single-file semantics instead of failing the install.
fn encode_parts(parts: &[HfGgufPart]) -> Option<String> {
    if parts.is_empty() {
        None
    } else {
        serde_json::to_string(parts).ok()
    }
}

/// Decodes the `parts` column back into a shard list. Never panics: a NULL,
/// empty, or unparseable value yields an empty `Vec` (single-file semantics),
/// so a hand-corrupted row can never crash a read or a refcount scan.
fn decode_parts(raw: Option<&str>) -> Vec<HfGgufPart> {
    match raw {
        Some(s) if !s.is_empty() => serde_json::from_str(s).unwrap_or_default(),
        _ => Vec::new(),
    }
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
        // NULL (a pre-column row) reads as `false`; the startup heal then
        // re-classifies it. A stored 0/1 is the classifier's verdict.
        reasoning_always: row
            .get::<_, Option<i32>>(12)?
            .map(|v| v != 0)
            .unwrap_or(false),
        parts: decode_parts(row.get::<_, Option<String>>(13)?.as_deref()),
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
            reasoning_always: false,
            mmproj_file: None,
            mmproj_sha256: None,
            parts: Vec::new(),
        }
    }

    /// Builds a part list from `(file, sha)` pairs with a fixed dummy size.
    fn parts_of(pairs: &[(&str, &str)]) -> Vec<HfGgufPart> {
        pairs
            .iter()
            .map(|(file, sha)| HfGgufPart {
                file: file.to_string(),
                sha256: sha.to_string(),
                size_bytes: 1_000_000,
            })
            .collect()
    }

    /// A multi-part model whose representative `sha256` is the first shard's and
    /// whose `parts` carry every shard sha (mirroring the real install shape).
    fn make_multipart_model(id: &str, shard_shas: &[&str]) -> InstalledModel {
        let pairs: Vec<(&str, &str)> = shard_shas
            .iter()
            .enumerate()
            .map(|(i, sha)| {
                let file: &str = Box::leak(
                    format!("{id}-{:05}-of-{:05}.gguf", i + 1, shard_shas.len()).into_boxed_str(),
                );
                (file, *sha)
            })
            .collect();
        InstalledModel {
            sha256: shard_shas[0].to_string(),
            parts: parts_of(&pairs),
            ..make_model(id, shard_shas[0])
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
        // A fresh insert replaces nothing, so nothing can be orphaned.
        assert!(insert(&conn, &m1).unwrap().is_empty());

        // Re-insert with a different display_name and sha256: the replaced
        // row's blob is no longer referenced and must be reported.
        let mut m2 = make_model("org/repo:model.gguf", "sha_v2");
        m2.display_name = "Updated Name".to_string();
        let orphans = insert(&conn, &m2).unwrap();
        assert_eq!(orphans, vec!["sha_v1".to_string()]);

        let rows = list(&conn).unwrap();
        assert_eq!(rows.len(), 1, "upsert must not create a second row");
        assert_eq!(rows[0].sha256, "sha_v2");
        assert_eq!(rows[0].display_name, "Updated Name");
    }

    #[test]
    fn reinsert_with_same_shas_reports_no_orphans() {
        let conn = open_in_memory().unwrap();
        let m = make_model_with_mmproj("org/repo:model.gguf", "sha_w", "sha_mm");
        insert(&conn, &m).unwrap();

        // Same content re-installed: the new row still references both SHAs,
        // so neither may be reported for removal.
        let orphans = insert(&conn, &m).unwrap();
        assert!(orphans.is_empty());
        assert_eq!(list(&conn).unwrap().len(), 1);
    }

    #[test]
    fn reinsert_with_changed_shas_reports_old_weights_and_mmproj() {
        let conn = open_in_memory().unwrap();
        let m1 = make_model_with_mmproj("org/repo:model.gguf", "sha_w_old", "sha_mm_old");
        insert(&conn, &m1).unwrap();

        // Upstream content changed: both old blobs are now unreferenced.
        let m2 = make_model_with_mmproj("org/repo:model.gguf", "sha_w_new", "sha_mm_new");
        let orphans = insert(&conn, &m2).unwrap();
        assert_eq!(orphans.len(), 2);
        assert!(orphans.contains(&"sha_w_old".to_string()));
        assert!(orphans.contains(&"sha_mm_old".to_string()));
    }

    #[test]
    fn reinsert_keeps_old_sha_shared_with_another_row() {
        let conn = open_in_memory().unwrap();
        // Two models share the same mmproj SHA.
        let m1 = make_model_with_mmproj("org/repo:model1.gguf", "sha_w1_old", "sha_shared_mm");
        let m2 = make_model_with_mmproj("org/repo:model2.gguf", "sha_w2", "sha_shared_mm");
        insert(&conn, &m1).unwrap();
        insert(&conn, &m2).unwrap();

        // Re-install model1 with changed content: its old weights blob is
        // orphaned, but the shared mmproj is still referenced by model2.
        let replacement =
            make_model_with_mmproj("org/repo:model1.gguf", "sha_w1_new", "sha_mm_new");
        let orphans = insert(&conn, &replacement).unwrap();
        assert_eq!(orphans, vec!["sha_w1_old".to_string()]);
    }

    #[test]
    fn reinsert_dedupes_row_whose_weights_and_mmproj_share_a_sha() {
        let conn = open_in_memory().unwrap();
        // Degenerate row whose weights and mmproj reference the same blob.
        let m1 = make_model_with_mmproj("org/repo:model.gguf", "sha_same", "sha_same");
        insert(&conn, &m1).unwrap();

        let m2 = make_model_with_mmproj("org/repo:model.gguf", "sha_new_w", "sha_new_mm");
        let orphans = insert(&conn, &m2).unwrap();
        assert_eq!(orphans, vec!["sha_same".to_string()]);
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
    fn reasoning_always_flag_roundtrips() {
        let conn = open_in_memory().unwrap();
        let m = InstalledModel {
            thinking: true,
            reasoning_always: true,
            ..make_model("org/repo:ra.gguf", "sha_ra")
        };
        insert(&conn, &m).unwrap();
        let found = get(&conn, "org/repo:ra.gguf").unwrap().unwrap();
        assert!(found.reasoning_always);
    }

    #[test]
    fn fresh_insert_is_not_unclassified() {
        // insert always writes a non-NULL reasoning_always, so a freshly
        // installed model is never picked up by the heal.
        let conn = open_in_memory().unwrap();
        insert(&conn, &make_model("org/repo:fresh.gguf", "sha_f")).unwrap();
        assert!(list_unclassified(&conn).unwrap().is_empty());
    }

    /// Forces a row's `reasoning_always` back to NULL to simulate a row written
    /// before the column existed.
    fn null_out_reasoning(conn: &Connection, id: &str) {
        conn.execute(
            "UPDATE installed_models SET reasoning_always = NULL WHERE id = ?1",
            params![id],
        )
        .unwrap();
    }

    #[test]
    fn null_reasoning_row_is_unclassified_and_reads_false() {
        let conn = open_in_memory().unwrap();
        let m = InstalledModel {
            reasoning_always: true,
            ..make_model("org/repo:legacy.gguf", "sha_l")
        };
        insert(&conn, &m).unwrap();
        null_out_reasoning(&conn, "org/repo:legacy.gguf");

        // NULL reads as false through row_to_model.
        let found = get(&conn, "org/repo:legacy.gguf").unwrap().unwrap();
        assert!(!found.reasoning_always);

        // ...and the row surfaces in the heal list.
        let pending = list_unclassified(&conn).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "org/repo:legacy.gguf");
    }

    #[test]
    fn update_classification_persists_and_clears_unclassified() {
        let conn = open_in_memory().unwrap();
        insert(&conn, &make_model("org/repo:u.gguf", "sha_u")).unwrap();
        null_out_reasoning(&conn, "org/repo:u.gguf");

        update_classification(&conn, "org/repo:u.gguf", true, true).unwrap();

        let found = get(&conn, "org/repo:u.gguf").unwrap().unwrap();
        assert!(found.thinking);
        assert!(found.reasoning_always);
        assert!(list_unclassified(&conn).unwrap().is_empty());
    }

    #[test]
    fn update_classification_can_set_none_class() {
        let conn = open_in_memory().unwrap();
        let m = InstalledModel {
            thinking: true,
            ..make_model("org/repo:n.gguf", "sha_n")
        };
        insert(&conn, &m).unwrap();
        null_out_reasoning(&conn, "org/repo:n.gguf");

        update_classification(&conn, "org/repo:n.gguf", false, false).unwrap();
        let found = get(&conn, "org/repo:n.gguf").unwrap().unwrap();
        assert!(!found.thinking);
        assert!(!found.reasoning_always);
        // No longer NULL, so cleared from the heal list.
        assert!(list_unclassified(&conn).unwrap().is_empty());
    }

    #[test]
    fn mark_reasoning_always_forces_both_flags() {
        let conn = open_in_memory().unwrap();
        insert(&conn, &make_model("org/repo:b.gguf", "sha_b")).unwrap();

        mark_reasoning_always(&conn, "org/repo:b.gguf").unwrap();
        let found = get(&conn, "org/repo:b.gguf").unwrap().unwrap();
        assert!(found.reasoning_always);
        assert!(found.thinking);
    }

    #[test]
    fn list_unclassified_propagates_sql_error_when_table_absent() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("DROP TABLE installed_models;").unwrap();
        assert!(list_unclassified(&conn).is_err());
    }

    #[test]
    fn update_classification_propagates_sql_error_when_table_absent() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("DROP TABLE installed_models;").unwrap();
        assert!(update_classification(&conn, "x:y.gguf", true, true).is_err());
    }

    #[test]
    fn mark_reasoning_always_propagates_sql_error_when_table_absent() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("DROP TABLE installed_models;").unwrap();
        assert!(mark_reasoning_always(&conn, "x:y.gguf").is_err());
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
        // The replaced-row snapshot SELECT is the first statement to fail.
        let conn = open_in_memory().unwrap();
        conn.execute_batch("DROP TABLE installed_models;").unwrap();
        let m = make_model("x:y.gguf", "sha");
        assert!(insert(&conn, &m).is_err());
    }

    #[test]
    fn insert_propagates_sql_error_on_insert_statement() {
        // Replace the table with a non-insertable view so the snapshot
        // SELECT still works but the INSERT OR REPLACE statement fails.
        // This exercises the `?` Err arm on the insert execute call.
        let conn = open_in_memory().unwrap();
        conn.execute_batch(
            "ALTER TABLE installed_models RENAME TO installed_models_real; \
             CREATE VIEW installed_models AS SELECT * FROM installed_models_real;",
        )
        .unwrap();
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

    // ── Multi-part (split) GGUF models ───────────────────────────────────────

    #[test]
    fn multipart_parts_roundtrip_through_insert_and_get() {
        let conn = open_in_memory().unwrap();
        let m = make_multipart_model("org/repo:split", &["sha_p1", "sha_p2", "sha_p3"]);
        insert(&conn, &m).unwrap();

        let found = get(&conn, &m.id).unwrap().unwrap();
        assert_eq!(found.parts.len(), 3);
        assert_eq!(
            found.parts.iter().map(|p| &p.sha256).collect::<Vec<_>>(),
            vec!["sha_p1", "sha_p2", "sha_p3"]
        );
        // The representative sha256 is the first shard's.
        assert_eq!(found.sha256, "sha_p1");
        // The full row, parts included, survives the round trip.
        assert_eq!(found, m);
    }

    #[test]
    fn single_file_model_stores_null_parts_and_reads_empty() {
        let conn = open_in_memory().unwrap();
        insert(&conn, &make_model("org/repo:one.gguf", "sha_one")).unwrap();

        // The `parts` column is SQL NULL for a single-file model.
        let raw: Option<String> = conn
            .query_row(
                "SELECT parts FROM installed_models WHERE id = ?1",
                params!["org/repo:one.gguf"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw, None);
        assert!(get(&conn, "org/repo:one.gguf")
            .unwrap()
            .unwrap()
            .parts
            .is_empty());
    }

    #[test]
    fn deleting_one_multipart_model_orphans_only_its_own_shards() {
        let conn = open_in_memory().unwrap();
        // Two distinct multi-part models with disjoint shard blobs.
        let a = make_multipart_model("org/repo:a", &["sha_a1", "sha_a2"]);
        let b = make_multipart_model("org/repo:b", &["sha_b1", "sha_b2"]);
        insert(&conn, &a).unwrap();
        insert(&conn, &b).unwrap();

        // Deleting A returns exactly A's shard shas; B's shards stay referenced.
        let mut orphans = delete(&conn, &a.id).unwrap();
        orphans.sort();
        assert_eq!(orphans, vec!["sha_a1".to_string(), "sha_a2".to_string()]);

        // B is untouched and still fully present.
        let b_row = get(&conn, &b.id).unwrap().unwrap();
        assert_eq!(b_row.parts.len(), 2);
    }

    #[test]
    fn deleting_a_single_file_model_is_unaffected_by_a_multipart_sibling() {
        let conn = open_in_memory().unwrap();
        insert(
            &conn,
            &make_multipart_model("org/repo:split", &["sha_s1", "sha_s2"]),
        )
        .unwrap();
        insert(&conn, &make_model("org/repo:plain.gguf", "sha_plain")).unwrap();

        let orphans = delete(&conn, "org/repo:plain.gguf").unwrap();
        assert_eq!(orphans, vec!["sha_plain".to_string()]);

        // The split model keeps all its shards.
        let split = get(&conn, "org/repo:split").unwrap().unwrap();
        assert_eq!(split.parts.len(), 2);
    }

    #[test]
    fn deleting_a_multipart_model_keeps_a_shard_shared_with_another_row() {
        let conn = open_in_memory().unwrap();
        // Two split models that happen to share one shard blob (identical content).
        let a = make_multipart_model("org/repo:a", &["sha_shared", "sha_a2"]);
        let b = make_multipart_model("org/repo:b", &["sha_shared", "sha_b2"]);
        insert(&conn, &a).unwrap();
        insert(&conn, &b).unwrap();

        // Deleting A orphans only its private shard; the shared one stays.
        let orphans = delete(&conn, &a.id).unwrap();
        assert_eq!(orphans, vec!["sha_a2".to_string()]);
    }

    #[test]
    fn deleting_a_multipart_model_keeps_a_shard_referenced_as_another_rows_weights() {
        let conn = open_in_memory().unwrap();
        // A split shard sha that is also a single-file model's weights sha.
        let a = make_multipart_model("org/repo:a", &["sha_x", "sha_a2"]);
        insert(&conn, &a).unwrap();
        insert(&conn, &make_model("org/repo:plain.gguf", "sha_x")).unwrap();

        // sha_x is still the plain model's weights, so only sha_a2 orphans.
        let orphans = delete(&conn, &a.id).unwrap();
        assert_eq!(orphans, vec!["sha_a2".to_string()]);
    }

    #[test]
    fn shared_mmproj_stays_referenced_when_a_multipart_owner_is_deleted() {
        let conn = open_in_memory().unwrap();
        // A multi-part model and a single-file model share one mmproj blob.
        let mut split = make_multipart_model("org/repo:split", &["sha_s1", "sha_s2"]);
        split.mmproj_file = Some("mmproj.gguf".to_string());
        split.mmproj_sha256 = Some("sha_mm".to_string());
        let other = make_model_with_mmproj("org/repo:other.gguf", "sha_o", "sha_mm");
        insert(&conn, &split).unwrap();
        insert(&conn, &other).unwrap();

        // Deleting the split model orphans its shards but not the shared mmproj.
        let mut orphans = delete(&conn, &split.id).unwrap();
        orphans.sort();
        assert_eq!(orphans, vec!["sha_s1".to_string(), "sha_s2".to_string()]);
    }

    #[test]
    fn reinstalling_a_multipart_model_orphans_only_truly_unreferenced_old_shards() {
        let conn = open_in_memory().unwrap();
        // Two split models sharing one shard; re-download of A changes its other
        // shard but keeps the shared one (same content) and re-adds a fresh shard.
        let a = make_multipart_model("org/repo:a", &["sha_shared", "sha_a_old"]);
        let b = make_multipart_model("org/repo:b", &["sha_shared", "sha_b2"]);
        insert(&conn, &a).unwrap();
        insert(&conn, &b).unwrap();

        // Re-install A: shared shard unchanged, old private shard replaced.
        let a_new = make_multipart_model("org/repo:a", &["sha_shared", "sha_a_new"]);
        let orphans = insert(&conn, &a_new).unwrap();
        // Only the dropped private shard orphans: the shared shard is still
        // referenced (by both A_new and B), and sha_a_new is the live row.
        assert_eq!(orphans, vec!["sha_a_old".to_string()]);

        let a_row = get(&conn, &a.id).unwrap().unwrap();
        assert_eq!(
            a_row.parts.iter().map(|p| &p.sha256).collect::<Vec<_>>(),
            vec!["sha_shared", "sha_a_new"]
        );
    }

    #[test]
    fn reinstalling_an_identical_multipart_model_orphans_nothing() {
        let conn = open_in_memory().unwrap();
        let m = make_multipart_model("org/repo:split", &["sha_p1", "sha_p2"]);
        insert(&conn, &m).unwrap();

        // Same content re-installed: every shard sha is still referenced.
        let orphans = insert(&conn, &m).unwrap();
        assert!(orphans.is_empty());
    }

    #[test]
    fn encode_parts_is_none_for_empty_and_some_json_otherwise() {
        assert_eq!(encode_parts(&[]), None);
        let json = encode_parts(&parts_of(&[("m-00001-of-00001.gguf", "sha_q")])).unwrap();
        assert!(json.contains("sha_q"));
        assert!(json.contains("m-00001-of-00001.gguf"));
    }

    #[test]
    fn referenced_shas_propagates_sql_error_when_table_absent() {
        // Exercises the `?` error arm on `referenced_shas`'s own query (the
        // post-mutation reference scan), the one SQL path not reached through
        // insert/delete since those fail earlier at their snapshot SELECT.
        let conn = open_in_memory().unwrap();
        conn.execute_batch("DROP TABLE installed_models;").unwrap();
        assert!(referenced_shas(&conn).is_err());
    }

    #[test]
    fn decode_parts_never_panics_on_bad_input() {
        // NULL, empty, and unparseable values all decode to an empty Vec.
        assert!(decode_parts(None).is_empty());
        assert!(decode_parts(Some("")).is_empty());
        assert!(decode_parts(Some("not json")).is_empty());
        // A well-formed array round-trips.
        let encoded = encode_parts(&parts_of(&[("m-00001-of-00002.gguf", "s1")])).unwrap();
        let decoded = decode_parts(Some(&encoded));
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].sha256, "s1");
    }
}
