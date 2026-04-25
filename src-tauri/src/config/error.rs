use std::path::PathBuf;
use thiserror::Error;

/// Errors returned by the `config` module.
///
/// Two flavors:
/// - **Loader/seed errors** (`SeedFailed`, `IoError`) — produced during initial
///   load and the legacy seed path. `SeedFailed` is fatal on startup; `IoError`
///   is the catch-all for write-side filesystem failures.
/// - **GUI-write errors** (`UnknownSection`, `UnknownField`, `TypeMismatch`,
///   `Parse`) — produced by `set_config_field` and `reset_config` when the
///   request is structurally invalid. These cross the IPC boundary as serialized
///   tagged enums (`{ "kind": "unknown_field", "section": "...", "key": "..." }`)
///   so the frontend can render typed inline error pills instead of opaque
///   strings.
#[derive(Debug, Error, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigError {
    /// First-run default-file seed failed. Fatal: `lib.rs` surfaces this via a
    /// Tauri dialog and quits.
    #[error("failed to seed default config file at {path}: {source}")]
    SeedFailed {
        path: PathBuf,
        #[serde(serialize_with = "serialize_io_error")]
        source: std::io::Error,
    },

    /// I/O error during atomic write (tmpfile creation, rename, etc.). Used by
    /// the writer and by `set_config_field` when disk operations fail mid-save.
    #[error("config file I/O error at {path}: {source}")]
    IoError {
        path: PathBuf,
        #[serde(serialize_with = "serialize_io_error")]
        source: std::io::Error,
    },

    /// `set_config_field` or `reset_config` referenced a section that is not
    /// in `defaults::ALLOWED_SECTIONS`.
    #[error("unknown config section: {section}")]
    UnknownSection { section: String },

    /// `set_config_field` referenced a `(section, key)` pair that is not in
    /// `defaults::ALLOWED_FIELDS`. Either the schema does not have that field,
    /// or the field is intentionally not user-tunable.
    #[error("unknown config field: {section}.{key}")]
    UnknownField { section: String, key: String },

    /// `set_config_field` was given a JSON value whose primitive type cannot be
    /// converted into the TOML type expected for the target field (e.g. a
    /// JSON object passed for an integer field).
    #[error("type mismatch on {section}.{key}: {message}")]
    TypeMismatch {
        section: String,
        key: String,
        message: String,
    },

    /// The on-disk `config.toml` failed to parse during a GUI write. This
    /// indicates the file was hand-edited into invalid TOML between the most
    /// recent successful load and the GUI write attempt. The frontend should
    /// surface a "config has been corrupted, restart Thuki" message.
    #[error("config file parse error at {path}: {message}")]
    Parse { path: PathBuf, message: String },
}

/// Serializes a `std::io::Error` as its `Display` string for IPC. The frontend
/// only needs the human-readable description; the raw `ErrorKind` and OS code
/// are not preserved across the boundary.
fn serialize_io_error<S>(err: &std::io::Error, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&err.to_string())
}
