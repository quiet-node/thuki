use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Snoozes that survive across app restarts. Stored as a JSON sidecar
/// (not in the user-editable TOML) because they are state-machine flags,
/// not preferences.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SnoozeSidecar {
    /// Unix seconds. `None` means not snoozed.
    pub settings_snoozed_until: Option<u64>,
    pub chat_snoozed_until: Option<u64>,
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
        // SnoozeSidecar holds two Option<u64> fields, so serde_json::to_string
        // is provably infallible here. expect() documents the invariant; if a
        // future field ever changes that, the panic surface is loud and local.
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
        inner.update = update;
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
    fn snooze_sidecar_load_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updater_state.json");
        std::fs::write(&path, "not valid json {{").unwrap();

        let loaded = SnoozeSidecar::load(&path).unwrap();
        assert_eq!(loaded, SnoozeSidecar::default());
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
