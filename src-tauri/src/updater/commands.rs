use crate::updater::poller;
use crate::updater::state::{UpdaterSnapshot, UpdaterState};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::UpdaterExt;

const SIDECAR_FILENAME: &str = "updater_state.json";

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub fn get_updater_state(state: State<'_, UpdaterState>) -> UpdaterSnapshot {
    state.snapshot()
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<UpdaterSnapshot, String> {
    poller::check_once(app.clone()).await;
    Ok(app.state::<UpdaterState>().snapshot())
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    let Some(update) = update else {
        return Err("no update available".into());
    };
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub fn snooze_update_chat(
    state: State<'_, UpdaterState>,
    app: AppHandle,
    hours: u64,
) -> Result<(), String> {
    let until = unix_now() + hours * 3600;
    state.set_chat_snooze(Some(until));
    persist_sidecar(&state, &app)
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub fn snooze_update_settings(
    state: State<'_, UpdaterState>,
    app: AppHandle,
    hours: u64,
) -> Result<(), String> {
    let until = unix_now() + hours * 3600;
    state.set_settings_snooze(Some(until));
    persist_sidecar(&state, &app)
}

/// Returns the current Unix timestamp in seconds. Returns 0 if the system
/// clock is before the Unix epoch (should never happen on any modern OS).
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn persist_sidecar(state: &UpdaterState, app: &AppHandle) -> Result<(), String> {
    let path = sidecar_path(app)?;
    let snooze = state.snooze_clone();
    snooze.save(&path).map_err(|e| e.to_string())
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn sidecar_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(SIDECAR_FILENAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_now_is_recent() {
        let now = unix_now();
        // Sanity: > 2023-01-01
        assert!(now > 1_700_000_000);
    }
}
