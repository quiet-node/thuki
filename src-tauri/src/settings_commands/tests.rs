//! Tests for `settings_commands`.
//!
//! Coverage strategy: every code path in `set_config_field`'s coercion +
//! patching pipeline is exercised against a temp-dir TOML file. The Tauri
//! command wrappers are coverage-excluded; their happy paths are exercised
//! indirectly through the testable internals (`patch_document`,
//! `coerce_json_to_toml`, `read_document`).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::json;
use toml_edit::DocumentMut;

use super::{
    add_openai_provider_to_disk, builtin_deactivated, cleanup_provider_secrets, clear_traces_dir,
    coerce_json_to_toml, is_allowed_field, is_allowed_section, is_http_url, json_type_name,
    json_value_to_toml_item, keep_warm_idle_minutes_changed, ollama_deactivated, patch_document,
    prune_traces_for_retention, prune_traces_older_than, read_document,
    remove_openai_provider_from_disk, reset_section_on_disk, trace_enabled_changed,
    trace_retention_days_changed, traces_stats_for, validate_provider_value,
    write_active_provider_to_disk, write_dismissed_memory_fit_added_to_disk,
    write_dismissed_memory_fit_removed_to_disk, write_field_to_disk, write_provider_field_to_disk,
};
use crate::config::defaults::{ALLOWED_FIELDS, ALLOWED_SECTIONS};
use crate::config::{AppConfig, ConfigError};

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
max_chat_height = 648.0

[quote]
max_display_lines = 4
max_display_chars = 300
max_context_length = 4096

[updater]
auto_check = true
check_interval_hours = 24
manifest_url = "https://github.com/quiet-node/thuki/releases/latest/download/latest.json"
"#;

fn parse_sample() -> DocumentMut {
    SAMPLE_CONFIG.parse().expect("sample config parses")
}

/// New-shape config carrying an explicit `[[inference.providers]]` array, used
/// to exercise `write_provider_field_to_disk`.
const PROVIDERS_CONFIG: &str = r#"
[inference]
active_provider = "ollama"
num_ctx = 16384
keep_warm_inactivity_minutes = 0

[[inference.providers]]
id = "builtin"
kind = "builtin"
label = "Built-in"
model = ""

[[inference.providers]]
id = "ollama"
kind = "ollama"
label = "Ollama"
base_url = "http://127.0.0.1:11434"
model = ""
"#;

/// PROVIDERS_CONFIG plus an OpenAI-compatible entry, for the add/remove/update
/// provider tests.
const OPENAI_PROVIDERS_CONFIG: &str = r#"
[inference]
active_provider = "ollama"
num_ctx = 16384
keep_warm_inactivity_minutes = 0

[[inference.providers]]
id = "builtin"
kind = "builtin"
label = "Built-in"
model = ""

[[inference.providers]]
id = "ollama"
kind = "ollama"
label = "Ollama"
base_url = "http://127.0.0.1:11434"
model = ""

[[inference.providers]]
id = "openai"
kind = "openai"
label = "LM Studio"
base_url = "http://127.0.0.1:1234"
model = ""
vision = false
"#;

// ─── ALLOWED_FIELDS / ALLOWED_SECTIONS ──────────────────────────────────────

#[test]
fn allowed_fields_count_matches_schema_field_count() {
    // + behavior(7) + debug(2) + updater(3) = 25 tunable flat fields.
    // The inference section's two flat tunables are `keep_warm_inactivity_minutes`
    // and `num_ctx`; `active_provider` and the `providers` array are NOT flat
    // fields: they are written through the dedicated `set_active_model` /
    // `set_ollama_url` commands, not the generic `set_config_field` path, so they
    // are intentionally absent from ALLOWED_FIELDS. The collapsed bar height and
    // hide-commit delay are baked into the frontend (see `WindowSection` doc)
    // because they have no perceptible effect across their usable range.
    // `prompt.system_customized` is an internal migration flag co-written by
    // set_config_field when prompt.system is saved; it is not directly user-tunable
    // and is intentionally absent from ALLOWED_FIELDS.
    // `behavior.dismissed_memory_fit_models` is an array, not a flat scalar: it is
    // written through the dedicated remember_model_memory_fit / forget_model_memory_fit
    // commands (mirroring the providers array), so it too is intentionally absent
    // from ALLOWED_FIELDS. If this assertion fails, the schema has drifted from the
    // allowlist and someone added a flat field without extending ALLOWED_FIELDS.
    assert_eq!(ALLOWED_FIELDS.len(), 25);
}

#[test]
fn allowed_sections_match_app_config_top_level_keys() {
    assert_eq!(
        ALLOWED_SECTIONS,
        &[
            "inference",
            "prompt",
            "window",
            "quote",
            "behavior",
            "debug",
            "updater"
        ]
    );
}

#[test]
fn is_allowed_field_accepts_known_pair() {
    assert!(is_allowed_field("inference", "num_ctx"));
    assert!(is_allowed_field("updater", "manifest_url"));
    assert!(is_allowed_field("window", "max_images"));
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
    // Unknown section names (including historical `search`) must reject.
    assert!(!is_allowed_section("search"));
}

// ─── coerce_json_to_toml ────────────────────────────────────────────────────

#[test]
fn coerce_integer_accepts_json_integer() {
    let doc = parse_sample();
    let item = doc
        .get("updater")
        .unwrap()
        .get("check_interval_hours")
        .unwrap();
    let coerced = coerce_json_to_toml(item, json!(48), "updater", "check_interval_hours").unwrap();
    assert_eq!(coerced.as_integer(), Some(48));
}

#[test]
fn coerce_integer_accepts_whole_float() {
    let doc = parse_sample();
    let item = doc
        .get("updater")
        .unwrap()
        .get("check_interval_hours")
        .unwrap();
    let coerced =
        coerce_json_to_toml(item, json!(48.0), "updater", "check_interval_hours").unwrap();
    assert_eq!(coerced.as_integer(), Some(48));
}

#[test]
fn coerce_integer_rejects_fractional_float() {
    let doc = parse_sample();
    let item = doc
        .get("updater")
        .unwrap()
        .get("check_interval_hours")
        .unwrap();
    let err =
        coerce_json_to_toml(item, json!(48.5), "updater", "check_interval_hours").unwrap_err();
    matches_type_mismatch(&err, "updater", "check_interval_hours");
}

#[test]
fn coerce_integer_rejects_string() {
    let doc = parse_sample();
    let item = doc
        .get("updater")
        .unwrap()
        .get("check_interval_hours")
        .unwrap();
    let err =
        coerce_json_to_toml(item, json!("nope"), "updater", "check_interval_hours").unwrap_err();
    matches_type_mismatch(&err, "updater", "check_interval_hours");
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
    let err = coerce_json_to_toml(item, json!(["a", 42]), "inference", "available").unwrap_err();
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
fn coerce_boolean_accepts_json_bool() {
    let doc: DocumentMut = "flag = true\n".parse().unwrap();
    let item = doc.get("flag").unwrap();
    let coerced = coerce_json_to_toml(item, json!(false), "section", "flag").unwrap();
    assert_eq!(coerced.as_bool(), Some(false));
}

#[test]
fn coerce_boolean_rejects_string() {
    let doc: DocumentMut = "flag = true\n".parse().unwrap();
    let item = doc.get("flag").unwrap();
    let err = coerce_json_to_toml(item, json!("true"), "section", "flag").unwrap_err();
    matches_type_mismatch(&err, "section", "flag");
}

#[test]
fn coerce_rejects_datetime_field() {
    let doc: DocumentMut = "stamp = 1979-05-27T07:32:00Z\n".parse().unwrap();
    let item = doc.get("stamp").unwrap();
    let err = coerce_json_to_toml(item, json!("nope"), "section", "stamp").unwrap_err();
    matches_type_mismatch(&err, "section", "stamp");
}

#[test]
fn coerce_rejects_inline_table_field() {
    let doc: DocumentMut = "obj = { a = 1 }\n".parse().unwrap();
    let item = doc.get("obj").unwrap();
    let err = coerce_json_to_toml(item, json!("nope"), "section", "obj").unwrap_err();
    matches_type_mismatch(&err, "section", "obj");
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
fn patch_document_inserts_missing_float_field() {
    // Simulate a hand-edited config where `overlay_width` was removed.
    let toml = "[window]\nmax_chat_height = 648.0\n";
    let mut doc: DocumentMut = toml.parse().unwrap();
    patch_document(&mut doc, "window", "overlay_width", json!(800.5)).unwrap();
    let inserted = doc
        .get("window")
        .unwrap()
        .get("overlay_width")
        .unwrap()
        .as_float()
        .expect("float");
    assert!((inserted - 800.5).abs() < f64::EPSILON);
}

#[test]
fn patch_document_inserts_missing_string_field() {
    let toml = "[inference]\navailable = [\"gemma4:e2b\"]\n";
    let mut doc: DocumentMut = toml.parse().unwrap();
    patch_document(
        &mut doc,
        "inference",
        "ollama_url",
        json!("http://10.0.0.1:11434"),
    )
    .unwrap();
    let inserted = doc
        .get("inference")
        .unwrap()
        .get("ollama_url")
        .unwrap()
        .as_str()
        .expect("string");
    assert_eq!(inserted, "http://10.0.0.1:11434");
}

#[test]
fn patch_document_inserts_missing_integer_field() {
    let toml = "[window]\noverlay_width = 600.0\n";
    let mut doc: DocumentMut = toml.parse().unwrap();
    patch_document(&mut doc, "window", "max_images", json!(5)).unwrap();
    let inserted = doc
        .get("window")
        .unwrap()
        .get("max_images")
        .unwrap()
        .as_integer()
        .expect("integer");
    assert_eq!(inserted, 5);
}

#[test]
fn patch_document_inserts_missing_array_field() {
    let toml = "[inference]\nollama_url = \"http://127.0.0.1:11434\"\n";
    let mut doc: DocumentMut = toml.parse().unwrap();
    patch_document(
        &mut doc,
        "inference",
        "available",
        json!(["gemma4:e2b", "qwen3:8b"]),
    )
    .unwrap();
    let arr = doc
        .get("inference")
        .unwrap()
        .get("available")
        .unwrap()
        .as_array()
        .expect("array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr.get(0).and_then(|v| v.as_str()), Some("gemma4:e2b"));
}

#[test]
fn patch_document_insert_rejects_object_for_missing_field() {
    let toml = "[window]\nmax_chat_height = 648.0\n";
    let mut doc: DocumentMut = toml.parse().unwrap();
    let err = patch_document(&mut doc, "window", "overlay_width", json!({"a": 1})).unwrap_err();
    matches_type_mismatch(&err, "window", "overlay_width");
}

#[test]
fn patch_document_insert_rejects_array_with_non_string_for_missing_field() {
    let toml = "[inference]\nollama_url = \"http://127.0.0.1:11434\"\n";
    let mut doc: DocumentMut = toml.parse().unwrap();
    let err = patch_document(&mut doc, "inference", "available", json!(["ok", 42])).unwrap_err();
    matches_type_mismatch(&err, "inference", "available");
}

#[test]
fn patch_document_inserts_missing_float_field_as_float_for_whole_number() {
    // Regression: when an f64-typed field is missing from the user's
    // config.toml (typical for users upgrading past a new field), a save
    // of a whole-number JSON value (e.g. 15) must still land in the doc
    // as TOML Float so that the *next* fractional save (e.g. 15.5) is
    // accepted by `coerce_json_to_toml`'s existing-Float branch. If the
    // missing-key path inferred the type from JSON shape, 15 would be
    // inserted as Integer and 15.5 would be rejected as type mismatch.
    let toml = "[window]\noverlay_width = 600.0\nmax_chat_height = 648.0\nmax_images = 3\n";
    let mut doc: DocumentMut = toml.parse().unwrap();

    patch_document(&mut doc, "window", "text_base_px", json!(15)).unwrap();
    let after_whole = doc
        .get("window")
        .and_then(|s| s.get("text_base_px"))
        .expect("text_base_px present after first save");
    assert!(
        after_whole.as_value().and_then(|v| v.as_float()).is_some(),
        "whole-number save into missing float field should land as TOML Float, got {after_whole:?}",
    );

    // Now the fractional save must succeed against the same doc.
    patch_document(&mut doc, "window", "text_base_px", json!(15.5)).unwrap();
    let after_fractional = doc
        .get("window")
        .and_then(|s| s.get("text_base_px"))
        .and_then(|i| i.as_value())
        .and_then(|v| v.as_float())
        .expect("fractional save preserves Float");
    assert!((after_fractional - 15.5).abs() < f64::EPSILON);
}

#[test]
fn patch_document_heals_legacy_integer_for_schema_float_field() {
    // Regression: a legacy config that already persisted `text_base_px` as
    // a TOML Integer (which is what would happen if the user first saved
    // a whole-number value through an older build that inferred the type
    // from the JSON payload) must accept a subsequent fractional save and
    // rewrite the field as TOML Float. The schema-derived template is now
    // authoritative over the on-disk type, so the file self-heals on the
    // very next save without requiring a migration sweep.
    let toml = "[window]\noverlay_width = 600.0\nmax_chat_height = 648.0\nmax_images = 3\ntext_base_px = 15\n";
    let mut doc: DocumentMut = toml.parse().unwrap();

    patch_document(&mut doc, "window", "text_base_px", json!(15.5)).unwrap();
    let healed = doc
        .get("window")
        .and_then(|s| s.get("text_base_px"))
        .and_then(|i| i.as_value())
        .and_then(|v| v.as_float())
        .expect("fractional save rewrites legacy Integer item as Float");
    assert!((healed - 15.5).abs() < f64::EPSILON);
}

#[test]
fn patch_document_falls_back_to_existing_item_for_unknown_key() {
    // Defense-in-depth: when a key is not in AppConfig::default() (so
    // schema_template_item returns None) but the key already exists in the
    // on-disk doc, patch_document should fall back to the existing item's
    // type for coercion. This branch is normally gated by ALLOWED_FIELDS,
    // but the function keeps the fallback to remain correct at the type
    // boundary if that guard is ever bypassed.
    let toml = "[window]\noverlay_width = 600.0\nlegacy_field = \"hello\"\n";
    let mut doc: DocumentMut = toml.parse().unwrap();
    patch_document(&mut doc, "window", "legacy_field", json!("updated")).unwrap();
    let val = doc
        .get("window")
        .and_then(|s| s.get("legacy_field"))
        .and_then(|i| i.as_value())
        .and_then(|v| v.as_str())
        .expect("legacy_field present after patch");
    assert_eq!(val, "updated");
}

#[test]
fn patch_document_infers_type_for_unknown_key_not_in_doc() {
    // Defense-in-depth: when a key is not in AppConfig::default() AND not
    // present in the on-disk doc, patch_document falls back to JSON type
    // inference via json_value_to_toml_item. This exercises the final else
    // branch in the schema-template / existing-item / inference cascade.
    let toml = "[window]\noverlay_width = 600.0\n";
    let mut doc: DocumentMut = toml.parse().unwrap();
    patch_document(&mut doc, "window", "new_field", json!("value")).unwrap();
    let val = doc
        .get("window")
        .and_then(|s| s.get("new_field"))
        .and_then(|i| i.as_value())
        .and_then(|v| v.as_str())
        .expect("new_field inserted after patch");
    assert_eq!(val, "value");
}

// ─── json_value_to_toml_item ─────────────────────────────────────────────────

#[test]
fn json_value_to_toml_item_inserts_bool() {
    let item = json_value_to_toml_item(json!(true), "s", "k").unwrap();
    assert_eq!(item.as_bool(), Some(true));
}

#[test]
fn json_value_to_toml_item_inserts_integer_as_toml_integer() {
    let item = json_value_to_toml_item(json!(42), "s", "k").unwrap();
    assert_eq!(item.as_integer(), Some(42));
}

#[test]
fn json_value_to_toml_item_inserts_float_as_toml_float() {
    // json!(3.14) has as_i64() == None, so the f64 branch is taken.
    let item = json_value_to_toml_item(json!(3.14), "s", "k").unwrap();
    let v = item.as_float().expect("should be float");
    assert!((v - 3.14).abs() < f64::EPSILON);
}

#[test]
fn json_value_to_toml_item_rejects_null() {
    let err = json_value_to_toml_item(json!(null), "sec", "key").unwrap_err();
    matches_type_mismatch(&err, "sec", "key");
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

// ─── write_field_to_disk ────────────────────────────────────────────────────

#[test]
fn write_field_to_disk_persists_and_returns_resolved_config() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    let resolved = write_field_to_disk(
        &path,
        "updater",
        "manifest_url",
        json!("https://example.com/latest.json"),
    )
    .unwrap();
    assert_eq!(
        resolved.updater.manifest_url,
        "https://example.com/latest.json"
    );

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("https://example.com/latest.json"));
}

#[test]
fn write_field_to_disk_creates_section_absent_from_older_file() {
    // Regression: a config.toml seeded before the [behavior] section was added
    // to the schema has no [behavior] table (SAMPLE_CONFIG reproduces this
    // older-file shape). Toggling behavior.auto_replace must create the section
    // rather than fail with UnknownSection; otherwise the setting can never be
    // turned on for any user whose config predates the section.
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    let resolved = write_field_to_disk(&path, "behavior", "auto_replace", json!(true)).unwrap();
    assert!(resolved.behavior.auto_replace);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("[behavior]"));
    assert!(on_disk.contains("auto_replace = true"));
}

// ─── dismissed_memory_fit_models add / remove ───────────────────────────────

/// A valid 64-char lowercase-hex weights SHA from a single hex nibble.
fn fit_sha(nibble: char) -> String {
    std::iter::repeat(nibble).take(64).collect()
}

#[test]
fn write_dismissed_added_persists_and_creates_section() {
    // SAMPLE_CONFIG has no [behavior] table: adding must materialize it.
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    let resolved = write_dismissed_memory_fit_added_to_disk(&path, &fit_sha('a')).unwrap();
    assert_eq!(
        resolved.behavior.dismissed_memory_fit_models,
        vec![fit_sha('a')]
    );

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("[behavior]"));
    assert!(on_disk.contains(&fit_sha('a')));
}

#[test]
fn write_dismissed_added_is_idempotent_and_appends_new() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    write_dismissed_memory_fit_added_to_disk(&path, &fit_sha('a')).unwrap();
    // Re-adding the same sha does not duplicate it.
    let again = write_dismissed_memory_fit_added_to_disk(&path, &fit_sha('a')).unwrap();
    assert_eq!(
        again.behavior.dismissed_memory_fit_models,
        vec![fit_sha('a')]
    );
    // A different sha appends.
    let two = write_dismissed_memory_fit_added_to_disk(&path, &fit_sha('b')).unwrap();
    assert_eq!(
        two.behavior.dismissed_memory_fit_models,
        vec![fit_sha('a'), fit_sha('b')]
    );
}

#[test]
fn write_dismissed_removed_deletes_by_sha() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        format!(
            "[behavior]\ndismissed_memory_fit_models = [\"{}\", \"{}\"]\n",
            fit_sha('a'),
            fit_sha('b')
        ),
    )
    .unwrap();

    let resolved = write_dismissed_memory_fit_removed_to_disk(&path, &fit_sha('a')).unwrap();
    assert_eq!(
        resolved.behavior.dismissed_memory_fit_models,
        vec![fit_sha('b')]
    );

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(!on_disk.contains(&fit_sha('a')));
    assert!(on_disk.contains(&fit_sha('b')));
}

#[test]
fn write_dismissed_removed_on_absent_sha_is_noop() {
    // Removing a sha the list never had (orphan-safe) leaves the file valid and
    // the list unchanged; the [behavior] section is materialized either way.
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    let resolved = write_dismissed_memory_fit_removed_to_disk(&path, &fit_sha('c')).unwrap();
    assert!(resolved.behavior.dismissed_memory_fit_models.is_empty());
}

#[test]
fn write_dismissed_added_propagates_io_error_when_parent_dir_is_readonly() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    // Read-only directory: the existing file reads, but the temp file for the
    // atomic rename swap cannot be created, so the write fails.
    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&dir, perms.clone()).unwrap();

    let err = write_dismissed_memory_fit_added_to_disk(&path, &fit_sha('a')).unwrap_err();

    let mut restore = perms;
    restore.set_mode(0o700);
    std::fs::set_permissions(&dir, restore).unwrap();

    matches!(err, ConfigError::IoError { .. });
}

#[test]
fn write_dismissed_removed_propagates_io_error_when_parent_dir_is_readonly() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        format!(
            "[behavior]\ndismissed_memory_fit_models = [\"{}\"]\n",
            fit_sha('a')
        ),
    )
    .unwrap();

    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&dir, perms.clone()).unwrap();

    let err = write_dismissed_memory_fit_removed_to_disk(&path, &fit_sha('a')).unwrap_err();

    let mut restore = perms;
    restore.set_mode(0o700);
    std::fs::set_permissions(&dir, restore).unwrap();

    matches!(err, ConfigError::IoError { .. });
}

#[test]
fn write_field_to_disk_writing_prompt_system_co_writes_customized_flag() {
    // Saving prompt.system must atomically set system_customized=true so a
    // subsequent boot does not mistake an intentional clear for the legacy
    // empty-default and restore the built-in persona.
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    let resolved = write_field_to_disk(&path, "prompt", "system", json!("")).unwrap();
    assert!(resolved.prompt.system_customized);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("system_customized = true"));
}

#[test]
fn write_field_to_disk_writing_prompt_system_preserves_customized_flag_for_non_empty() {
    // Saving a non-empty system prompt also sets system_customized=true.
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    let resolved =
        write_field_to_disk(&path, "prompt", "system", json!("You are a custom AI.")).unwrap();
    assert!(resolved.prompt.system_customized);
    assert_eq!(resolved.prompt.system, "You are a custom AI.");
}

#[test]
fn write_field_to_disk_rejects_unknown_section() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();
    let err = write_field_to_disk(&path, "activation", "hotkey", json!("ctrl+ctrl")).unwrap_err();
    match err {
        ConfigError::UnknownSection { section } => assert_eq!(section, "activation"),
        other => panic!("expected UnknownSection, got {other:?}"),
    }
}

#[test]
fn write_field_to_disk_rejects_unknown_field() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();
    let err =
        write_field_to_disk(&path, "inference", "secret_api_key", json!("hunter2")).unwrap_err();
    match err {
        ConfigError::UnknownField { section, key } => {
            assert_eq!(section, "inference");
            assert_eq!(key, "secret_api_key");
        }
        other => panic!("expected UnknownField, got {other:?}"),
    }
}

#[test]
fn write_field_to_disk_propagates_read_error_for_missing_file() {
    let dir = tempdir();
    let path = dir.join("missing.toml");
    let err = write_field_to_disk(&path, "updater", "manifest_url", json!("http://x")).unwrap_err();
    matches!(err, ConfigError::IoError { .. });
}

#[test]
fn write_field_to_disk_propagates_patch_error_for_type_mismatch() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();
    let err = write_field_to_disk(&path, "updater", "manifest_url", json!(42)).unwrap_err();
    matches_type_mismatch(&err, "updater", "manifest_url");
}

#[cfg(unix)]
#[test]
fn write_field_to_disk_propagates_io_error_when_parent_dir_is_readonly() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    // Read-only directory: atomic_write_bytes can read the existing file but
    // cannot create the temp file alongside it for the rename swap.
    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&dir, perms.clone()).unwrap();

    let err = write_field_to_disk(
        &path,
        "updater",
        "manifest_url",
        json!("https://example.com/latest2.json"),
    )
    .unwrap_err();

    // Restore writability so the OS can clean up the tempdir later.
    let mut restore = perms;
    restore.set_mode(0o700);
    std::fs::set_permissions(&dir, restore).unwrap();

    matches!(err, ConfigError::IoError { .. });
}

// ─── write_provider_field_to_disk ───────────────────────────────────────────

#[test]
fn write_provider_field_patches_base_url_and_preserves_builtin() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();

    let resolved =
        write_provider_field_to_disk(&path, "ollama", "base_url", "http://10.0.0.2:11434").unwrap();
    let ollama = resolved
        .inference
        .providers
        .iter()
        .find(|p| p.id == "ollama")
        .unwrap();
    assert_eq!(ollama.base_url, "http://10.0.0.2:11434");
    assert!(resolved
        .inference
        .providers
        .iter()
        .any(|p| p.id == "builtin"));

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("http://10.0.0.2:11434"));
}

#[test]
fn write_provider_field_patches_model() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();

    let resolved = write_provider_field_to_disk(&path, "ollama", "model", "llama3.1:8b").unwrap();
    let ollama = resolved
        .inference
        .providers
        .iter()
        .find(|p| p.id == "ollama")
        .unwrap();
    assert_eq!(ollama.model, "llama3.1:8b");
}

#[test]
fn write_provider_field_rejects_unknown_field() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();
    let err = write_provider_field_to_disk(&path, "ollama", "id", "x").unwrap_err();
    match err {
        ConfigError::UnknownField { key, .. } => assert_eq!(key, "id"),
        other => panic!("expected UnknownField, got {other:?}"),
    }
}

#[test]
fn write_provider_field_patches_label() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();

    let resolved = write_provider_field_to_disk(&path, "ollama", "label", "  My Ollama  ").unwrap();
    let ollama = resolved
        .inference
        .providers
        .iter()
        .find(|p| p.id == "ollama")
        .unwrap();
    assert_eq!(ollama.label, "My Ollama");
}

#[test]
fn write_provider_field_heals_empty_openai_label_to_default() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, OPENAI_PROVIDERS_CONFIG).unwrap();

    let resolved = write_provider_field_to_disk(&path, "openai", "label", "   ").unwrap();
    let openai = resolved
        .inference
        .providers
        .iter()
        .find(|p| p.id == "openai")
        .unwrap();
    assert_eq!(openai.label, crate::config::defaults::DEFAULT_OPENAI_LABEL);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains(crate::config::defaults::DEFAULT_OPENAI_LABEL));
}

#[test]
fn validate_provider_value_heals_only_empty_openai_labels() {
    // Non-empty labels trim for every kind.
    let item = validate_provider_value("openai", "label", "  Jan  ").unwrap();
    assert_eq!(item.as_str(), Some("Jan"));
    // A trimmed-empty label on a non-openai kind is not healed.
    let item = validate_provider_value("ollama", "label", "   ").unwrap();
    assert_eq!(item.as_str(), Some(""));
    // A trimmed-empty label on the openai kind heals to the default.
    let item = validate_provider_value("openai", "label", "").unwrap();
    assert_eq!(
        item.as_str(),
        Some(crate::config::defaults::DEFAULT_OPENAI_LABEL)
    );
}

#[test]
fn write_provider_field_patches_vision_as_boolean() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, OPENAI_PROVIDERS_CONFIG).unwrap();

    let resolved = write_provider_field_to_disk(&path, "openai", "vision", "true").unwrap();
    let openai = resolved
        .inference
        .providers
        .iter()
        .find(|p| p.id == "openai")
        .unwrap();
    assert!(openai.vision);

    // Stored as a real TOML boolean, not the string "true".
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("vision = true"));

    let resolved = write_provider_field_to_disk(&path, "openai", "vision", "false").unwrap();
    let openai = resolved
        .inference
        .providers
        .iter()
        .find(|p| p.id == "openai")
        .unwrap();
    assert!(!openai.vision);
}

#[test]
fn write_provider_field_rejects_malformed_vision_value() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, OPENAI_PROVIDERS_CONFIG).unwrap();
    let err = write_provider_field_to_disk(&path, "openai", "vision", "yes").unwrap_err();
    match err {
        ConfigError::TypeMismatch { key, .. } => assert_eq!(key, "vision"),
        other => panic!("expected TypeMismatch, got {other:?}"),
    }
}

#[test]
fn write_provider_field_rejects_non_http_base_url() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();
    let err = write_provider_field_to_disk(&path, "ollama", "base_url", "ftp://x").unwrap_err();
    match err {
        ConfigError::TypeMismatch { key, message, .. } => {
            assert_eq!(key, "base_url");
            assert!(message.contains("http://"));
        }
        other => panic!("expected TypeMismatch, got {other:?}"),
    }
}

#[test]
fn write_provider_field_rejects_builtin_base_url() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();
    let err =
        write_provider_field_to_disk(&path, "builtin", "base_url", "http://10.0.0.1").unwrap_err();
    match err {
        ConfigError::TypeMismatch { message, .. } => {
            assert!(message.contains("built-in"));
        }
        other => panic!("expected TypeMismatch, got {other:?}"),
    }
}

#[test]
fn validate_provider_value_rejects_field_outside_allowlist() {
    // The wrapper gates the field name first, so this arm is only reachable
    // by calling the helper directly; cover it here.
    let err = validate_provider_value("ollama", "kind", "x").unwrap_err();
    match err {
        ConfigError::UnknownField { key, .. } => assert_eq!(key, "kind"),
        other => panic!("expected UnknownField, got {other:?}"),
    }
}

// ─── is_http_url ─────────────────────────────────────────────────────────────

#[test]
fn is_http_url_accepts_http_and_https_with_surrounding_whitespace() {
    assert!(is_http_url("http://127.0.0.1:1234"));
    assert!(is_http_url("https://example.com/v1"));
    assert!(is_http_url("  http://host  "));
}

#[test]
fn is_http_url_rejects_other_schemes_and_empty() {
    assert!(!is_http_url(""));
    assert!(!is_http_url("   "));
    assert!(!is_http_url("ftp://host"));
    assert!(!is_http_url("127.0.0.1:1234"));
}

#[test]
fn write_provider_field_rejects_unknown_provider() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();
    let err = write_provider_field_to_disk(&path, "ghost", "model", "x").unwrap_err();
    match err {
        ConfigError::UnknownField { key, .. } => assert_eq!(key, "ghost"),
        other => panic!("expected UnknownField, got {other:?}"),
    }
}

#[test]
fn write_provider_field_errors_when_no_providers_array() {
    // SAMPLE_CONFIG is the pre-providers shape (no [[inference.providers]]).
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();
    let err = write_provider_field_to_disk(&path, "ollama", "base_url", "http://x").unwrap_err();
    match err {
        ConfigError::UnknownSection { section } => assert_eq!(section, "inference.providers"),
        other => panic!("expected UnknownSection, got {other:?}"),
    }
}

#[test]
fn write_provider_field_propagates_read_error_for_missing_file() {
    let dir = tempdir();
    let path = dir.join("missing.toml");
    let err = write_provider_field_to_disk(&path, "ollama", "model", "x").unwrap_err();
    matches!(err, ConfigError::IoError { .. });
}

// ─── write_active_provider_to_disk ──────────────────────────────────────────

#[test]
fn set_active_provider_updates_active_and_mirror() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();

    // Give the builtin provider a model first, so the mirror decision below
    // exercises the Some(non-empty) arm the command relies on.
    write_provider_field_to_disk(&path, "builtin", "model", "org/repo:w.gguf").unwrap();

    let resolved = write_active_provider_to_disk(&path, "builtin").unwrap();
    assert_eq!(resolved.inference.active_provider, "builtin");
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("active_provider = \"builtin\""));

    // The command refreshes the ActiveModelState mirror through this exact
    // decision helper: the new active provider's model, empty mapped to None.
    assert_eq!(
        crate::models::should_refresh_active_model("builtin", &resolved),
        Some(Some("org/repo:w.gguf".to_string()))
    );

    // Switching back to a provider with no model clears the mirror.
    let resolved = write_active_provider_to_disk(&path, "ollama").unwrap();
    assert_eq!(resolved.inference.active_provider, "ollama");
    assert_eq!(
        crate::models::should_refresh_active_model("ollama", &resolved),
        Some(None)
    );
}

#[test]
fn set_active_provider_rejects_unknown_id() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();
    let err = write_active_provider_to_disk(&path, "ghost").unwrap_err();
    match err {
        ConfigError::UnknownField { section, key } => {
            assert_eq!(section, "inference.providers");
            assert_eq!(key, "ghost");
        }
        other => panic!("expected UnknownField, got {other:?}"),
    }
    // The file is untouched: the active provider pointer keeps its old value.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("active_provider = \"ollama\""));
}

#[test]
fn set_active_provider_errors_when_no_providers_array() {
    // SAMPLE_CONFIG is the pre-providers shape (no [[inference.providers]]).
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();
    let err = write_active_provider_to_disk(&path, "ollama").unwrap_err();
    match err {
        ConfigError::UnknownSection { section } => assert_eq!(section, "inference.providers"),
        other => panic!("expected UnknownSection, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn set_active_provider_propagates_io_error_when_parent_dir_is_readonly() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();

    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&dir, perms.clone()).unwrap();

    let err = write_active_provider_to_disk(&path, "builtin").unwrap_err();

    // Restore writability so the OS can clean up the tempdir later.
    let mut restore = perms;
    restore.set_mode(0o700);
    std::fs::set_permissions(&dir, restore).unwrap();

    matches!(err, ConfigError::IoError { .. });
}

// ─── add_openai_provider_to_disk ─────────────────────────────────────────────

#[test]
fn add_openai_appends_provider_with_custom_label() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();

    let resolved =
        add_openai_provider_to_disk(&path, "LM Studio", "http://127.0.0.1:1234").unwrap();
    let openai = resolved
        .inference
        .providers
        .iter()
        .find(|p| p.kind == "openai")
        .unwrap();
    assert_eq!(openai.id, "openai");
    assert_eq!(openai.label, "LM Studio");
    assert_eq!(openai.base_url, "http://127.0.0.1:1234");
    assert_eq!(openai.model, "");
    assert!(!openai.vision);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("kind = \"openai\""));
    assert!(on_disk.contains("http://127.0.0.1:1234"));
}

#[test]
fn add_openai_defaults_empty_label() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();

    let resolved = add_openai_provider_to_disk(&path, "   ", "https://10.0.0.5:1234").unwrap();
    let openai = resolved
        .inference
        .providers
        .iter()
        .find(|p| p.kind == "openai")
        .unwrap();
    assert_eq!(openai.label, "OpenAI-compatible");
}

#[test]
fn add_openai_rejects_non_http_base_url() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();
    let err = add_openai_provider_to_disk(&path, "x", "localhost:1234").unwrap_err();
    match err {
        ConfigError::TypeMismatch { key, .. } => assert_eq!(key, "base_url"),
        other => panic!("expected TypeMismatch, got {other:?}"),
    }
}

#[test]
fn add_openai_rejects_second_openai_provider() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, OPENAI_PROVIDERS_CONFIG).unwrap();
    let err = add_openai_provider_to_disk(&path, "Another", "http://127.0.0.1:9999").unwrap_err();
    match err {
        ConfigError::TypeMismatch { message, .. } => {
            assert!(message.contains("already exists"));
        }
        other => panic!("expected TypeMismatch, got {other:?}"),
    }
}

#[test]
fn add_openai_errors_when_no_providers_array() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();
    let err = add_openai_provider_to_disk(&path, "x", "http://127.0.0.1:1234").unwrap_err();
    match err {
        ConfigError::UnknownSection { section } => assert_eq!(section, "inference.providers"),
        other => panic!("expected UnknownSection, got {other:?}"),
    }
}

#[test]
fn add_openai_propagates_read_error_for_missing_file() {
    let dir = tempdir();
    let path = dir.join("missing.toml");
    let err = add_openai_provider_to_disk(&path, "x", "http://127.0.0.1:1234").unwrap_err();
    matches!(err, ConfigError::IoError { .. });
}

#[cfg(unix)]
#[test]
fn add_openai_propagates_io_error_when_parent_dir_is_readonly() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();

    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&dir, perms.clone()).unwrap();

    let err = add_openai_provider_to_disk(&path, "x", "http://127.0.0.1:1234").unwrap_err();

    let mut restore = perms;
    restore.set_mode(0o700);
    std::fs::set_permissions(&dir, restore).unwrap();

    matches!(err, ConfigError::IoError { .. });
}

// ─── remove_openai_provider_from_disk ────────────────────────────────────────

#[test]
fn remove_openai_deletes_entry_and_keeps_active_pointer() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, OPENAI_PROVIDERS_CONFIG).unwrap();

    let (resolved, removed_ids) = remove_openai_provider_from_disk(&path).unwrap();
    assert!(!resolved
        .inference
        .providers
        .iter()
        .any(|p| p.kind == "openai"));
    // Active was "ollama" and stays "ollama".
    assert_eq!(resolved.inference.active_provider, "ollama");
    // The removed ids feed the Keychain cleanup in the command wrapper.
    assert_eq!(removed_ids, vec!["openai".to_string()]);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(!on_disk.contains("kind = \"openai\""));
}

#[test]
fn remove_openai_returns_custom_id_for_keychain_cleanup() {
    // A hand-edited file can carry an arbitrary id on the openai-kind row;
    // the frontend stores the API key under that id, so the removal must
    // surface it for cleanup instead of assuming the fixed default id.
    let dir = tempdir();
    let path = dir.join("config.toml");
    let custom = OPENAI_PROVIDERS_CONFIG.replace("id = \"openai\"", "id = \"my-llm\"");
    std::fs::write(&path, custom).unwrap();

    let (resolved, removed_ids) = remove_openai_provider_from_disk(&path).unwrap();
    assert!(!resolved
        .inference
        .providers
        .iter()
        .any(|p| p.kind == "openai"));
    assert_eq!(removed_ids, vec!["my-llm".to_string()]);

    // Feeding the removed ids through the cleanup helper deletes exactly
    // that id's secret and leaves unrelated entries alone.
    let store = crate::keychain::FakeSecretStore::new();
    use crate::keychain::SecretStore;
    store.set("my-llm", "sk-custom").unwrap();
    store.set("unrelated", "sk-keep").unwrap();
    cleanup_provider_secrets(&store, &removed_ids);
    assert_eq!(store.get("my-llm").unwrap(), None);
    assert_eq!(store.get("unrelated").unwrap(), Some("sk-keep".to_string()));
}

#[test]
fn cleanup_provider_secrets_skips_empty_ids() {
    // A removed row missing an `id` collapses to "" in removed_ids; cleanup
    // must skip it rather than issuing a delete for an empty account name.
    let store = crate::keychain::FakeSecretStore::new();
    use crate::keychain::SecretStore;
    store.set("", "sentinel").unwrap();
    store.set("openai", "sk-gone").unwrap();
    cleanup_provider_secrets(&store, &[String::new(), "openai".to_string()]);
    assert_eq!(store.get("").unwrap(), Some("sentinel".to_string()));
    assert_eq!(store.get("openai").unwrap(), None);
}

#[test]
fn remove_openai_falls_back_to_builtin_when_it_was_active() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, OPENAI_PROVIDERS_CONFIG).unwrap();
    write_active_provider_to_disk(&path, "openai").unwrap();

    let (resolved, _removed_ids) = remove_openai_provider_from_disk(&path).unwrap();
    assert_eq!(resolved.inference.active_provider, "builtin");
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("active_provider = \"builtin\""));

    // The command re-mirrors the in-memory active model through this exact
    // decision helper: builtin has no model yet, so the mirror clears.
    assert_eq!(
        crate::models::should_refresh_active_model("builtin", &resolved),
        Some(None)
    );
}

#[test]
fn remove_openai_errors_when_no_openai_provider() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();
    let err = remove_openai_provider_from_disk(&path).unwrap_err();
    match err {
        ConfigError::UnknownField { key, .. } => assert_eq!(key, "openai"),
        other => panic!("expected UnknownField, got {other:?}"),
    }
}

#[test]
fn remove_openai_errors_when_no_providers_array() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();
    let err = remove_openai_provider_from_disk(&path).unwrap_err();
    match err {
        ConfigError::UnknownSection { section } => assert_eq!(section, "inference.providers"),
        other => panic!("expected UnknownSection, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn remove_openai_propagates_io_error_when_parent_dir_is_readonly() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, OPENAI_PROVIDERS_CONFIG).unwrap();

    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&dir, perms.clone()).unwrap();

    let err = remove_openai_provider_from_disk(&path).unwrap_err();

    let mut restore = perms;
    restore.set_mode(0o700);
    std::fs::set_permissions(&dir, restore).unwrap();

    matches!(err, ConfigError::IoError { .. });
}

#[cfg(unix)]
#[test]
fn write_provider_field_propagates_io_error_when_parent_dir_is_readonly() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, PROVIDERS_CONFIG).unwrap();

    // Read-only directory: the patch succeeds in memory but the atomic write
    // cannot stage its temp file alongside the target.
    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&dir, perms.clone()).unwrap();

    let err = write_provider_field_to_disk(&path, "ollama", "base_url", "http://10.0.0.2:11434")
        .unwrap_err();

    // Restore writability so the OS can clean up the tempdir later.
    let mut restore = perms;
    restore.set_mode(0o700);
    std::fs::set_permissions(&dir, restore).unwrap();

    matches!(err, ConfigError::IoError { .. });
}

// ─── reset_section_on_disk ──────────────────────────────────────────────────

#[test]
fn reset_section_on_disk_replaces_named_section_with_defaults() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    // SAMPLE_CONFIG's [inference] is the legacy shape (ollama_url + available,
    // no providers). Resetting the section must restore the new providers shape:
    // active_provider + the built-in/Ollama array-of-tables.
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    let resolved = reset_section_on_disk(&path, Some("inference")).unwrap();
    // Section reset restores compiled defaults: the built-in engine is the default active provider.
    assert_eq!(resolved.inference.active_provider, "builtin");
    assert!(resolved
        .inference
        .providers
        .iter()
        .any(|p| p.id == "builtin"));
    let ollama = resolved
        .inference
        .providers
        .iter()
        .find(|p| p.id == "ollama")
        .unwrap();
    assert_eq!(ollama.base_url, "http://127.0.0.1:11434");

    // The reset wrote a `[[inference.providers]]` array-of-tables to disk that
    // round-trips back through the loader.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        on_disk.contains("[[inference.providers]]"),
        "section reset must persist the providers array-of-tables: {on_disk}"
    );
}

#[test]
fn reset_section_on_disk_preserves_other_sections() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    // Change the updater section, then reset only inference.
    write_field_to_disk(&path, "updater", "check_interval_hours", json!(48)).unwrap();

    // Reset only inference; updater.check_interval_hours should still be 48.
    let resolved = reset_section_on_disk(&path, Some("inference")).unwrap();
    assert_eq!(resolved.updater.check_interval_hours, 48);
}

#[test]
fn reset_section_on_disk_whole_file_resets_everything() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    write_field_to_disk(&path, "updater", "check_interval_hours", json!(48)).unwrap();
    let resolved = reset_section_on_disk(&path, None).unwrap();
    // Default is 24 per defaults.rs.
    assert_eq!(resolved.updater.check_interval_hours, 24);
}

#[test]
fn reset_section_on_disk_rejects_unknown_section() {
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();
    let err = reset_section_on_disk(&path, Some("activation")).unwrap_err();
    match err {
        ConfigError::UnknownSection { section } => assert_eq!(section, "activation"),
        other => panic!("expected UnknownSection, got {other:?}"),
    }
}

#[test]
fn reset_section_on_disk_propagates_read_error_for_missing_file_named_section() {
    let dir = tempdir();
    let path = dir.join("missing.toml");
    let err = reset_section_on_disk(&path, Some("inference")).unwrap_err();
    matches!(err, ConfigError::IoError { .. });
}

#[cfg(unix)]
#[test]
fn reset_section_on_disk_propagates_io_error_when_parent_dir_is_readonly() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&dir, perms.clone()).unwrap();

    let err = reset_section_on_disk(&path, Some("inference")).unwrap_err();

    let mut restore = perms;
    restore.set_mode(0o700);
    std::fs::set_permissions(&dir, restore).unwrap();

    matches!(err, ConfigError::IoError { .. });
}

#[cfg(unix)]
#[test]
fn reset_section_on_disk_whole_file_propagates_io_error_when_parent_dir_is_readonly() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, SAMPLE_CONFIG).unwrap();

    let mut perms = std::fs::metadata(&dir).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&dir, perms.clone()).unwrap();

    let err = reset_section_on_disk(&path, None).unwrap_err();

    let mut restore = perms;
    restore.set_mode(0o700);
    std::fs::set_permissions(&dir, restore).unwrap();

    matches!(err, ConfigError::IoError { .. });
}

// ─── trace_enabled_changed ───────────────────────────────────────────────────

#[test]
fn trace_enabled_changed_detects_off_to_on() {
    let mut cfg = AppConfig::default();
    cfg.debug.trace_enabled = true;
    assert!(trace_enabled_changed(false, &cfg));
}

#[test]
fn trace_enabled_changed_detects_on_to_off() {
    let mut cfg = AppConfig::default();
    cfg.debug.trace_enabled = false;
    assert!(trace_enabled_changed(true, &cfg));
}

#[test]
fn trace_enabled_changed_returns_false_when_value_unchanged() {
    let mut cfg = AppConfig::default();
    cfg.debug.trace_enabled = true;
    assert!(!trace_enabled_changed(true, &cfg));
    cfg.debug.trace_enabled = false;
    assert!(!trace_enabled_changed(false, &cfg));
}

// ─── trace_retention_days_changed ────────────────────────────────────────────

#[test]
fn trace_retention_days_changed_returns_new_value_on_change() {
    let mut cfg = AppConfig::default();
    cfg.debug.trace_retention_days = 30;
    assert_eq!(trace_retention_days_changed(7, &cfg), Some(30));
    // A change onto the keep-forever sentinel is still a change.
    cfg.debug.trace_retention_days = -1;
    assert_eq!(trace_retention_days_changed(7, &cfg), Some(-1));
}

#[test]
fn trace_retention_days_changed_returns_none_when_unchanged() {
    let mut cfg = AppConfig::default();
    cfg.debug.trace_retention_days = 14;
    assert_eq!(trace_retention_days_changed(14, &cfg), None);
}

// ─── keep_warm_idle_minutes_changed ──────────────────────────────────────────

#[test]
fn keep_warm_idle_minutes_changed_returns_translated_value_on_positive_change() {
    let mut cfg = AppConfig::default();
    cfg.inference.keep_warm_inactivity_minutes = 45;
    // Positive N passes straight through the translator.
    assert_eq!(keep_warm_idle_minutes_changed(0, &cfg), Some(45));
}

#[test]
fn keep_warm_idle_minutes_changed_translates_forever_to_disabled() {
    let mut cfg = AppConfig::default();
    cfg.inference.keep_warm_inactivity_minutes = -1;
    // -1 (forever) translates to the runner's "0 = disabled".
    assert_eq!(keep_warm_idle_minutes_changed(0, &cfg), Some(0));
}

#[test]
fn keep_warm_idle_minutes_changed_translates_zero_to_short_default() {
    let mut cfg = AppConfig::default();
    cfg.inference.keep_warm_inactivity_minutes = 0;
    // 0 (natural short default) translates to the baked-in ~5-minute timer.
    assert_eq!(
        keep_warm_idle_minutes_changed(-1, &cfg),
        Some(crate::config::defaults::DEFAULT_BUILTIN_IDLE_MINUTES)
    );
}

#[test]
fn keep_warm_idle_minutes_changed_returns_none_when_unchanged() {
    let mut cfg = AppConfig::default();
    cfg.inference.keep_warm_inactivity_minutes = 45;
    assert_eq!(keep_warm_idle_minutes_changed(45, &cfg), None);
}

// ─── builtin_deactivated ─────────────────────────────────────────────────────

/// `AppConfig::default()` with the active provider pointed at `id` (the
/// default providers list carries `builtin` and `ollama`).
fn config_with_active(id: &str) -> AppConfig {
    let mut cfg = AppConfig::default();
    cfg.inference.active_provider = id.to_string();
    cfg
}

#[test]
fn builtin_deactivated_detects_switch_away_from_builtin() {
    assert!(builtin_deactivated(
        "builtin",
        &config_with_active("ollama")
    ));
}

#[test]
fn builtin_deactivated_ignores_switch_onto_builtin() {
    assert!(!builtin_deactivated(
        "ollama",
        &config_with_active("builtin")
    ));
}

#[test]
fn builtin_deactivated_ignores_non_builtin_transitions_and_no_ops() {
    // ollama -> ollama: nothing changed.
    assert!(!builtin_deactivated(
        "ollama",
        &config_with_active("ollama")
    ));
    // builtin -> builtin: still active; must not unload.
    assert!(!builtin_deactivated(
        "builtin",
        &config_with_active("builtin")
    ));
    // Unresolved prior kind (empty) never counts as builtin.
    assert!(!builtin_deactivated("", &config_with_active("ollama")));
}

// ─── ollama_deactivated ──────────────────────────────────────────────────────

#[test]
fn ollama_deactivated_detects_switch_away_from_ollama() {
    // ollama -> builtin and ollama -> openai both free the Ollama model.
    assert!(ollama_deactivated("ollama", &config_with_active("builtin")));
    assert!(ollama_deactivated("ollama", &config_with_active("openai")));
}

#[test]
fn ollama_deactivated_ignores_switch_onto_ollama() {
    assert!(!ollama_deactivated(
        "builtin",
        &config_with_active("ollama")
    ));
}

#[test]
fn ollama_deactivated_ignores_non_ollama_transitions_and_no_ops() {
    // ollama -> ollama: nothing changed.
    assert!(!ollama_deactivated("ollama", &config_with_active("ollama")));
    // builtin -> builtin: never an Ollama deactivation.
    assert!(!ollama_deactivated(
        "builtin",
        &config_with_active("builtin")
    ));
    // Unresolved prior kind (empty) never counts as ollama.
    assert!(!ollama_deactivated("", &config_with_active("builtin")));
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

// ─── clear_traces_dir ────────────────────────────────────────────────────────

#[test]
fn clear_traces_dir_removes_top_level_files_and_leaves_empty_root() {
    let root = tempdir().join("traces");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.jsonl"), b"one").unwrap();
    std::fs::write(root.join("b.jsonl"), b"two").unwrap();

    clear_traces_dir(&root).expect("clear should succeed");

    assert!(root.exists(), "root must be recreated");
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        0,
        "root must be empty after clear"
    );
}

#[test]
fn clear_traces_dir_removes_nested_subdirectories() {
    let root = tempdir().join("traces");
    let chat = root.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    std::fs::write(chat.join("x.jsonl"), b"nested").unwrap();

    clear_traces_dir(&root).expect("clear should succeed");

    assert!(root.exists(), "root must be recreated");
    assert!(!chat.exists(), "nested subdirectory must be gone");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn clear_traces_dir_missing_root_is_noop_ok() {
    let root = tempdir().join("never-created");
    assert!(!root.exists());

    clear_traces_dir(&root).expect("missing root must be a no-op Ok");

    // A no-op leaves the missing directory absent (it is not created).
    assert!(!root.exists(), "missing root must stay absent on no-op");
}

// ─── traces_stats_for ────────────────────────────────────────────────────────

#[test]
fn traces_stats_for_counts_files_and_sums_bytes_including_nested() {
    let root = tempdir().join("traces");
    let chat = root.join("chat");
    std::fs::create_dir_all(&chat).unwrap();
    // Top-level file + two nested files: 3 files, 3 + 5 + 7 = 15 bytes.
    std::fs::write(root.join("top.jsonl"), b"abc").unwrap();
    std::fs::write(chat.join("a.jsonl"), b"hello").unwrap();
    std::fs::write(chat.join("b.jsonl"), b"seventy").unwrap();

    let (count, bytes) = traces_stats_for(&root).expect("stats should succeed");

    assert_eq!(count, 3, "every regular file across subdirs is counted");
    assert_eq!(bytes, 15, "byte total sums file sizes across subdirs");
}

#[test]
fn traces_stats_for_missing_root_is_zero() {
    let root = tempdir().join("never-created");
    assert!(!root.exists());

    assert_eq!(
        traces_stats_for(&root).expect("missing root must be Ok"),
        (0, 0),
        "missing root reports an empty footprint"
    );
}

#[test]
fn traces_stats_for_empty_root_is_zero() {
    let root = tempdir().join("traces");
    std::fs::create_dir_all(&root).unwrap();

    assert_eq!(
        traces_stats_for(&root).expect("empty root must be Ok"),
        (0, 0),
        "an existing but empty root reports zero"
    );
}

#[cfg(unix)]
#[test]
fn traces_stats_for_does_not_follow_symlinks() {
    let root = tempdir().join("traces");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("real.jsonl"), b"real").unwrap();
    // A symlink to the real file must NOT be counted (link, not a regular file).
    std::os::unix::fs::symlink(root.join("real.jsonl"), root.join("link.jsonl")).unwrap();

    let (count, bytes) = traces_stats_for(&root).expect("stats should succeed");

    assert_eq!(
        count, 1,
        "the symlink is skipped, only the real file counts"
    );
    assert_eq!(bytes, 4, "only the real file's bytes are summed");
}

// ─── prune_traces_older_than ─────────────────────────────────────────────────

/// Writes `name` under `dir` and stamps its modification time to `mtime` so
/// prune tests are deterministic without sleeping on the wall clock.
fn write_trace_with_mtime(dir: &Path, name: &str, contents: &[u8], mtime: SystemTime) {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
}

#[test]
fn prune_traces_older_than_deletes_old_keeps_fresh_across_subdirs() {
    let root = tempdir().join("traces");
    let chat = root.join("chat");
    std::fs::create_dir_all(&chat).unwrap();

    // Fix a stable reference time; back-date each file explicitly relative to it.
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let ten_days = Duration::from_secs(10 * 86_400);
    let one_day = Duration::from_secs(86_400);
    // Old: top-level + nested, both older than the 7-day window.
    write_trace_with_mtime(&root, "old-top.jsonl", b"old", now - ten_days);
    write_trace_with_mtime(&chat, "old-nested.jsonl", b"old", now - ten_days);
    // Fresh: within the window, must survive.
    write_trace_with_mtime(&chat, "fresh.jsonl", b"new", now - one_day);

    let pruned =
        prune_traces_older_than(&root, Duration::from_secs(7 * 86_400), now).expect("prune ok");

    assert_eq!(pruned, 2, "both old files (top + nested) are removed");
    assert!(!root.join("old-top.jsonl").exists());
    assert!(!chat.join("old-nested.jsonl").exists());
    assert!(chat.join("fresh.jsonl").exists(), "fresh file is kept");
    assert!(chat.exists(), "directory structure is left intact");
}

#[test]
fn prune_traces_older_than_keeps_everything_when_now_predates_files() {
    let root = tempdir().join("traces");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.jsonl"), b"a").unwrap();
    std::fs::write(root.join("b.jsonl"), b"b").unwrap();

    // `now` far in the past: every real file's mtime is in the future, so
    // `duration_since` returns Err and nothing is pruned (clock-ambiguity safe).
    let pruned =
        prune_traces_older_than(&root, Duration::from_secs(86_400), SystemTime::UNIX_EPOCH)
            .expect("prune ok");

    assert_eq!(pruned, 0, "future-dated files are never deleted");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
}

#[test]
fn prune_traces_older_than_missing_root_is_zero() {
    let root = tempdir().join("never-created");
    assert!(!root.exists());
    assert_eq!(
        prune_traces_older_than(&root, Duration::from_secs(86_400), SystemTime::now())
            .expect("missing root must be Ok"),
        0
    );
}

#[cfg(unix)]
#[test]
fn prune_traces_older_than_does_not_follow_symlinks() {
    let root = tempdir().join("traces");
    std::fs::create_dir_all(&root).unwrap();
    // A real, old target OUTSIDE the traces tree.
    let outside = tempdir();
    let target = outside.join("secret.jsonl");
    std::fs::write(&target, b"keep me").unwrap();
    // A symlink INSIDE the tree pointing at it. The link's own file_type is
    // symlink, so it is neither counted as a file (deleted) nor a dir
    // (descended): the target must be left untouched.
    std::os::unix::fs::symlink(&target, root.join("link.jsonl")).unwrap();

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let pruned =
        prune_traces_older_than(&root, Duration::from_secs(86_400), now).expect("prune ok");

    assert_eq!(pruned, 0, "the symlink is skipped, never followed");
    assert!(target.exists(), "the symlink target is left untouched");
    assert!(
        root.join("link.jsonl").exists(),
        "the symlink itself remains"
    );
}

// ─── prune_traces_for_retention ──────────────────────────────────────────────

#[test]
fn prune_traces_for_retention_forever_sentinel_skips_walk() {
    let root = tempdir().join("traces");
    std::fs::create_dir_all(&root).unwrap();
    // A file old enough to prune under any positive window.
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    write_trace_with_mtime(
        &root,
        "old.jsonl",
        b"old",
        now - Duration::from_secs(365 * 86_400),
    );

    let pruned = prune_traces_for_retention(&root, -1, now).expect("prune ok");

    assert_eq!(pruned, 0, "-1 keeps files forever, no walk");
    assert!(root.join("old.jsonl").exists(), "the old file is untouched");
}

#[test]
fn prune_traces_for_retention_positive_days_prunes_old_files() {
    let root = tempdir().join("traces");
    std::fs::create_dir_all(&root).unwrap();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    write_trace_with_mtime(
        &root,
        "old.jsonl",
        b"old",
        now - Duration::from_secs(10 * 86_400),
    );
    write_trace_with_mtime(
        &root,
        "fresh.jsonl",
        b"new",
        now - Duration::from_secs(3600),
    );

    // 7-day retention: the 10-day-old file goes, the hour-old file stays.
    let pruned = prune_traces_for_retention(&root, 7, now).expect("prune ok");

    assert_eq!(pruned, 1);
    assert!(!root.join("old.jsonl").exists());
    assert!(root.join("fresh.jsonl").exists());
}
