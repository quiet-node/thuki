/*!
 * Thuki Core Library
 *
 * Application bootstrap for the Thuki desktop agent. Configures the macOS
 * status bar presence, system tray menu, double-tap Option hotkey, and
 * window lifecycle (hide-on-close instead of quit).
 *
 * On macOS the main window is converted to an NSPanel via `tauri-nspanel`.
 * This allows the overlay to appear on top of native fullscreen applications
 * — something a standard NSWindow cannot do regardless of window level.
 *
 * The overlay is toggled via a system-level activation trigger (macOS only),
 * managed by the `activator` module.
 */

pub mod commands;

#[cfg(target_os = "macos")]
mod activator;
pub mod context;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, RunEvent, WebviewWindow,
};

#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;

#[cfg(target_os = "macos")]
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt,
};

// ─── NSPanel definition (macOS only) ────────────────────────────────────────

// ThukiPanel — custom NSPanel subclass for the overlay.
// `can_become_key_window: true` allows keyboard input for the chat.
// `is_floating_panel: true` keeps the panel above normal windows.
#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(ThukiPanel {
        config: {
            can_become_key_window: true,
            is_floating_panel: true
        }
    })
}

// ─── Window helpers ─────────────────────────────────────────────────────────

/// Expected logical width of the overlay window for spawn-position calculations.
const OVERLAY_LOGICAL_WIDTH: f64 = 600.0;
/// Collapsed bar height used for Y-clamp at show time. The window starts collapsed;
/// the ResizeObserver expands it after mount.
const OVERLAY_LOGICAL_HEIGHT_COLLAPSED: f64 = 80.0;

/// Frontend event used to synchronize show/hide animations with native window visibility.
const OVERLAY_VISIBILITY_EVENT: &str = "thuki://visibility";
const OVERLAY_VISIBILITY_SHOW: &str = "show";
const OVERLAY_VISIBILITY_HIDE_REQUEST: &str = "hide-request";

/// Tracks the intended visibility state of the overlay, preventing race conditions
/// between the frontend exit animation and rapid activation toggles.
static OVERLAY_INTENDED_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Fixed-bottom anchor emitted when the bar is positioned above the selection.
/// The frontend pins the window bottom to `bottom_y` as the conversation grows.
#[derive(Clone, serde::Serialize)]
struct WindowAnchor {
    /// Logical X of the window top-left (preserved during height changes).
    x: f64,
    /// Logical Y the window bottom must stay pinned to.
    bottom_y: f64,
    /// Minimum Y the window top may reach (monitor top + menu-bar clearance).
    /// On above-monitors this is negative, preventing the frontend's clamp
    /// from yanking the window back onto the primary display.
    min_y: f64,
}

/// Payload emitted to the frontend on every visibility transition.
#[derive(Clone, serde::Serialize)]
struct VisibilityPayload {
    /// "show" or "hide-request"
    state: &'static str,
    /// Selected text captured at activation time, if any.
    selected_text: Option<String>,
    /// Present when the window was flipped above the selection. The frontend
    /// uses this to keep the window bottom anchored as the chat grows.
    window_anchor: Option<WindowAnchor>,
}

/// Emits a visibility transition to the frontend animation controller.
fn emit_overlay_visibility(
    app_handle: &tauri::AppHandle,
    state: &'static str,
    selected_text: Option<String>,
    window_anchor: Option<WindowAnchor>,
) {
    let _ = app_handle.emit(
        OVERLAY_VISIBILITY_EVENT,
        VisibilityPayload {
            state,
            selected_text,
            window_anchor,
        },
    );
}

/// CoreGraphics display lookup — uses macOS-native `CGGetDisplaysWithPoint`
/// for hit-testing instead of manual iteration + containment checks.
/// All coordinates are in the Quartz display coordinate space (top-left of
/// primary display, Y-down), matching the AX API and `CGEventGetLocation`.
#[cfg(target_os = "macos")]
mod cg_displays {
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

    fn rect_to_tuple(r: CGRect) -> (f64, f64, f64, f64) {
        (r.origin.x, r.origin.y, r.size.width, r.size.height)
    }

    /// Returns `(origin_x, origin_y, width, height)` in Quartz points for
    /// the display containing `(global_x, global_y)`.
    pub fn display_for_point(global_x: f64, global_y: f64) -> Option<(f64, f64, f64, f64)> {
        unsafe {
            let point = CGPoint::new(global_x, global_y);
            let mut ids = [0u32; 4];
            let mut count: u32 = 0;
            let err = CGGetDisplaysWithPoint(point, 4, ids.as_mut_ptr(), &mut count);
            if err != 0 || count == 0 {
                return None;
            }
            Some(rect_to_tuple(CGDisplayBounds(ids[0])))
        }
    }

    /// Returns `(origin_x, origin_y, width, height)` of the main (menu-bar) display.
    pub fn main_display() -> (f64, f64, f64, f64) {
        unsafe { rect_to_tuple(CGDisplayBounds(CGMainDisplayID())) }
    }
}

/// Minimum Y offset from the top of any monitor — menu bar plus edge margin.
/// Must match `MENU_BAR_HEIGHT + SCREEN_MARGIN` in `context.rs`.
const MONITOR_TOP_CLEARANCE: f64 = 40.0;

/// Returns the Quartz-coordinate bounds of the display containing
/// `(global_x, global_y)`, falling back to the main display.
#[cfg(target_os = "macos")]
fn find_target_monitor(global_x: f64, global_y: f64) -> (f64, f64, f64, f64) {
    cg_displays::display_for_point(global_x, global_y).unwrap_or_else(cg_displays::main_display)
}

/// Returns Quartz-coordinate bounds of the main display as a fallback
/// when no anchor point is available.
#[cfg(target_os = "macos")]
fn monitor_info_fallback() -> (f64, f64, f64, f64) {
    cg_displays::main_display()
}

/// Shows the overlay and requests the frontend to replay its entrance animation.
///
/// Uses `show_and_make_key()` to guarantee the NSPanel becomes the key window,
/// which is required for the WebView input to receive keyboard focus reliably.
///
/// AX bounds and mouse position arrive in **global** screen coordinates that span
/// all monitors. We find which monitor the activation happened on, convert to
/// monitor-local coordinates for the positioning math, then convert the result
/// back to global coordinates for `set_position`.
#[cfg(target_os = "macos")]
fn show_overlay(app_handle: &tauri::AppHandle, ctx: crate::context::ActivationContext) {
    if OVERLAY_INTENDED_VISIBLE.swap(true, Ordering::SeqCst) {
        return;
    }

    // Extract before building local_ctx to avoid an extra clone.
    let selected_text = ctx.selected_text;

    // Position the window before making it visible.
    let placement = if let Some(window) = app_handle.get_webview_window("main") {
        // Pick an anchor point to identify the target monitor.
        let anchor_point = ctx
            .bounds
            .map(|r| (r.x + r.width / 2.0, r.y + r.height / 2.0))
            .or(ctx.mouse_position);

        let (mon_x, mon_y, screen_w, screen_h) = if let Some((ax, ay)) = anchor_point {
            find_target_monitor(ax, ay)
        } else {
            monitor_info_fallback()
        };

        // Convert global coordinates to monitor-local for the positioning math.
        let local_ctx = crate::context::ActivationContext {
            selected_text: selected_text.clone(),
            bounds: ctx.bounds.map(|r| crate::context::ScreenRect {
                x: r.x - mon_x,
                y: r.y - mon_y,
                width: r.width,
                height: r.height,
            }),
            mouse_position: ctx.mouse_position.map(|(mx, my)| (mx - mon_x, my - mon_y)),
        };

        let p = crate::context::calculate_window_position(
            &local_ctx,
            screen_w,
            screen_h,
            OVERLAY_LOGICAL_WIDTH,
            OVERLAY_LOGICAL_HEIGHT_COLLAPSED,
        );

        // Convert back to global screen coordinates.
        let global = crate::context::WindowPlacement {
            x: p.x + mon_x,
            y: p.y + mon_y,
            anchor_bottom_y: p.anchor_bottom_y.map(|y| y + mon_y),
        };

        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            global.x, global.y,
        )));
        // Menu-bar clearance in global coordinates for this monitor.
        let global_min_y = mon_y + MONITOR_TOP_CLEARANCE;
        Some((global, global_min_y))
    } else {
        None
    };

    let window_anchor = placement.and_then(|(p, min_y)| {
        p.anchor_bottom_y.map(|bottom_y| WindowAnchor {
            x: p.x,
            bottom_y,
            min_y,
        })
    });

    match app_handle.get_webview_panel("main") {
        Ok(panel) => {
            panel.show_and_make_key();
            emit_overlay_visibility(
                app_handle,
                OVERLAY_VISIBILITY_SHOW,
                selected_text,
                window_anchor,
            );
        }
        Err(_) => {
            // Reset the flag so future activation attempts are not permanently blocked.
            OVERLAY_INTENDED_VISIBLE.store(false, Ordering::SeqCst);
        }
    }
}

/// Requests an animated hide sequence from the frontend. The actual native
/// window hide is deferred until the frontend exit animation completes.
fn request_overlay_hide(app_handle: &tauri::AppHandle) {
    if OVERLAY_INTENDED_VISIBLE.swap(false, Ordering::SeqCst) {
        emit_overlay_visibility(app_handle, OVERLAY_VISIBILITY_HIDE_REQUEST, None, None);
    }
}

/// Shows the overlay and requests the frontend to replay its entrance animation.
///
/// Window positioning is intentionally deferred on non-macOS platforms — the
/// activation context is forwarded to the frontend for selected-text display,
/// but no positioning logic is applied until platform-specific activators
/// (e.g. Windows global hotkey) are implemented.
#[cfg(not(target_os = "macos"))]
fn show_overlay(app_handle: &tauri::AppHandle, ctx: crate::context::ActivationContext) {
    if OVERLAY_INTENDED_VISIBLE.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        emit_overlay_visibility(app_handle, OVERLAY_VISIBILITY_SHOW, ctx.selected_text, None);
    }
}

/// Toggles the overlay between visible and hidden states.
///
/// Uses an atomic flag as the single source of truth for intended visibility,
/// which avoids race conditions with the native panel state during animations.
fn toggle_overlay(app_handle: &tauri::AppHandle, ctx: crate::context::ActivationContext) {
    if OVERLAY_INTENDED_VISIBLE.load(Ordering::SeqCst) {
        request_overlay_hide(app_handle);
    } else {
        show_overlay(app_handle, ctx);
    }
}

/// Repositions and resizes the main window atomically.
///
/// Regular Tauri commands run on a Tokio thread pool. Calling `set_position`
/// then `set_size` from a pool thread dispatches each as a *separate* event to
/// the macOS main thread, which can render as two distinct display frames and
/// produce a visible stutter when the window grows upward (position + size both
/// change on every token during streaming).
///
/// Wrapping both calls in a single `run_on_main_thread` closure ensures they
/// arrive on the main thread together in the same event-loop iteration. AppKit
/// then coalesces the geometry change into one compositor frame.
#[tauri::command]
fn set_window_frame(app_handle: tauri::AppHandle, x: f64, y: f64, width: f64, height: f64) {
    // Reject non-finite values (NaN, Infinity) from the frontend to prevent
    // undefined AppKit behaviour when forwarded to native window APIs.
    if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
        return;
    }
    let width = width.clamp(1.0, 10_000.0);
    let height = height.clamp(1.0, 10_000.0);

    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("main") {
            let _ =
                window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
            let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, height)));
        }
    });
}

/// Synchronizes the Rust-side visibility tracking when the frontend
/// completes its exit animation and hides the native window.
#[tauri::command]
fn notify_overlay_hidden() {
    OVERLAY_INTENDED_VISIBLE.store(false, Ordering::SeqCst);
}

// ─── NSPanel initialisation ─────────────────────────────────────────────────

/// Converts the main Tauri window into an NSPanel and applies the overlay
/// configuration required to appear over fullscreen macOS applications.
///
/// The four critical settings are:
/// - `PanelLevel::Floating` — floats above normal windows
/// - `CollectionBehavior::full_screen_auxiliary()` — allows coexistence with
///   fullscreen Spaces (this is what standard `alwaysOnTop` cannot do)
/// - `StyleMask::nonactivating_panel()` — prevents the panel from stealing
///   focus/activation from the fullscreen application
/// - `set_has_shadow(false)` — disables the native compositor shadow, which
///   renders differently for key vs. non-key windows, causing a visible change
///   when the user clicks elsewhere. CSS `shadow-bar` provides a consistent
///   elevation effect independent of key-window state.
#[cfg(target_os = "macos")]
fn init_panel(app_handle: &tauri::AppHandle) {
    let window: WebviewWindow = app_handle
        .get_webview_window("main")
        .expect("main window must exist at setup time");

    let panel = window
        .to_panel::<ThukiPanel>()
        .expect("NSPanel conversion must succeed on macOS");

    panel.set_level(PanelLevel::Floating.value());

    panel.set_style_mask(StyleMask::empty().nonactivating_panel().into());

    panel.set_collection_behavior(
        CollectionBehavior::new()
            .full_screen_auxiliary()
            .can_join_all_spaces()
            .into(),
    );

    // Keep the panel visible when the user clicks back into the fullscreen app.
    panel.set_hides_on_deactivate(false);

    // Disable the native compositor shadow. macOS renders visually distinct
    // shadows for key vs. non-key windows, which causes the overlay to appear
    // different after the user clicks elsewhere. The CSS `shadow-bar` provides
    // a stable, focus-independent elevation effect.
    panel.set_has_shadow(false);
}

// ─── Application entry point ─────────────────────────────────────────────────

/// Initialises and runs the Tauri application.
///
/// Setup order:
/// 1. `ActivationPolicy::Accessory` suppresses the Dock icon.
/// 2. The main window is converted to an NSPanel for fullscreen overlay.
/// 3. System tray is registered; double-tap Option listener starts.
/// 4. `CloseRequested` is intercepted to hide instead of destroy.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to initialise.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    builder
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(ActivationPolicy::Accessory);

            // ── NSPanel conversion (macOS only) ──────────────────────────
            #[cfg(target_os = "macos")]
            init_panel(app.app_handle());

            // ── System tray icon + menu ───────────────────────────────────
            let show_item = MenuItem::with_id(app, "show", "Open Thuki", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png"))
                .expect("Failed to load tray icon");

            let _tray = TrayIconBuilder::new()
                .icon(tray_icon)
                .icon_as_template(false)
                .tooltip("Thuki")
                .menu(&tray_menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        show_overlay(app, crate::context::ActivationContext::empty());
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Right,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_overlay(
                            tray.app_handle(),
                            crate::context::ActivationContext::empty(),
                        );
                    }
                })
                .build(app)?;

            // ── Activation listener (macOS only) ─────────────────────────
            #[cfg(target_os = "macos")]
            {
                let app_handle = app.handle().clone();
                let activator = activator::OverlayActivator::new();
                activator.start(move || {
                    // Skip AX + clipboard when hiding — no context needed and
                    // simulating Cmd+C against Thuki's own WebView would produce
                    // a macOS alert sound.
                    let is_visible = OVERLAY_INTENDED_VISIBLE.load(Ordering::SeqCst);
                    let ctx = crate::context::capture_activation_context(is_visible);
                    let handle = app_handle.clone();
                    let _ = app_handle.run_on_main_thread(move || toggle_overlay(&handle, ctx));
                });
                app.manage(activator);
            }

            // ── Persistent HTTP client ────────────────────────────────
            app.manage(reqwest::Client::new());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ask_ollama,
            notify_overlay_hidden,
            set_window_frame
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } = event
            {
                if label == "main" {
                    api.prevent_close();

                    request_overlay_hide(app_handle);
                }
            }
        });
}
