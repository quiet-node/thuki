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

// ─── Tauri command ──────────────────────────────────────────────────────────

/// Captures a user-selected screen region and returns it as base64-encoded PNG.
///
/// Flow:
/// 1. Hide the main window (so it doesn't appear in the screenshot).
/// 2. Sleep 200 ms to let the window fully disappear before the crosshair appears.
/// 3. Run `screencapture -i -x <path>` — blocks until the user selects a region
///    or presses Escape. `-i` = interactive, `-x` = no shutter sound.
/// 4. Show + focus the window again regardless of outcome.
/// 5. If the file was not created (user cancelled), return `Ok(None)`.
/// 6. Otherwise read the file, delete it, and return `Ok(Some(base64))`.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn capture_screenshot_command(
    app_handle: tauri::AppHandle,
) -> Result<Option<String>, String> {
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

    // Always re-show even if capture failed, so the overlay is not stuck hidden.
    let _ = window.show();
    let _ = window.set_focus();

    if !path.exists() {
        return Ok(None); // user cancelled
    }

    let bytes =
        std::fs::read(&path).map_err(|e| format!("failed to read screenshot file: {e}"))?;
    let _ = std::fs::remove_file(&path);

    Ok(Some(encode_as_base64(&bytes)))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_screenshot_path_is_in_tmp_and_ends_with_png() {
        let path = temp_screenshot_path();
        let s = path.to_str().unwrap();
        assert!(s.starts_with("/tmp/"), "expected /tmp/ prefix, got: {s}");
        assert!(s.ends_with("-thuki.png"), "expected -thuki.png suffix, got: {s}");
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
