//! Captures contextual information at the moment of overlay activation.
//!
//! Queries the macOS Accessibility API to detect any currently selected text
//! and its screen bounds. Falls back gracefully when the focused app does not
//! fully implement the AX protocol.
//!
//! `ActivationContext` and `calculate_window_position` are cross-platform.
//! The AX capture implementation is macOS-only.

// ─── Cross-platform public types ─────────────────────────────────────────────

/// Platform-independent screen rectangle in logical points (top-left origin).
#[derive(Debug, Clone, Copy)]
pub struct ScreenRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Context captured at the moment of overlay activation.
#[derive(Debug, Clone)]
pub struct ActivationContext {
    /// The currently selected text in the focused app, if any.
    pub selected_text: Option<String>,
    /// Screen bounds of the selection in logical points.
    /// `None` when AX cannot provide bounds for the selection (e.g. Chromium apps).
    pub bounds: Option<ScreenRect>,
    /// Mouse cursor position in logical screen coordinates at activation time.
    /// Used as a positioning anchor when `bounds` is unavailable but text was captured.
    pub mouse_position: Option<(f64, f64)>,
}

impl ActivationContext {
    /// Returns an empty context with no selection, bounds, or mouse position.
    /// Used for menu-item and tray-icon activations where no host-app context
    /// is available.
    pub fn empty() -> Self {
        Self {
            selected_text: None,
            bounds: None,
            mouse_position: None,
        }
    }
}

// ─── macOS AX capture ────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
mod macos {
    use std::ffi::c_void;

    use core_foundation::base::{CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};

    use super::{ActivationContext, ScreenRect};

    type AXUIElementRef = *const c_void;
    type AXError = i32;
    const K_AX_ERROR_SUCCESS: AXError = 0;
    /// AXValueType constant for CGRect (kAXValueCGRectType = 3).
    const K_AX_VALUE_TYPE_CG_RECT: u32 = 3;

    // ApplicationServices is already linked by activator.rs.
    extern "C" {
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        fn AXUIElementCopyParameterizedAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            parameter: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        fn AXValueGetValue(value: CFTypeRef, the_type: u32, out: *mut c_void) -> bool;
        fn CFRelease(cf: CFTypeRef);
        // CoreGraphics: mouse position and keyboard event simulation.
        fn CGEventCreate(source: *const c_void) -> CFTypeRef;
        fn CGEventGetLocation(event: CFTypeRef) -> CGPoint;
        fn CGEventCreateKeyboardEvent(
            source: *const c_void,
            virtual_key: u16,
            key_down: bool,
        ) -> CFTypeRef;
        fn CGEventSetFlags(event: CFTypeRef, flags: u64);
        fn CGEventPost(tap_location: u32, event: CFTypeRef);
    }

    /// macOS virtual keycode for 'c'.
    const KEY_C: u16 = 0x08;
    /// CGEventTapLocation::kCGHIDEventTap
    const K_CG_HID_EVENT_TAP: u32 = 0;
    /// CGEventFlags::kCGEventFlagMaskCommand
    const K_CG_EVENT_FLAG_MASK_COMMAND: u64 = 0x0010_0000;

    /// Returns the current mouse cursor position in logical screen coordinates.
    unsafe fn current_mouse_position() -> (f64, f64) {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return (0.0, 0.0);
        }
        let pt = CGEventGetLocation(event);
        CFRelease(event);
        (pt.x, pt.y)
    }

    /// Posts a synthetic Cmd+C key-down / key-up pair to the focused application.
    ///
    /// # Safety
    /// Caller must ensure Accessibility permission is granted before calling.
    unsafe fn simulate_cmd_c() {
        let down = CGEventCreateKeyboardEvent(std::ptr::null(), KEY_C, true);
        if !down.is_null() {
            CGEventSetFlags(down, K_CG_EVENT_FLAG_MASK_COMMAND);
            CGEventPost(K_CG_HID_EVENT_TAP, down);
            CFRelease(down);
        }
        let up = CGEventCreateKeyboardEvent(std::ptr::null(), KEY_C, false);
        if !up.is_null() {
            CGEventSetFlags(up, K_CG_EVENT_FLAG_MASK_COMMAND);
            CGEventPost(K_CG_HID_EVENT_TAP, up);
            CFRelease(up);
        }
    }

    /// Reads the macOS general pasteboard as plain UTF-8.
    fn clipboard_text() -> String {
        std::process::Command::new("pbpaste")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default()
    }

    /// Replaces the macOS general pasteboard with the given string.
    fn write_clipboard(text: &str) {
        use std::io::Write as _;
        if let Ok(mut child) = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    }

    /// Clipboard-based fallback for apps that don't expose selection via AX
    /// (e.g. VS Code / Electron apps using Monaco editor).
    ///
    /// Saves the current clipboard, simulates Cmd+C to copy whatever is selected
    /// in the focused application, reads the new clipboard, then restores the
    /// original clipboard contents. Returns the newly copied text, or `None` if
    /// the clipboard didn't change.
    /// Concurrent calls are prevented by the caller-level
    /// `OVERLAY_INTENDED_VISIBLE` atomic guard in `lib.rs`, which ensures only
    /// one activation path is active at a time.
    fn clipboard_fallback() -> Option<String> {
        let before = clipboard_text();
        // SAFETY: Accessibility permission is checked before the activator starts.
        unsafe { simulate_cmd_c() };
        // Poll the pasteboard with exponential backoff instead of a fixed sleep.
        // Fast machines return in ~10ms; slower machines get up to ~150ms total.
        let mut after = before.clone();
        for delay_ms in [10, 20, 40, 80] {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            after = clipboard_text();
            if after != before {
                break;
            }
        }
        // Always restore the original clipboard regardless of outcome.
        if after != before {
            write_clipboard(&before);
        }
        let trimmed = after.trim().to_string();
        if after != before && !trimmed.is_empty() {
            Some(trimmed)
        } else {
            None
        }
    }

    unsafe fn focused_element() -> Option<AXUIElementRef> {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            // system is null; CFRelease must not be called on a null pointer.
            return None;
        }
        let key = CFString::new("AXFocusedUIElement");
        let mut value: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(system, key.as_concrete_TypeRef(), &mut value);
        CFRelease(system as CFTypeRef);
        if err == K_AX_ERROR_SUCCESS && !value.is_null() {
            Some(value as AXUIElementRef)
        } else {
            None
        }
    }

    unsafe fn selected_text(element: AXUIElementRef) -> Option<String> {
        let key = CFString::new("AXSelectedText");
        let mut value: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(element, key.as_concrete_TypeRef(), &mut value);
        if err != K_AX_ERROR_SUCCESS || value.is_null() {
            return None;
        }
        let cf_str = CFString::wrap_under_create_rule(value as CFStringRef);
        let text = cf_str.to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    unsafe fn selection_bounds(element: AXUIElementRef) -> Option<ScreenRect> {
        let range_key = CFString::new("AXSelectedTextRange");
        let mut range_value: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            element,
            range_key.as_concrete_TypeRef(),
            &mut range_value,
        );
        if err != K_AX_ERROR_SUCCESS || range_value.is_null() {
            return None;
        }

        let bounds_key = CFString::new("AXBoundsForRange");
        let mut bounds_value: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyParameterizedAttributeValue(
            element,
            bounds_key.as_concrete_TypeRef(),
            range_value,
            &mut bounds_value,
        );
        CFRelease(range_value);

        if err != K_AX_ERROR_SUCCESS || bounds_value.is_null() {
            return None;
        }

        let mut rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: 0.0,
                height: 0.0,
            },
        };
        let ok = AXValueGetValue(
            bounds_value,
            K_AX_VALUE_TYPE_CG_RECT,
            &mut rect as *mut CGRect as *mut c_void,
        );
        CFRelease(bounds_value);

        if ok && rect.size.width > 0.0 {
            Some(ScreenRect {
                x: rect.origin.x,
                y: rect.origin.y,
                width: rect.size.width,
                height: rect.size.height,
            })
        } else {
            None
        }
    }

    pub fn capture() -> ActivationContext {
        // SAFETY: All AX API calls are wrapped in this function. `element` is released
        // at the end of this function and is not retained by `selected_text` or
        // `selection_bounds` — both helpers use the pointer only within their call duration.
        unsafe {
            let mouse = current_mouse_position();

            let Some(element) = focused_element() else {
                // No focused element — try clipboard fallback before giving up.
                let text = clipboard_fallback();
                return ActivationContext {
                    selected_text: text,
                    bounds: None,
                    mouse_position: Some(mouse),
                };
            };

            let ax_text = selected_text(element);
            let bounds = if ax_text.is_some() {
                selection_bounds(element)
            } else {
                None
            };
            CFRelease(element as CFTypeRef);

            // If AX returned no text (VS Code / Electron apps with Monaco), fall back
            // to clipboard simulation so the user still gets context.
            let text = if ax_text.is_some() {
                ax_text
            } else {
                clipboard_fallback()
            };

            ActivationContext {
                selected_text: text,
                bounds,
                mouse_position: Some(mouse),
            }
        }
    }
}

/// Captures the current activation context at the moment of the hotkey press.
///
/// When `overlay_is_visible` is `true` the hotkey will hide the overlay, so
/// no context is needed — skip AX queries and clipboard simulation entirely.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn capture_activation_context(overlay_is_visible: bool) -> ActivationContext {
    if overlay_is_visible {
        return ActivationContext::empty();
    }

    #[cfg(target_os = "macos")]
    {
        macos::capture()
    }

    #[cfg(not(target_os = "macos"))]
    {
        ActivationContext::empty()
    }
}

// ─── Positioning ──────────────────────────────────────────────────────────────

/// Distance (logical pts) to the right of the anchor point before the bar.
const ANCHOR_OFFSET_X: f64 = 8.0;
/// Distance (logical pts) above the anchor bottom edge for the bar top.
const ANCHOR_OFFSET_Y: f64 = 2.0;
/// Bottom padding of the overlay window in logical pts (pb-6 = 24 pt + motion py-2
/// bottom = 8 pt). Added when positioning the bar **above** a selection so the
/// bar's visible content bottom — not the transparent window edge — aligns with
/// the selection boundary.
const WINDOW_BOTTOM_PADDING: f64 = 32.0;
/// Minimum distance from any screen edge (logical pts).
pub(crate) const SCREEN_MARGIN: f64 = 16.0;
/// macOS menu bar height approximation (logical pts).
pub(crate) const MENU_BAR_HEIGHT: f64 = 24.0;
/// Minimum screen space (logical pts) needed below the initial window bottom
/// for the conversation to expand freely. Derived from the frontend's
/// `max-h-[600px]` CSS constraint plus a small safety margin.
/// When less space is available, the window is pinned to grow upward instead.
const UPWARD_GROWTH_THRESHOLD: f64 = 600.0;

/// Result of the window placement calculation.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowPlacement {
    /// Logical X of the window's top-left corner.
    pub x: f64,
    /// Logical Y of the window's top-left corner.
    pub y: f64,
    /// When `Some`, the bar was flipped **above** the selection because the screen
    /// bottom was too close. The value is the logical Y the window bottom should
    /// stay pinned to as the conversation grows (so the frontend can reposition
    /// upward by computing `y = anchor_bottom_y - current_window_height`).
    pub anchor_bottom_y: Option<f64>,
}

/// Returns the top-center position for the no-selection spawn point.
fn top_center(
    screen_width: f64,
    _screen_height: f64,
    window_width: f64,
    _window_height: f64,
) -> WindowPlacement {
    let x_min = SCREEN_MARGIN;
    let x_max = (screen_width - window_width - SCREEN_MARGIN).max(x_min);
    let x = ((screen_width - window_width) / 2.0).clamp(x_min, x_max);
    let y = MENU_BAR_HEIGHT + SCREEN_MARGIN + 120.0;
    WindowPlacement {
        x,
        y,
        anchor_bottom_y: None,
    }
}

/// Positions the window to the right of `anchor_x / anchor_bottom_y`, flipping
/// horizontally when it would overflow the right screen edge and vertically when
/// it would overflow the bottom screen edge.
///
/// - `anchor_bottom_y`: bottom of the selection or mouse cursor Y.
/// - `anchor_top_y`: top of the selection (equals `anchor_bottom_y` for the
///   mouse-cursor case where there is no extent).
/// - `start_x`: left edge of the selection, used for the horizontal flip.
#[allow(clippy::too_many_arguments)]
fn anchor_near(
    anchor_x: f64,
    anchor_bottom_y: f64,
    anchor_top_y: f64,
    start_x: f64,
    screen_width: f64,
    screen_height: f64,
    window_width: f64,
    window_height: f64,
) -> WindowPlacement {
    // ── Horizontal ──────────────────────────────────────────────────────────
    let preferred_x = anchor_x + ANCHOR_OFFSET_X;
    let x = if preferred_x + window_width <= screen_width - SCREEN_MARGIN {
        preferred_x
    } else {
        // Bar grows leftward: right edge at (start_x - ANCHOR_OFFSET_X).
        (start_x - window_width - ANCHOR_OFFSET_X).max(SCREEN_MARGIN)
    };

    // ── Vertical ────────────────────────────────────────────────────────────
    let y_min = MENU_BAR_HEIGHT + SCREEN_MARGIN;
    let below_y = anchor_bottom_y - ANCHOR_OFFSET_Y;

    if below_y + window_height <= screen_height - SCREEN_MARGIN {
        // Enough room below → normal downward placement.
        WindowPlacement {
            x,
            y: below_y.max(y_min),
            anchor_bottom_y: None,
        }
    } else {
        // Flip above: shift the window bottom down by WINDOW_BOTTOM_PADDING so
        // the bar's visible content bottom (not the transparent window edge) sits
        // ANCHOR_OFFSET_Y pts above anchor_top_y. Clamped to screen_height so
        // the window never extends off the screen's lower edge.
        let fixed_bottom =
            (anchor_top_y - ANCHOR_OFFSET_Y + WINDOW_BOTTOM_PADDING).min(screen_height);
        let y = (fixed_bottom - window_height).max(y_min);
        WindowPlacement {
            x,
            y,
            anchor_bottom_y: Some(fixed_bottom),
        }
    }
}

/// Computes the window top-left position in logical screen coordinates.
///
/// - `screen_width` / `screen_height`: monitor size in logical points
/// - `window_width` / `window_height`: expected window size in logical points
pub fn calculate_window_position(
    ctx: &ActivationContext,
    screen_width: f64,
    screen_height: f64,
    window_width: f64,
    window_height: f64,
) -> WindowPlacement {
    let placement = if let Some(rect) = ctx.bounds {
        // AX provided full bounds → anchor to the end of the selection.
        anchor_near(
            rect.x + rect.width,
            rect.y + rect.height,
            rect.y,
            rect.x,
            screen_width,
            screen_height,
            window_width,
            window_height,
        )
    } else if ctx.selected_text.is_some() {
        // AX returned text but no bounds (Chromium apps) → anchor to mouse cursor.
        if let Some((mx, my)) = ctx.mouse_position {
            anchor_near(
                mx,
                my,
                my,
                mx,
                screen_width,
                screen_height,
                window_width,
                window_height,
            )
        } else {
            top_center(screen_width, screen_height, window_width, window_height)
        }
    } else {
        // No selection → top center of screen.
        top_center(screen_width, screen_height, window_width, window_height)
    };

    // Secondary check: if the flip logic above did not set an anchor, determine
    // whether there is enough room below for the conversation to expand fully.
    // If not, pin the window bottom so the conversation can grow upward instead
    // of being clipped by the screen edge.
    if placement.anchor_bottom_y.is_none() {
        let initial_bottom = placement.y + window_height;
        let space_below = screen_height - SCREEN_MARGIN - initial_bottom;
        if space_below < UPWARD_GROWTH_THRESHOLD {
            return WindowPlacement {
                anchor_bottom_y: Some(initial_bottom),
                ..placement
            };
        }
    }
    placement
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_bounds(x: f64, y: f64, w: f64, h: f64) -> ActivationContext {
        ActivationContext {
            selected_text: Some("hello".to_string()),
            bounds: Some(ScreenRect {
                x,
                y,
                width: w,
                height: h,
            }),
            mouse_position: None,
        }
    }

    fn ctx_no_selection() -> ActivationContext {
        ActivationContext {
            selected_text: None,
            bounds: None,
            mouse_position: None,
        }
    }

    fn ctx_text_no_bounds_with_mouse(mx: f64, my: f64) -> ActivationContext {
        ActivationContext {
            selected_text: Some("hello".to_string()),
            bounds: None,
            mouse_position: Some((mx, my)),
        }
    }

    const SW: f64 = 1440.0;
    const SH: f64 = 900.0;
    const WW: f64 = 600.0;
    const WH: f64 = 80.0;

    #[test]
    fn no_selection_returns_top_center() {
        let p = calculate_window_position(&ctx_no_selection(), SW, SH, WW, WH);
        assert_eq!(p.x, (SW - WW) / 2.0);
        assert_eq!(p.y, MENU_BAR_HEIGHT + SCREEN_MARGIN);
        assert_eq!(p.anchor_bottom_y, None);
    }

    #[test]
    fn text_with_no_bounds_and_no_mouse_falls_back_to_top_center() {
        // Same top-center position — no anchor needed since the bar grows downward.
        let ctx = ActivationContext {
            selected_text: Some("hello world".to_string()),
            bounds: None,
            mouse_position: None,
        };
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        let x_min = SCREEN_MARGIN;
        let x_max = (SW - WW - SCREEN_MARGIN).max(x_min);
        assert_eq!(p.x, ((SW - WW) / 2.0).clamp(x_min, x_max));
        assert_eq!(p.y, MENU_BAR_HEIGHT + SCREEN_MARGIN);
        assert_eq!(p.anchor_bottom_y, None);
    }

    #[test]
    fn text_with_no_bounds_uses_mouse_as_anchor() {
        // Mouse at (400, 300). placement.y ≈ 298. space_below = 900-16-378 = 506 < 600 → anchor.
        let ctx = ctx_text_no_bounds_with_mouse(400.0, 300.0);
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        assert_eq!(p.x, 400.0 + ANCHOR_OFFSET_X);
        let expected_y = 300.0 - ANCHOR_OFFSET_Y;
        assert!((p.y - expected_y).abs() < 0.01);
        assert_eq!(p.anchor_bottom_y, Some(expected_y + WH));
    }

    #[test]
    fn selection_with_room_anchors_to_end() {
        // Selection at x=100, y=300, w=80, h=20 → end at (180, 320).
        // placement.y ≈ 318. space_below = 900-16-398 = 486 < 600 → anchor pinned.
        let ctx = ctx_with_bounds(100.0, 300.0, 80.0, 20.0);
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        assert_eq!(p.x, 180.0 + ANCHOR_OFFSET_X);
        let expected_y = 320.0 - ANCHOR_OFFSET_Y;
        assert!((p.y - expected_y).abs() < 0.01);
        assert_eq!(p.anchor_bottom_y, Some(expected_y + WH));
    }

    #[test]
    fn no_anchor_when_plenty_of_room_below() {
        // Selection near top of screen: placement.y ≈ 18. space_below = 900-16-98 = 786 > 600.
        let ctx = ctx_with_bounds(100.0, 0.0, 80.0, 20.0);
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        // below_y = 20-2 = 18, clamped to y_min = 40.
        assert_eq!(p.y, MENU_BAR_HEIGHT + SCREEN_MARGIN);
        assert_eq!(p.anchor_bottom_y, None);
    }

    #[test]
    fn selection_near_right_edge_flips_to_start() {
        // Selection end at 980. Window (600) would reach 1588 → overflows 1440-16=1424.
        let ctx = ctx_with_bounds(900.0, 300.0, 80.0, 20.0);
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        assert_eq!(p.x, 900.0 - WW - ANCHOR_OFFSET_X);
    }

    #[test]
    fn flipped_x_is_clamped_by_screen_margin() {
        // Selection starts at x=10, end at x=1430 (near right edge).
        // preferred_x = 1430 + 8 = 1438. 1438 + 600 = 2038 > 1440 - 16 = 1424 → flip.
        // flipped_x = (10.0 - 600.0 - 8.0).max(16.0) = (-598.0).max(16.0) = 16.0
        let ctx = ctx_with_bounds(10.0, 300.0, 1420.0, 20.0);
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        assert_eq!(p.x, SCREEN_MARGIN);
    }

    #[test]
    fn y_flips_above_when_selection_near_screen_bottom() {
        // Selection: y=870, h=20 → bottom=890.
        // below_y = 888. 888+80=968 > 900-16=884 → flip above.
        // fixed_bottom = min(870 - 2 + 32, 900) = min(900, 900) = 900.
        // y = (900-80).max(40) = 820.
        // Visible content bottom = 900 - WINDOW_BOTTOM_PADDING(32) = 868 → 2px above sel top(870).
        let ctx = ctx_with_bounds(100.0, 870.0, 80.0, 20.0);
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        assert_eq!(p.y, 820.0);
        assert_eq!(p.anchor_bottom_y, Some(900.0));
    }

    #[test]
    fn y_is_clamped_when_near_menu_bar() {
        // Selection bottom at 30 → below_y = 28. 28+80=108 < 884 → no flip.
        // below_y.max(y_min) = 28.max(40) = 40.
        let ctx = ctx_with_bounds(100.0, 10.0, 80.0, 20.0);
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        assert_eq!(p.y, MENU_BAR_HEIGHT + SCREEN_MARGIN);
        assert_eq!(p.anchor_bottom_y, None);
    }

    #[test]
    fn zero_sized_selection_rect() {
        let ctx = ctx_with_bounds(200.0, 400.0, 0.0, 0.0);
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        assert_eq!(p.x, 200.0 + ANCHOR_OFFSET_X);
    }

    #[test]
    fn selection_spanning_full_screen_width() {
        let ctx = ctx_with_bounds(0.0, 300.0, SW, 20.0);
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        assert_eq!(p.x, SCREEN_MARGIN);
    }

    #[test]
    fn very_tall_screen_no_anchor_bottom() {
        let ctx = ctx_with_bounds(100.0, 100.0, 80.0, 20.0);
        let tall_screen = 2000.0;
        let p = calculate_window_position(&ctx, SW, tall_screen, WW, WH);
        assert_eq!(p.anchor_bottom_y, None);
    }

    #[test]
    fn mouse_near_screen_edge_flips() {
        let ctx = ctx_text_no_bounds_with_mouse(1430.0, 300.0);
        let p = calculate_window_position(&ctx, SW, SH, WW, WH);
        assert_eq!(p.x, (1430.0 - WW - ANCHOR_OFFSET_X).max(SCREEN_MARGIN));
    }

    #[test]
    fn capture_activation_context_returns_empty_when_visible() {
        let ctx = capture_activation_context(true);
        assert!(ctx.selected_text.is_none());
        assert!(ctx.bounds.is_none());
        assert!(ctx.mouse_position.is_none());
    }

    #[test]
    fn activation_context_empty_has_no_fields() {
        let ctx = ActivationContext::empty();
        assert!(ctx.selected_text.is_none());
        assert!(ctx.bounds.is_none());
        assert!(ctx.mouse_position.is_none());
    }

    #[test]
    fn top_center_on_small_screen() {
        let small_w = WW + 2.0 * SCREEN_MARGIN;
        let p = calculate_window_position(&ctx_no_selection(), small_w, SH, WW, WH);
        assert_eq!(p.x, SCREEN_MARGIN);
    }
}
