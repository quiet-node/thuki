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
pub use writer::atomic_write;

/// File name of the user config file inside the OS config dir.
pub const CONFIG_FILE_NAME: &str = "config.toml";

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
