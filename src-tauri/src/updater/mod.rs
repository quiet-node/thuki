//! Auto-update orchestration.
//!
//! Detection lives in `poller`. State lives in `state`. Frontend-facing
//! Tauri commands live in `commands`. The updater plugin itself
//! (`tauri_plugin_updater`) handles download, verify, swap, and relaunch.

pub mod commands;
pub mod poller;
pub mod state;

pub use state::{AvailableUpdate, SnoozeSidecar, UpdaterSnapshot, UpdaterState};
