/*!
 * Permissions Module
 *
 * Exposes Tauri commands for querying and requesting macOS privacy permissions
 * required by Thuki (Accessibility and Screen Recording), plus the pure-logic
 * helper that decides whether the onboarding screen must be shown.
 *
 * Architecture: thin command wrappers (excluded from coverage) delegate to
 * small, testable functions. The only logic exercised at test-time is
 * `needs_onboarding`, which is a pure predicate with no OS side-effects.
 */

// ─── Pure Logic ──────────────────────────────────────────────────────────────

/// Returns `true` when at least one required permission has not been granted.
///
/// All three permissions must be granted for Thuki to function fully:
/// - Accessibility: required to create the CGEventTap
/// - Input Monitoring: required for the tap to receive events from other apps
/// - Screen Recording: required for the /screen command
///
/// If any is missing the onboarding screen is shown instead of the normal overlay.
pub fn needs_onboarding(
    accessibility: bool,
    screen_recording: bool,
    input_monitoring: bool,
) -> bool {
    !accessibility || !screen_recording || !input_monitoring
}

// ─── macOS Permission Checks ─────────────────────────────────────────────────

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

/// IOKit constants for Input Monitoring permission checks.
#[cfg(target_os = "macos")]
const IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;
#[cfg(target_os = "macos")]
const IOHID_ACCESS_TYPE_GRANTED: u32 = 1;

#[cfg(target_os = "macos")]
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    /// Checks whether the process has Input Monitoring access.
    /// Returns kIOHIDAccessTypeGranted (1) if granted, 0 if not determined,
    /// 2 if denied.
    fn IOHIDCheckAccess(request_type: u32) -> u32;

    /// Requests Input Monitoring access, showing the system permission dialog
    /// on first call. Returns true if access was granted.
    fn IOHIDRequestAccess(request_type: u32) -> bool;
}

/// Returns whether the process currently has Accessibility permission.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn is_accessibility_granted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Returns whether the process currently has Input Monitoring permission.
///
/// Input Monitoring (`kTCCServiceListenEvent`) is required for a CGEventTap at
/// Session level to receive keyboard events from other applications. Without it,
/// the tap only sees events generated within the Thuki process itself, making
/// the double-tap hotkey invisible when the user's focus is elsewhere.
///
/// Unlike Screen Recording, Input Monitoring does not require a process restart
/// after being granted; the CGEventTap immediately begins receiving cross-app events.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn is_input_monitoring_granted() -> bool {
    unsafe { IOHIDCheckAccess(IOHID_REQUEST_TYPE_LISTEN_EVENT) == IOHID_ACCESS_TYPE_GRANTED }
}

/// Returns whether the process currently has Screen Recording permission.
///
/// Uses `CGPreflightScreenCaptureAccess`, which only returns `true` after
/// a full restart post-grant (unlike `CGWindowListCopyWindowInfo` which
/// returns non-null immediately but before pixels are actually accessible).
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn is_screen_recording_granted() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    unsafe { CGPreflightScreenCaptureAccess() }
}

// ─── Tauri Commands ──────────────────────────────────────────────────────────

/// Returns whether Accessibility permission has been granted.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn check_accessibility_permission() -> bool {
    is_accessibility_granted()
}

/// Returns whether Input Monitoring permission has been granted.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn check_input_monitoring_permission() -> bool {
    is_input_monitoring_granted()
}

/// Triggers the macOS Input Monitoring permission dialog.
///
/// `IOHIDRequestAccess` registers the app in TCC and shows the system-level
/// "X would like to monitor input events" prompt. If the user previously denied
/// the permission, the dialog is skipped and the call returns false; the
/// onboarding UI then directs the user to System Settings directly.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn request_input_monitoring_access() {
    unsafe {
        IOHIDRequestAccess(IOHID_REQUEST_TYPE_LISTEN_EVENT);
    }
}

/// Opens System Settings to the Input Monitoring privacy pane.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn open_input_monitoring_settings() -> Result<(), String> {
    std::process::Command::new("open")
        .arg(
            "x-apple.systempreferences:com.apple.preference.security\
             ?Privacy_ListenEvent",
        )
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Opens System Settings to the Accessibility privacy pane so the user can
/// enable the permission without encountering the native system popup.
///
/// This gives a consistent onboarding experience: both Accessibility and
/// Screen Recording are granted via System Settings rather than native dialogs.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn open_accessibility_settings() -> Result<(), String> {
    std::process::Command::new("open")
        .arg(
            "x-apple.systempreferences:com.apple.preference.security\
             ?Privacy_Accessibility",
        )
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Returns whether Screen Recording permission has been granted.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn check_screen_recording_permission() -> bool {
    is_screen_recording_granted()
}

/// Opens System Settings to the Screen Recording privacy pane so the user
/// can enable the permission without navigating there manually.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn open_screen_recording_settings() -> Result<(), String> {
    std::process::Command::new("open")
        .arg(
            "x-apple.systempreferences:com.apple.preference.security\
             ?Privacy_ScreenCapture",
        )
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Registers Thuki in the Screen Recording privacy pane and shows the macOS
/// permission prompt.
///
/// `CGRequestScreenCaptureAccess` is the only API that both adds the app to
/// System Settings > Privacy & Security > Screen & System Audio Recording and
/// triggers the native "allow screen recording" alert. Without calling this
/// first, Thuki will not appear in the Screen Recording list at all.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn request_screen_recording_access() {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGRequestScreenCaptureAccess() -> bool;
    }
    unsafe {
        CGRequestScreenCaptureAccess();
    }
}

/// Returns `true` if Screen & System Audio Recording permission is currently
/// granted. Delegates to `CGPreflightScreenCaptureAccess`, which correctly
/// returns `false` when the permission has not been granted, fixing the
/// historical false-positive from `CGWindowListCopyWindowInfo(0, 0)`.
///
/// Called by PermissionsStep during onboarding polling so the "Quit & Reopen"
/// prompt appears once the user toggles the permission on in System Settings.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn check_screen_recording_tcc_granted() -> bool {
    is_screen_recording_granted()
}

/// Quits Thuki and immediately relaunches it.
///
/// Called after the user grants Screen Recording permission. macOS requires
/// a full process restart before the new permission takes effect.
///
/// Writes "intro" to the DB before restarting so `notify_frontend_ready`
/// shows the intro screen on the next launch without calling any permission
/// API. Permission APIs (CGPreflightScreenCaptureAccess) can return stale
/// results immediately after a restart on macOS 15+; trusting the DB stage
/// avoids that unreliability entirely.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn quit_and_relaunch(app_handle: tauri::AppHandle, db: tauri::State<crate::history::Database>) {
    if let Ok(conn) = db.0.lock() {
        let _ = crate::onboarding::set_stage(&conn, &crate::onboarding::OnboardingStage::Intro);
    }
    app_handle.restart();
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_onboarding_false_when_all_granted() {
        assert!(!needs_onboarding(true, true, true));
    }

    #[test]
    fn needs_onboarding_true_when_accessibility_missing() {
        assert!(needs_onboarding(false, true, true));
    }

    #[test]
    fn needs_onboarding_true_when_screen_recording_missing() {
        assert!(needs_onboarding(true, false, true));
    }

    #[test]
    fn needs_onboarding_true_when_input_monitoring_missing() {
        assert!(needs_onboarding(true, true, false));
    }

    #[test]
    fn needs_onboarding_true_when_all_missing() {
        assert!(needs_onboarding(false, false, false));
    }
}
