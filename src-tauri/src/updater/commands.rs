use crate::config::defaults::{DEFAULT_UPDATER_STATE_FILENAME, MAX_UPDATER_SNOOZE_HOURS};
use crate::updater::poller;
use crate::updater::state::{SnoozeSidecar, UpdaterSnapshot, UpdaterState};
use crate::updater::tcc_reset;
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
    install_update_inner(app).await
}

/// Shared install-and-restart routine. Re-checks the manifest (rather than
/// trusting the in-memory `UpdaterState`), downloads the signed payload,
/// verifies the ed25519 signature against the public key compiled into the
/// app, swaps the running `.app`, and relaunches.
///
/// Exposed to the tray click handler so clicking "Update Thuki to vX.Y.Z"
/// triggers the install directly without forcing the user to detour through
/// the Settings banner. The Settings banner button calls the
/// `install_update` Tauri command, which delegates here.
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn install_update_inner(app: AppHandle) -> Result<(), String> {
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

/// Writes the current in-memory snooze sidecar to disk. Pub so the
/// poller can also persist (e.g., after a manifest poll cleared snoozes
/// in response to a new version arriving).
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn persist_sidecar(state: &UpdaterState, app: &AppHandle) -> Result<(), String> {
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

/// Stores `service` on `sidecar` so the post-restart launch can resume the
/// grant flow. Returns `Err` when the service string is not one of the
/// values Thuki resets at click time, so callers cannot smuggle arbitrary
/// strings into a later `tccutil reset` invocation.
pub fn prepare_pending_reregister(
    sidecar: &mut SnoozeSidecar,
    service: &str,
) -> Result<&'static str, String> {
    let canonical = tcc_reset::validate_click_time_service(service)
        .ok_or_else(|| format!("unsupported tcc service: {service}"))?;
    sidecar.pending_reregister = Some(canonical.to_string());
    Ok(canonical)
}

/// Removes `pending_reregister` from `sidecar` and returns its previous
/// value. The caller is responsible for persisting the cleared sidecar so
/// the resume flow does not loop on the next restart.
pub fn take_pending_reregister(sidecar: &mut SnoozeSidecar) -> Option<String> {
    sidecar.pending_reregister.take()
}

/// Pure decision helper. Returns `true` when the click-time reset can be
/// skipped because the startup path already cleared TCC for the running
/// version. The marker survives A's reset+restart by being persisted to
/// the sidecar, which is why the comparison is meaningful even though
/// `was_reset_at_startup` would always be `false` on a freshly relaunched
/// process.
pub fn click_time_reset_can_skip(
    last_reset_for_version: Option<&str>,
    running_version: &str,
) -> bool {
    last_reset_for_version == Some(running_version)
}

/// Click-time grant flow: persist a "resume after restart" marker, clear
/// the stale TCC entry for the requested service, and relaunch. The
/// frontend hands the service string straight in, so the validator inside
/// `prepare_pending_reregister` is the trust boundary.
///
/// Returns `true` when a relaunch has been scheduled and `false` when the
/// running process already has a clean TCC slate (the startup path's most
/// recent reset matches the running version). In the `false` case the
/// frontend should run the in-line open-Settings + polling flow without
/// expecting a relaunch.
///
/// Sequencing matters when a relaunch is scheduled. Sidecar must be saved
/// BEFORE `tccutil reset` runs so a crash between the two does not leave
/// the user with a cleared grant and no resume marker. The restart is
/// deferred so Tauri can finish dispatching the IPC reply (otherwise the
/// frontend sees a disconnect error rather than a clean relaunch).
#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub fn reset_and_relaunch_for_grant(
    app: AppHandle,
    state: State<'_, UpdaterState>,
    service: String,
) -> Result<bool, String> {
    // Validate first so a hostile string never reaches `tccutil` even when
    // the startup-clean path skips the reset.
    let canonical = tcc_reset::validate_click_time_service(&service)
        .ok_or_else(|| format!("unsupported tcc service: {service}"))?;

    let running = app.package_info().version.to_string();
    let snooze = state.snooze_clone();
    if click_time_reset_can_skip(snooze.last_reset_for_version.as_deref(), &running) {
        // Startup path already reset TCC for this exact version, so the
        // running binary's csreq already owns whatever TCC entries (if
        // any) System Settings will display. A second reset+relaunch
        // would only add a jarring quit on every grant click.
        return Ok(false);
    }

    let mut snooze = snooze;
    prepare_pending_reregister(&mut snooze, canonical)?;

    let path = sidecar_path(&app)?;
    snooze.save(&path).map_err(|e| e.to_string())?;
    state.set_pending_reregister(Some(canonical.to_string()));

    let bundle_id = app.config().identifier.clone();
    tcc_reset::tccutil_reset_service(&bundle_id, canonical);

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        eprintln!(
            "thuki: [updater] relaunching after click-time TCC reset \
             to refresh tccd PID tracking"
        );
        app_handle.restart();
    });

    Ok(true)
}

/// Frontend-facing companion to `reset_and_relaunch_for_grant`. Reads the
/// `pending_reregister` flag, clears it (in memory and on disk), and
/// returns the value so PermissionsStep can resume the right step on a
/// fresh launch without forcing the user to click a second time.
#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub fn consume_pending_grant_resume(
    app: AppHandle,
    state: State<'_, UpdaterState>,
) -> Result<Option<String>, String> {
    let mut snooze = state.snooze_clone();
    let value = take_pending_reregister(&mut snooze);
    if value.is_some() {
        let path = sidecar_path(&app)?;
        snooze.save(&path).map_err(|e| e.to_string())?;
        state.set_pending_reregister(None);
    }
    Ok(value)
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

    #[test]
    fn prepare_pending_reregister_accepts_accessibility() {
        let mut sidecar = SnoozeSidecar::default();
        let canonical = prepare_pending_reregister(&mut sidecar, "Accessibility").unwrap();
        assert_eq!(canonical, "Accessibility");
        assert_eq!(sidecar.pending_reregister.as_deref(), Some("Accessibility"));
    }

    #[test]
    fn prepare_pending_reregister_accepts_screen_capture() {
        let mut sidecar = SnoozeSidecar::default();
        let canonical = prepare_pending_reregister(&mut sidecar, "ScreenCapture").unwrap();
        assert_eq!(canonical, "ScreenCapture");
        assert_eq!(sidecar.pending_reregister.as_deref(), Some("ScreenCapture"));
    }

    #[test]
    fn prepare_pending_reregister_rejects_unsupported_service() {
        let mut sidecar = SnoozeSidecar::default();
        let err = prepare_pending_reregister(&mut sidecar, "Camera").unwrap_err();
        assert!(err.contains("Camera"), "error must surface offending value");
        // Sidecar must remain untouched on rejection so a hostile call
        // cannot pollute the persisted resume marker.
        assert!(sidecar.pending_reregister.is_none());
    }

    #[test]
    fn take_pending_reregister_returns_and_clears_value() {
        let mut sidecar = SnoozeSidecar {
            pending_reregister: Some("Accessibility".to_string()),
            ..SnoozeSidecar::default()
        };
        assert_eq!(
            take_pending_reregister(&mut sidecar),
            Some("Accessibility".to_string()),
        );
        assert!(sidecar.pending_reregister.is_none());
    }

    #[test]
    fn take_pending_reregister_returns_none_when_unset() {
        let mut sidecar = SnoozeSidecar::default();
        assert!(take_pending_reregister(&mut sidecar).is_none());
    }

    #[test]
    fn click_time_reset_can_skip_when_versions_match() {
        assert!(click_time_reset_can_skip(Some("0.8.5"), "0.8.5"));
    }

    #[test]
    fn click_time_reset_does_not_skip_when_versions_differ() {
        assert!(!click_time_reset_can_skip(Some("0.8.4"), "0.8.5"));
    }

    #[test]
    fn click_time_reset_does_not_skip_when_marker_is_absent() {
        // No prior startup reset for this binary recorded: a stale csreq
        // grant could still be on disk, so the click MUST clean it up.
        assert!(!click_time_reset_can_skip(None, "0.8.5"));
    }
}
