/*!
 * CoreGraphics display lookup helpers (macOS).
 *
 * Wraps `CGGetDisplaysWithPoint` for hit-testing and `CGDisplayBounds` for
 * resolving a display's Quartz-coordinate rectangle. All coordinates are in
 * the Quartz display coordinate space (top-left of the primary display,
 * Y-down), matching the AX API and `CGEventGetLocation`.
 *
 * Used by the activator to position the overlay on the correct monitor, and
 * by the screenshot pipeline to capture the display the user is actually on
 * (rather than always capturing the primary display).
 */

#![cfg(target_os = "macos")]

use core_graphics::geometry::{CGPoint, CGRect};

type CGDirectDisplayID = u32;

extern "C" {
    fn CGGetDisplaysWithPoint(
        point: CGPoint,
        max_displays: u32,
        displays: *mut CGDirectDisplayID,
        matching_display_count: *mut u32,
    ) -> i32;
    fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    fn CGMainDisplayID() -> CGDirectDisplayID;
}

/// Returns `(origin_x, origin_y, width, height)` in Quartz points for the
/// display containing `(global_x, global_y)`. Returns `None` when the point
/// lies outside every active display.
///
/// Excluded from coverage: thin wrapper over CoreGraphics FFI that requires a
/// live window server to exercise.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn display_for_point(global_x: f64, global_y: f64) -> Option<(f64, f64, f64, f64)> {
    unsafe {
        let point = CGPoint::new(global_x, global_y);
        let mut ids = [0u32; 4];
        let mut count: u32 = 0;
        let err = CGGetDisplaysWithPoint(point, 4, ids.as_mut_ptr(), &mut count);
        if err != 0 || count == 0 {
            return None;
        }
        let r = CGDisplayBounds(ids[0]);
        Some((r.origin.x, r.origin.y, r.size.width, r.size.height))
    }
}

/// Returns `(origin_x, origin_y, width, height)` of the main (menu-bar) display.
///
/// Excluded from coverage: thin wrapper over CoreGraphics FFI that requires a
/// live window server to exercise.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn main_display() -> (f64, f64, f64, f64) {
    unsafe {
        let r = CGDisplayBounds(CGMainDisplayID());
        (r.origin.x, r.origin.y, r.size.width, r.size.height)
    }
}

/// Returns `(origin_x, origin_y, width, height)` in Quartz points for a
/// specific `CGDirectDisplayID`. Used by the screenshot pipeline once the
/// display ID of the window's NSScreen is known.
///
/// Excluded from coverage: thin wrapper over CoreGraphics FFI that requires a
/// live window server to exercise.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn bounds_for_display(display_id: u32) -> (f64, f64, f64, f64) {
    unsafe {
        let r = CGDisplayBounds(display_id);
        (r.origin.x, r.origin.y, r.size.width, r.size.height)
    }
}
