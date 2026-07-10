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

use std::path::Path;

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

#[cfg(test)]
mod tests;
