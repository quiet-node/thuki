/*!
 * Screenshot capture.
 *
 * Exposes a single Tauri command that hides the main window, invokes the
 * macOS `screencapture -i` tool (interactive crosshair region select), and
 * returns the captured image as a base64 string — or `None` if the user
 * cancelled (pressed Escape without selecting).
 *
 * `temp_screenshot_path` and `encode_as_base64` are pure helpers extracted
 * from the command wrapper so they can be unit-tested without Tauri context.
 * The command wrapper itself is excluded from coverage (thin I/O wrapper).
 */

use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use tauri::Manager;

/// Returns a unique `/tmp/<uuid>-thuki.png` path for a single screenshot capture.
/// A new UUID is generated on every call, preventing collisions.
pub fn temp_screenshot_path() -> PathBuf {
    PathBuf::from(format!("/tmp/{}-thuki.png", uuid::Uuid::new_v4()))
}

/// Encodes raw bytes to a standard base64 string for IPC transfer.
pub fn encode_as_base64(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

/// Converts a captured screenshot temp file into a base64-encoded PNG string.
///
/// Returns `Ok(None)` if the file was not created (user cancelled via Escape).
/// Returns `Ok(Some(base64))` on success, deleting the temp file after reading.
/// Returns `Err` if the file exists but cannot be read.
pub fn process_screenshot_result(path: &PathBuf) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None); // user cancelled — screencapture creates no file on Escape
    }
    let bytes = std::fs::read(path).map_err(|e| format!("failed to read screenshot file: {e}"))?;
    let _ = std::fs::remove_file(path);
    Ok(Some(encode_as_base64(&bytes)))
}

// ─── Tauri command ──────────────────────────────────────────────────────────

/// Captures a user-selected screen region and returns it as base64-encoded PNG.
///
/// Flow:
/// 1. Hide the main window (so it doesn't appear in the screenshot).
/// 2. Sleep 200 ms to let the window fully disappear before the crosshair appears.
/// 3. Run `screencapture -i -x <path>` — blocks until the user selects a region
///    or presses Escape. `-i` = interactive, `-x` = no shutter sound.
/// 4. Re-show the window via `show_and_make_key()` so the NSPanel becomes the
///    key window and the WebView textarea receives keyboard focus reliably.
/// 5. Delegate result handling to `process_screenshot_result`.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn capture_screenshot_command(
    app_handle: tauri::AppHandle,
) -> Result<Option<String>, String> {
    use tauri_nspanel::ManagerExt;

    let window = app_handle
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;

    window
        .hide()
        .map_err(|e| format!("failed to hide window: {e}"))?;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let path = temp_screenshot_path();
    let path_str = path
        .to_str()
        .ok_or_else(|| "temp path is not valid UTF-8".to_string())?;

    // Ignore exit status — user cancellation exits 0 but creates no file.
    let _ = std::process::Command::new("screencapture")
        .args(["-i", "-x", path_str])
        .status();

    // Re-show via show_and_make_key() so the NSPanel becomes the key window,
    // guaranteeing the WebView textarea receives keyboard focus (mirrors lib.rs).
    match app_handle.get_webview_panel("main") {
        Ok(panel) => panel.show_and_make_key(),
        Err(_) => {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }

    process_screenshot_result(&path)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_screenshot_result_returns_none_when_file_missing() {
        let path = PathBuf::from(format!("/tmp/{}-missing.png", uuid::Uuid::new_v4()));
        assert_eq!(process_screenshot_result(&path).unwrap(), None);
    }

    #[test]
    fn process_screenshot_result_returns_base64_and_deletes_file() {
        let path = temp_screenshot_path();
        let content = b"fake png content";
        std::fs::write(&path, content).unwrap();
        let result = process_screenshot_result(&path).unwrap();
        assert_eq!(result, Some(encode_as_base64(content)));
        assert!(
            !path.exists(),
            "temp file should be deleted after processing"
        );
    }

    #[test]
    fn process_screenshot_result_returns_error_when_file_unreadable() {
        // A directory path exists but cannot be read as a file.
        let dir = std::env::temp_dir();
        let err = process_screenshot_result(&dir).unwrap_err();
        assert!(
            err.contains("failed to read screenshot file"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn temp_screenshot_path_is_in_tmp_and_ends_with_png() {
        let path = temp_screenshot_path();
        let s = path.to_str().unwrap();
        assert!(s.starts_with("/tmp/"), "expected /tmp/ prefix, got: {s}");
        assert!(
            s.ends_with("-thuki.png"),
            "expected -thuki.png suffix, got: {s}"
        );
    }

    #[test]
    fn temp_screenshot_path_generates_unique_paths() {
        let a = temp_screenshot_path();
        let b = temp_screenshot_path();
        assert_ne!(a, b, "two calls should return different paths");
    }

    #[test]
    fn encode_as_base64_roundtrip() {
        let original = b"hello screenshot world";
        let encoded = encode_as_base64(original);
        let decoded = BASE64.decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn encode_as_base64_empty_input() {
        assert_eq!(encode_as_base64(b""), "");
    }
}
