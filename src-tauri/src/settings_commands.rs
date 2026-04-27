//! Settings panel Tauri command surface.
//!
//! Implements the IPC contract used by the Settings window:
//!
//! - [`get_config`] — reads the current `AppConfig` snapshot from managed state.
//! - [`set_config_field`] — security-validated per-field write that round-trips
//!   through the loader so clamp / cross-field invariants always apply.
//! - [`reset_config`] — replaces one section (or the whole file) with the
//!   compiled defaults.
//! - [`reload_config_from_disk`] — re-reads the file (called on Settings
//!   window focus, replaces the file watcher subsystem the eng review collapsed).
//! - [`get_corrupt_marker`] — returns and consumes the recovery marker the
//!   loader wrote when a corrupt config file was renamed.
//! - [`reveal_config_in_finder`] — opens Finder with the config file selected.
//!
//! ## Security model
//!
//! `set_config_field` is the only frontend-callable surface that mutates user
//! configuration on disk. It:
//!
//! 1. Validates `(section, key)` against `defaults::ALLOWED_FIELDS`. Any pair
//!    not in the allowlist is rejected with a typed `UnknownField` error.
//!    This prevents the GUI from writing fields that do not exist or are
//!    intentionally not user-tunable (e.g. activation timing, vision limits).
//! 2. Coerces the inbound `serde_json::Value` to the TOML type already present
//!    in the on-disk file. Type drift (string for an integer field, etc.) is
//!    rejected with a typed `TypeMismatch` error rather than silently coerced.
//! 3. Round-trips through `loader::load_from_path` so the loader's clamp /
//!    empty-fallback / cross-field invariant rules apply identically to GUI
//!    edits and hand-edits. The loader is the single source of truth for what
//!    constitutes a valid `AppConfig`; the GUI cannot bypass it.
//!
//! Concurrency: serialized via the `parking_lot::RwLock<AppConfig>` write
//! guard. Concurrent invokes execute in order; last-write-wins on the same
//! field is the intended semantic (matches user expectation when rapidly
//! tabbing between fields).

use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tauri::{AppHandle, Emitter, Manager, State};
use toml_edit::{value as toml_value, Array, DocumentMut, Item, Value as TomlValue};

use crate::config::{
    self,
    defaults::{ALLOWED_FIELDS, ALLOWED_SECTIONS},
    AppConfig, ConfigError, CorruptMarker, CONFIG_FILE_NAME,
};

/// Frontend event emitted to every webview after the in-memory `AppConfig`
/// has been replaced. Subscribers (the main overlay's `ConfigProvider` and
/// the Settings window) refetch via `get_config` so React state matches the
/// authoritative `RwLock<AppConfig>` snapshot. Without this broadcast, only
/// backend-side consumers (e.g. `ask_ollama` reading `State<RwLock<AppConfig>>` per
/// invocation) see config edits; frontend-driven values like window dims
/// stay frozen at the mount-time snapshot.
pub const CONFIG_UPDATED_EVENT: &str = "thuki://config-updated";

/// Emits `CONFIG_UPDATED_EVENT` to every webview. Errors are intentionally
/// swallowed: an emit failure must not break a successful disk write.
#[cfg_attr(coverage_nightly, coverage(off))]
fn emit_config_updated(app: &AppHandle) {
    let _ = app.emit(CONFIG_UPDATED_EVENT, ());
}

/// Resolves the absolute path to the user config file.
///
/// Centralizes the `app.path().app_config_dir() + CONFIG_FILE_NAME` join used
/// across the settings commands. On a successful lookup the returned path
/// matches the path the loader uses, so writes round-trip cleanly.
#[cfg_attr(coverage_nightly, coverage(off))]
fn config_path(app: &AppHandle) -> Result<PathBuf, ConfigError> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|source| ConfigError::IoError {
            path: PathBuf::from("<app_config_dir>"),
            source: std::io::Error::other(source.to_string()),
        })?;
    Ok(dir.join(CONFIG_FILE_NAME))
}

/// Returns whether a `(section, key)` pair is permitted by the allowlist.
fn is_allowed_field(section: &str, key: &str) -> bool {
    ALLOWED_FIELDS
        .iter()
        .any(|(s, k)| *s == section && *k == key)
}

/// Returns whether a section name is permitted by the section allowlist.
fn is_allowed_section(section: &str) -> bool {
    ALLOWED_SECTIONS.contains(&section)
}

// ─── Tauri command surface ──────────────────────────────────────────────────

/// Returns the current resolved `AppConfig` snapshot.
///
/// The Settings window invokes this on mount to seed form state without
/// depending on event delivery (Tauri silently drops emits to closed
/// windows; mount-time fetch + focus-event reload guarantees the open
/// window always reflects the on-disk truth).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn get_config(state: State<'_, RwLock<AppConfig>>) -> AppConfig {
    state.read().clone()
}

/// Writes one field of the config file, returning the resolved `AppConfig`
/// after the loader has clamped / corrected the new value.
///
/// See module docs for the full security and concurrency contract.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn set_config_field(
    section: String,
    key: String,
    value: JsonValue,
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let resolved = {
        let mut guard = state.write();
        let resolved = write_field_to_disk(&path, &section, &key, value)?;
        *guard = resolved.clone();
        resolved
    };
    emit_config_updated(&app);
    Ok(resolved)
}

/// Patches one `(section, key)` to disk and returns the resolved `AppConfig`
/// the loader produces from the new file. Pulled out of the Tauri wrapper so
/// the allowlist guard, document patch, atomic write, and post-write reload
/// are all exercised by the test suite without needing an `AppHandle`.
pub(crate) fn write_field_to_disk(
    path: &Path,
    section: &str,
    key: &str,
    value: JsonValue,
) -> Result<AppConfig, ConfigError> {
    if !is_allowed_section(section) {
        return Err(ConfigError::UnknownSection {
            section: section.to_string(),
        });
    }
    if !is_allowed_field(section, key) {
        return Err(ConfigError::UnknownField {
            section: section.to_string(),
            key: key.to_string(),
        });
    }

    let mut doc = read_document(path)?;
    patch_document(&mut doc, section, key, value)?;

    config::atomic_write_bytes(path, doc.to_string().as_bytes()).map_err(|source| {
        ConfigError::IoError {
            path: path.to_path_buf(),
            source,
        }
    })?;

    config::load_from_path(path)
}

/// Resets one section (or the whole file when `section` is `None`) to the
/// compiled defaults, returning the resulting `AppConfig`.
///
/// Section reset is implemented by replacing only the named section's table in
/// the on-disk document with the table from `AppConfig::default()`. Other
/// sections, top-level comments, and key ordering inside untouched sections
/// are preserved.
///
/// Whole-file reset rewrites the file with `atomic_write(&AppConfig::default)`,
/// which produces byte-for-byte identical output to a fresh first-run seed.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn reset_config(
    section: Option<String>,
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let resolved = {
        let mut guard = state.write();
        let resolved = reset_section_on_disk(&path, section.as_deref())?;
        *guard = resolved.clone();
        resolved
    };
    emit_config_updated(&app);
    Ok(resolved)
}

/// Replaces one section (or the entire file when `section` is `None`) with
/// the compiled defaults and returns the resolved `AppConfig`. Pulled out of
/// the Tauri wrapper so the allowlist guard, table-replacement, atomic
/// write, and post-write reload are exercised by the test suite without
/// needing an `AppHandle`.
pub(crate) fn reset_section_on_disk(
    path: &Path,
    section: Option<&str>,
) -> Result<AppConfig, ConfigError> {
    if let Some(section_name) = section {
        if !is_allowed_section(section_name) {
            return Err(ConfigError::UnknownSection {
                section: section_name.to_string(),
            });
        }
        let mut doc = read_document(path)?;
        let defaults = AppConfig::default();
        let defaults_str =
            toml::to_string_pretty(&defaults).expect("AppConfig is always serializable to TOML");
        let defaults_doc: DocumentMut = defaults_str
            .parse()
            .expect("defaults serialize to a parseable TOML document");
        // is_allowed_section above guarantees `section_name` is one of the
        // top-level keys produced by `AppConfig::default()` serialization, so
        // the lookup is infallible by construction.
        let new_section = defaults_doc
            .get(section_name)
            .cloned()
            .expect("ALLOWED_SECTIONS implies AppConfig::default has this section");
        doc.insert(section_name, new_section);
        config::atomic_write_bytes(path, doc.to_string().as_bytes()).map_err(|source| {
            ConfigError::IoError {
                path: path.to_path_buf(),
                source,
            }
        })?;
    } else {
        config::atomic_write(path, &AppConfig::default()).map_err(|source| {
            ConfigError::IoError {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }

    config::load_from_path(path)
}

/// Re-reads the config file from disk and replaces the in-memory `AppConfig`.
///
/// Bound to the Settings window's `tauri://focus` event and to the explicit
/// "↻ Refresh from disk" button in the About tab. Replaces the file-watcher
/// subsystem the eng review collapsed (see design doc Outside Voice
/// Resolution).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn reload_config_from_disk(
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let resolved = {
        let mut guard = state.write();
        let resolved = config::load_from_path(&path)?;
        *guard = resolved.clone();
        resolved
    };
    emit_config_updated(&app);
    Ok(resolved)
}

/// Returns and consumes the corrupt-recovery marker, if one exists.
///
/// The Settings window invokes this on mount; if a marker is returned, it
/// renders a dismissible recovery banner. The marker is deleted from disk on
/// read so the banner appears at most once per corrupt event.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn get_corrupt_marker(app: AppHandle) -> Result<Option<CorruptMarker>, ConfigError> {
    let path = config_path(&app)?;
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(config::consume_corrupt_marker(&dir))
}

/// Opens Finder with the user's `config.toml` selected.
///
/// Thin FFI wrapper (excluded from coverage) over `open -R`, which is the
/// macOS-native "reveal in Finder" affordance.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn reveal_config_in_finder(app: AppHandle) -> Result<(), String> {
    let path = config_path(&app).map_err(|e| e.to_string())?;
    std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ─── Document I/O + JSON→TOML coercion (testable internals) ─────────────────

/// Information returned to the frontend in the rare case where `set_config_field`
/// is called with a value the loader silently corrected (e.g. the cross-field
/// invariant `reader_batch_timeout_s > reader_per_url_timeout_s`).
///
/// The corrected value is already in the returned `AppConfig`; this struct
/// exists for test harnesses that want to assert on the correction path.
#[derive(Debug, Serialize, PartialEq)]
pub struct PatchOutcome {
    pub section: String,
    pub key: String,
}

/// Reads and parses the TOML document. Maps any I/O or parse error to the
/// appropriate `ConfigError` variant so the IPC boundary surfaces a typed
/// failure.
pub(crate) fn read_document(path: &Path) -> Result<DocumentMut, ConfigError> {
    let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::IoError {
        path: path.to_path_buf(),
        source,
    })?;
    contents
        .parse::<DocumentMut>()
        .map_err(|e| ConfigError::Parse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
}

/// Locates `[section][key]` inside `doc` and overwrites it with `value`,
/// preserving the existing TOML type. Rejects type drift with `TypeMismatch`.
///
/// If the key is absent from the section (e.g. the user hand-edited it out),
/// a new item is inserted with the type inferred from the JSON value rather
/// than returning an error. Inference rules for absent keys:
///
/// | JSON type             | Inserted TOML type |
/// | :-------------------- | :----------------- |
/// | Bool                  | Boolean            |
/// | Integer number        | Integer            |
/// | Float number          | Float              |
/// | String                | String             |
/// | Array of strings      | Array              |
/// | Object / null / other | TypeMismatch error |
///
/// Type-coercion rules for existing items (existing item type -> accepted JSON):
///
/// | Existing TOML type | Accepted JSON                          |
/// | :----------------- | :------------------------------------- |
/// | Integer            | Number with no fractional part         |
/// | Float              | Number (integer also accepted)         |
/// | String             | String                                 |
/// | Boolean            | Bool                                   |
/// | Array              | Array of strings                       |
///
/// Other primitive combinations (object, null, mixed-type arrays) are
/// rejected.
pub(crate) fn patch_document(
    doc: &mut DocumentMut,
    section: &str,
    key: &str,
    value: JsonValue,
) -> Result<(), ConfigError> {
    let table = doc
        .get_mut(section)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| ConfigError::UnknownSection {
            section: section.to_string(),
        })?;

    let coerced = if let Some(existing) = table.get(key) {
        coerce_json_to_toml(existing, value, section, key)?
    } else {
        json_value_to_toml_item(value, section, key)?
    };
    table.insert(key, coerced);
    Ok(())
}

/// Converts a JSON value to a TOML item by inferring the type from the JSON,
/// used when the target key is absent from the on-disk document.
pub(crate) fn json_value_to_toml_item(
    value: JsonValue,
    section: &str,
    key: &str,
) -> Result<Item, ConfigError> {
    let type_mismatch = |msg: &str| ConfigError::TypeMismatch {
        section: section.to_string(),
        key: key.to_string(),
        message: msg.to_string(),
    };

    Ok(match &value {
        JsonValue::Bool(b) => toml_value(*b),
        JsonValue::Number(n) => {
            // Else branch: u64 above i64::MAX only; unreachable via ALLOWED_FIELDS
            // (all tunables are u32/u64 within i64::MAX). Loader clamps regardless.
            if let Some(i) = n.as_i64() {
                toml_value(i)
            } else {
                toml_value(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        JsonValue::String(s) => toml_value(s.as_str()),
        JsonValue::Array(arr) => {
            let mut toml_arr = Array::new();
            for item in arr {
                let s = item.as_str().ok_or_else(|| ConfigError::TypeMismatch {
                    section: section.to_string(),
                    key: key.to_string(),
                    message: "array elements must be strings".into(),
                })?;
                toml_arr.push(s);
            }
            toml_value(toml_arr)
        }
        _ => {
            return Err(type_mismatch(&format!(
                "cannot infer TOML type from {}",
                json_type_name(&value)
            )));
        }
    })
}

/// Coerces `value` to a `toml_edit::Item` whose primitive type matches the
/// type of `existing`. The function inspects the existing item's discriminator
/// rather than the schema, so it stays in lock-step with whatever the loader
/// most-recently wrote (which, after seeding, includes every tunable field).
pub(crate) fn coerce_json_to_toml(
    existing: &Item,
    value: JsonValue,
    section: &str,
    key: &str,
) -> Result<Item, ConfigError> {
    let mismatch = |expected: &str| ConfigError::TypeMismatch {
        section: section.to_string(),
        key: key.to_string(),
        message: format!("expected {expected}, got {}", json_type_name(&value)),
    };

    let existing_value = existing
        .as_value()
        .ok_or_else(|| ConfigError::TypeMismatch {
            section: section.to_string(),
            key: key.to_string(),
            message: "existing field is not a primitive".into(),
        })?;

    Ok(match existing_value {
        TomlValue::Integer(_) => {
            let n = value.as_i64().or_else(|| {
                value.as_f64().and_then(|f| {
                    if f.fract() == 0.0 && f.is_finite() {
                        Some(f as i64)
                    } else {
                        None
                    }
                })
            });
            let n = n.ok_or_else(|| mismatch("integer number"))?;
            toml_value(n)
        }
        TomlValue::Float(_) => {
            // `serde_json::Value::as_f64` already widens integer payloads to
            // f64 (it inspects the inner Number, returning Some for both
            // i64/u64 variants), so the legacy `or_else(as_i64)` fallback
            // here was unreachable and dead. Drop it.
            let f = value.as_f64().ok_or_else(|| mismatch("number"))?;
            toml_value(f)
        }
        TomlValue::String(_) => {
            let s = value.as_str().ok_or_else(|| mismatch("string"))?;
            toml_value(s)
        }
        TomlValue::Boolean(_) => {
            let b = value.as_bool().ok_or_else(|| mismatch("boolean"))?;
            toml_value(b)
        }
        TomlValue::Array(_) => {
            let json_arr = value
                .as_array()
                .ok_or_else(|| mismatch("array of strings"))?;
            let mut arr = Array::new();
            for item in json_arr {
                let s = item.as_str().ok_or_else(|| ConfigError::TypeMismatch {
                    section: section.to_string(),
                    key: key.to_string(),
                    message: "array elements must be strings".into(),
                })?;
                arr.push(s);
            }
            toml_value(arr)
        }
        TomlValue::Datetime(_) | TomlValue::InlineTable(_) => {
            return Err(ConfigError::TypeMismatch {
                section: section.to_string(),
                key: key.to_string(),
                message: "field type not supported by GUI writes".into(),
            })
        }
    })
}

/// Returns a stable, human-readable name for a JSON value's primitive type.
/// Used in error messages so the frontend can surface "expected integer, got
/// string" without inspecting the raw `Value` itself.
fn json_type_name(v: &JsonValue) -> &'static str {
    match v {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "float"
            }
        }
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests;
