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

/// Subset of `SERVICES` accepted by the click-time reset command. Held as
/// a separate const so any addition to the runtime services list is an
/// explicit, reviewed change to the click-time API surface.
const CLICK_TIME_SERVICES: &[&str] = &["Accessibility", "ScreenCapture"];

/// Returns the canonical TCC service string when `service` is one Thuki
/// is willing to reset on demand, or `None` otherwise. The frontend hands
/// the string straight to a Tauri command so this validator is the trust
/// boundary that prevents `tccutil reset <arbitrary> <bundle>` from
/// shelling out with caller-controlled args.
pub fn validate_click_time_service(service: &str) -> Option<&'static str> {
    CLICK_TIME_SERVICES
        .iter()
        .copied()
        .find(|allowed| *allowed == service)
}

/// Pure decision function. Returns `true` when the running binary may not
/// match the code requirement of any TCC entry currently on disk.
///
/// - Recorded version differs from running version: upgrade just happened.
/// - No recorded version: either a truly first-ever launch (no prior grant
///   to clear, so the reset is a harmless no-op) OR an upgrade from a
///   pre-0.8.2 build that never wrote the sidecar (and almost certainly
///   has stale grants whose csreq no longer matches). Treating both the
///   same costs one extra restart on first install but is the only way to
///   migrate users coming from pre-sidecar builds without leaving them
///   stuck with a stale "granted" toggle.
/// - Recorded equals running: normal subsequent launch, nothing to do.
pub fn should_reset_for_upgrade(recorded: Option<&str>, running: &str) -> bool {
    match recorded {
        Some(prev) => prev != running,
        None => true,
    }
}

/// Shells out to `/usr/bin/tccutil reset <service> <bundle_id>`. Logs
/// failures but never propagates them: TCC reset is a UX nicety, not a
/// correctness requirement.
#[cfg_attr(coverage_nightly, coverage(off))]
fn tccutil_reset_one(bundle_id: &str, service: &str) {
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

/// Resets every TCC service Thuki uses for `bundle_id`. Called from the
/// startup upgrade path so a relaunched binary starts with a clean slate.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn tccutil_reset(bundle_id: &str) {
    for service in SERVICES {
        tccutil_reset_one(bundle_id, service);
    }
}

/// Resets a single TCC service for `bundle_id`. Called from the click-time
/// reset flow so the user only blows away the grant for the permission
/// they are actively re-requesting.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn tccutil_reset_service(bundle_id: &str, service: &str) {
    tccutil_reset_one(bundle_id, service);
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
    fn reset_when_recorded_is_absent() {
        // No sidecar version: either first-ever launch (reset is a no-op)
        // or an upgrade from a pre-0.8.2 build whose stale csreq must be
        // cleared. Both cases must reset so pre-sidecar users are not
        // stranded with a stale "granted" toggle.
        assert!(should_reset_for_upgrade(None, "0.8.1"));
    }

    #[test]
    fn reset_when_recorded_version_is_higher_than_running() {
        // Downgrade still counts as a binary swap. The csreq differs in
        // either direction, so the stale grant must be cleared.
        assert!(should_reset_for_upgrade(Some("0.9.0"), "0.8.1"));
    }

    #[test]
    fn validate_click_time_service_accepts_accessibility() {
        assert_eq!(
            validate_click_time_service("Accessibility"),
            Some("Accessibility"),
        );
    }

    #[test]
    fn validate_click_time_service_accepts_screen_capture() {
        assert_eq!(
            validate_click_time_service("ScreenCapture"),
            Some("ScreenCapture"),
        );
    }

    #[test]
    fn validate_click_time_service_rejects_arbitrary_strings() {
        // Trust boundary check: anything not in the allow-list must be
        // rejected so the Tauri command does not shell out with
        // caller-controlled service args.
        assert!(validate_click_time_service("").is_none());
        assert!(validate_click_time_service("Camera").is_none());
        assert!(validate_click_time_service("accessibility").is_none());
        assert!(validate_click_time_service("Accessibility; rm -rf /").is_none());
    }
}
