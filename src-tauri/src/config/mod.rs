//! Application configuration module.
//!
//! This module is the single source of truth for Thuki's runtime configuration.
//! Every subsystem reads resolved values from a Tauri-managed `AppConfig`
//! state. Compiled defaults live in `defaults`; the on-disk file at
//! `~/Library/Application Support/com.quietnode.thuki/config.toml` overlays
//! user customizations on top.
//!
//! ## Public surface
//!
//! - [`AppConfig`] - the typed configuration shape.
//! - [`load`] - Tauri-aware entry point called once during app setup.
//! - [`load_from_path`] - pure, test-friendly variant that takes a `Path`.
//! - [`atomic_write`] - safe write that never produces a torn file.
//! - [`ConfigError`] - error type returned by loader and writer.
//!
//! v1 is read-only: the `set_config` Tauri command and the `RwLock<AppConfig>`
//! wrapper arrive with the future settings-panel PR.

pub mod defaults;
pub mod error;
pub mod loader;
pub mod schema;
pub mod writer;

pub use error::ConfigError;
pub use loader::load_from_path;
pub use schema::{AppConfig, InferenceSection, PromptSection, QuoteSection, WindowSection};
pub use writer::{atomic_write, atomic_write_bytes};

/// File name of the user config file inside the OS config dir.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Marker file written by the loader's corrupt-rename path so the Settings
/// window can render a recovery banner on first open after the rename. Format:
/// two lines — absolute path of the `<config>.corrupt-<ts>` file, then the
/// numeric unix timestamp. The marker is consumed (read + deleted) by the
/// `get_corrupt_marker` Tauri command.
pub const CORRUPT_MARKER_FILE_NAME: &str = ".corrupt-recovery-pending";

/// Information about the most recent corrupt-rename event, surfaced to the
/// Settings UI so users know their hand-edited file was rejected and where
/// to find it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CorruptMarker {
    /// Absolute path to the `<config>.corrupt-<ts>` file the loader produced.
    pub path: String,
    /// Unix timestamp (seconds) when the rename happened.
    pub ts: u64,
}

/// Reads (and deletes) the corrupt-recovery marker from `dir` if one exists.
/// Returns `None` if no marker is present, or the marker payload could not be
/// parsed (malformed marker is treated as no marker; the corrupt file itself
/// is still on disk for the user to recover).
///
/// Pure I/O; tested via the `consume_corrupt_marker_*` cases in `tests.rs`.
pub fn consume_corrupt_marker(dir: &std::path::Path) -> Option<CorruptMarker> {
    let marker_path = dir.join(CORRUPT_MARKER_FILE_NAME);
    let contents = std::fs::read_to_string(&marker_path).ok()?;
    // Best-effort cleanup. The marker has done its job once we have read it.
    let _ = std::fs::remove_file(&marker_path);
    let mut lines = contents.lines();
    let path = lines.next()?.trim().to_string();
    let ts: u64 = lines.next()?.trim().parse().ok()?;
    if path.is_empty() {
        return None;
    }
    Some(CorruptMarker { path, ts })
}

/// Tauri-aware entry point. Resolves the per-user config path via
/// `AppHandle.path().app_config_dir()` (which on macOS yields
/// `~/Library/Application Support/<bundle_id>/`), then delegates to
/// [`load_from_path`].
///
/// This wrapper is excluded from coverage because it exercises the real
/// macOS filesystem and requires a fully-constructed `AppHandle`. All of
/// its logic is in `load_from_path`, which has full coverage.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn load(app: &tauri::AppHandle) -> Result<AppConfig, ConfigError> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|source| ConfigError::IoError {
            path: std::path::PathBuf::from("<app_config_dir>"),
            source: std::io::Error::other(source.to_string()),
        })?;
    let path = dir.join(CONFIG_FILE_NAME);
    load_from_path(&path)
}

/// Shows a native macOS alert describing the fatal config error and exits
/// the process with a non-zero code. Called from `lib.rs` setup when
/// [`load`] returns `Err`. On a non-sandboxed macOS app the only realistic
/// cause is a broken `~/Library/Application Support/` (permission, disk full,
/// read-only filesystem), which the user cannot repair from the UI.
///
/// Uses `osascript` to avoid pulling in `tauri-plugin-dialog` for a code path
/// that runs at most once per user in the app's lifetime.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn show_fatal_dialog_and_exit(err: &ConfigError) -> ! {
    let raw = format!(
        "Thuki could not start because of a configuration error.\n\n{err}\n\nCheck write permissions on ~/Library/Application Support/"
    );
    // Escape quotes and backslashes for AppleScript string literal.
    let escaped = raw.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display alert \"Thuki\" message \"{escaped}\" as critical buttons {{\"Quit\"}} default button \"Quit\""
    );
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output();
    // Also print to stderr so `bun run dev` surfaces the error in-terminal.
    eprintln!("thuki: [config] fatal: {err}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests;
