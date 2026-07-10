//! Launch circuit breaker for issue #296.
//!
//! On a memory-constrained Mac, Thuki froze the whole machine while
//! auto-loading a large model at startup. After a forced power-off macOS
//! relaunched the app (its reopen-after-unclean-shutdown behavior) and the
//! same no-user-action auto-startup work re-ran and re-froze the machine
//! before the user could intervene: a deadloop.
//!
//! Thuki hides on window close and quits only from the tray, so
//! `RunEvent::Exit` almost never fires during a healthy session. Any
//! crash-loop detection therefore MUST NOT depend on a clean-exit signal.
//! Instead this module uses a "dirty on launch, cleared when healthy"
//! sentinel: every launch writes a dirty marker before any dangerous
//! auto-op runs, and the app clears it (via `mark_startup_healthy`) only
//! once it has reached a responsive state. A launch that finds the previous
//! marker still dirty therefore knows the previous launch never became
//! healthy: the crash-loop signature.
//!
//! The module is split into pure decision logic (fully unit-tested, no I/O)
//! and thin `coverage(off)` I/O wrappers that mirror the forgiving,
//! never-panic-on-bad-input contract of `config::loader`/`config::writer`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The sentinel state persisted to disk between launches.
///
/// `launch_dirty` is set true at the start of every launch and cleared only
/// when the app reports it reached a healthy/responsive state.
/// `consecutive_unclean` counts how many launches in a row failed to reach
/// that healthy state, so a threshold can distinguish a one-off from a loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedGuardState {
    /// True while a launch is in progress and not yet confirmed healthy.
    /// A launch that reads this as true was preceded by a launch that never
    /// reached a responsive state.
    pub launch_dirty: bool,
    /// Number of consecutive launches that were dirty on start (previous
    /// launch never became healthy). Reset to zero on any clean launch and
    /// by an explicit healthy signal.
    pub consecutive_unclean: u32,
}

impl Default for PersistedGuardState {
    /// The first-run / clean default: no launch in progress, no unclean
    /// streak. Also the value substituted for a missing or unparseable
    /// sentinel file, so a corrupt file can never by itself trip safe mode.
    fn default() -> Self {
        Self {
            launch_dirty: false,
            consecutive_unclean: 0,
        }
    }
}

/// The outcome of the pure startup decision: what this launch should do and
/// what to persist for the next launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupDecision {
    /// Whether this launch should enter safe mode (skip dangerous auto-ops).
    pub safe_mode: bool,
    /// The consecutive-unclean count computed for this launch. Mirrored into
    /// the managed state so the UI can show why safe mode engaged.
    pub unclean_count: u32,
    /// The sentinel to persist for THIS launch: always dirty, carrying the
    /// updated count. It is cleared later by the healthy signal.
    pub next_state: PersistedGuardState,
}

/// Pure decision logic: given the sentinel the previous launch left behind and
/// the safe-mode threshold, compute this launch's decision. No I/O.
///
/// Semantics (per issue #296):
/// - If `prior.launch_dirty` is true, the previous launch never reached a
///   healthy state, so this is another unclean launch and the streak grows by
///   one. Otherwise the streak resets to zero.
/// - Safe mode engages once the streak reaches `threshold`.
/// - The state to persist for this launch is always dirty, carrying the new
///   streak count; it is cleared only by [`healthy_state`].
pub fn decide(prior: PersistedGuardState, threshold: u32) -> StartupDecision {
    let unclean_count = if prior.launch_dirty {
        // Previous launch wrote dirty and never cleared it: it did not reach
        // a healthy state before this launch began.
        prior.consecutive_unclean.saturating_add(1)
    } else {
        0
    };
    let safe_mode = unclean_count >= threshold;
    StartupDecision {
        safe_mode,
        unclean_count,
        next_state: PersistedGuardState {
            launch_dirty: true,
            consecutive_unclean: unclean_count,
        },
    }
}

/// The sentinel written when the app confirms it reached a healthy state:
/// not dirty, streak cleared. Pure; the persistence is done by the caller.
pub fn healthy_state() -> PersistedGuardState {
    PersistedGuardState {
        launch_dirty: false,
        consecutive_unclean: 0,
    }
}

/// Immutable managed state holding this launch's circuit-breaker verdict.
///
/// Read from multiple subsystems and threads (the auto-prime gate in
/// `show_overlay`, the download gates in `models`, the `startup_safety`
/// command). Both fields are set once at construction by [`from_decision`] and
/// are never mutated for the lifetime of the process, so the struct is a plain
/// pair of primitives. Immutable primitives are `Sync`, so it satisfies Tauri's
/// managed-state `Send + Sync` requirement with no atomics or locks.
///
/// Invariant: the verdict is a FACT about THIS launch and must stay fixed for
/// the whole session. It is deliberately NOT reset by the healthy signal. The
/// dangerous auto-op this breaker exists to stop (the overlay-show auto-prime
/// in `lib.rs`) runs AFTER the frontend has mounted and fired
/// `mark_startup_healthy`; clearing the verdict on that mount signal would
/// erase the gate before the op it guards ever runs. The healthy signal instead
/// governs only the NEXT launch, by clearing the on-disk sentinel.
///
/// [`from_decision`]: StartupSafety::from_decision
#[derive(Debug)]
pub struct StartupSafety {
    safe_mode: bool,
    unclean_count: u32,
}

impl StartupSafety {
    /// Builds the managed state from a [`StartupDecision`]. Used once at startup
    /// after the sentinel has been read and the decision computed; the resulting
    /// verdict is then immutable for the process lifetime.
    pub fn from_decision(decision: &StartupDecision) -> Self {
        Self {
            safe_mode: decision.safe_mode,
            unclean_count: decision.unclean_count,
        }
    }

    /// Whether this launch is in safe mode. Cheap field read; safe to call from
    /// any thread on any auto-op path.
    pub fn safe_mode(&self) -> bool {
        self.safe_mode
    }

    /// The consecutive-unclean count that produced the current verdict.
    pub fn unclean_count(&self) -> u32 {
        self.unclean_count
    }

    /// A serializable view of the current verdict for the frontend.
    pub fn snapshot(&self) -> StartupSafetySnapshot {
        StartupSafetySnapshot {
            safe_mode: self.safe_mode(),
            unclean_count: self.unclean_count(),
        }
    }
}

/// The wire shape returned by the `startup_safety` command. The frontend
/// renders a recovery screen from this after an unclean-launch streak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StartupSafetySnapshot {
    /// Whether this launch skipped the dangerous auto-startup operations.
    pub safe_mode: bool,
    /// How many consecutive launches failed to reach a healthy state.
    pub unclean_count: u32,
}

/// Reads the persisted sentinel from `path`, forgivingly.
///
/// Missing file (first run) or unparseable JSON both map to the clean
/// [`PersistedGuardState::default`], never an error and never a panic, so a
/// corrupt sentinel can never by itself trip safe mode. Mirrors the forgiving
/// contract of `config::loader`.
#[cfg_attr(coverage_nightly, coverage(off))]
fn read_state(path: &Path) -> PersistedGuardState {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PersistedGuardState::default(),
        Err(source) => {
            eprintln!(
                "thuki: [startup_guard] cannot read {}: {source}. treating as clean",
                path.display()
            );
            PersistedGuardState::default()
        }
    }
}

/// Atomically writes the sentinel to `path`, reusing the config writer's
/// temp-file-plus-rename guarantee so a crash mid-write can never leave a torn
/// sentinel. Failures are logged, never propagated: the guard is best-effort
/// and must never block or crash startup.
#[cfg_attr(coverage_nightly, coverage(off))]
fn write_state(path: &Path, state: &PersistedGuardState) {
    // The struct holds a bool and a u32, so serialization is infallible.
    let bytes = serde_json::to_vec(state).expect("PersistedGuardState serializes");
    if let Err(e) = crate::config::writer::atomic_write_bytes(path, &bytes) {
        eprintln!(
            "thuki: [startup_guard] failed to persist sentinel to {}: {e}",
            path.display()
        );
    }
}

/// Runs the circuit breaker once at startup: resolve the sentinel path, read
/// the prior sentinel, compute the decision, persist this launch's dirty
/// sentinel, and return the managed state to install. Best-effort throughout:
/// any I/O failure is logged and the in-memory decision still stands, so the
/// guard degrades to "no safe mode" rather than blocking the app.
///
/// Coverage-off thin wrapper: the decision logic is [`decide`], the I/O is
/// [`read_state`]/[`write_state`], all covered or explicitly exempt.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn run_startup_guard(app: &tauri::AppHandle, threshold: u32) -> StartupSafety {
    let Some(path) = guard_path(app) else {
        // The per-user config dir could not be resolved. Config load already
        // ran fatally if the dir were truly unusable, so this is only the
        // theoretical resolver-failure path: degrade to no safe mode rather
        // than block startup.
        return StartupSafety::from_decision(&decide(PersistedGuardState::default(), threshold));
    };
    let prior = read_state(&path);
    let decision = decide(prior, threshold);
    // Persist BEFORE any dangerous auto-op runs so a freeze during this launch
    // leaves the dirty marker behind for the next launch to detect.
    write_state(&path, &decision.next_state);
    StartupSafety::from_decision(&decision)
}

/// Persists the healthy sentinel to `path`. Coverage-off thin wrapper over
/// [`healthy_state`] + [`write_state`]; called by `mark_startup_healthy`.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn mark_healthy(path: &Path) {
    write_state(path, &healthy_state());
}

/// Sets Thuki's own `NSQuitAlwaysKeepsWindows` user default to false.
///
/// Defense-in-depth layered on top of the sentinel, not a replacement for it:
/// this asks macOS not to reopen the overlay window automatically after an
/// unclean shutdown, reducing the chance the dangerous auto-startup path is
/// re-entered without user action in the first place. If macOS reopens anyway,
/// the sentinel still catches the loop.
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

/// Resolves the sentinel file path next to `config.toml` in the per-user app
/// config dir. Returns `None` only if macOS cannot yield the directory, in
/// which case the guard silently degrades to no-op. Coverage-off: requires a
/// real `AppHandle` and the macOS filesystem.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn guard_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(crate::config::defaults::DEFAULT_STARTUP_GUARD_FILENAME))
}

/// Command: returns the current circuit-breaker verdict so the frontend can
/// render a recovery screen after an unclean-launch streak. Thin coverage-off
/// wrapper over the managed [`StartupSafety`] state.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn startup_safety(state: tauri::State<'_, StartupSafety>) -> StartupSafetySnapshot {
    state.snapshot()
}

/// Command: the "we reached a responsive state" signal, replacing clean-exit
/// as the circuit breaker's reset mechanism. Persists the healthy sentinel and
/// does NOTHING else.
///
/// It deliberately does not touch this launch's in-memory [`StartupSafety`]
/// verdict. The dangerous auto-op the breaker guards (the overlay-show
/// auto-prime in `lib.rs`) runs on summon, AFTER the frontend mounts and fires
/// this command, so clearing the verdict here would defeat the very gate the
/// breaker exists to enforce. The health signal only proves the app became
/// responsive: it governs the NEXT launch by clearing the on-disk sentinel, not
/// this launch's verdict. Thin coverage-off wrapper over [`mark_healthy`].
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn mark_startup_healthy(app: tauri::AppHandle) {
    if let Some(path) = guard_path(&app) {
        mark_healthy(&path);
    }
}

// ---------------------------------------------------------------------------
// Session-liveness circuit breaker (issue #296, phase 1: additive).
//
// The sentinel above clears from the FRONTEND about a second into launch, so
// every dangerous auto-op (overlay auto-prime, downloads, first-message model
// load) runs AFTER the "healthy" clear and escapes the gate. The reported
// incident froze during a model DOWNLOAD while a model was already loaded,
// which operation-bracketing also misses.
//
// This mechanism instead proves liveness with a process-lifetime advisory lock
// (kernel releases it on death by ANY cause) plus a write-ahead session record
// that is durably `clean_exit: false` on disk BEFORE any dangerous op begins. A
// launch that finds the previous record still `clean_exit: false` knows the
// previous process died abnormally. Modeled on Firefox nsAppStartup's profile
// lock + last_success and Sentry's crashed-vs-abnormal session distinction.
//
// Phase 1 builds this alongside the old mechanism; phase 2 wires it into
// `lib.rs`/`App.tsx` and deletes the old one. Unwired items carry
// `#[allow(dead_code)]`.
// ---------------------------------------------------------------------------

/// The session-record schema version this build reads and writes.
// why: wired in the follow-up pass
#[allow(dead_code)]
const SESSION_SCHEMA: u32 = 1;

/// Filename of the JSON session record, next to `config.toml`.
// why: phase 2 relocates this to `config::defaults` and migrates the legacy
// `startup_guard.json`; kept local so phase 1 stays additive and never touches
// `config/defaults.rs` (owned by another change in flight).
#[allow(dead_code)]
const SESSION_RECORD_FILENAME: &str = "session.json";

/// Filename of the empty advisory-lock file, next to `config.toml`.
// why: phase 2 relocates this to `config::defaults`.
#[allow(dead_code)]
const SESSION_LOCK_FILENAME: &str = "session.lock";

/// Liveness state of a session, as recorded on disk.
// why: wired in the follow-up pass
#[allow(dead_code)]
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
// why: wired in the follow-up pass
#[allow(dead_code)]
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

/// The activity in flight when the record was last written.
// why: wired in the follow-up pass
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionActivity {
    /// The class of work in flight.
    pub(crate) kind: ActivityKind,
    /// The model involved, when the activity concerns a specific model.
    pub(crate) model_id: Option<String>,
}

/// The persisted session record. Written durably at launch with
/// `clean_exit: false`, updated as activity changes, and flipped to
/// `clean_exit: true` only on a real exit.
// why: wired in the follow-up pass (`schema`, `boot_time_secs`, `state`,
// `clean_exit`, `consecutive_abnormal` are read by `decide_session`;
// `started_at_secs`, `activity`, and `model_id` are read only by phase 2's UI).
#[allow(dead_code)]
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
    // why: wired in the follow-up pass
    #[allow(dead_code)]
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
            activity: SessionActivity {
                kind: ActivityKind::Idle,
                model_id: None,
            },
            consecutive_abnormal,
        }
    }
}

/// Whether the previous session ended cleanly or abnormally.
// why: wired in the follow-up pass
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionOutcome {
    /// Previous session exited cleanly (or there was none / it was unreadable).
    Clean,
    /// Previous session died without marking a clean exit.
    Abnormal,
}

/// The classified cause of an abnormal previous session. Drives only the
/// recovery message, never the safe-mode decision.
// why: wired in the follow-up pass
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbnormalCause {
    /// The panic hook recorded a Rust panic.
    Crashed,
    /// The machine rebooted between launches (boot time changed).
    MachineRestart,
    /// Same boot, no panic recorded: a freeze, SIGKILL, or OS OOM-kill.
    ProcessDied,
}

/// The pure verdict computed from the previous session record.
// why: wired in the follow-up pass
#[allow(dead_code)]
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
///
/// `threshold` is a parameter, not read from config here: phase 2 passes the
/// compiled constant.
// why: wired in the follow-up pass
#[allow(dead_code)]
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

/// Reads `kern.boottime` (seconds) via `sysctlbyname`. Stable for a boot,
/// changes only on reboot, so it distinguishes a machine restart from a
/// same-boot process death. On failure it returns 0, which can never differ
/// between launches, so the cause degrades to `ProcessDied` and safe mode
/// (which never reads boot time) is unaffected.
///
/// Coverage-off: pure `libc` FFI. It performs no arithmetic to delegate.
// why: wired in the follow-up pass
#[allow(dead_code)]
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
// why: wired in the follow-up pass
#[allow(dead_code)]
pub(crate) enum SessionLock {
    /// The lock was acquired. The caller MUST keep this `File` alive for the
    /// whole process: dropping it releases the lock. Phase 2 stores it in Tauri
    /// managed state.
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
#[allow(dead_code)]
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
// why: wired in the follow-up pass
#[allow(dead_code)]
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
// why: wired in the follow-up pass
#[allow(dead_code)]
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
#[allow(dead_code)]
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
// why: wired in the follow-up pass
#[allow(dead_code)]
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
// why: wired in the follow-up pass
#[allow(dead_code)]
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
/// never an error and never a panic, mirroring `read_state` and the config
/// loader. A corrupt record can therefore never by itself trip safe mode.
///
/// Coverage-off: filesystem I/O, exercised live by the read-forgiveness tests.
// why: wired in the follow-up pass
#[allow(dead_code)]
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
// why: wired in the follow-up pass
#[allow(dead_code)]
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
// why: wired in the follow-up pass
#[allow(dead_code)]
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
// why: wired in the follow-up pass
#[allow(dead_code)]
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
/// managed-state bounds.
// why: wired in the follow-up pass
#[allow(dead_code)]
pub(crate) struct SessionWriter {
    /// Path of the session record this writer owns.
    path: PathBuf,
    /// The in-memory record, mutated under lock before each durable write.
    record: std::sync::Mutex<SessionRecord>,
}

impl SessionWriter {
    /// Wraps the launch `record`, which the caller has already durably written,
    /// so subsequent mutations start from the on-disk state.
    // why: wired in the follow-up pass
    #[allow(dead_code)]
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
    // why: wired in the follow-up pass
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
    /// place a clean exit is recorded. Phase 2 calls this on `RunEvent::Exit`.
    ///
    /// Coverage-off: durable filesystem I/O, exercised live by
    /// `mark_clean_exit_persists`.
    // why: wired in the follow-up pass
    #[allow(dead_code)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub(crate) fn mark_clean_exit(&self) -> std::io::Result<()> {
        let mut record = self.record.lock().expect("session record mutex poisoned");
        record.clean_exit = true;
        durable_write_record(&self.path, &record)
    }
}

/// Resolves the session-record path beside `config.toml`. Coverage-off:
/// requires a real `AppHandle` and the macOS filesystem.
// why: wired in the follow-up pass
#[allow(dead_code)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn session_record_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(SESSION_RECORD_FILENAME))
}

/// Resolves the session-lock path beside `config.toml`. Coverage-off: requires
/// a real `AppHandle` and the macOS filesystem.
// why: wired in the follow-up pass
#[allow(dead_code)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn session_lock_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(SESSION_LOCK_FILENAME))
}

#[cfg(test)]
mod tests;
