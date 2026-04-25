//! Tests for `settings_commands`.
//!
//! Coverage strategy: every code path in `set_config_field`'s coercion +
//! patching pipeline is exercised against a temp-dir TOML file. The Tauri
//! command wrappers are coverage-excluded; their happy paths are exercised
//! indirectly through the testable internals (`patch_document`,
//! `coerce_json_to_toml`, `read_document`).

use std::path::PathBuf;

use serde_json::json;
use toml_edit::DocumentMut;

use super::{
    coerce_json_to_toml, is_allowed_field, is_allowed_section, json_type_name, patch_document,
    read_document,
};
use crate::config::defaults::{ALLOWED_FIELDS, ALLOWED_SECTIONS};
use crate::config::ConfigError;

// ─── Test fixtures ──────────────────────────────────────────────────────────

const SAMPLE_CONFIG: &str = r#"# Top-level comment preserved across GUI patches.
[inference]
available = ["gemma4:e2b"]
ollama_url = "http://127.0.0.1:11434"

[prompt]
# Custom persona note from the user
system = ""

[window]
overlay_width = 600.0
collapsed_height = 80.0
max_chat_height = 648.0
hide_commit_delay_ms = 350

[quote]
max_display_lines = 4
max_display_chars = 300
max_context_length = 4096

[search]
searxng_url = "http://127.0.0.1:25017"
reader_url = "http://127.0.0.1:25018"
max_iterations = 3
top_k_urls = 10
searxng_max_results = 10
search_timeout_s = 20
reader_per_url_timeout_s = 10
reader_batch_timeout_s = 30
judge_timeout_s = 30
router_timeout_s = 45
"#;

fn parse_sample() -> DocumentMut {
    SAMPLE_CONFIG.parse().expect("sample config parses")
}

// ─── ALLOWED_FIELDS / ALLOWED_SECTIONS ──────────────────────────────────────

#[test]
fn allowed_fields_count_matches_schema_field_count() {
    // Hand-counted from `AppConfig`: inference(1) + prompt(1) + window(4) + quote(3)
    // + search(10) = 19 tunable fields. The active model slug lives in the
    // SQLite app_config table via ActiveModelState, not in TOML. If this
    // assertion fails, the schema has drifted from the allowlist and someone
    // added a field without extending ALLOWED_FIELDS.
    assert_eq!(ALLOWED_FIELDS.len(), 19);
}

#[test]
fn allowed_sections_match_app_config_top_level_keys() {
    assert_eq!(
        ALLOWED_SECTIONS,
        &["inference", "prompt", "window", "quote", "search"]
    );
}

#[test]
fn is_allowed_field_accepts_known_pair() {
    assert!(is_allowed_field("inference", "ollama_url"));
    assert!(is_allowed_field("search", "router_timeout_s"));
}

#[test]
fn is_allowed_field_rejects_unknown_pair() {
    assert!(!is_allowed_field("inference", "secret_api_key"));
    assert!(!is_allowed_field("activation", "hotkey"));
}

#[test]
fn is_allowed_section_accepts_known() {
    for section in ALLOWED_SECTIONS {
        assert!(is_allowed_section(section));
    }
}

#[test]
fn is_allowed_section_rejects_unknown() {
    assert!(!is_allowed_section("activation"));
    assert!(!is_allowed_section(""));
}

// ─── coerce_json_to_toml ────────────────────────────────────────────────────

#[test]
fn coerce_integer_accepts_json_integer() {
    let doc = parse_sample();
    let item = doc
        .get("window")
        .unwrap()
        .get("hide_commit_delay_ms")
        .unwrap();
    let coerced = coerce_json_to_toml(item, json!(500), "window", "hide_commit_delay_ms").unwrap();
    assert_eq!(coerced.as_integer(), Some(500));
}

#[test]
fn coerce_integer_accepts_whole_float() {
    let doc = parse_sample();
    let item = doc
        .get("window")
        .unwrap()
        .get("hide_commit_delay_ms")
        .unwrap();
    let coerced =
        coerce_json_to_toml(item, json!(500.0), "window", "hide_commit_delay_ms").unwrap();
    assert_eq!(coerced.as_integer(), Some(500));
}

#[test]
fn coerce_integer_rejects_fractional_float() {
    let doc = parse_sample();
    let item = doc
        .get("window")
        .unwrap()
        .get("hide_commit_delay_ms")
        .unwrap();
    let err =
        coerce_json_to_toml(item, json!(500.5), "window", "hide_commit_delay_ms").unwrap_err();
    matches_type_mismatch(&err, "window", "hide_commit_delay_ms");
}

#[test]
fn coerce_integer_rejects_string() {
    let doc = parse_sample();
    let item = doc
        .get("window")
        .unwrap()
        .get("hide_commit_delay_ms")
        .unwrap();
    let err =
        coerce_json_to_toml(item, json!("nope"), "window", "hide_commit_delay_ms").unwrap_err();
    matches_type_mismatch(&err, "window", "hide_commit_delay_ms");
}

#[test]
fn coerce_float_accepts_float_and_integer() {
    let doc = parse_sample();
    let item = doc.get("window").unwrap().get("overlay_width").unwrap();
    let from_float = coerce_json_to_toml(item, json!(720.5), "window", "overlay_width").unwrap();
    assert_eq!(from_float.as_float(), Some(720.5));

    let from_int = coerce_json_to_toml(item, json!(720), "window", "overlay_width").unwrap();
    assert_eq!(from_int.as_float(), Some(720.0));
}

#[test]
fn coerce_float_rejects_string() {
    let doc = parse_sample();
    let item = doc.get("window").unwrap().get("overlay_width").unwrap();
    let err = coerce_json_to_toml(item, json!("720"), "window", "overlay_width").unwrap_err();
    matches_type_mismatch(&err, "window", "overlay_width");
}

#[test]
fn coerce_string_accepts_json_string() {
    let doc = parse_sample();
    let item = doc.get("inference").unwrap().get("ollama_url").unwrap();
    let coerced = coerce_json_to_toml(
        item,
        json!("http://10.0.0.1:11434"),
        "inference",
        "ollama_url",
    )
    .unwrap();
    assert_eq!(coerced.as_str(), Some("http://10.0.0.1:11434"));
}

#[test]
fn coerce_string_rejects_integer() {
    let doc = parse_sample();
    let item = doc.get("inference").unwrap().get("ollama_url").unwrap();
    let err = coerce_json_to_toml(item, json!(42), "inference", "ollama_url").unwrap_err();
    matches_type_mismatch(&err, "inference", "ollama_url");
}

#[test]
fn coerce_array_accepts_string_array() {
    let doc = parse_sample();
    let item = doc.get("inference").unwrap().get("available").unwrap();
    let coerced = coerce_json_to_toml(
        item,
        json!(["gemma4:e2b", "qwen3:8b"]),
        "inference",
        "available",
    )
    .unwrap();
    let arr = coerced.as_array().expect("array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr.get(0).and_then(|v| v.as_str()), Some("gemma4:e2b"));
    assert_eq!(arr.get(1).and_then(|v| v.as_str()), Some("qwen3:8b"));
}

#[test]
fn coerce_array_rejects_non_string_element() {
    let doc = parse_sample();
    let item = doc.get("inference").unwrap().get("available").unwrap();
    let err =
        coerce_json_to_toml(item, json!(["a", 42]), "inference", "available").unwrap_err();
    matches_type_mismatch(&err, "inference", "available");
}

#[test]
fn coerce_array_rejects_non_array_value() {
    let doc = parse_sample();
    let item = doc.get("inference").unwrap().get("available").unwrap();
    let err = coerce_json_to_toml(item, json!("nope"), "inference", "available").unwrap_err();
    matches_type_mismatch(&err, "inference", "available");
}

#[test]
fn coerce_rejects_when_existing_is_not_primitive() {
    // Construct an Item that is not a primitive value (e.g. a sub-table) to
    // exercise the early-return error branch.
    let mut doc = DocumentMut::new();
    doc.insert("section", toml_edit::Item::Table(toml_edit::Table::new()));
    let table_item = doc.get("section").unwrap();
    let err = coerce_json_to_toml(table_item, json!("v"), "section", "key").unwrap_err();
    matches_type_mismatch(&err, "section", "key");
}

// ─── patch_document ─────────────────────────────────────────────────────────

#[test]
fn patch_document_overwrites_existing_field() {
    let mut doc = parse_sample();
    patch_document(
        &mut doc,
        "inference",
        "ollama_url",
        json!("http://1.2.3.4:11434"),
    )
    .unwrap();
    let new_url = doc
        .get("inference")
        .unwrap()
        .get("ollama_url")
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(new_url, "http://1.2.3.4:11434");
}

#[test]
fn patch_document_preserves_top_level_comment() {
    let mut doc = parse_sample();
    patch_document(
        &mut doc,
        "inference",
        "ollama_url",
        json!("http://1.2.3.4:11434"),
    )
    .unwrap();
    let serialized = doc.to_string();
    assert!(
        serialized.contains("# Top-level comment preserved"),
        "comment was lost: {serialized}"
    );
    assert!(
        serialized.contains("# Custom persona note"),
        "section comment was lost: {serialized}"
    );
}

#[test]
fn patch_document_unknown_section_errors() {
    let mut doc = parse_sample();
    let err = patch_document(&mut doc, "activation", "hotkey", json!("ctrl+ctrl")).unwrap_err();
    match err {
        ConfigError::UnknownSection { section } => assert_eq!(section, "activation"),
        other => panic!("expected UnknownSection, got {other:?}"),
    }
}

#[test]
fn patch_document_unknown_field_errors() {
    let mut doc = parse_sample();
    let err =
        patch_document(&mut doc, "inference", "secret_api_key", json!("hunter2")).unwrap_err();
    match err {
        ConfigError::UnknownField { section, key } => {
            assert_eq!(section, "inference");
            assert_eq!(key, "secret_api_key");
        }
        other => panic!("expected UnknownField, got {other:?}"),
    }
}

// ─── read_document ──────────────────────────────────────────────────────────

#[test]
fn read_document_parses_existing_file() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();
    let doc = read_document(&path).unwrap();
    assert!(doc.get("inference").is_some());
}

#[test]
fn read_document_io_error_for_missing_file() {
    let dir = tempdir();
    let path = dir.join("missing.toml");
    let err = read_document(&path).unwrap_err();
    matches!(err, ConfigError::IoError { .. });
}

#[test]
fn read_document_parse_error_for_invalid_toml() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, "this is not = valid toml [oops\n").unwrap();
    let err = read_document(&path).unwrap_err();
    match err {
        ConfigError::Parse { path: p, .. } => assert_eq!(p, path),
        other => panic!("expected Parse, got {other:?}"),
    }
}

// ─── consume_corrupt_marker ─────────────────────────────────────────────────

#[test]
fn consume_corrupt_marker_returns_none_when_absent() {
    let dir = tempdir();
    assert!(crate::config::consume_corrupt_marker(&dir).is_none());
}

#[test]
fn consume_corrupt_marker_reads_and_deletes() {
    let dir = tempdir();
    let marker_path = dir.join(crate::config::CORRUPT_MARKER_FILE_NAME);
    std::fs::write(&marker_path, "/tmp/old-config.toml.corrupt-1234\n1234\n").unwrap();
    let marker = crate::config::consume_corrupt_marker(&dir).expect("marker present");
    assert_eq!(marker.path, "/tmp/old-config.toml.corrupt-1234");
    assert_eq!(marker.ts, 1234);
    assert!(
        !marker_path.exists(),
        "marker should be deleted after consume"
    );
}

#[test]
fn consume_corrupt_marker_rejects_malformed_payload() {
    let dir = tempdir();
    let marker_path = dir.join(crate::config::CORRUPT_MARKER_FILE_NAME);
    std::fs::write(&marker_path, "only-one-line\n").unwrap();
    assert!(crate::config::consume_corrupt_marker(&dir).is_none());
}

#[test]
fn consume_corrupt_marker_rejects_empty_path() {
    let dir = tempdir();
    let marker_path = dir.join(crate::config::CORRUPT_MARKER_FILE_NAME);
    std::fs::write(&marker_path, "\n1234\n").unwrap();
    assert!(crate::config::consume_corrupt_marker(&dir).is_none());
}

#[test]
fn consume_corrupt_marker_rejects_unparseable_ts() {
    let dir = tempdir();
    let marker_path = dir.join(crate::config::CORRUPT_MARKER_FILE_NAME);
    std::fs::write(&marker_path, "/tmp/x\nnot-a-number\n").unwrap();
    assert!(crate::config::consume_corrupt_marker(&dir).is_none());
}

// ─── json_type_name ─────────────────────────────────────────────────────────

#[test]
fn json_type_name_covers_every_variant() {
    assert_eq!(json_type_name(&json!(null)), "null");
    assert_eq!(json_type_name(&json!(true)), "boolean");
    assert_eq!(json_type_name(&json!(42)), "integer");
    assert_eq!(json_type_name(&json!(42u64)), "integer");
    assert_eq!(json_type_name(&json!(2.5_f64)), "float");
    assert_eq!(json_type_name(&json!("s")), "string");
    assert_eq!(json_type_name(&json!([1, 2])), "array");
    assert_eq!(json_type_name(&json!({"a": 1})), "object");
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn matches_type_mismatch(err: &ConfigError, section: &str, key: &str) {
    match err {
        ConfigError::TypeMismatch {
            section: s, key: k, ..
        } => {
            assert_eq!(s, section);
            assert_eq!(k, key);
        }
        other => panic!("expected TypeMismatch on {section}.{key}, got {other:?}"),
    }
}

/// Unique per-test directory under the OS temp dir so concurrent tests do not
/// collide. Cleanup is the OS's responsibility (these are `cargo test` runs,
/// not production code).
fn tempdir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("thuki-settings-cmd-{pid}-{nanos}-{n}"));
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}
