use crate::updater::state::{AvailableUpdate, UpdaterState};
use semver::Version;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

/// Pure helper: should we poll now given the last-poll wall clock?
pub fn should_poll(last_poll_unix: Option<u64>, interval_hours: u64, now_unix: u64) -> bool {
    match last_poll_unix {
        None => true,
        Some(last) => now_unix.saturating_sub(last) >= interval_hours * 3600,
    }
}

/// Pure helper: is `remote` a strictly newer semver than `local`?
pub fn is_newer(remote: &str, local: &str) -> bool {
    match (Version::parse(remote), Version::parse(local)) {
        (Ok(r), Ok(l)) => r > l,
        _ => false,
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn check_once(app: AppHandle) {
    let state = app.state::<UpdaterState>();
    let current = app.package_info().version.to_string();

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("updater builder failed: {e}");
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            let remote_version = update.version.clone();
            if is_newer(&remote_version, &current) {
                state.set_update(Some(AvailableUpdate {
                    version: remote_version,
                    notes_url: None,
                }));
                let _ = app.emit("update-available", state.snapshot());
                return;
            }
            state.set_update(None);
        }
        Ok(None) => {
            state.set_update(None);
        }
        Err(e) => {
            eprintln!("updater check failed: {e}");
        }
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
pub fn spawn(app: AppHandle, interval_hours: u64) {
    let app = Arc::new(app);
    tauri::async_runtime::spawn(async move {
        loop {
            check_once((*app).clone()).await;
            tokio::time::sleep(Duration::from_secs(interval_hours * 3600)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_poll_when_never_polled() {
        assert!(should_poll(None, 24, 1_000));
    }

    #[test]
    fn should_not_poll_when_inside_window() {
        assert!(!should_poll(Some(1_000), 24, 2_000));
    }

    #[test]
    fn should_poll_when_past_window() {
        assert!(should_poll(Some(0), 24, 90_000));
    }

    #[test]
    fn should_poll_at_exact_window_boundary() {
        assert!(should_poll(Some(0), 24, 86_400));
    }

    #[test]
    fn should_poll_handles_clock_skew_gracefully() {
        // now_unix smaller than last (clock went backwards). saturating_sub returns 0.
        assert!(!should_poll(Some(2_000), 24, 1_000));
    }

    #[test]
    fn is_newer_recognizes_higher_minor() {
        assert!(is_newer("0.8.0", "0.7.1"));
    }

    #[test]
    fn is_newer_rejects_equal() {
        assert!(!is_newer("0.7.1", "0.7.1"));
    }

    #[test]
    fn is_newer_rejects_lower() {
        assert!(!is_newer("0.6.0", "0.7.1"));
    }

    #[test]
    fn is_newer_returns_false_for_unparseable() {
        assert!(!is_newer("not-a-version", "0.7.1"));
        assert!(!is_newer("0.8.0", "garbage"));
    }
}
