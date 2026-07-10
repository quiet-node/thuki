//! Startup reaper for orphaned `llama-server` sidecars (issue #296).
//!
//! Thuki spawns `llama-server` as a child process. When Thuki dies by `SIGKILL`,
//! a kernel panic, or a machine freeze, no shutdown path runs (those are
//! uncatchable by construction), and macOS has no `PR_SET_PDEATHSIG`, so the
//! child is not taken down with the parent. An idle sidecar never writes to its
//! now-closed stderr pipe, so it never sees `EPIPE` and never notices its parent
//! is gone: it reparents to `launchd` (`ppid == 1`) and lingers, each instance
//! holding roughly 2 GB. Three were seen at once on the reporting user's machine.
//!
//! The polite-stop path (`startup_guard`'s `sigwait` thread) covers `SIGINT` and
//! `SIGTERM`, but the uncatchable deaths can only be cleaned up at the NEXT
//! launch. This module enumerates processes and kills a sidecar left behind by a
//! previous Thuki, gated on a strict three-clause predicate.
//!
//! ## The predicate: all three clauses, never fewer
//!
//! A process is reaped ONLY when all of the following hold ([`is_reapable_orphan`]
//! is the sole authorizer, at every check including the pre-`SIGKILL` re-verify):
//! 1. `ppid == 1`. A live Thuki's sidecar has `ppid` equal to that Thuki's pid,
//!    so this clause can never match a running instance's child, including a
//!    second Thuki the user has open. This is the load-bearing discriminator,
//!    and it is why reaping runs at startup only: while Thuki is alive it cannot
//!    create its own orphan.
//! 2. The canonicalized exec path equals our canonicalized sidecar binary path.
//!    A user's own llama.cpp build lives elsewhere and must never be touched.
//! 3. The real uid equals our real uid (`getuid`).
//!
//! Safety asymmetry that governs every ambiguous case: a false negative leaks
//! ~2 GB, which is bad but recoverable at the next launch. A false positive
//! `SIGKILL`s a process that is not ours, which is catastrophic and NOT
//! recoverable. When any clause is uncertain (a short syscall read, an
//! unresolvable path), the answer is "do not kill".
//!
//! ## Why `proc_pidinfo`, not `sysctl(KERN_PROC_ALL)`
//!
//! `sysctl(KERN_PROC_ALL)` returns an array of `kinfo_proc`, but the pinned
//! `libc` does not define `kinfo_proc` (nor `extern_proc` / `eproc`) on Apple.
//! Using that path would force a hand-declared copy of the full macOS
//! `kinfo_proc` ABI just to read three fields, with no compiler check against
//! the real header, underneath a process-killing predicate: a struct-layout
//! mismatch would misread pid/ppid/uid and could `SIGKILL` the wrong process.
//! `proc_listallpids` + `proc_pidinfo(PROC_PIDTBSDINFO)` reads pid/ppid/ruid from
//! `libc::proc_bsdinfo`, a compiler-checked libc type, with strictly less unsafe
//! surface. Do not "correct" this back to `sysctl`.
//!
//! ## Shape
//!
//! Like `models::memory` and `startup_guard`, this splits into a pure, fully
//! unit-tested core and thin `coverage(off)` syscall wrappers. The escalation
//! sequence (`SIGTERM`, grace, re-verify, `SIGKILL`) lives in [`reap_with`],
//! which takes its enumeration / probe / signal / grace steps as injected seams
//! so the whole ordered decision is tested with fakes; [`reap_orphaned_sidecars`]
//! is the thin wrapper that passes the real syscalls in.

use std::path::{Path, PathBuf};

/// Cheap per-process facts, read from `proc_bsdinfo` before any path resolution.
///
/// Used to pre-filter candidates on the two cheap clauses so the expensive exec
/// path resolution runs only for survivors. NEVER authorizes a kill on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcIdent {
    /// The process id.
    pub pid: i32,
    /// The parent process id. `1` (`launchd`) marks a reparented orphan.
    pub ppid: i32,
    /// The real uid of the process owner.
    pub ruid: u32,
}

/// A process observed during enumeration, reduced to the fields the reaping
/// predicate needs. `exec_path` is already canonicalized by the enumerator so
/// the pure predicate is a plain equality against our canonicalized binary path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    /// The process id.
    pub pid: i32,
    /// The parent process id. `1` (`launchd`) marks a reparented orphan.
    pub ppid: i32,
    /// The real uid of the process owner.
    pub ruid: u32,
    /// The canonicalized executable path.
    pub exec_path: PathBuf,
}

/// Pure: the two CHEAP clauses (`ppid == 1` and `ruid == our_uid`). A candidate
/// that fails these can never be a reapable orphan, so its exec path is never
/// resolved. This is an ordering optimization ONLY: it never authorizes a kill.
/// A kill always requires the full three-clause [`is_reapable_orphan`], which
/// re-checks both of these plus the path.
pub fn passes_cheap_clauses(ident: &ProcIdent, our_uid: u32) -> bool {
    ident.ppid == 1 && ident.ruid == our_uid
}

/// Pure predicate and SOLE kill authorizer: is `proc` an orphaned sidecar of
/// OURS that we may reap? All three clauses must hold; any one failing yields
/// `false` (see the module safety asymmetry: never kill on doubt).
///
/// Both `proc.exec_path` and `our_binary` are expected canonicalized by the
/// caller, so the path clause is an exact `==` on fully-resolved paths.
pub fn is_reapable_orphan(proc: &ProcInfo, our_binary: &Path, our_uid: u32) -> bool {
    proc.ppid == 1 && proc.exec_path == our_binary && proc.ruid == our_uid
}

/// The escalation core, covered with fakes. Enumeration, per-pid probe, signal
/// delivery, and the grace sleep are injected so the ordered decision is tested
/// without syscalls.
///
/// Sequence:
/// 1. Enumerate cheap idents, drop any that fail [`passes_cheap_clauses`], and
///    for survivors resolve the full [`ProcInfo`] via `probe`. Authorize each
///    with [`is_reapable_orphan`]. This is the only place a candidate becomes an
///    orphan to act on.
/// 2. If none matched, return without signalling and without invoking `grace`.
/// 3. `SIGTERM` every orphan (the child no longer inherits our blocked signal
///    mask, per commit `0f5687e`, so `SIGTERM` actually lands), then run `grace`.
/// 4. `SIGKILL` a survivor ONLY after re-running the FULL predicate against a
///    fresh `probe`: during the grace the target may have exited and macOS may
///    have recycled its pid onto an innocent process, so a `None` re-probe or any
///    clause mismatch skips the `SIGKILL`. This guard is the single most
///    consequential branch in the feature and is asserted by ordered-log tests.
fn reap_with(
    our_binary: &Path,
    our_uid: u32,
    enumerate: impl Fn() -> Vec<ProcIdent>,
    probe: impl Fn(i32) -> Option<ProcInfo>,
    mut signal: impl FnMut(i32, libc::c_int),
    grace: impl FnOnce(),
) {
    let orphans: Vec<i32> = enumerate()
        .into_iter()
        .filter(|ident| passes_cheap_clauses(ident, our_uid))
        .filter_map(|ident| probe(ident.pid))
        .filter(|info| is_reapable_orphan(info, our_binary, our_uid))
        .map(|info| info.pid)
        .collect();
    if orphans.is_empty() {
        return;
    }

    for &pid in &orphans {
        signal(pid, libc::SIGTERM);
    }
    grace();
    for &pid in &orphans {
        // Re-verify the FULL predicate against a fresh read: a failed read or a
        // recycled pid no longer matches, so it is never SIGKILLed.
        if probe(pid).is_some_and(|info| is_reapable_orphan(&info, our_binary, our_uid)) {
            signal(pid, libc::SIGKILL);
        }
    }
}

// ---------------------------------------------------------------------------
// Thin syscall wrappers (coverage-off) + the startup entry point.
// ---------------------------------------------------------------------------

/// This process's real uid via `getuid`.
///
/// Coverage-off: a single infallible `libc` call with no logic to test.
#[cfg_attr(coverage_nightly, coverage(off))]
fn current_ruid() -> u32 {
    // SAFETY: `getuid` always succeeds and takes no arguments.
    unsafe { libc::getuid() }
}

/// Every live pid via `proc_listallpids`.
///
/// Coverage-off: a two-step `libc` syscall (size, then fill) with a shape
/// conversion, no decision logic. `proc_listallpids` returns a COUNT of pids
/// (its libproc wrapper divides the byte length by `sizeof(int)`); the buffer
/// size argument is in bytes. A generous slack over the sized count absorbs
/// processes started between the two calls, and a final `> 0` filter drops any
/// unfilled zero slots regardless of the exact return semantics.
#[cfg_attr(coverage_nightly, coverage(off))]
fn all_pids() -> Vec<i32> {
    // SAFETY: a null buffer with size 0 asks libproc only for the current count.
    let needed = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return Vec::new();
    }
    let cap = (needed as usize).saturating_add(32);
    let mut buf: Vec<libc::c_int> = vec![0; cap];
    let byte_len = (cap * std::mem::size_of::<libc::c_int>()) as libc::c_int;
    // SAFETY: `buf` owns `cap` `c_int` slots; `byte_len` is its size in bytes, so
    // libproc writes at most `cap` pids into a buffer that holds exactly that.
    let filled = unsafe { libc::proc_listallpids(buf.as_mut_ptr() as *mut libc::c_void, byte_len) };
    if filled <= 0 {
        return Vec::new();
    }
    buf.truncate((filled as usize).min(cap));
    buf.retain(|&pid| pid > 0);
    buf
}

/// Reads `proc_bsdinfo` for `pid` via `proc_pidinfo(PROC_PIDTBSDINFO)`, or `None`
/// if the process vanished mid-scan or could not be read fully.
///
/// Coverage-off: a single `libc` syscall. A short read (return value not equal
/// to the struct size) means the process is gone or inaccessible: `None`.
#[cfg_attr(coverage_nightly, coverage(off))]
fn bsdinfo_of(pid: i32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a valid, zeroed `proc_bsdinfo`; `size` is its exact byte
    // size, so `proc_pidinfo` fills it in place and returns the bytes written.
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut libc::proc_bsdinfo as *mut libc::c_void,
            size,
        )
    };
    if n != size {
        return None;
    }
    Some(info)
}

/// The cheap [`ProcIdent`] for `pid` (pid/ppid/ruid), no path resolution.
///
/// Coverage-off: `bsdinfo_of` plus a field copy. This is the per-pid step of the
/// cheap-clause pre-filter, so the (expensive) path resolution never runs for a
/// process that already fails `ppid == 1 && ruid == our_uid`.
#[cfg_attr(coverage_nightly, coverage(off))]
fn ident_of(pid: i32) -> Option<ProcIdent> {
    let info = bsdinfo_of(pid)?;
    Some(ProcIdent {
        pid,
        ppid: info.pbi_ppid as i32,
        ruid: info.pbi_ruid,
    })
}

/// Every live process reduced to its cheap [`ProcIdent`], for the pre-filter.
///
/// Coverage-off: `all_pids` mapped through `ident_of`; a vanished pid is dropped.
#[cfg_attr(coverage_nightly, coverage(off))]
fn all_idents() -> Vec<ProcIdent> {
    all_pids().into_iter().filter_map(ident_of).collect()
}

/// The canonicalized executable path of `pid` via `proc_pidpath`, or `None` if
/// the process vanished or its path cannot be resolved.
///
/// Coverage-off: a single `libc` syscall plus a `canonicalize` shape conversion.
/// Canonicalizing here (and the caller canonicalizing our own binary) is what
/// makes the path clause survive symlinks (`/var` -> `/private/var`, worktree
/// links): comparing two same-typed canonical paths. A canonicalize failure
/// means we cannot prove identity, so it maps to `None`: never a match.
#[cfg_attr(coverage_nightly, coverage(off))]
fn exec_path_of(pid: i32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `buf` is a valid, correctly sized byte buffer; `proc_pidpath`
    // writes at most `buf.len()` bytes and returns the path length or <= 0.
    let n =
        unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    let raw = PathBuf::from(std::ffi::OsString::from_vec(buf));
    std::fs::canonicalize(raw).ok()
}

/// The full [`ProcInfo`] for `pid` (pid/ppid/ruid + canonicalized path), or
/// `None` if the process vanished or could not be read fully.
///
/// Coverage-off: `bsdinfo_of` + `exec_path_of`, a fresh full read from a pid.
/// Used to authorize a survivor and, crucially, to RE-verify immediately before
/// `SIGKILL` (a fresh read, never a reused stale ident, so a recycled pid is
/// caught).
#[cfg_attr(coverage_nightly, coverage(off))]
fn proc_info_of(pid: i32) -> Option<ProcInfo> {
    let info = bsdinfo_of(pid)?;
    let exec_path = exec_path_of(pid)?;
    Some(ProcInfo {
        pid,
        ppid: info.pbi_ppid as i32,
        ruid: info.pbi_ruid,
        exec_path,
    })
}

/// Sends `sig` to `pid` via `kill`, ignoring the result.
///
/// Coverage-off: one async-signal-safe `libc` call. A stale pid yields `ESRCH`,
/// which is harmless and deliberately ignored.
#[cfg_attr(coverage_nightly, coverage(off))]
fn send_signal(pid: i32, sig: libc::c_int) {
    // SAFETY: `kill` is async-signal-safe and total over any pid/sig; a stale or
    // unknown pid simply returns an error we do not act on.
    unsafe {
        libc::kill(pid, sig);
    }
}

/// Reaps every `llama-server` sidecar orphaned by a previous Thuki (issue #296).
///
/// Startup only. Thin `coverage(off)` wrapper: it canonicalizes our own sidecar
/// path (returning without reaping if that fails, so nothing is matched against
/// an unresolved path), reads our real uid, and hands the real syscalls to the
/// covered [`reap_with`], whose ordered escalation is what the tests exercise.
/// Meant to run on a detached thread so the (rare) grace sleep never delays
/// startup.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn reap_orphaned_sidecars(our_binary: &Path) {
    let Ok(our_binary) = std::fs::canonicalize(our_binary) else {
        return;
    };
    let our_uid = current_ruid();
    reap_with(
        &our_binary,
        our_uid,
        all_idents,
        proc_info_of,
        send_signal,
        || {
            std::thread::sleep(std::time::Duration::from_millis(
                crate::config::defaults::ORPHAN_REAP_SIGTERM_GRACE_MS,
            ))
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, VecDeque};

    const OUR_UID: u32 = 501;

    fn our_binary() -> PathBuf {
        PathBuf::from("/apps/Thuki.app/Contents/MacOS/llama-server")
    }

    fn ident(pid: i32, ppid: i32, ruid: u32) -> ProcIdent {
        ProcIdent { pid, ppid, ruid }
    }

    /// A `ProcInfo` that satisfies all three clauses against `our_binary`/`OUR_UID`.
    fn matching_info(pid: i32) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: 1,
            ruid: OUR_UID,
            exec_path: our_binary(),
        }
    }

    /// A `ProcInfo` with a foreign exec path: the shape a recycled pid takes.
    fn foreign_info(pid: i32) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: 1,
            ruid: OUR_UID,
            exec_path: PathBuf::from("/opt/homebrew/bin/some-daemon"),
        }
    }

    /// Runs [`reap_with`] against a scripted enumeration and a per-pid FIFO of
    /// probe responses, returning the ordered `(pid, signal)` log and whether the
    /// grace ran. Order is the property under test, so the log is a `Vec`, never
    /// a count.
    fn run(
        idents: Vec<ProcIdent>,
        probes: Vec<(i32, Vec<Option<ProcInfo>>)>,
    ) -> (Vec<(i32, libc::c_int)>, bool) {
        let binary = our_binary();
        let signals: RefCell<Vec<(i32, libc::c_int)>> = RefCell::new(Vec::new());
        let grace_ran = Cell::new(false);
        let scripts: RefCell<HashMap<i32, VecDeque<Option<ProcInfo>>>> = RefCell::new(
            probes
                .into_iter()
                .map(|(pid, seq)| (pid, seq.into_iter().collect()))
                .collect(),
        );

        reap_with(
            &binary,
            OUR_UID,
            || idents.clone(),
            |pid| {
                scripts
                    .borrow_mut()
                    .get_mut(&pid)
                    .and_then(|q| q.pop_front())
                    .flatten()
            },
            |pid, sig| signals.borrow_mut().push((pid, sig)),
            || grace_ran.set(true),
        );

        (signals.into_inner(), grace_ran.get())
    }

    #[test]
    fn matching_orphan_is_sigtermed_then_sigkilled() {
        // A genuine orphan: SIGTERM, then SIGKILL after the grace, in that order.
        let (log, grace) = run(
            vec![ident(4242, 1, OUR_UID)],
            vec![(
                4242,
                vec![Some(matching_info(4242)), Some(matching_info(4242))],
            )],
        );
        assert_eq!(log, vec![(4242, libc::SIGTERM), (4242, libc::SIGKILL)]);
        assert!(grace, "grace runs between SIGTERM and the SIGKILL sweep");
    }

    #[test]
    fn orphan_that_exits_during_grace_is_never_sigkilled() {
        // Matched at enumeration, but the re-probe returns None (it exited during
        // the grace): it is SIGTERMed once and never SIGKILLed.
        let (log, grace) = run(
            vec![ident(4242, 1, OUR_UID)],
            vec![(4242, vec![Some(matching_info(4242)), None])],
        );
        assert_eq!(log, vec![(4242, libc::SIGTERM)]);
        assert!(grace);
    }

    #[test]
    fn recycled_pid_with_foreign_path_is_never_sigkilled() {
        // The catastrophic case: matched at enumeration, but during the grace the
        // orphan exited and macOS recycled its pid onto an innocent process with a
        // foreign exec path. The pre-SIGKILL re-verify rejects it, so the stranger
        // is SIGTERMed at most and NEVER SIGKILLed.
        let (log, _grace) = run(
            vec![ident(4242, 1, OUR_UID)],
            vec![(
                4242,
                vec![Some(matching_info(4242)), Some(foreign_info(4242))],
            )],
        );
        assert_eq!(log, vec![(4242, libc::SIGTERM)]);
        assert!(
            !log.contains(&(4242, libc::SIGKILL)),
            "a recycled foreign pid must never be SIGKILLed"
        );
    }

    #[test]
    fn non_matching_processes_receive_no_signal() {
        // None of these become an orphan, so nothing is signalled and the grace
        // never runs. Covers every rejection branch:
        //   pid 1: cheap-reject on ppid (probe never called)
        //   pid 2: cheap-reject on ruid (probe never called)
        //   pid 3: cheap-pass, probe returns None (vanished) -> dropped
        //   pid 4: cheap-pass, probe returns a foreign path -> predicate rejects
        let (log, grace) = run(
            vec![
                ident(1, 999, OUR_UID),
                ident(2, 1, 502),
                ident(3, 1, OUR_UID),
                ident(4, 1, OUR_UID),
            ],
            vec![(3, vec![None]), (4, vec![Some(foreign_info(4))])],
        );
        assert!(log.is_empty(), "no candidate authorized a signal");
        assert!(!grace, "empty orphan set returns before the grace");
    }

    #[test]
    fn empty_enumeration_sends_nothing_and_skips_grace() {
        let (log, grace) = run(vec![], vec![]);
        assert!(log.is_empty());
        assert!(!grace);
    }

    #[test]
    fn reapable_when_all_three_clauses_hold() {
        let binary = our_binary();
        assert!(is_reapable_orphan(&matching_info(4242), &binary, OUR_UID));
    }

    #[test]
    fn not_reapable_when_ppid_is_not_one() {
        // Right path and uid, but the process is still parented to a live Thuki
        // (ppid != 1): the load-bearing discriminator rejects it.
        let binary = our_binary();
        let mut proc = matching_info(4242);
        proc.ppid = 9931; // a live Thuki's pid, not launchd
        assert!(!is_reapable_orphan(&proc, &binary, OUR_UID));
    }

    #[test]
    fn not_reapable_when_exec_path_is_foreign() {
        // Right ppid and uid, but a user's own llama.cpp build lives elsewhere.
        let binary = our_binary();
        let mut proc = matching_info(4242);
        proc.exec_path = PathBuf::from("/opt/homebrew/bin/llama-server");
        assert!(!is_reapable_orphan(&proc, &binary, OUR_UID));
    }

    #[test]
    fn not_reapable_when_uid_is_foreign() {
        // Right ppid and path, but owned by a different user: never touch it.
        let binary = our_binary();
        let mut proc = matching_info(4242);
        proc.ruid = 502;
        assert!(!is_reapable_orphan(&proc, &binary, OUR_UID));
    }
}
