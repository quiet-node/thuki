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
//! A process is reaped ONLY when all of the following hold:
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
//! Split, like `models::memory` and `startup_guard`, into a pure predicate
//! (fully unit-tested, no FFI) and thin `coverage(off)` syscall wrappers.

use std::path::{Path, PathBuf};

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

/// Pure predicate: is `proc` an orphaned sidecar of OURS that we may reap? All
/// three clauses must hold; any one failing yields `false` (see the module
/// safety asymmetry: never kill on doubt).
///
/// Both `proc.exec_path` and `our_binary` are expected canonicalized by the
/// caller, so the path clause is an exact `==` on fully-resolved paths.
pub fn is_reapable_orphan(proc: &ProcInfo, our_binary: &Path, our_uid: u32) -> bool {
    proc.ppid == 1 && proc.exec_path == our_binary && proc.ruid == our_uid
}

// ---------------------------------------------------------------------------
// Thin syscall wrappers (coverage-off) + the startup orchestration.
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

/// The `ProcInfo` for `pid` via `proc_pidinfo(PROC_PIDTBSDINFO)` + `exec_path_of`,
/// or `None` if the process vanished mid-scan or could not be read fully.
///
/// Coverage-off: two `libc` syscalls and a struct-to-`ProcInfo` shape conversion.
/// A short `proc_pidinfo` read (return value not equal to the struct size) means
/// the process is gone or inaccessible: mapped to `None`, never a match.
#[cfg_attr(coverage_nightly, coverage(off))]
fn proc_info_of(pid: i32) -> Option<ProcInfo> {
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

/// Reaps every `llama-server` sidecar orphaned by a previous Thuki, matched by
/// [`is_reapable_orphan`] against our canonicalized sidecar binary path.
///
/// Startup only. `SIGTERM` first (the child no longer inherits our blocked
/// signal mask, per commit `0f5687e`, so `SIGTERM` actually lands), then a
/// bounded grace, then `SIGKILL` for any survivor. Immediately before `SIGKILL`
/// the FULL predicate is re-run for that pid: during the grace the target may
/// have exited and macOS may have recycled its pid onto an innocent process, so
/// escalation only happens if the pid STILL names an orphan of ours.
///
/// If our own sidecar path cannot be canonicalized, no process can be proven
/// ours, so nothing is reaped.
///
/// Coverage-off: orchestration over the thin syscall wrappers above; its only
/// decision, "is this an orphan of ours", is the covered pure
/// [`is_reapable_orphan`]. Meant to run on a detached thread so the (rare)
/// grace sleep never delays startup.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn reap_orphaned_sidecars(our_binary: &Path) {
    // Canonicalize our own sidecar path once. If it cannot be resolved we cannot
    // prove any process is ours: reap nothing (never match on an unresolved path).
    let Ok(our_binary) = std::fs::canonicalize(our_binary) else {
        return;
    };
    let our_uid = current_ruid();

    let orphans: Vec<i32> = all_pids()
        .into_iter()
        .filter_map(proc_info_of)
        .filter(|info| is_reapable_orphan(info, &our_binary, our_uid))
        .map(|info| info.pid)
        .collect();
    if orphans.is_empty() {
        return;
    }

    for &pid in &orphans {
        send_signal(pid, libc::SIGTERM);
    }
    std::thread::sleep(std::time::Duration::from_millis(
        crate::config::defaults::ORPHAN_REAP_SIGTERM_GRACE_MS,
    ));
    for &pid in &orphans {
        // Re-verify the full predicate against a fresh read: a failed read or a
        // recycled pid no longer matches, so it is never SIGKILLed.
        if proc_info_of(pid).is_some_and(|info| is_reapable_orphan(&info, &our_binary, our_uid)) {
            send_signal(pid, libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a `ProcInfo` that satisfies all three clauses, so each test can
    /// break exactly one clause and assert the predicate flips to `false`.
    fn orphan(our_binary: &Path, our_uid: u32) -> ProcInfo {
        ProcInfo {
            pid: 4242,
            ppid: 1,
            ruid: our_uid,
            exec_path: our_binary.to_path_buf(),
        }
    }

    #[test]
    fn reapable_when_all_three_clauses_hold() {
        let binary = PathBuf::from("/apps/Thuki.app/Contents/MacOS/llama-server");
        let uid = 501;
        assert!(is_reapable_orphan(&orphan(&binary, uid), &binary, uid));
    }

    #[test]
    fn not_reapable_when_ppid_is_not_one() {
        // Right path and uid, but the process is still parented to a live Thuki
        // (ppid != 1): the load-bearing discriminator rejects it.
        let binary = PathBuf::from("/apps/Thuki.app/Contents/MacOS/llama-server");
        let uid = 501;
        let mut proc = orphan(&binary, uid);
        proc.ppid = 9931; // a live Thuki's pid, not launchd
        assert!(!is_reapable_orphan(&proc, &binary, uid));
    }

    #[test]
    fn not_reapable_when_exec_path_is_foreign() {
        // Right ppid and uid, but a user's own llama.cpp build lives elsewhere.
        let binary = PathBuf::from("/apps/Thuki.app/Contents/MacOS/llama-server");
        let uid = 501;
        let mut proc = orphan(&binary, uid);
        proc.exec_path = PathBuf::from("/opt/homebrew/bin/llama-server");
        assert!(!is_reapable_orphan(&proc, &binary, uid));
    }

    #[test]
    fn not_reapable_when_uid_is_foreign() {
        // Right ppid and path, but owned by a different user: never touch it.
        let binary = PathBuf::from("/apps/Thuki.app/Contents/MacOS/llama-server");
        let uid = 501;
        let mut proc = orphan(&binary, uid);
        proc.ruid = 502;
        assert!(!is_reapable_orphan(&proc, &binary, uid));
    }
}
