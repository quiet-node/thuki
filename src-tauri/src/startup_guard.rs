//! Launch circuit breaker for issue #296.
//!
//! On a memory-constrained Mac, Thuki froze the whole machine while
//! auto-loading a large model at startup. After a forced power-off macOS
//! relaunched the app (its reopen-after-unclean-shutdown behavior) and the same
//! no-user-action auto-startup work re-ran and re-froze the machine before the
//! user could intervene: a deadloop.
//!
//! Thuki hides on window close and quits only from the tray, so a clean-exit
//! signal almost never fires during a healthy session. Crash-loop detection
//! therefore MUST NOT depend on one. This module proves liveness two ways that
//! survive process death by ANY cause:
//!
//! - a process-lifetime advisory lock (`flock`) the kernel releases on death by
//!   any cause (clean exit, panic, SIGKILL, OS OOM-kill, power loss), and
//! - a write-ahead session record that is durably `clean_exit: false` on disk
//!   BEFORE any dangerous auto-op runs. A launch that finds the previous
//!   record still `clean_exit: false` knows the previous process died
//!   abnormally.
//!
//! Modeled on Firefox `nsAppStartup`'s profile lock and Sentry's
//! crashed-vs-abnormal session distinction. Safe mode engages purely on
//! `clean_exit == false` plus the consecutive-abnormal streak reaching the
//! threshold; `boot_time_secs` and `activity` classify the CAUSE for the
//! recovery message only and never gate safety.
//!
//! The module is split into pure decision logic (fully unit-tested, no I/O) and
//! thin `coverage(off)` I/O/FFI wrappers that mirror the forgiving,
//! never-panic-on-bad-input contract of `config::loader`/`config::writer`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The session-record schema version this build reads and writes. A record
/// whose schema is not this value is treated as clean (fails open), so an
/// older on-disk format can never by itself trip safe mode.
const SESSION_SCHEMA: u32 = 1;

// ---------------------------------------------------------------------------
// This launch's verdict (immutable managed state) and its wire shape.
// ---------------------------------------------------------------------------

/// Immutable managed state holding this launch's circuit-breaker verdict.
///
/// Read from multiple subsystems and threads (the auto-prime gate in
/// `show_overlay`, the `startup_safety` command). Every field is set once at
/// construction by [`from_decision`] and is never mutated for the lifetime of
/// the process, so the struct is a plain bundle of primitives plus one small
/// enum/struct. Immutable values are `Sync`, so it satisfies Tauri's
/// managed-state `Send + Sync` requirement with no atomics or locks.
///
/// Invariant: the verdict is a FACT about THIS launch and must stay fixed for
/// the whole session. The on-disk session record mutates (activity, clean
/// exit) through [`SessionWriter`], which is a SEPARATE managed value; the two
/// must never be conflated. Clearing the verdict mid-launch would erase the
/// auto-prime gate before the dangerous op it guards ever runs.
///
/// [`from_decision`]: StartupSafety::from_decision
#[derive(Debug)]
pub struct StartupSafety {
    safe_mode: bool,
    unclean_count: u32,
    cause: Option<AbnormalCause>,
    activity: SessionActivity,
}

impl StartupSafety {
    /// Builds the managed verdict from a [`SessionDecision`] plus the activity
    /// the previous session was performing at its last write (context for the
    /// recovery message). Used once at startup; the result is then immutable
    /// for the process lifetime.
    pub(crate) fn from_decision(
        decision: &SessionDecision,
        prev_activity: SessionActivity,
    ) -> Self {
        Self {
            safe_mode: decision.safe_mode,
            unclean_count: decision.streak,
            cause: decision.cause,
            activity: prev_activity,
        }
    }

    /// Whether this launch is in safe mode. Cheap field read; safe to call from
    /// any thread on any auto-op path.
    pub fn safe_mode(&self) -> bool {
        self.safe_mode
    }

    /// The consecutive-abnormal streak that produced the current verdict.
    pub fn unclean_count(&self) -> u32 {
        self.unclean_count
    }

    /// A serializable view of the current verdict for the frontend.
    pub fn snapshot(&self) -> StartupSafetySnapshot {
        StartupSafetySnapshot {
            safe_mode: self.safe_mode,
            unclean_count: self.unclean_count,
            cause: self.cause.map(AbnormalCause::as_wire_str),
            activity: self.activity.clone(),
        }
    }
}

/// The wire shape returned by the `startup_safety` command. The frontend
/// renders a recovery screen from this after an abnormal-launch streak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupSafetySnapshot {
    /// Whether this launch skipped the dangerous auto-startup operations.
    pub safe_mode: bool,
    /// How many consecutive launches failed to reach a clean exit.
    pub unclean_count: u32,
    /// The classified cause of the abnormal streak, or `None` on a clean
    /// launch. One of `"crashed"`, `"machine_restart"`, `"process_died"`.
    pub cause: Option<&'static str>,
    /// What the previous session was doing at its last write. Recovery-UI
    /// context only; never gates safety.
    pub activity: SessionActivity,
}

/// Command: returns the current circuit-breaker verdict so the frontend can
/// render a recovery screen after an abnormal-launch streak. Thin coverage-off
/// wrapper over the managed [`StartupSafety`] state.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn startup_safety(state: tauri::State<'_, StartupSafety>) -> StartupSafetySnapshot {
    state.snapshot()
}

// ---------------------------------------------------------------------------
// The single launch routine and the process-lifetime session handle.
// ---------------------------------------------------------------------------

/// Process-lifetime managed state that keeps the advisory lock held and owns
/// the live session record's writer.
///
/// The lock `File` is stored here so its fd stays open for the whole process:
/// dropping it releases the lock, so it MUST live as long as the app. The
/// `writer` mutates the on-disk record (activity, clean exit) under its own
/// mutex; it is `None` only when this process does not own the session (another
/// instance already held the lock, or the config dir could not be resolved).
pub struct SessionGuard {
    /// Held for the whole process; dropping releases the kernel advisory lock.
    /// Never read: its sole job is to keep the fd alive.
    _lock: Option<File>,
    /// Writer for the live record, or `None` when this process does not own the
    /// session and therefore must not write it.
    writer: Option<SessionWriter>,
}

impl SessionGuard {
    /// Borrows the session-record writer, when this process owns the session.
    /// Callers (clean-exit marking, and the follow-up activity wiring) get
    /// `None` when another instance owns the lock and must not write.
    pub(crate) fn writer(&self) -> Option<&SessionWriter> {
        self.writer.as_ref()
    }
}

/// The pieces `lib.rs` installs as managed state after the launch routine runs:
/// this launch's immutable verdict and the process-lifetime session handle.
pub struct StartupInit {
    /// This launch's immutable safe-mode verdict.
    pub safety: StartupSafety,
    /// The advisory lock + record writer, kept alive for the process lifetime.
    pub guard: SessionGuard,
}

/// Runs the launch circuit breaker once at startup, BEFORE any dangerous
/// auto-op can run, and returns the state `lib.rs` installs.
///
/// Sequence:
/// 1. Take the process-lifetime advisory lock. If another instance already
///    holds it, this is a legitimate second instance, NOT a crash: proceed with
///    a clean, non-safe-mode verdict and do not touch the record.
/// 2. Install the panic hook so a Rust panic durably records `state: crashed`.
/// 3. Read the previous record, classify it with [`decide_session`].
/// 4. Durably write THIS launch's record (`clean_exit: false`) so a freeze
///    during the dangerous window leaves the abnormal marker behind.
///
/// Best-effort throughout: any I/O failure is logged and the in-memory decision
/// still stands, so the guard degrades to "no safe mode" rather than blocking
/// the app.
///
/// Coverage-off thin wrapper: the safety decision is [`decide_session`] and the
/// I/O is the covered/exempt helpers below.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn run_startup_guard(app: &tauri::AppHandle, threshold: u32) -> StartupInit {
    let (Some(record_path), Some(lock_path)) = (session_record_path(app), session_lock_path(app))
    else {
        // The per-user config dir could not be resolved. Config load already
        // ran fatally if the dir were truly unusable, so this is only the
        // theoretical resolver-failure path: degrade to no safe mode, no lock,
        // no writer, rather than block startup.
        return clean_init(threshold);
    };

    match acquire_session_lock(&lock_path) {
        Ok(SessionLock::Acquired(file)) => {
            // We own the session. Refine the CAUSE to `crashed` if a panic
            // unwinds before exit.
            install_panic_hook(record_path.clone());

            let boot = boot_time_secs();
            let prev = read_record(&record_path);
            // The previous session's activity is context for the recovery
            // message; captured before `prev` is consumed by `decide_session`.
            let prev_activity = prev
                .as_ref()
                .map(|p| p.activity.clone())
                .unwrap_or_else(SessionActivity::idle);
            let decision = decide_session(prev, boot, threshold);

            let record = SessionRecord::launch(boot, now_secs(), decision.streak);
            // why: WRITE-AHEAD durability. The launch record must be on disk
            // with `clean_exit: false` BEFORE this function returns, because it
            // returns before any dangerous auto-op (engine spawn, overlay
            // auto-prime, downloads) can run. A freeze anywhere in that window
            // then leaves `clean_exit: false` behind for the next launch to
            // detect. This is a write-ahead guarantee, not merely atomicity.
            if let Err(e) = durable_write_record(&record_path, &record) {
                eprintln!(
                    "thuki: [startup_guard] failed to write launch record to {}: {e}",
                    record_path.display()
                );
            }
            let writer = SessionWriter::new(record_path, record);

            StartupInit {
                safety: StartupSafety::from_decision(&decision, prev_activity),
                guard: SessionGuard {
                    _lock: Some(file),
                    writer: Some(writer),
                },
            }
        }
        Ok(SessionLock::AlreadyRunning) => {
            // Another live instance owns the session. Do NOT infer a crash,
            // rewrite the record, install the panic hook, or enter safe mode:
            // proceed clean and leave the live instance's record untouched.
            clean_init(threshold)
        }
        Err(e) => {
            eprintln!("thuki: [startup_guard] could not take session lock: {e}. treating as clean");
            clean_init(threshold)
        }
    }
}

/// Builds a clean, non-safe-mode [`StartupInit`] that owns no lock and no
/// writer. Used on the resolver-failure, lock-error, and already-running paths.
///
/// Coverage-off: constructed only from real-startup paths; the pure decision it
/// wraps ([`decide_session`] with `None`) is covered directly.
#[cfg_attr(coverage_nightly, coverage(off))]
fn clean_init(threshold: u32) -> StartupInit {
    let decision = decide_session(None, boot_time_secs(), threshold);
    StartupInit {
        safety: StartupSafety::from_decision(&decision, SessionActivity::idle()),
        guard: SessionGuard {
            _lock: None,
            writer: None,
        },
    }
}

/// Unix seconds now, saturating to 0 on the (impossible) pre-epoch clock.
///
/// Coverage-off: reads the system clock; carries no branch logic to test.
#[cfg_attr(coverage_nightly, coverage(off))]
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Session record schema + pure decision logic (no I/O).
// ---------------------------------------------------------------------------

/// Liveness state of a session, as recorded on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SessionState {
    /// Running normally, or exited cleanly.
    Ok,
    /// The panic hook fired: the process is unwinding from a Rust panic.
    Crashed,
}

/// What the app was doing when the record was last written. Context for the
/// recovery UI only; it never influences the safe-mode decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivityKind {
    /// No dangerous auto-op in flight.
    Idle,
    /// A model load / prime is in flight.
    LoadingModel,
    /// A model download is in flight.
    Downloading,
}

/// The activity in flight when the record was last written. Part of the
/// `startup_safety` command's wire shape (see [`StartupSafetySnapshot`]), so it
/// is `pub`; its fields stay crate-internal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionActivity {
    /// The class of work in flight.
    pub(crate) kind: ActivityKind,
    /// The model involved, when the activity concerns a specific model.
    pub(crate) model_id: Option<String>,
}

impl SessionActivity {
    /// The no-activity value: nothing dangerous in flight, no model involved.
    pub(crate) fn idle() -> Self {
        Self {
            kind: ActivityKind::Idle,
            model_id: None,
        }
    }
}

/// The persisted session record. Written durably at launch with
/// `clean_exit: false`, updated as activity changes, and flipped to
/// `clean_exit: true` only on a real exit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionRecord {
    /// Schema version; a record whose schema is not [`SESSION_SCHEMA`] is
    /// treated as clean (fails open).
    pub(crate) schema: u32,
    /// `kern.boottime` at launch, used only to classify the abnormal cause.
    pub(crate) boot_time_secs: i64,
    /// Unix seconds at launch.
    pub(crate) started_at_secs: i64,
    /// False at launch; true ONLY on a real exit. The sole safety input.
    pub(crate) clean_exit: bool,
    /// Liveness state; set to [`SessionState::Crashed`] by the panic hook.
    pub(crate) state: SessionState,
    /// The activity in flight at last write.
    pub(crate) activity: SessionActivity,
    /// Count of consecutive abnormal launches, carried across launches.
    pub(crate) consecutive_abnormal: u32,
}

impl SessionRecord {
    /// Builds the launch record: `clean_exit: false`, `state: Ok`, idle
    /// activity, carrying the abnormal streak this launch computed.
    pub(crate) fn launch(
        boot_time_secs: i64,
        started_at_secs: i64,
        consecutive_abnormal: u32,
    ) -> Self {
        Self {
            schema: SESSION_SCHEMA,
            boot_time_secs,
            started_at_secs,
            clean_exit: false,
            state: SessionState::Ok,
            activity: SessionActivity::idle(),
            consecutive_abnormal,
        }
    }
}

/// Whether the previous session ended cleanly or abnormally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionOutcome {
    /// Previous session exited cleanly (or there was none / it was unreadable).
    Clean,
    /// Previous session died without marking a clean exit.
    Abnormal,
}

/// The classified cause of an abnormal previous session. Drives only the
/// recovery message, never the safe-mode decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbnormalCause {
    /// The panic hook recorded a Rust panic.
    Crashed,
    /// The machine rebooted between launches (boot time changed).
    MachineRestart,
    /// Same boot, no panic recorded: a freeze, SIGKILL, or OS OOM-kill.
    ProcessDied,
}

impl AbnormalCause {
    /// The stable snake_case wire string the frontend switches on.
    pub(crate) fn as_wire_str(self) -> &'static str {
        match self {
            AbnormalCause::Crashed => "crashed",
            AbnormalCause::MachineRestart => "machine_restart",
            AbnormalCause::ProcessDied => "process_died",
        }
    }
}

/// The pure verdict computed from the previous session record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionDecision {
    /// Whether the previous session ended cleanly or abnormally.
    pub(crate) outcome: SessionOutcome,
    /// The abnormal streak this launch represents (0 when clean).
    pub(crate) streak: u32,
    /// Whether this launch should enter safe mode.
    pub(crate) safe_mode: bool,
    /// The classified cause, present only when abnormal.
    pub(crate) cause: Option<AbnormalCause>,
}

/// Pure decision logic: classify the previous session and decide safe mode. No
/// I/O. `prev` is the previously persisted record, or `None` when missing /
/// unreadable / corrupt.
///
/// - `None`, a wrong-schema record, or a `clean_exit: true` record all yield
///   [`SessionOutcome::Clean`], streak 0, safe mode off, cause `None`. The
///   breaker fails open: a bad file can never by itself trip safe mode.
/// - Otherwise the launch is abnormal: the streak grows by one (saturating) and
///   safe mode engages once the streak reaches `threshold`.
pub(crate) fn decide_session(
    prev: Option<SessionRecord>,
    current_boot_secs: i64,
    threshold: u32,
) -> SessionDecision {
    // why: safe_mode is decided ONLY by `clean_exit == false` plus the streak,
    // and by NOTHING else. `boot_time_secs` and `activity` are deliberately
    // excluded. An OS OOM-kill of Thuki during a model load happens on the SAME
    // boot; gating safe mode on "boot time changed" would skip safe mode for
    // exactly the memory-pressure class this feature exists to catch. `activity`
    // is recovery-UI context only.
    match prev {
        Some(p) if p.schema == SESSION_SCHEMA && !p.clean_exit => {
            let streak = p.consecutive_abnormal.saturating_add(1);
            let safe_mode = streak >= threshold;
            // Cause classification (never influences safe_mode): a recorded
            // panic wins over everything, else a changed boot means the machine
            // restarted, else the process died on the same boot (freeze / kill).
            let cause = if p.state == SessionState::Crashed {
                AbnormalCause::Crashed
            } else if p.boot_time_secs != current_boot_secs {
                AbnormalCause::MachineRestart
            } else {
                AbnormalCause::ProcessDied
            };
            SessionDecision {
                outcome: SessionOutcome::Abnormal,
                streak,
                safe_mode,
                cause: Some(cause),
            }
        }
        _ => SessionDecision {
            outcome: SessionOutcome::Clean,
            streak: 0,
            safe_mode: false,
            cause: None,
        },
    }
}

// ---------------------------------------------------------------------------
// I/O and FFI wrappers (coverage-off, exercised live by the tests below).
// ---------------------------------------------------------------------------

/// Reads `kern.boottime` (seconds) via `sysctlbyname`. Stable for a boot,
/// changes only on reboot, so it distinguishes a machine restart from a
/// same-boot process death. On failure it returns 0, which can never differ
/// between launches, so the cause degrades to `ProcessDied` and safe mode
/// (which never reads boot time) is unaffected.
///
/// Coverage-off: pure `libc` FFI. It performs no arithmetic to delegate.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn boot_time_secs() -> i64 {
    let mut tv = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let mut size = std::mem::size_of::<libc::timeval>();
    let name = c"kern.boottime";
    // Safety: `sysctlbyname` writes at most `size` bytes into `tv`; the buffer
    // is exactly one `timeval`, matching what `kern.boottime` returns.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut tv as *mut libc::timeval).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return 0;
    }
    tv.tv_sec
}

/// The outcome of trying to take the process-lifetime session lock.
pub(crate) enum SessionLock {
    /// The lock was acquired. The caller MUST keep this `File` alive for the
    /// whole process: dropping it releases the lock. It is stored in
    /// [`SessionGuard`] managed state.
    Acquired(File),
    /// The lock is already held: another Thuki instance is alive. NOT a crash.
    AlreadyRunning,
}

/// Opens the lock file the way [`acquire_session_lock`] does.
///
/// Coverage-off: filesystem I/O. Its CLOEXEC guarantee is asserted live by the
/// `session_lock_fd_is_cloexec` test.
// why: the fd MUST NOT leak into the spawned `llama-server` child. Rust sets
// `O_CLOEXEC` on every fd it opens, so the lock is dropped across the `exec`
// that starts the sidecar. If it leaked, the sidecar would hold the lock after
// Thuki dies and the next launch would read "still alive" and never detect the
// crash: a SILENT failure in the unsafe direction, hence the explicit test.
#[cfg_attr(coverage_nightly, coverage(off))]
fn open_lock_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Takes a non-blocking exclusive advisory lock on the session-lock file and
/// returns the held `File`, or reports that another instance already holds it.
///
/// The lock is released by the kernel on process death by ANY cause (clean
/// exit, panic, SIGKILL, OS OOM-kill, power loss); that is what makes crash
/// detection work without a clean-exit signal. The caller never unlocks
/// explicitly and must keep the returned `File` alive for the process lifetime.
///
/// Coverage-off: `libc::flock` FFI. The `Acquired` and `AlreadyRunning` arms
/// are exercised live by `session_lock_twice_reports_already_running`.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn acquire_session_lock(path: &Path) -> std::io::Result<SessionLock> {
    let file = open_lock_file(path)?;
    // Safety: `file` owns a valid fd for the duration of this call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(SessionLock::Acquired(file));
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        // Another live instance holds it. Do NOT infer a crash, rewrite the
        // record, or enter safe mode: surface a distinct outcome instead.
        return Ok(SessionLock::AlreadyRunning);
    }
    Err(err)
}

/// Returns a per-process, per-call temporary path beside `target`.
///
/// Coverage-off: time-dependent filesystem helper, exercised live by the
/// durable-write test.
#[cfg_attr(coverage_nightly, coverage(off))]
fn tmp_path_for(target: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mut s = target.as_os_str().to_os_string();
    s.push(format!(".tmp-{pid}-{nanos}"));
    s.into()
}

/// Forces the file's data AND the drive's own write cache to disk.
///
/// Coverage-off: `libc::fcntl` FFI, exercised live by the durable-write test.
// why: on macOS a plain `fsync`/`sync_all` flushes to the drive but does NOT
// force the drive to flush its onboard write cache; only `F_FULLFSYNC` does.
// Without it a power loss right after a "successful" write can still lose the
// launch record, defeating the write-ahead durability guarantee.
#[cfg_attr(coverage_nightly, coverage(off))]
fn full_fsync(file: &File) -> std::io::Result<()> {
    // Safety: `file` owns a valid fd; `F_FULLFSYNC` takes no argument.
    let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) };
    if rc == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Fsyncs a directory so a rename inside it is durable across power loss.
///
/// Coverage-off: filesystem I/O, exercised live by the durable-write test.
#[cfg_attr(coverage_nightly, coverage(off))]
fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Durably writes `bytes` to `path`: write a temp file, `F_FULLFSYNC` it,
/// rename over the target, then fsync the parent directory.
///
/// This is stronger than [`crate::config::writer::atomic_write_bytes`], which
/// gives atomicity but not power-loss durability. Used ONLY by this module; the
/// existing callers of `atomic_write_bytes` are left unchanged.
///
/// Coverage-off: filesystem I/O, exercised live by the durable-write test.
#[cfg_attr(coverage_nightly, coverage(off))]
fn durable_write_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "session path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;

    let tmp = tmp_path_for(path);
    {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        // why: F_FULLFSYNC on macOS, see `full_fsync`.
        full_fsync(&file)?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // Best-effort: the record itself is already durable via the temp fsync.
    let _ = fsync_dir(parent);
    Ok(())
}

/// Reads the session record at `path`, forgivingly.
///
/// Missing, unreadable, or unparseable all map to `None` (the clean default),
/// never an error and never a panic, mirroring the config loader. A corrupt or
/// old-format record can therefore never by itself trip safe mode.
///
/// Coverage-off: filesystem I/O, exercised live by the read-forgiveness tests.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn read_record(path: &Path) -> Option<SessionRecord> {
    match std::fs::read_to_string(path) {
        Ok(contents) => match serde_json::from_str::<SessionRecord>(&contents) {
            Ok(record) => Some(record),
            Err(e) => {
                eprintln!(
                    "thuki: [startup_guard] session record at {} unparseable: {e}. treating as clean",
                    path.display()
                );
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            eprintln!(
                "thuki: [startup_guard] cannot read session record {}: {source}. treating as clean",
                path.display()
            );
            None
        }
    }
}

/// Durably serializes and writes `record` to `path`.
///
/// Coverage-off: thin wrapper over `serde_json` + [`durable_write_bytes`],
/// exercised live by the durable-write test.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn durable_write_record(path: &Path, record: &SessionRecord) -> std::io::Result<()> {
    // SessionRecord is scalars, strings, and enums: serialization is infallible.
    let bytes = serde_json::to_vec(record).expect("SessionRecord serializes");
    durable_write_bytes(path, &bytes)
}

/// Durably marks the session record at `path` as `state: crashed`.
///
/// Reads the current record (or a degenerate launch record if none exists),
/// flips `state`, and durably rewrites it. Best-effort: failures are logged,
/// never propagated, so it is safe from a panic hook.
///
/// Coverage-off: filesystem I/O, exercised live by `mark_crashed_sets_state`.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn mark_crashed(path: &Path) {
    // Read-modify-write from disk rather than through the shared writer: a panic
    // hook may run with the session mutex held or poisoned, so we never touch
    // it here.
    let mut record = read_record(path).unwrap_or_else(|| SessionRecord::launch(0, 0, 0));
    record.state = SessionState::Crashed;
    if let Err(e) = durable_write_record(path, &record) {
        eprintln!(
            "thuki: [startup_guard] failed to mark session crashed at {}: {e}",
            path.display()
        );
    }
}

/// Installs a `std::panic::set_hook` that durably records `state: crashed`
/// before the process unwinds, then chains to the previous hook.
///
/// This CANNOT catch SIGKILL, an OS OOM-kill, a kernel panic, or power loss:
/// those kill the process without running any hook, by construction. Those
/// cases are still caught as abnormal by the kernel-released lock plus the
/// `clean_exit: false` record; the hook only refines the CAUSE to `Crashed`.
///
/// Coverage-off: installs a global process hook; its effect is exercised live
/// through [`mark_crashed`].
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn install_panic_hook(path: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        mark_crashed(&path);
        previous(info);
    }));
}

/// Single-owner, thread-safe writer for the live session record.
///
/// The record is guarded by a `Mutex` and each mutation holds the lock across
/// the durable write, so the on-disk file has exactly one writer at a time even
/// when called concurrently from spawned threads and async tasks. `PathBuf` and
/// `Mutex<SessionRecord>` are both `Send + Sync`, so the writer satisfies Tauri
/// managed-state bounds (it lives inside [`SessionGuard`]).
pub(crate) struct SessionWriter {
    /// Path of the session record this writer owns.
    path: PathBuf,
    /// The in-memory record, mutated under lock before each durable write.
    record: std::sync::Mutex<SessionRecord>,
}

impl SessionWriter {
    /// Wraps the launch `record`, which the caller has already durably written,
    /// so subsequent mutations start from the on-disk state.
    pub(crate) fn new(path: PathBuf, record: SessionRecord) -> Self {
        Self {
            path,
            record: std::sync::Mutex::new(record),
        }
    }

    /// Updates the recorded activity and durably persists it. Safe to call from
    /// any thread or async task.
    ///
    /// Coverage-off: durable filesystem I/O, exercised live by
    /// `set_activity_persists`.
    // why: dead in this pass only. This is the writer API the follow-up
    // activity-tracking job wires at the model-load and download call sites;
    // per the task, THIS pass deliberately adds no call site, so the method has
    // no non-test caller yet. Its behavior is locked by `set_activity_persists`.
    #[allow(dead_code)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub(crate) fn set_activity(&self, activity: SessionActivity) -> std::io::Result<()> {
        // Hold the lock across the write so concurrent callers cannot interleave
        // writes to the single record file.
        let mut record = self.record.lock().expect("session record mutex poisoned");
        record.activity = activity;
        durable_write_record(&self.path, &record)
    }

    /// Flips the record to `clean_exit: true` and durably persists it: the ONLY
    /// place a clean exit is recorded. Called from `lib.rs` on a genuine exit.
    ///
    /// Coverage-off: durable filesystem I/O, exercised live by
    /// `mark_clean_exit_persists`.
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub(crate) fn mark_clean_exit(&self) -> std::io::Result<()> {
        let mut record = self.record.lock().expect("session record mutex poisoned");
        record.clean_exit = true;
        durable_write_record(&self.path, &record)
    }
}

/// Resolves the session-record path beside `config.toml`. Coverage-off:
/// requires a real `AppHandle` and the macOS filesystem.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn session_record_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(crate::config::defaults::DEFAULT_SESSION_RECORD_FILENAME))
}

/// Resolves the session-lock path beside `config.toml`. Coverage-off: requires
/// a real `AppHandle` and the macOS filesystem.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn session_lock_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(crate::config::defaults::DEFAULT_SESSION_LOCK_FILENAME))
}

/// Sets Thuki's own `NSQuitAlwaysKeepsWindows` user default to false.
///
/// Defense-in-depth layered on top of the session record, not a replacement for
/// it: this asks macOS not to reopen the overlay window automatically after an
/// unclean shutdown, reducing the chance the dangerous auto-startup path is
/// re-entered without user action in the first place. If macOS reopens anyway,
/// the session record still catches the loop.
///
/// Coverage-off: pure objc2 FFI against `NSUserDefaults`, consistent with the
/// NSPanel objc usage elsewhere in `lib.rs`.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn disable_quit_keeps_windows() {
    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;

    // Safety: standard AppKit main-thread calls against NSUserDefaults with
    // valid selectors and argument types. Wrapped in an autorelease pool so
    // the transient NSString key is released promptly.
    autoreleasepool(|_| unsafe {
        let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
        if defaults.is_null() {
            return;
        }
        let key = NSString::from_str("NSQuitAlwaysKeepsWindows");
        let _: () = msg_send![defaults, setBool: false, forKey: &*key];
    });
}

// ---------------------------------------------------------------------------
// Shutdown-signal handling (issue #296): a signal-requested stop is a CLEAN exit.
// ---------------------------------------------------------------------------

/// The polite-stop signals a controlling process sends to ask Thuki to quit:
/// `SIGINT` (Ctrl+C in the dev terminal) and `SIGTERM` (what macOS sends every
/// app during a system shutdown or restart).
///
/// Neither runs the Tauri `RunEvent` handlers, so without a handler the stop
/// would leave `clean_exit: false` behind and be miscounted as abnormal. Both
/// mean "a controlling process asked us to stop", which is a clean exit.
///
// why SIGKILL is deliberately ABSENT: `SIGKILL` (`kill -9`), a kernel panic, a
// power cut, and a machine freeze are uncatchable BY CONSTRUCTION: the kernel
// tears the process down without running any handler or thread. Those must
// still yield an abnormal session (that is the memory-pressure class this
// feature exists to catch), so we never trap them. `sigaddset` cannot even add
// `SIGKILL` to a mask; the membership test below documents its absence.
pub(crate) const SHUTDOWN_SIGNALS: [libc::c_int; 2] = [libc::SIGINT, libc::SIGTERM];

/// Builds a `sigset_t` containing exactly the [`SHUTDOWN_SIGNALS`].
///
/// Deterministic and side-effect-free: it only populates a local `sigset_t` and
/// raises no signal, so it is safe to call from a test. The signal thread and
/// the mask installer both derive their set from here, so the "which signals do
/// we trap" decision lives in exactly one place.
fn shutdown_sigset() -> libc::sigset_t {
    // Safety: `sigemptyset`/`sigaddset` write only into the local `set`; they
    // touch no process state and raise no signal, so this is deterministic.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut set);
        for sig in SHUTDOWN_SIGNALS {
            libc::sigaddset(&mut set, sig);
        }
    }
    set
}

/// Blocks the [`SHUTDOWN_SIGNALS`] in the CALLING thread.
///
/// MUST be called as the very first thing in `run()`, before any other thread
/// is spawned: a newly spawned thread inherits the signal mask of the thread
/// that spawned it, so blocking on the main thread up front makes every
/// subsequent thread inherit the block. That guarantees the process-directed
/// signal is delivered to the one thread that consumes it via `sigwait`
/// ([`spawn_shutdown_signal_thread`]) instead of running the default
/// (terminate-now) disposition on some arbitrary thread that never got the
/// block.
///
// why safe for the sidecar child: the spawned `llama-server` inherits this
// block across fork+exec, but Thuki kills it with `SIGKILL` (tokio
// `Child::kill`), which is unblockable, so the inherited block is inert and the
// no-orphan-on-quit guarantee is unaffected.
///
/// Coverage-off: mutates the process's per-thread signal mask via `libc` FFI;
/// running it under the test harness would swallow the harness's own Ctrl+C.
/// The set it installs is built by the covered [`shutdown_sigset`].
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn block_shutdown_signals() {
    let set = shutdown_sigset();
    // Safety: blocks the shutdown signals in the calling thread. A null oldset
    // discards the previous mask, which we never need to restore.
    unsafe {
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

/// Spawns the dedicated thread that waits for a shutdown signal, runs
/// `on_shutdown` on that ORDINARY thread, then restores the signal's default
/// disposition and re-raises it so the process dies exactly as the controlling
/// process expects.
///
// why signal-safe: a durable write (`F_FULLFSYNC`, rename, directory fsync) is
// NOT async-signal-safe and must never run inside a signal handler. This uses
// the standard `sigwait` pattern instead: the shutdown signals are blocked
// process-wide (see `block_shutdown_signals`), and this thread parks in
// `sigwait`, which returns in NORMAL execution context: not a signal handler.
// `on_shutdown` therefore runs the durable clean-exit write on a plain thread,
// where blocking syscalls are perfectly legal.
//
// why re-raise with the default handler: after recording the clean exit we
// restore `SIG_DFL`, unblock the signal in this thread (it is still blocked
// here, so without unblocking the re-raise would stay pending forever), and
// re-raise it. The process then terminates under the signal's own disposition,
// so its wait-status (`WIFSIGNALED`) and any parent's expectations stay
// correct, exactly as if we had never intercepted it.
///
/// `on_shutdown` runs at most once: after re-raising, the process is gone.
///
/// Coverage-off: pure process-lifecycle + `libc` FFI glue that parks a thread
/// in `sigwait` for the process lifetime; it raises no signal in tests. The
/// only decision it embeds (which signals to trap) is covered via
/// [`shutdown_sigset`].
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn spawn_shutdown_signal_thread<F>(on_shutdown: F)
where
    F: FnOnce() + Send + 'static,
{
    std::thread::spawn(move || {
        let set = shutdown_sigset();
        let mut signum: libc::c_int = 0;
        // Safety: `set` is a valid sigset for the process lifetime of this call;
        // `signum` is a valid out-param. `sigwait` blocks until one of the
        // (process-wide blocked) shutdown signals is delivered, then returns it.
        let rc = unsafe { libc::sigwait(&set, &mut signum) };
        if rc != 0 {
            // sigwait failed: leave the process running untouched. The
            // kernel-released lock plus the `clean_exit: false` record still
            // classify an eventual abnormal death correctly, so nothing is lost.
            eprintln!(
                "thuki: [startup_guard] sigwait failed: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        // Durable clean-exit write on this NORMAL thread, never in a signal
        // handler. Routed through the caller's single clean-exit path.
        on_shutdown();
        // Safety: restore the default disposition for the caught signal, unblock
        // it in this thread so the re-raise is delivered rather than left
        // pending, then re-raise it to terminate under its own disposition.
        unsafe {
            libc::signal(signum, libc::SIG_DFL);
            libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
            libc::raise(signum);
        }
    });
}

#[cfg(test)]
mod tests;
