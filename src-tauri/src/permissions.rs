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
/// Both Accessibility (hotkey listener) and Screen Recording (/screen command)
/// must be granted for Thuki to function fully. If either is missing the
/// onboarding screen is shown instead of the normal overlay.
pub fn needs_onboarding(accessibility: bool, screen_recording: bool) -> bool {
    !accessibility || !screen_recording
}

// ─── macOS Permission Checks ─────────────────────────────────────────────────

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

/// Returns whether the process currently has Accessibility permission.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn is_accessibility_granted() -> bool {
    unsafe { AXIsProcessTrusted() }
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

/// Returns `true` if Screen Recording has been granted in TCC without requiring
/// a process restart.
///
/// `CGPreflightScreenCaptureAccess` only returns `true` after a full process
/// restart. `CGWindowListCopyWindowInfo` returns a non-null array as soon as
/// TCC records the grant, making it suitable for polling during onboarding so
/// the "Quit and Reopen" prompt appears immediately when the user toggles the
/// permission on in System Settings.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn check_screen_recording_tcc_granted() -> bool {
    use std::ffi::c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        // option: kCGWindowListOptionAll = 0, relativeToWindow: kCGNullWindowID = 0
        fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }
    unsafe {
        let list = CGWindowListCopyWindowInfo(0, 0);
        if list.is_null() {
            return false;
        }
        CFRelease(list);
        true
    }
}

/// Quits Thuki and immediately relaunches it.
///
/// Called after the user grants Screen Recording permission. macOS requires
/// a full process restart before the new permission takes effect.
///
/// Advances the onboarding stage to "intro" before restarting so the next
/// launch shows the intro screen rather than re-checking permissions via
/// CGPreflightScreenCaptureAccess, which can return false on macOS 15 even
/// after a successful grant.
#[tauri::command]
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn quit_and_relaunch(app_handle: tauri::AppHandle) {
    // No DB write needed here. The onboarding stage remains "permissions"
    // across the restart. On the next launch, notify_frontend_ready detects
    // that both permissions are now granted and stage is still "permissions",
    // advances the stage to "intro", and shows the intro screen.
    app_handle.restart();
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_onboarding_false_when_both_granted() {
        assert!(!needs_onboarding(true, true));
    }

    #[test]
    fn needs_onboarding_true_when_accessibility_missing() {
        assert!(needs_onboarding(false, true));
    }

    #[test]
    fn needs_onboarding_true_when_screen_recording_missing() {
        assert!(needs_onboarding(true, false));
    }

    #[test]
    fn needs_onboarding_true_when_both_missing() {
        assert!(needs_onboarding(false, false));
    }
}
