use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Updater state that survives across app restarts. Stored as a JSON
/// sidecar (not in the user-editable TOML) because these are state-machine
/// flags, not preferences. Holds: per-surface snooze deadlines (so "Later"
/// persists across launches) and the last-launched binary version (so the
/// startup sequence can detect a fresh upgrade and reset stale TCC grants).
///
/// Kept named `SnoozeSidecar` for back-compat with existing sidecar files
/// on user disks. Renaming would orphan their snooze state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SnoozeSidecar {
    /// Unix seconds. `None` means not snoozed.
    #[serde(default)]
    pub settings_snoozed_until: Option<u64>,
    #[serde(default)]
    pub chat_snoozed_until: Option<u64>,
    /// SemVer string of the binary that wrote this sidecar last. Used to
    /// detect upgrades on startup so we can reset the stale TCC grants
    /// macOS keeps for the previous code signature. Absent on first ever
    /// launch and on sidecars written by pre-0.8.2 builds; both cases are
    /// treated as "no upgrade detected, do nothing."
    #[serde(default)]
    pub last_launched_version: Option<String>,
    /// SemVer string of the most recent available update the manifest
    /// poller surfaced. Used to invalidate snooze deadlines when a new
    /// version arrives: a "Later" click against v0.8.2 should not silently
    /// suppress the banner once v0.8.3 ships. Tracked in the sidecar (not
    /// just in memory) so the comparison survives an app restart.
    #[serde(default)]
    pub last_seen_update_version: Option<String>,
}

impl SnoozeSidecar {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(serde_json::from_str(&s).unwrap_or_default()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        // The struct holds plain Option<u64>/Option<String> fields, so
        // serde_json::to_string is provably infallible here. expect()
        // documents the invariant; if a future field ever changes that,
        // the panic surface is loud and local.
        let s = serde_json::to_string(self).expect("SnoozeSidecar serializes");
        std::fs::write(path, s)
    }
}

/// In-memory state held in Tauri-managed state.
#[derive(Debug, Default)]
pub struct UpdaterState {
    inner: Mutex<UpdaterStateInner>,
}

#[derive(Debug, Default)]
struct UpdaterStateInner {
    pub last_check_at: Option<SystemTime>,
    pub update: Option<AvailableUpdate>,
    pub snooze: SnoozeSidecar,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AvailableUpdate {
    pub version: String,
    pub notes_url: Option<String>,
}

impl UpdaterState {
    pub fn snapshot(&self) -> UpdaterSnapshot {
        let inner = self.inner.lock().expect("updater state mutex");
        UpdaterSnapshot {
            last_check_at_unix: inner.last_check_at.and_then(system_time_to_unix),
            update: inner.update.clone(),
            settings_snoozed_until: inner.snooze.settings_snoozed_until,
            chat_snoozed_until: inner.snooze.chat_snoozed_until,
        }
    }

    pub fn set_update(&self, update: Option<AvailableUpdate>) {
        let mut inner = self.inner.lock().expect("updater state mutex");

        // Clear snoozes when a different available version arrives. A
        // "Later" click against v0.8.2 must not silently suppress the
        // banner once v0.8.3 ships. We compare against the
        // `last_seen_update_version` recorded in the sidecar (not just
        // `inner.update`, which is wiped on every app restart) so the
        // distinction between "same version, snooze still applies" and
        // "new version, snooze invalidated" survives across launches.
        let next_version = update.as_ref().map(|u| u.version.clone());
        if next_version.is_some()
            && next_version.as_deref() != inner.snooze.last_seen_update_version.as_deref()
        {
            inner.snooze.settings_snoozed_until = None;
            inner.snooze.chat_snoozed_until = None;
            inner.snooze.last_seen_update_version = next_version;
        }

        inner.update = update;
        inner.last_check_at = Some(SystemTime::now());
    }

    /// Records that a check was attempted at the current wall clock without
    /// touching `update`. Use this on transient failures (network errors,
    /// 4xx/5xx, malformed manifest) so the UI can show "Last checked X
    /// seconds ago" instead of "Never". Preserves any previously known
    /// available update so a flaky network does not erase real signal.
    pub fn mark_check_attempted(&self) {
        let mut inner = self.inner.lock().expect("updater state mutex");
        inner.last_check_at = Some(SystemTime::now());
    }

    pub fn set_chat_snooze(&self, until_unix: Option<u64>) {
        let mut inner = self.inner.lock().expect("updater state mutex");
        inner.snooze.chat_snoozed_until = until_unix;
    }

    pub fn set_settings_snooze(&self, until_unix: Option<u64>) {
        let mut inner = self.inner.lock().expect("updater state mutex");
        inner.snooze.settings_snoozed_until = until_unix;
    }

    /// Restore the previously-seen available version from a sidecar load.
    /// Called at app boot before the poller fires, so the first
    /// `set_update` call can correctly distinguish "same version, snooze
    /// still applies" from "new version, snooze invalidated."
    pub fn set_last_seen_update_version(&self, version: Option<String>) {
        let mut inner = self.inner.lock().expect("updater state mutex");
        inner.snooze.last_seen_update_version = version;
    }

    pub fn snooze_clone(&self) -> SnoozeSidecar {
        self.inner
            .lock()
            .expect("updater state mutex")
            .snooze
            .clone()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdaterSnapshot {
    pub last_check_at_unix: Option<u64>,
    pub update: Option<AvailableUpdate>,
    pub settings_snoozed_until: Option<u64>,
    pub chat_snoozed_until: Option<u64>,
}

/// Converts a `SystemTime` to Unix seconds. Returns `None` if the time is
/// before the Unix epoch (pre-epoch times cannot be represented as u64).
pub fn system_time_to_unix(t: SystemTime) -> Option<u64> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Converts a pre-built `Duration` since the Unix epoch to Unix seconds.
/// Extracted for testability: callers that need to force the None branch
/// can skip this function and pass a pre-epoch `SystemTime` to `system_time_to_unix`.
pub fn duration_to_unix_secs(d: Duration) -> u64 {
    d.as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snooze_sidecar_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updater_state.json");

        let original = SnoozeSidecar {
            settings_snoozed_until: Some(1_700_000_000),
            chat_snoozed_until: Some(1_700_001_000),
            last_launched_version: None,
            last_seen_update_version: None,
        };
        original.save(&path).unwrap();

        let loaded = SnoozeSidecar::load(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn snooze_sidecar_load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updater_state.json");

        let loaded = SnoozeSidecar::load(&path).unwrap();
        assert_eq!(loaded, SnoozeSidecar::default());
    }

    #[test]
    fn snooze_sidecar_round_trips_last_launched_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updater_state.json");

        let original = SnoozeSidecar {
            settings_snoozed_until: None,
            chat_snoozed_until: None,
            last_launched_version: Some("0.8.1".to_string()),
            last_seen_update_version: None,
        };
        original.save(&path).unwrap();

        let loaded = SnoozeSidecar::load(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn snooze_sidecar_back_compat_old_file_without_version_field() {
        // Old (pre-0.8.2) sidecar files were written without the
        // `last_launched_version` field. Loading must default it to None
        // rather than fail, otherwise existing snooze state would be lost.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updater_state.json");
        std::fs::write(
            &path,
            r#"{"settings_snoozed_until":1700000000,"chat_snoozed_until":null}"#,
        )
        .unwrap();

        let loaded = SnoozeSidecar::load(&path).unwrap();
        assert_eq!(loaded.settings_snoozed_until, Some(1_700_000_000));
        assert!(loaded.chat_snoozed_until.is_none());
        assert!(loaded.last_launched_version.is_none());
    }

    #[test]
    fn snooze_sidecar_load_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updater_state.json");
        std::fs::write(&path, "not valid json {{").unwrap();

        let loaded = SnoozeSidecar::load(&path).unwrap();
        assert_eq!(loaded, SnoozeSidecar::default());
    }

    #[test]
    fn set_update_clears_snoozes_when_new_version_arrives() {
        let state = UpdaterState::default();
        // User previously saw v0.8.2 and clicked Later on both surfaces.
        state.set_last_seen_update_version(Some("0.8.2".to_string()));
        state.set_settings_snooze(Some(1_700_000_000));
        state.set_chat_snooze(Some(1_700_000_000));

        // Manifest now reports v0.8.3.
        state.set_update(Some(AvailableUpdate {
            version: "0.8.3".to_string(),
            notes_url: None,
        }));

        let snap = state.snapshot();
        assert!(
            snap.settings_snoozed_until.is_none(),
            "settings snooze should clear when version changes"
        );
        assert!(
            snap.chat_snoozed_until.is_none(),
            "chat snooze should clear when version changes"
        );
        assert_eq!(snap.update.as_ref().unwrap().version, "0.8.3");
    }

    #[test]
    fn set_update_preserves_snoozes_when_same_version_repeats() {
        let state = UpdaterState::default();
        // User saw v0.8.2 and snoozed both surfaces.
        state.set_last_seen_update_version(Some("0.8.2".to_string()));
        state.set_settings_snooze(Some(1_700_000_000));
        state.set_chat_snooze(Some(1_700_000_001));

        // Poll runs again, manifest still reports v0.8.2.
        state.set_update(Some(AvailableUpdate {
            version: "0.8.2".to_string(),
            notes_url: None,
        }));

        let snap = state.snapshot();
        assert_eq!(
            snap.settings_snoozed_until,
            Some(1_700_000_000),
            "settings snooze must persist across same-version polls"
        );
        assert_eq!(
            snap.chat_snoozed_until,
            Some(1_700_000_001),
            "chat snooze must persist across same-version polls"
        );
    }

    #[test]
    fn set_update_records_last_seen_for_first_ever_seen_version() {
        let state = UpdaterState::default();
        // Fresh state: no snooze, no recorded last-seen version.
        state.set_update(Some(AvailableUpdate {
            version: "0.8.3".to_string(),
            notes_url: None,
        }));

        // Now seeing v0.8.3 a second time should be a no-op for snoozes.
        state.set_settings_snooze(Some(1_700_000_000));
        state.set_update(Some(AvailableUpdate {
            version: "0.8.3".to_string(),
            notes_url: None,
        }));

        let snap = state.snapshot();
        assert_eq!(
            snap.settings_snoozed_until,
            Some(1_700_000_000),
            "subsequent same-version polls must preserve user snooze"
        );
    }

    #[test]
    fn set_update_with_none_does_not_touch_snoozes() {
        let state = UpdaterState::default();
        // Manually arrange: user has a snooze and a recorded last-seen
        // version (carried over from a previous session via the sidecar).
        state.set_last_seen_update_version(Some("0.8.2".to_string()));
        state.set_settings_snooze(Some(1_700_000_000));

        // Manifest reports no update available (caught up).
        state.set_update(None);

        let snap = state.snapshot();
        assert_eq!(
            snap.settings_snoozed_until,
            Some(1_700_000_000),
            "snooze must not be cleared when manifest says no update"
        );
    }

    #[test]
    fn set_update_records_last_check_at() {
        let state = UpdaterState::default();
        state.set_update(Some(AvailableUpdate {
            version: "0.8.0".to_string(),
            notes_url: None,
        }));
        let snap = state.snapshot();
        assert!(snap.last_check_at_unix.is_some());
        assert_eq!(snap.update.as_ref().unwrap().version, "0.8.0");
    }

    #[test]
    fn mark_check_attempted_updates_timestamp_without_touching_update() {
        let state = UpdaterState::default();
        // No update yet, no last_check_at.
        assert!(state.snapshot().last_check_at_unix.is_none());

        state.mark_check_attempted();
        let snap = state.snapshot();
        assert!(snap.last_check_at_unix.is_some());
        assert!(snap.update.is_none());
    }

    #[test]
    fn mark_check_attempted_preserves_existing_update() {
        let state = UpdaterState::default();
        state.set_update(Some(AvailableUpdate {
            version: "0.9.0".to_string(),
            notes_url: None,
        }));
        let before = state.snapshot();
        let prior_ts = before.last_check_at_unix.unwrap();

        // Sleep a tick so the new timestamp differs.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        state.mark_check_attempted();
        let after = state.snapshot();

        // Update info preserved across the failed attempt.
        assert_eq!(after.update.as_ref().unwrap().version, "0.9.0");
        // Timestamp moved forward.
        assert!(after.last_check_at_unix.unwrap() > prior_ts);
    }

    #[test]
    fn set_chat_snooze_persists_in_snapshot() {
        let state = UpdaterState::default();
        state.set_chat_snooze(Some(123_456));
        assert_eq!(state.snapshot().chat_snoozed_until, Some(123_456));
    }

    #[test]
    fn set_settings_snooze_persists_in_snapshot() {
        let state = UpdaterState::default();
        state.set_settings_snooze(Some(789_012));
        assert_eq!(state.snapshot().settings_snoozed_until, Some(789_012));
    }

    #[test]
    fn snooze_clone_returns_independent_copy() {
        let state = UpdaterState::default();
        state.set_chat_snooze(Some(1));
        state.set_settings_snooze(Some(2));
        let snap = state.snooze_clone();
        assert_eq!(snap.chat_snoozed_until, Some(1));
        assert_eq!(snap.settings_snoozed_until, Some(2));
    }

    #[test]
    fn system_time_to_unix_returns_some_for_now() {
        let now = SystemTime::now();
        assert!(system_time_to_unix(now).is_some());
    }

    #[test]
    fn snooze_sidecar_load_returns_err_for_real_io_error() {
        // Reading a directory as a file produces an IsADirectory io::Error,
        // which is not NotFound and should propagate as Err.
        let dir = tempfile::tempdir().unwrap();
        // The tempdir path itself is a directory; read_to_string on it fails
        // with IsADirectory (not NotFound) on macOS/Linux.
        let result = SnoozeSidecar::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn system_time_to_unix_returns_none_for_pre_epoch() {
        // Construct a time before the Unix epoch so duration_since returns Err.
        let pre_epoch = UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(system_time_to_unix(pre_epoch), None);
    }

    #[test]
    fn duration_to_unix_secs_extracts_seconds() {
        assert_eq!(duration_to_unix_secs(Duration::from_secs(42)), 42);
        assert_eq!(duration_to_unix_secs(Duration::from_millis(1500)), 1);
    }
}
