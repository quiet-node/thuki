//! Unit tests for the launch circuit breaker.
//!
//! Only the pure surface is exercised here: `decide_session`, the
//! `StartupSafety` verdict and its snapshot, and the durable record helpers.
//! The `coverage(off)` I/O and FFI wrappers (`run_startup_guard`, `clean_init`,
//! `boot_time_secs`, the lock/fsync helpers, path resolvers,
//! `disable_quit_keeps_windows`) are thin and exempt by contract; several are
//! still driven live below to assert their real behavior.

use super::*;

use std::os::unix::io::AsRawFd;

// ---------------------------------------------------------------------------
// StartupSafety verdict + wire snapshot.
// ---------------------------------------------------------------------------

/// A clean decision produces a non-safe-mode verdict with a null cause and idle
/// activity: the normal first-run / clean-restart path.
#[test]
fn startup_safety_from_clean_decision() {
    let decision = decide_session(None, 0, 2);
    let safety = StartupSafety::from_decision(&decision, SessionActivity::idle());
    assert!(!safety.safe_mode());
    assert_eq!(safety.unclean_count(), 0);
    assert_eq!(
        safety.snapshot(),
        StartupSafetySnapshot {
            safe_mode: false,
            unclean_count: 0,
            cause: None,
            activity: SessionActivity::idle(),
        }
    );
}

/// An abnormal decision at threshold engages safe mode, mirrors the streak into
/// `unclean_count`, carries the classified cause string, and preserves the
/// previous session's activity as recovery-UI context.
#[test]
fn startup_safety_from_abnormal_decision() {
    let decision = decide_session(Some(abnormal_prev(1, 5, SessionState::Ok)), 5, 2);
    let activity = SessionActivity {
        kind: ActivityKind::LoadingModel,
        model_id: Some("llama".into()),
    };
    let safety = StartupSafety::from_decision(&decision, activity.clone());
    assert!(safety.safe_mode());
    assert_eq!(safety.unclean_count(), 2);
    assert_eq!(
        safety.snapshot(),
        StartupSafetySnapshot {
            safe_mode: true,
            unclean_count: 2,
            cause: Some("process_died"),
            activity,
        }
    );
}

/// Each abnormal cause maps to its stable snake_case wire string, and a clean
/// verdict maps to a null cause.
#[test]
fn snapshot_maps_every_cause_string() {
    let cases = [
        (AbnormalCause::Crashed, "crashed"),
        (AbnormalCause::MachineRestart, "machine_restart"),
        (AbnormalCause::ProcessDied, "process_died"),
    ];
    for (cause, wire) in cases {
        let decision = SessionDecision {
            outcome: SessionOutcome::Abnormal,
            streak: 2,
            safe_mode: true,
            cause: Some(cause),
        };
        let snap = StartupSafety::from_decision(&decision, SessionActivity::idle()).snapshot();
        assert_eq!(snap.cause, Some(wire));
    }
    let clean = SessionDecision {
        outcome: SessionOutcome::Clean,
        streak: 0,
        safe_mode: false,
        cause: None,
    };
    assert_eq!(
        StartupSafety::from_decision(&clean, SessionActivity::idle())
            .snapshot()
            .cause,
        None
    );
}

// ---------------------------------------------------------------------------
// Pure decision logic.
// ---------------------------------------------------------------------------

/// Builds an abnormal previous record (`clean_exit: false`) with the given
/// streak, boot time, and liveness state; other fields are fixed defaults.
fn abnormal_prev(streak: u32, boot: i64, state: SessionState) -> SessionRecord {
    SessionRecord {
        schema: SESSION_SCHEMA,
        boot_time_secs: boot,
        started_at_secs: 1_000,
        clean_exit: false,
        state,
        activity: SessionActivity {
            kind: ActivityKind::Idle,
            model_id: None,
        },
        consecutive_abnormal: streak,
    }
}

/// A missing previous record is clean: no safe mode, no cause.
#[test]
fn decide_session_missing_prev_is_clean() {
    let d = decide_session(None, 123, 1);
    assert_eq!(d.outcome, SessionOutcome::Clean);
    assert_eq!(d.streak, 0);
    assert!(!d.safe_mode);
    assert_eq!(d.cause, None);
}

/// A clean-exit previous record resets any prior streak to zero.
#[test]
fn decide_session_clean_prev_resets_streak() {
    let mut prev = abnormal_prev(9, 5, SessionState::Ok);
    prev.clean_exit = true;
    let d = decide_session(Some(prev), 5, 1);
    assert_eq!(d.outcome, SessionOutcome::Clean);
    assert_eq!(d.streak, 0);
    assert!(!d.safe_mode);
    assert_eq!(d.cause, None);
}

/// A record whose schema is not the current one fails open to clean.
#[test]
fn decide_session_wrong_schema_is_clean() {
    let mut prev = abnormal_prev(3, 5, SessionState::Ok);
    prev.schema = SESSION_SCHEMA + 1;
    let d = decide_session(Some(prev), 5, 1);
    assert_eq!(d.outcome, SessionOutcome::Clean);
    assert!(!d.safe_mode);
    assert_eq!(d.cause, None);
}

/// An abnormal previous record grows the streak by one; below threshold it does
/// not engage safe mode, and same-boot no-panic classifies as ProcessDied.
#[test]
fn decide_session_abnormal_increments_streak_below_threshold() {
    let d = decide_session(Some(abnormal_prev(2, 5, SessionState::Ok)), 5, 5);
    assert_eq!(d.outcome, SessionOutcome::Abnormal);
    assert_eq!(d.streak, 3);
    assert!(!d.safe_mode);
    assert_eq!(d.cause, Some(AbnormalCause::ProcessDied));
}

/// The streak reaching the threshold engages safe mode.
#[test]
fn decide_session_reaches_threshold_engages_safe_mode() {
    let d = decide_session(Some(abnormal_prev(1, 5, SessionState::Ok)), 5, 2);
    assert!(d.safe_mode);
    assert_eq!(d.streak, 2);
}

/// The default threshold semantics (2): a single abnormal launch stays out of
/// safe mode, and only a second consecutive abnormal launch engages it. This is
/// the "one hard reboot / `kill -9` does not nag" behavior the threshold bump
/// exists to give.
#[test]
fn threshold_two_requires_two_consecutive() {
    // First abnormal launch: prior streak 0 -> this streak 1, still below 2.
    let first = decide_session(Some(abnormal_prev(0, 5, SessionState::Ok)), 5, 2);
    assert_eq!(first.streak, 1);
    assert!(
        !first.safe_mode,
        "one abnormal launch must not enter safe mode"
    );
    // Second consecutive abnormal launch: prior streak 1 -> this streak 2.
    let second = decide_session(Some(abnormal_prev(1, 5, SessionState::Ok)), 5, 2);
    assert_eq!(second.streak, 2);
    assert!(
        second.safe_mode,
        "two consecutive abnormal launches enter safe mode"
    );
}

/// The streak saturates at u32::MAX rather than overflowing.
#[test]
fn decide_session_streak_saturates() {
    let d = decide_session(Some(abnormal_prev(u32::MAX, 5, SessionState::Ok)), 5, 1);
    assert_eq!(d.streak, u32::MAX);
    assert!(d.safe_mode);
}

/// A recorded panic classifies as Crashed even when the boot time also changed:
/// state wins over boot.
#[test]
fn cause_crashed_wins_over_boot_change() {
    let d = decide_session(Some(abnormal_prev(0, 5, SessionState::Crashed)), 999, 1);
    assert_eq!(d.cause, Some(AbnormalCause::Crashed));
}

/// A changed boot time with no recorded panic classifies as MachineRestart.
#[test]
fn cause_machine_restart_when_boot_differs() {
    let d = decide_session(Some(abnormal_prev(0, 5, SessionState::Ok)), 6, 1);
    assert_eq!(d.cause, Some(AbnormalCause::MachineRestart));
}

/// The same boot time with no recorded panic classifies as ProcessDied.
#[test]
fn cause_process_died_when_boot_same() {
    let d = decide_session(Some(abnormal_prev(0, 42, SessionState::Ok)), 42, 1);
    assert_eq!(d.cause, Some(AbnormalCause::ProcessDied));
}

/// INVARIANT: safe_mode is identical whether or not the boot time changed, for
/// the same `clean_exit`/streak inputs. This locks in the rule that boot time
/// never gates safety: it only classifies the cause. This is the exact rule the
/// task warns not to get backwards (an OOM-kill happens on the same boot).
#[test]
fn safe_mode_ignores_boot_time() {
    let same_boot = decide_session(Some(abnormal_prev(2, 100, SessionState::Ok)), 100, 3);
    let diff_boot = decide_session(Some(abnormal_prev(2, 100, SessionState::Ok)), 555, 3);
    assert_eq!(same_boot.safe_mode, diff_boot.safe_mode);
    assert_eq!(same_boot.streak, diff_boot.streak);
    // Only the cause differs, proving boot affects classification alone.
    assert_eq!(same_boot.cause, Some(AbnormalCause::ProcessDied));
    assert_eq!(diff_boot.cause, Some(AbnormalCause::MachineRestart));
}

/// INVARIANT: safe_mode is identical across every activity value, proving
/// activity is UI context only and never gates safety.
#[test]
fn safe_mode_ignores_activity() {
    let kinds = [
        ActivityKind::Idle,
        ActivityKind::LoadingModel,
        ActivityKind::Downloading,
    ];
    let mut verdicts = kinds.iter().map(|&kind| {
        let mut prev = abnormal_prev(1, 7, SessionState::Ok);
        prev.activity = SessionActivity {
            kind,
            model_id: Some("m".into()),
        };
        decide_session(Some(prev), 7, 2).safe_mode
    });
    let first = verdicts.next().unwrap();
    assert!(first, "streak 2 at threshold 2 must engage safe mode");
    assert!(verdicts.all(|v| v == first));
}

// ---------------------------------------------------------------------------
// Durable record I/O (driven live).
// ---------------------------------------------------------------------------

/// A record written through the durable helper reads back identical.
#[test]
fn session_record_round_trips_through_durable_write() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    let record = SessionRecord::launch(1234, 5678, 4);
    // Exercise the derived Clone on the non-Copy record so the coverage gate
    // sees it.
    assert_eq!(record.clone(), record);
    durable_write_record(&path, &record).unwrap();
    assert_eq!(read_record(&path), Some(record));
}

/// A missing record file reads as None (never an error, never a panic).
#[test]
fn read_record_missing_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    assert_eq!(read_record(&dir.path().join("absent.json")), None);
}

/// A corrupt record file reads as None, so a bad file cannot trip safe mode.
#[test]
fn read_record_corrupt_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    std::fs::write(&path, b"{ not valid json").unwrap();
    assert_eq!(read_record(&path), None);
}

/// A well-formed but OLD-FORMAT file (the pre-rework `startup_guard.json`
/// shape, which has none of the session-record fields) reads as None and
/// therefore decides clean. This is the upgrade migration path: the stale file
/// cannot deserialize into a `SessionRecord`, so it fails open and never trips
/// safe mode.
#[test]
fn old_format_record_reads_as_clean() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    std::fs::write(&path, br#"{"launch_dirty":true,"consecutive_unclean":5}"#).unwrap();
    let prev = read_record(&path);
    assert_eq!(prev, None);
    let d = decide_session(prev, 42, 2);
    assert_eq!(d.outcome, SessionOutcome::Clean);
    assert!(!d.safe_mode);
    assert_eq!(d.cause, None);
}

/// `mark_crashed` durably flips state to Crashed without marking a clean exit.
#[test]
fn mark_crashed_sets_state() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    durable_write_record(&path, &SessionRecord::launch(1, 2, 0)).unwrap();
    mark_crashed(&path);
    let after = read_record(&path).unwrap();
    assert_eq!(after.state, SessionState::Crashed);
    assert!(!after.clean_exit);
}

/// The lock fd is opened CLOEXEC, so it cannot leak into the spawned
/// `llama-server` child and outlive Thuki. Asserted directly rather than
/// trusting Rust's default.
#[test]
fn session_lock_fd_is_cloexec() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("session.lock");
    let file = open_lock_file(&path).unwrap();
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    assert!(flags >= 0, "F_GETFD failed");
    assert!(flags & libc::FD_CLOEXEC != 0, "lock fd must be CLOEXEC");
}

/// Taking the lock a second time while the first is still held reports
/// AlreadyRunning rather than inferring a crash.
#[test]
fn session_lock_twice_reports_already_running() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("session.lock");
    // Bind the first lock so its File outlives the second acquire; a bare `_`
    // would drop it immediately and release the lock.
    let _guard = match acquire_session_lock(&path).unwrap() {
        SessionLock::Acquired(f) => f,
        SessionLock::AlreadyRunning => panic!("first acquire must succeed"),
    };
    match acquire_session_lock(&path).unwrap() {
        SessionLock::AlreadyRunning => {}
        SessionLock::Acquired(_) => {
            panic!("second acquire must report AlreadyRunning, not infer a crash")
        }
    }
}

/// The activity setter durably persists the new activity.
#[test]
fn set_activity_persists() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    let launch = SessionRecord::launch(1, 2, 0);
    durable_write_record(&path, &launch).unwrap();
    let writer = SessionWriter::new(path.clone(), launch);
    let activity = SessionActivity {
        kind: ActivityKind::Downloading,
        model_id: Some("qwen".into()),
    };
    writer.set_activity(activity.clone()).unwrap();
    assert_eq!(read_record(&path).unwrap().activity, activity);
}

/// `SessionGuard::writer` exposes the record writer when this process owns the
/// session and `None` when it does not (another instance held the lock).
#[test]
fn session_guard_writer_reflects_ownership() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    let writer = SessionWriter::new(path, SessionRecord::launch(1, 2, 0));
    let owned = SessionGuard {
        _lock: None,
        writer: Some(writer),
    };
    assert!(owned.writer().is_some());
    let not_owned = SessionGuard {
        _lock: None,
        writer: None,
    };
    assert!(not_owned.writer().is_none());
}

/// Marking a clean exit durably sets `clean_exit: true`.
#[test]
fn mark_clean_exit_persists() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    let launch = SessionRecord::launch(1, 2, 0);
    durable_write_record(&path, &launch).unwrap();
    let writer = SessionWriter::new(path.clone(), launch);
    writer.mark_clean_exit().unwrap();
    assert!(read_record(&path).unwrap().clean_exit);
}

/// The wire format is stable: enums serialize to the documented snake_case /
/// lowercase strings, and a record round-trips through JSON unchanged. Also
/// exercises the `LoadingModel` and `Crashed` variants.
#[test]
fn session_record_serde_wire_format() {
    let mut record = SessionRecord::launch(10, 20, 1);
    record.state = SessionState::Crashed;
    record.activity = SessionActivity {
        kind: ActivityKind::LoadingModel,
        model_id: Some("m".into()),
    };
    let json = serde_json::to_string(&record).unwrap();
    assert!(json.contains("\"state\":\"crashed\""), "{json}");
    assert!(json.contains("\"loading_model\""), "{json}");
    let back: SessionRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(back, record);
}
