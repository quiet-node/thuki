use crate::config::defaults::{DEFAULT_UPDATER_STATE_FILENAME, MAX_UPDATER_SNOOZE_HOURS};
use crate::updater::poller;
use crate::updater::state::{UpdaterSnapshot, UpdaterState};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::UpdaterExt;

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
    let until = snooze_deadline(unix_now(), hours);
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
    let until = snooze_deadline(unix_now(), hours);
    state.set_settings_snooze(Some(until));
    persist_sidecar(&state, &app)
}

/// Computes the absolute Unix-second deadline for a snooze request crossing
/// the IPC trust boundary. Clamps `hours` to `MAX_UPDATER_SNOOZE_HOURS` and
/// uses saturating arithmetic so a hostile or buggy caller cannot wrap the
/// `u64` math and produce a deadline in the past.
pub fn snooze_deadline(now_unix: u64, hours: u64) -> u64 {
    let clamped = hours.min(MAX_UPDATER_SNOOZE_HOURS);
    now_unix.saturating_add(clamped.saturating_mul(3600))
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
    Ok(dir.join(DEFAULT_UPDATER_STATE_FILENAME))
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

    #[test]
    fn snooze_deadline_handles_normal_input() {
        // 24 h from epoch landmark: 1_700_000_000 + 86_400.
        assert_eq!(snooze_deadline(1_700_000_000, 24), 1_700_086_400);
    }

    #[test]
    fn snooze_deadline_clamps_to_max_hours() {
        // Anything beyond MAX is treated as MAX.
        let capped = snooze_deadline(0, MAX_UPDATER_SNOOZE_HOURS + 1);
        let at_cap = snooze_deadline(0, MAX_UPDATER_SNOOZE_HOURS);
        assert_eq!(capped, at_cap);
    }

    #[test]
    fn snooze_deadline_saturates_on_extreme_input() {
        // Even uncapped, saturating arithmetic must never wrap to a small
        // value. Pass the maximum possible u64 to confirm the saturation.
        let result = snooze_deadline(u64::MAX, u64::MAX);
        assert_eq!(result, u64::MAX);
    }

    #[test]
    fn snooze_deadline_zero_hours_is_now() {
        assert_eq!(snooze_deadline(1_700_000_000, 0), 1_700_000_000);
    }
}
