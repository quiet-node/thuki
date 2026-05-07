//! macOS TCC grant reset on app upgrade.
//!
//! Background. Thuki is ad-hoc signed (no Apple Developer ID). macOS keys
//! TCC (Transparency, Consent, Control) grants by code requirement, not
//! bundle ID. When the auto-updater swaps the binary, the new code
//! requirement does not match the stored grant, so System Settings shows
//! "Thuki: granted" but `AXIsProcessTrusted` returns false. The toggle is a
//! visual lie.
//!
//! `tccutil reset <service> <bundle-id>` removes the entry for that bundle
//! ID under that service. On the next permission request, macOS adds a
//! fresh entry tied to the current binary's code requirement, which then
//! actually grants the running app when the user toggles it on.
//!
//! This module:
//!
//! 1. Defines which TCC services Thuki uses.
//! 2. Provides a pure helper, `should_reset_for_upgrade`, that decides
//!    whether the running version differs from what the sidecar last
//!    recorded.
//! 3. Provides `tccutil_reset`, a thin wrapper around `/usr/bin/tccutil`
//!    that fails open: any error is logged and ignored. A failed reset
//!    leaves the user with the existing manual toggle-off / toggle-on
//!    workaround, which is no worse than today's behavior.

use std::process::Command;

/// TCC services Thuki actively uses and whose stale grants need clearing
/// on an upgrade. `Accessibility` powers the global Control hotkey;
/// `ScreenCapture` powers the `/screen` command.
const SERVICES: &[&str] = &["Accessibility", "ScreenCapture"];

/// Pure decision function. Returns `true` when the recorded version
/// differs from the running version, indicating an upgrade just happened.
/// Returns `false` when:
/// - The sidecar has no recorded version (first ever launch; nothing to
///   reset because no prior binary ever held grants).
/// - The recorded version equals the running version (normal launch).
pub fn should_reset_for_upgrade(recorded: Option<&str>, running: &str) -> bool {
    match recorded {
        Some(prev) => prev != running,
        None => false,
    }
}

/// Shells out to `/usr/bin/tccutil reset <service> <bundle_id>` for each
/// TCC service Thuki uses. Logs failures but never propagates them: TCC
/// reset is a UX nicety, not a correctness requirement.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn tccutil_reset(bundle_id: &str) {
    for service in SERVICES {
        let result = Command::new("/usr/bin/tccutil")
            .args(["reset", service, bundle_id])
            .status();
        match result {
            Ok(status) if status.success() => {
                eprintln!("thuki: [updater] cleared stale TCC grant for {service} ({bundle_id})");
            }
            Ok(status) => {
                eprintln!(
                    "thuki: [updater] tccutil reset {service} exited with {status}; \
                     leaving any existing grant in place"
                );
            }
            Err(e) => {
                eprintln!(
                    "thuki: [updater] tccutil invocation failed: {e}; \
                     leaving any existing grant in place"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_reset_when_recorded_version_matches() {
        assert!(!should_reset_for_upgrade(Some("0.8.1"), "0.8.1"));
    }

    #[test]
    fn reset_when_recorded_version_differs() {
        assert!(should_reset_for_upgrade(Some("0.8.0"), "0.8.1"));
    }

    #[test]
    fn no_reset_on_first_ever_launch_when_recorded_is_absent() {
        // First launch: nothing recorded, nothing to invalidate.
        assert!(!should_reset_for_upgrade(None, "0.8.1"));
    }

    #[test]
    fn reset_when_recorded_version_is_higher_than_running() {
        // Downgrade still counts as a binary swap — the csreq differs in
        // either direction, so the stale grant must be cleared.
        assert!(should_reset_for_upgrade(Some("0.9.0"), "0.8.1"));
    }
}
