/*!
 * Screenshot capture.
 *
 * Exposes two Tauri commands:
 *
 * 1. `capture_screenshot_command`: hides the main window, invokes the
 *    macOS `screencapture -i` tool (interactive crosshair region select), and
 *    returns the captured image as a base64 string, or `None` if the user
 *    cancelled (pressed Escape without selecting).
 *
 * 2. `capture_full_screen_command`: silently captures all screens using
 *    CoreGraphics `CGWindowListCreateImageFromArray`, excluding Thuki's own
 *    windows by PID. No window hide, no flicker. Returns the absolute file
 *    path of the saved image in `<app_data_dir>/images/`.
 *
 * `temp_screenshot_path` and `encode_as_base64` are pure helpers extracted
 * from the command wrapper so they can be unit-tested without Tauri context.
 * The command wrappers themselves are excluded from coverage (thin I/O wrappers).
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
        return Ok(None); // user cancelled: screencapture creates no file on Escape
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
/// 3. Run `screencapture -i -x <path>`, which blocks until the user selects a region
///    or presses Escape. `-i` = interactive, `-x` = no shutter sound.
/// 4. Re-show the window via `show_and_make_key()` so the NSPanel becomes the
///    key window and the WebView textarea receives keyboard focus reliably.
/// 5. Delegate result handling to `process_screenshot_result`.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn capture_screenshot_command(
    app_handle: tauri::AppHandle,
) -> Result<Option<String>, String> {
    // Hide the window on the main thread. Tauri commands run on a tokio pool
    // thread, but AppKit window APIs (hide, show, makeKey) must only be called
    // from the main thread to avoid crashes.
    let hide_handle = app_handle.clone();
    app_handle
        .run_on_main_thread(move || {
            if let Some(w) = hide_handle.get_webview_window("main") {
                let _ = w.hide();
            }
        })
        .map_err(|e| format!("failed to hide window: {e}"))?;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let path = temp_screenshot_path();
    let path_str = path
        .to_str()
        .ok_or_else(|| "temp path is not valid UTF-8".to_string())?;

    // Ignore exit status: user cancellation exits 0 but creates no file.
    let _ = std::process::Command::new("screencapture")
        .args(["-i", "-x", path_str])
        .status();

    // Re-show on the main thread via show_and_make_key() so the NSPanel
    // becomes the key window, guaranteeing the WebView textarea receives
    // keyboard focus (mirrors the pattern in lib.rs).
    let show_handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        use tauri_nspanel::ManagerExt;
        match show_handle.get_webview_panel("main") {
            Ok(panel) => panel.show_and_make_key(),
            Err(_) => {
                if let Some(w) = show_handle.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        }
    });

    process_screenshot_result(&path)
}

// ─── Full-screen silent capture (macOS) ────────────────────────────────────

/// Captures raw RGBA pixel bytes of the full screen using CoreGraphics.
///
/// Captures all on-screen content below Thuki's own window in the Z-order,
/// effectively excluding Thuki from the screenshot without hiding the window.
/// Returns `(width, height, rgba_bytes)` on success.
///
/// `anchor` is a logical-point coordinate (Quartz space) used to pick which
/// display to capture in multi-monitor setups: the display containing the
/// anchor is captured. When `None` or the anchor lies outside every active
/// display, falls back to the main (menu-bar) display. The typical anchor is
/// the center of Thuki's own window, which is the display the user is
/// actually looking at when they invoke `/screen`.
///
/// MUST run on the macOS main thread. CoreGraphics APIs internally dispatch
/// to the main thread; calling them from a background thread deadlocks.
///
/// Requires Screen Recording permission (macOS Privacy & Security). If the
/// permission has not been granted, `CGWindowListCopyWindowInfo` returns NULL
/// and this function returns an informative error string.
///
/// Excluded from coverage: thin wrapper over macOS CoreGraphics FFI that
/// requires Screen Recording permission and a running display server.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn capture_full_screen_raw(anchor: Option<(f64, f64)>) -> Result<(u32, u32, Vec<u8>), String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    use std::ffi::c_void;

    // CoreFoundation / CoreGraphics opaque pointer types for our raw FFI.
    type CFArrayRef = *const c_void;
    type CFDictionaryRef = *const c_void;

    // CGWindowListOption flags.
    const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1;
    const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_BELOW_WINDOW: u32 = 1 << 2;
    const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const K_CG_NULL_WINDOW_ID: u32 = 0;
    const K_CG_WINDOW_IMAGE_DEFAULT: u32 = 0;

    // CFNumber type selector: kCFNumberSInt32Type (PID and window ID are 32-bit).
    const K_CF_NUMBER_S_INT32_TYPE: i32 = 3;

    // CGBitmapInfo for BGRA (native macOS little-endian, premultiplied alpha).
    const K_CG_BITMAP_BYTE_ORDER32_HOST: u32 = 2 << 12; // 8192
    const K_CG_IMAGE_ALPHA_PREMULTIPLIED_FIRST: u32 = 2;
    const BGRA_BITMAP_INFO: u32 =
        K_CG_BITMAP_BYTE_ORDER32_HOST | K_CG_IMAGE_ALPHA_PREMULTIPLIED_FIRST;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        /// Returns true when the current process has active Screen Recording access.
        /// Unlike CGWindowListCopyWindowInfo (which returns non-null immediately after
        /// the user clicks "Allow"), this function returns true only after the app is
        /// restarted post-grant: the reliable way to detect the
        /// "permission granted but pending restart" state that produces black captures.
        fn CGPreflightScreenCaptureAccess() -> bool;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: u32) -> CFArrayRef;
        fn CGWindowListCreateImage(
            screenBounds: CGRect,
            listOption: u32,
            relativeToWindow: u32,
            imageOption: u32,
        ) -> *const c_void;
        fn CGImageGetWidth(image: *const c_void) -> usize;
        fn CGImageGetHeight(image: *const c_void) -> usize;
        fn CGImageRelease(image: *const c_void);
        fn CGColorSpaceCreateDeviceRGB() -> *const c_void;
        fn CGColorSpaceRelease(cs: *const c_void);
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bitsPerComponent: usize,
            bytesPerRow: usize,
            colorSpace: *const c_void,
            bitmapInfo: u32,
        ) -> *const c_void;
        fn CGContextDrawImage(ctx: *const c_void, rect: CGRect, image: *const c_void);
        fn CGContextRelease(ctx: *const c_void);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFArrayGetCount(array: CFArrayRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: isize) -> *const c_void;
        fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(number: *const c_void, theType: i32, valuePtr: *mut c_void) -> bool;
        fn CFRelease(cf: *const c_void);
    }

    let our_pid = std::process::id() as i32;

    unsafe {
        // Resolve which display to capture. CGWindowListCreateImage requires a
        // concrete CGRect: passing CGRectNull/CGRectInfinite has platform-
        // dependent representations that can return null, so we always pass
        // the bounds of a specific display.
        //
        // In multi-monitor setups, capture the display containing the anchor
        // point (typically the center of Thuki's own window). This matches the
        // monitor the user is actually looking at when they invoke `/screen`.
        // If no anchor is provided or it lies outside every active display,
        // fall back to the main (menu-bar) display.
        let (sb_x, sb_y, sb_w, sb_h) = match anchor {
            Some((x, y)) => crate::cg_displays::display_for_point(x, y)
                .unwrap_or_else(crate::cg_displays::main_display),
            None => crate::cg_displays::main_display(),
        };
        let screen_bounds = CGRect {
            origin: CGPoint::new(sb_x, sb_y),
            size: CGSize::new(sb_w, sb_h),
        };

        // Two-stage permission check for Screen Recording.
        //
        // Stage 1: CGPreflightScreenCaptureAccess returns true only when
        // capture is truly active in this process. After the user clicks
        // "Allow" in the system dialog, TCC records the grant but the
        // running process still cannot read pixel data until it restarts.
        // CGWindowListCopyWindowInfo returns non-null in that window (the
        // grant is visible to TCC), but CGWindowListCreateImage returns an
        // all-black image. CGPreflightScreenCaptureAccess is the accurate
        // gate that tells us whether actual pixels are available right now.
        let option =
            K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;

        if !CGPreflightScreenCaptureAccess() {
            // Distinguish "never granted" from "granted but pending restart"
            // by probing the window list: it returns null only when permission
            // has never been granted (or was revoked).
            let probe = CGWindowListCopyWindowInfo(option, K_CG_NULL_WINDOW_ID);
            if probe.is_null() {
                return Err("Screen Recording permission is required to use /screen. \
                     Grant it in System Settings > Privacy & Security > Screen Recording."
                    .to_string());
            }
            CFRelease(probe);
            return Err(
                "Screen Recording permission was just granted but needs a restart to \
                 activate. Please quit and relaunch Thuki, then try /screen again."
                    .to_string(),
            );
        }

        let window_info_list = CGWindowListCopyWindowInfo(option, K_CG_NULL_WINDOW_ID);
        if window_info_list.is_null() {
            // Defensive: should not happen after preflight passed, but handle gracefully.
            return Err("Screen Recording permission check failed unexpectedly. \
                 Try restarting Thuki."
                .to_string());
        }

        // Find Thuki's own topmost window ID so we can capture everything
        // below it in Z-order. The window list is front-to-back, so the
        // first entry matching our PID is the topmost.
        let count = CFArrayGetCount(window_info_list);
        let pid_key = CFString::new("kCGWindowOwnerPID");
        let wid_key = CFString::new("kCGWindowNumber");

        let mut our_window_id: u32 = K_CG_NULL_WINDOW_ID;
        for i in 0..count {
            let dict = CFArrayGetValueAtIndex(window_info_list, i) as CFDictionaryRef;
            if dict.is_null() {
                continue;
            }
            let pid_val =
                CFDictionaryGetValue(dict, pid_key.as_concrete_TypeRef() as *const c_void);
            if pid_val.is_null() {
                continue;
            }
            let mut owner_pid: i32 = 0;
            CFNumberGetValue(
                pid_val,
                K_CF_NUMBER_S_INT32_TYPE,
                &mut owner_pid as *mut i32 as *mut c_void,
            );
            if owner_pid == our_pid {
                let wid_val =
                    CFDictionaryGetValue(dict, wid_key.as_concrete_TypeRef() as *const c_void);
                if !wid_val.is_null() {
                    let mut wid: u32 = 0;
                    CFNumberGetValue(
                        wid_val,
                        K_CF_NUMBER_S_INT32_TYPE,
                        &mut wid as *mut u32 as *mut c_void,
                    );
                    our_window_id = wid;
                }
                break;
            }
        }
        CFRelease(window_info_list);

        // Capture all on-screen windows below our panel, including the desktop
        // wallpaper and Dock. Omitting kCGWindowListExcludeDesktopElements is
        // intentional: that flag strips the desktop window (the wallpaper layer),
        // which produces a black image on an empty desktop. Including it gives a
        // faithful "what the user sees" composite, matching macOS Screenshot.app.
        // kCGWindowListOptionOnScreenBelowWindow already excludes Thuki itself by
        // compositing only windows lower than our_window_id in Z-order.
        //
        // Fallback (our_window_id == 0, should not occur in practice): capture all
        // on-screen windows. Thuki is transparent so its presence in the list does
        // not corrupt the image.
        let (list_option, relative_to) = if our_window_id != K_CG_NULL_WINDOW_ID {
            (
                K_CG_WINDOW_LIST_OPTION_ON_SCREEN_BELOW_WINDOW,
                our_window_id,
            )
        } else {
            (K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY, K_CG_NULL_WINDOW_ID)
        };

        let cg_image = CGWindowListCreateImage(
            screen_bounds,
            list_option,
            relative_to,
            K_CG_WINDOW_IMAGE_DEFAULT,
        );

        if cg_image.is_null() {
            return Err(
                "Screen capture failed. Ensure Screen Recording permission is \
                 granted in System Settings > Privacy & Security > Screen Recording."
                    .to_string(),
            );
        }

        let width = CGImageGetWidth(cg_image);
        let height = CGImageGetHeight(cg_image);

        if width == 0 || height == 0 {
            CGImageRelease(cg_image);
            return Err("Screen capture returned an empty image.".to_string());
        }

        // Render CGImage into a BGRA bitmap buffer.
        let bytes_per_row = width * 4;
        let mut pixel_bytes: Vec<u8> = vec![0u8; height * bytes_per_row];

        let color_space = CGColorSpaceCreateDeviceRGB();
        let ctx = CGBitmapContextCreate(
            pixel_bytes.as_mut_ptr() as *mut c_void,
            width,
            height,
            8,
            bytes_per_row,
            color_space,
            BGRA_BITMAP_INFO,
        );
        CGColorSpaceRelease(color_space);

        if ctx.is_null() {
            CGImageRelease(cg_image);
            return Err("Failed to create bitmap context for screen capture.".to_string());
        }

        let draw_rect = CGRect {
            origin: CGPoint::new(0.0, 0.0),
            size: CGSize::new(width as f64, height as f64),
        };
        CGContextDrawImage(ctx, draw_rect, cg_image);
        CGContextRelease(ctx);
        CGImageRelease(cg_image);

        // Convert BGRA to RGBA in-place (swap B and R channels).
        // CoreGraphics BGRA layout: [B, G, R, A] per pixel.
        // image crate Rgba layout:  [R, G, B, A] per pixel.
        for chunk in pixel_bytes.chunks_exact_mut(4) {
            chunk.swap(0, 2); // Swap B <-> R
        }

        Ok((width as u32, height as u32, pixel_bytes))
    }
}

/// Captures raw RGBA pixel bytes from the screen. Must be called on the macOS
/// main thread because CoreGraphics APIs internally dispatch there and will
/// deadlock if called from a background thread.
///
/// `anchor` selects which display to capture in multi-monitor setups. See
/// `capture_full_screen_raw` for the resolution rules.
///
/// Returns `(width, height, rgba_bytes)` on success.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn capture_full_screen_pixels(anchor: Option<(f64, f64)>) -> Result<(u32, u32, Vec<u8>), String> {
    capture_full_screen_raw(anchor)
}

/// Non-macOS stub: full-screen capture is macOS-only.
#[cfg(not(target_os = "macos"))]
fn capture_full_screen_pixels(_anchor: Option<(f64, f64)>) -> Result<(u32, u32, Vec<u8>), String> {
    Err("full-screen capture is only supported on macOS".to_string())
}

/// Reads the `CGDirectDisplayID` of the `NSScreen` the given `NSWindow` lives
/// on. This is the canonical way to ask "which monitor is this window
/// currently shown on?" on macOS, and it avoids the coordinate-conversion
/// mismatches that arise from manually computing logical points across
/// mixed-DPI multi-monitor setups (e.g. a 2x retina primary + 1x secondary).
///
/// Uses raw Objective-C runtime messaging so we do not need to enable extra
/// `objc2-app-kit` features. MUST be called on the macOS main thread: AppKit
/// window/screen APIs are main-thread-only.
///
/// Returns `None` when:
/// - the pointer is null,
/// - the window has no current screen (offscreen / mid-transition),
/// - the device-description dictionary lacks `NSScreenNumber`, or
/// - any runtime message returns nil.
///
/// Excluded from coverage: pure Objective-C runtime messaging that requires a
/// live window server and a real `NSWindow` instance.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
unsafe fn nswindow_display_id(ns_window: *mut std::ffi::c_void) -> Option<u32> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSString;

    if ns_window.is_null() {
        return None;
    }
    let ns_window: *mut AnyObject = ns_window.cast();

    let ns_screen: *mut AnyObject = msg_send![ns_window, screen];
    if ns_screen.is_null() {
        return None;
    }

    let device_desc: *mut AnyObject = msg_send![ns_screen, deviceDescription];
    if device_desc.is_null() {
        return None;
    }

    let key = NSString::from_str("NSScreenNumber");
    let key_ref: *const NSString = &*key;
    let value: *mut AnyObject = msg_send![device_desc, objectForKey: key_ref];
    if value.is_null() {
        return None;
    }

    let display_id: u32 = msg_send![value, unsignedIntValue];
    Some(display_id)
}

/// Returns the Quartz-coordinate center of a display rectangle expressed as
/// `(origin_x, origin_y, width, height)`. Pure helper, used to derive an
/// anchor point from a known display's bounds.
fn display_bounds_center(bounds: (f64, f64, f64, f64)) -> (f64, f64) {
    let (x, y, w, h) = bounds;
    (x + w / 2.0, y + h / 2.0)
}

/// Tauri command: silently captures the full screen (excluding Thuki's own
/// windows) and returns the absolute file path of the saved image.
///
/// CoreGraphics APIs internally dispatch to the main thread, so calling them
/// from a tokio pool thread (via `spawn_blocking`) causes a deadlock. Instead,
/// `capture_full_screen` runs on the main thread via `run_on_main_thread`,
/// producing raw RGBA pixel bytes. The heavy image encoding and disk I/O then
/// happen on a blocking thread to avoid stalling the UI.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn capture_full_screen_command(
    app_handle: tauri::AppHandle,
    conversation_id: String,
    trace_recorder: tauri::State<'_, std::sync::Arc<crate::trace::LiveTraceRecorder>>,
) -> Result<String, String> {
    use crate::trace::TraceRecorder;
    let base_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;

    // Resolve the Thuki window so we can ask AppKit which display it lives on.
    // The handle is read here (off the main thread) but only dereferenced
    // inside the main-thread closure below: AppKit window/screen APIs are
    // strictly main-thread-only.
    let main_window = app_handle.get_webview_window("main");

    // Phase 1: Capture raw RGBA pixels on the main thread (CoreGraphics
    // requirement). Returns (width, height, rgba_bytes).
    //
    // The anchor point steers multi-monitor capture: we look up the
    // `CGDirectDisplayID` of the `NSScreen` the Thuki window is on, then take
    // the center of that display's bounds. Fallback chain: window missing →
    // `ns_window` unavailable → `NSScreen` nil → `None`, which downstream
    // resolves to the main (menu-bar) display.
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(u32, u32, Vec<u8>), String>>();
    app_handle
        .run_on_main_thread(move || {
            #[cfg(target_os = "macos")]
            let anchor = main_window
                .as_ref()
                .and_then(|w| w.ns_window().ok())
                .and_then(|p| unsafe { nswindow_display_id(p) })
                .map(|id| display_bounds_center(crate::cg_displays::bounds_for_display(id)));
            #[cfg(not(target_os = "macos"))]
            let anchor: Option<(f64, f64)> = {
                let _ = &main_window;
                None
            };
            tx.send(capture_full_screen_pixels(anchor)).ok();
        })
        .map_err(|e| format!("failed to dispatch capture to main thread: {e}"))?;

    let (width, height, rgba_bytes) = rx
        .await
        .map_err(|_| "main thread capture channel closed unexpectedly".to_string())??;

    // Phase 2: Encode to PNG and save via the images pipeline on a blocking
    // thread so the main thread stays responsive.
    let saved_path = tokio::task::spawn_blocking(move || {
        let buf =
            image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, rgba_bytes)
                .ok_or_else(|| "Failed to create image buffer from captured pixels.".to_string())?;
        let dynamic = image::DynamicImage::ImageRgba8(buf);

        let mut png: Vec<u8> = Vec::new();
        dynamic
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| format!("Failed to encode screen capture as PNG: {e}"))?;

        crate::images::save_image(&base_dir, &png)
    })
    .await
    .map_err(|e| format!("image encoding task failed: {e}"))??;

    // Mirror the capture into the unified trace recorder. Records the
    // saved-file path (paths only, never image bytes) and the displays
    // count (currently always 1 because `capture_full_screen_pixels`
    // returns a merged image; future multi-monitor work can refine).
    trace_recorder.record(
        &crate::trace::ConversationId::new(conversation_id),
        crate::trace::RecorderEvent::ScreenCaptured {
            image_path: saved_path.clone(),
            displays: 1,
        },
    );
    Ok(saved_path)
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn capture_full_screen_returns_err_on_non_macos() {
        let result = capture_full_screen_pixels(None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only supported on macOS"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn capture_full_screen_returns_err_on_non_macos_with_anchor() {
        let result = capture_full_screen_pixels(Some((100.0, 100.0)));
        assert!(result.is_err());
    }

    #[test]
    fn display_bounds_center_returns_midpoint_for_primary_display() {
        // Primary display at origin (0, 0), 1920x1080: center is (960, 540).
        assert_eq!(
            display_bounds_center((0.0, 0.0, 1920.0, 1080.0)),
            (960.0, 540.0)
        );
    }

    #[test]
    fn display_bounds_center_returns_midpoint_for_offset_display() {
        // Secondary display at (1920, 0), 1920x1080: center is (2880, 540).
        // This is the case that the multi-monitor fix targets: anchoring on
        // a non-primary display so the screen capture picks the right one.
        assert_eq!(
            display_bounds_center((1920.0, 0.0, 1920.0, 1080.0)),
            (2880.0, 540.0)
        );
    }

    #[test]
    fn display_bounds_center_handles_negative_origin() {
        // Display positioned left of the primary has a negative origin in
        // Quartz coordinates (origin at primary's top-left).
        assert_eq!(
            display_bounds_center((-1280.0, 0.0, 1280.0, 720.0)),
            (-640.0, 360.0)
        );
    }

    #[test]
    fn display_bounds_center_handles_zero_size() {
        // Defensive: a zero-sized rect collapses to its origin. We never
        // expect to see this in practice (CGDisplayBounds returns a real
        // rect), but the helper is pure and must not panic.
        assert_eq!(
            display_bounds_center((100.0, 200.0, 0.0, 0.0)),
            (100.0, 200.0)
        );
    }
}
