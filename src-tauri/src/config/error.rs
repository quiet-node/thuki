use std::path::PathBuf;
use thiserror::Error;

/// Errors returned by the `config` module.
///
/// `SeedFailed` is fatal on startup (the app cannot write its default file,
/// which on macOS signals a genuinely broken environment the user cannot fix
/// from the UI). All other variants are recoverable at load time: the loader
/// renames the offending file and reseeds defaults.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// First-run default-file seed failed.
    /// `lib.rs` surfaces this via a Tauri dialog and quits.
    #[error("failed to seed default config file at {path}: {source}")]
    SeedFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    /// An I/O error occurred during atomic write (tmpfile creation, rename,
    /// etc.). Used by the writer. Converted to `SeedFailed` by the loader
    /// when the context is first-run seeding.
    #[error("config file I/O error at {path}: {source}")]
    IoError {
        path: PathBuf,
        source: std::io::Error,
    },

    /// The file declares a `schema_version` newer than this build understands.
    /// Recoverable: loader renames the file and reseeds defaults. The next
    /// time the user downgrades, they will get defaults instead of a panic.
    #[error("config schema version {found} is newer than supported version {supported}")]
    TooNew { found: u32, supported: u32 },

    /// The file declares a `schema_version` older than this build supports and
    /// no migration path exists yet. v1 has no ancestors, so any value other
    /// than 1 takes this branch.
    #[error("config schema version {found} is not supported (no migration available)")]
    NoMigrationYet { found: u32 },
}
