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
pub use schema::{
    ActivationSection, AppConfig, ModelSection, PromptSection, QuoteSection, WindowSection,
};
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

#[cfg(test)]
mod tests;
