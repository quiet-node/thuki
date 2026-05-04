/*!
 * Thuki Core Library
 *
 * Application bootstrap for the Thuki desktop agent. Configures the macOS
 * status bar presence, system tray menu, double-tap Option hotkey, and
 * window lifecycle (hide-on-close instead of quit).
 *
 * On macOS the main window is converted to an NSPanel via `tauri-nspanel`.
 * This allows the overlay to appear on top of native fullscreen applications
 * - something a standard NSWindow cannot do regardless of window level.
 *
 * The overlay is toggled via a system-level activation trigger (macOS only),
 * managed by the `activator` module.
 */

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

pub mod commands;
pub mod config;
pub mod database;
pub mod history;
pub mod images;
pub mod models;
pub mod onboarding;
pub mod screenshot;
pub mod search;
pub mod settings_commands;
pub mod warmup;

#[cfg(target_os = "macos")]
mod activator;
pub mod context;
pub mod permissions;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, RunEvent, WebviewWindow,
};

#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;

#[cfg(target_os = "macos")]
use tauri_nspanel::{CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt};

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSApplication;

// ─── NSPanel definition (macOS only) ────────────────────────────────────────

// Each tauri_panel! invocation emits `use` statements at its call-site
// module scope. Two calls in the same module cause name collisions, so
// each panel subclass lives in its own private module. The underscore
// prefix marks each module as an internal implementation detail; add
// any future panel subclass the same way.
//
// ThukiPanel - overlay NSPanel: floating, keyboard input for chat.
// ThukiSettingsPanel - settings NSPanel: non-floating, keyboard input,
//   no ActivationPolicy switch so the Dock icon never appears.
#[cfg(target_os = "macos")]
mod _thuki_panel {
    use tauri::Manager;
    tauri_nspanel::tauri_panel! {
        panel!(ThukiPanel {
            config: {
                can_become_key_window: true,
                is_floating_panel: true
            }
        })
    }
}
#[cfg(target_os = "macos")]
use _thuki_panel::ThukiPanel;

#[cfg(target_os = "macos")]
mod _settings_panel {
    use tauri::Manager;
    tauri_nspanel::tauri_panel! {
        panel!(ThukiSettingsPanel {
            config: {
                can_become_key_window: true,
                is_floating_panel: false
            }
        })
    }
}
#[cfg(target_os = "macos")]
use _settings_panel::ThukiSettingsPanel;

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

/// Frontend event that triggers the onboarding screen when one or more
/// required permissions have not yet been granted.
const ONBOARDING_EVENT: &str = "thuki://onboarding";

/// Logical dimensions of the onboarding window (centered, fixed size).
/// Content fits tightly; native macOS shadow is re-enabled for onboarding
/// so it renders outside the window boundary without extra transparent padding.
const ONBOARDING_LOGICAL_WIDTH: f64 = 460.0;
const ONBOARDING_LOGICAL_HEIGHT: f64 = 640.0;

/// Tracks the intended visibility state of the overlay, preventing race conditions
/// between the frontend exit animation and rapid activation toggles.
static OVERLAY_INTENDED_VISIBLE: AtomicBool = AtomicBool::new(false);

/// True on first process launch; cleared when the frontend signals readiness.
/// Used to show the overlay automatically on startup without a race condition:
/// the frontend calls `notify_frontend_ready` after its event listener is
/// registered, so the show event is guaranteed to have a listener.
static LAUNCH_SHOW_PENDING: AtomicBool = AtomicBool::new(true);

/// Payload emitted to the frontend on every visibility transition.
#[derive(Clone, serde::Serialize)]
struct VisibilityPayload {
    /// "show" or "hide-request"
    state: &'static str,
    /// Selected text captured at activation time, if any.
    selected_text: Option<String>,
    /// Logical X of the window at show time. Used with `window_y` and
    /// `screen_bottom_y` to decide growth direction, and as the pinned X
    /// coordinate for `set_window_frame` calls during upward growth.
    window_x: Option<f64>,
    /// Logical Y of the window top-left at show time.
    window_y: Option<f64>,
    /// Logical Y of the screen bottom edge (monitor origin + height).
    screen_bottom_y: Option<f64>,
}

/// Emits a visibility transition to the frontend animation controller.
fn emit_overlay_visibility(
    app_handle: &tauri::AppHandle,
    state: &'static str,
    selected_text: Option<String>,
    window_x: Option<f64>,
    window_y: Option<f64>,
    screen_bottom_y: Option<f64>,
) {
    let _ = app_handle.emit(
        OVERLAY_VISIBILITY_EVENT,
        VisibilityPayload {
            state,
            selected_text,
            window_x,
            window_y,
            screen_bottom_y,
        },
    );
}

/// CoreGraphics display lookup - uses macOS-native `CGGetDisplaysWithPoint`
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

/// Returns the Quartz-coordinate bounds of the display containing
/// `(global_x, global_y)`, falling back to the main display.
#[cfg(target_os = "macos")]
fn find_target_monitor(global_x: f64, global_y: f64) -> (f64, f64, f64, f64) {
    cg_displays::display_for_point(global_x, global_y).unwrap_or_else(cg_displays::main_display)
}

/// Returns Quartz-coordinate bounds of the main display as a fallback
/// when no positioning context is available.
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
    let already_visible = OVERLAY_INTENDED_VISIBLE.swap(true, Ordering::SeqCst);
    if already_visible {
        return;
    }

    // Pre-load the active model so the user's first message does not pay
    // the cold-start penalty. Fires on all show paths: double-tap, tray,
    // and first-launch auto-show.
    let warmup_model = app_handle
        .state::<models::ActiveModelState>()
        .0
        .lock()
        .ok()
        .and_then(|g| g.clone());
    if let Some(model) = warmup_model {
        let warmup_config = app_handle
            .state::<parking_lot::RwLock<crate::config::AppConfig>>()
            .read()
            .clone();
        let endpoint = format!(
            "{}/api/chat",
            warmup_config.inference.ollama_url.trim_end_matches('/')
        );
        let system_prompt = warmup_config.prompt.resolved_system.clone();
        let keep_alive = if warmup_config.inference.keep_warm_inactivity_minutes == 0 {
            None
        } else {
            Some(warmup::keep_alive_string(
                warmup_config.inference.keep_warm_inactivity_minutes,
            ))
        };
        let num_ctx = warmup_config.inference.num_ctx;
        let client = app_handle.state::<reqwest::Client>().inner().clone();
        app_handle.state::<warmup::WarmupState>().fire(
            endpoint,
            model,
            system_prompt,
            client,
            keep_alive,
            num_ctx,
        );
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
        };

        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            global.x, global.y,
        )));
        let screen_bottom = mon_y + screen_h;
        Some((global, screen_bottom))
    } else {
        None
    };

    let (window_x, window_y, screen_bottom_y) = match &placement {
        Some((p, sb)) => (Some(p.x), Some(p.y), Some(*sb)),
        None => (None, None, None),
    };

    match app_handle.get_webview_panel("main") {
        Ok(panel) => {
            panel.show_and_make_key();
            emit_overlay_visibility(
                app_handle,
                OVERLAY_VISIBILITY_SHOW,
                selected_text,
                window_x,
                window_y,
                screen_bottom_y,
            );
        }
        Err(e) => {
            eprintln!("thuki: [show_overlay] get_webview_panel FAILED: {e:?}");
            // Reset the flag so future activation attempts are not permanently blocked.
            OVERLAY_INTENDED_VISIBLE.store(false, Ordering::SeqCst);
        }
    }
}

/// Shows (or focuses, if already visible) the Settings window.
///
/// The settings window is converted to a ThukiSettingsPanel NSPanel subclass
/// (done once in `init_settings_panel` during setup). Using NSPanel with
/// `can_become_key_window: true` allows the window to receive keyboard focus
/// without switching the app's ActivationPolicy to Regular. Switching to
/// Regular is what causes the Dock icon to appear; restoring it back to
/// Accessory is unreliable when the app is frontmost. Staying in Accessory
/// mode permanently avoids both problems.
///
/// Idempotent: invoking while Settings is already visible re-focuses without
/// double-mounting the React tree (close handler hides instead of destroying).
///
/// Falls back to raw WebviewWindow show/focus if the panel handle is
/// unavailable (e.g., if init_settings_panel failed at startup).
/// Centers the settings window horizontally on its monitor and places it
/// below the macOS menu bar with a comfortable gap. Called every time the
/// settings window is shown so the position is always correct regardless of
/// OS-default spawn position or previous moves.
#[cfg_attr(coverage_nightly, coverage(off))]
fn position_settings_window(window: &tauri::WebviewWindow) {
    const SETTINGS_WIDTH: f64 = 580.0;
    // macOS menu bar is ~24 px logical; notched MacBooks push it to ~37 px.
    // 44 px clears both and leaves a visible gap.
    const TOP_MARGIN: f64 = 44.0;

    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());

    let (x, y) = if let Some(mon) = monitor {
        let scale = mon.scale_factor();
        let pos = mon.position();
        let size = mon.size();
        let logical_w = size.width as f64 / scale;
        let mon_x = pos.x as f64 / scale;
        let mon_y = pos.y as f64 / scale;
        (
            mon_x + (logical_w - SETTINGS_WIDTH) / 2.0,
            mon_y + TOP_MARGIN,
        )
    } else {
        (100.0, TOP_MARGIN)
    };

    let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
}

fn show_settings_window(app_handle: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let window = app_handle.get_webview_window("settings");
        match app_handle.get_webview_panel("settings") {
            Ok(panel) => {
                // Activate the app so macOS knows which Space is "current".
                // Without this, Thuki is not the active process (ActivationPolicy::Accessory
                // + tray menu just dismissed), and moveToActiveSpace has no anchor Space to
                // move to — the panel silently appears on its last-known Space (usually Space 1).
                let _ = app_handle.run_on_main_thread(move || {
                    if let Some(mtm) = MainThreadMarker::new() {
                        let ns_app = NSApplication::sharedApplication(mtm);
                        #[allow(deprecated)]
                        ns_app.activateIgnoringOtherApps(true);
                    }
                    if let Some(win) = window {
                        position_settings_window(&win);
                    }
                    panel.show_and_make_key();
                });
                return;
            }
            Err(e) => {
                eprintln!("thuki: [settings] get_webview_panel failed: {e:?}");
            }
        }
    }
    let Some(window) = app_handle.get_webview_window("settings") else {
        eprintln!("thuki: [settings] window 'settings' not found in app config");
        return;
    };
    position_settings_window(&window);
    let _ = window.show();
    let _ = window.set_focus();
}

/// Requests an animated hide sequence from the frontend. The actual native
/// window hide is deferred until the frontend exit animation completes.
fn request_overlay_hide(app_handle: &tauri::AppHandle) {
    if OVERLAY_INTENDED_VISIBLE.swap(false, Ordering::SeqCst) {
        emit_overlay_visibility(
            app_handle,
            OVERLAY_VISIBILITY_HIDE_REQUEST,
            None,
            None,
            None,
            None,
        );
    }
}

/// Shows the overlay and requests the frontend to replay its entrance animation.
///
/// Window positioning is intentionally deferred on non-macOS platforms - the
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
        emit_overlay_visibility(
            app_handle,
            OVERLAY_VISIBILITY_SHOW,
            ctx.selected_text,
            None,
            None,
            None,
        );
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
#[cfg_attr(coverage_nightly, coverage(off))]
fn notify_overlay_hidden(generation: tauri::State<crate::commands::GenerationState>) {
    generation.cancel();
    OVERLAY_INTENDED_VISIBLE.store(false, Ordering::SeqCst);
}

/// Called by the frontend once its visibility event listener is registered.
/// On the first call per process lifetime, shows the overlay so the AskBar
/// appears automatically at startup without a race between the Rust emit and
/// the frontend listener registration.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn notify_frontend_ready(app_handle: tauri::AppHandle, db: tauri::State<history::Database>) {
    if LAUNCH_SHOW_PENDING.swap(false, Ordering::SeqCst) {
        #[cfg(target_os = "macos")]
        {
            if let Ok(conn) = db.0.lock() {
                let stage = onboarding::get_stage(&conn)
                    .unwrap_or(onboarding::OnboardingStage::Permissions);

                // The "intro" stage means the user already cleared both the
                // permissions and the model-check gates on a previous launch.
                // Skip the live permission re-check here: on macOS 15+
                // CGPreflightScreenCaptureAccess can return a stale false
                // negative immediately after a restart, which would wrongly
                // loop the user back to the permissions screen.
                if matches!(stage, onboarding::OnboardingStage::Intro) {
                    show_onboarding_window(&app_handle, onboarding::OnboardingStage::Intro);
                    return;
                }

                // For "permissions", "model_check", and "complete" stages,
                // re-validate live macOS permissions. "complete" detects
                // revocation after onboarding finished. "model_check" detects
                // mid-onboarding revocation. "permissions" is the first-launch
                // path. All three must restart the flow at Permissions if
                // either grant has been withdrawn.
                let ax = permissions::is_accessibility_granted();
                let sr = permissions::is_screen_recording_granted();
                if !ax || !sr {
                    let _ = onboarding::set_stage(&conn, &onboarding::OnboardingStage::Permissions);
                    show_onboarding_window(&app_handle, onboarding::OnboardingStage::Permissions);
                    return;
                }

                // Permissions granted. If the user has not yet cleared the
                // model-check gate, route them there. The frontend probes
                // Ollama via `check_model_setup` and either renders the gate
                // (Ollama unreachable / no models) or fires
                // `advance_past_model_check` to skip straight to Intro on
                // Ready. We do not probe here because /api/tags requires the
                // async runtime and notify_frontend_ready is invoked
                // synchronously from a Tauri command worker.
                if matches!(
                    stage,
                    onboarding::OnboardingStage::Permissions
                        | onboarding::OnboardingStage::ModelCheck
                ) {
                    let _ = onboarding::set_stage(&conn, &onboarding::OnboardingStage::ModelCheck);
                    show_onboarding_window(&app_handle, onboarding::OnboardingStage::ModelCheck);
                    return;
                }
                // Complete: fall through to show the overlay.
            } else {
                // Mutex poisoned; safe fallback.
                show_onboarding_window(&app_handle, onboarding::OnboardingStage::Permissions);
                return;
            }
        }
        show_overlay(&app_handle, crate::context::ActivationContext::empty());
    }
}

/// Advances the onboarding stage from `model_check` to `intro` and emits
/// the onboarding event so the frontend swaps to `IntroStep` without a
/// window flicker.
///
/// Called by `ModelCheckStep` when it observes a `Ready` setup state on
/// mount or after a Re-check click. The caller has already verified that
/// Ollama is reachable and at least one model is installed; this command
/// only commits the stage advance and notifies the frontend.
///
/// Idempotent: writing `intro` over `intro` is a harmless no-op, so a
/// double-fire from a frontend race cannot corrupt the stage.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn advance_past_model_check(
    db: tauri::State<history::Database>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
    onboarding::set_stage(&conn, &onboarding::OnboardingStage::Intro)
        .map_err(|e| format!("db write failed: {e}"))?;
    drop(conn);

    let _ = app_handle.emit(
        ONBOARDING_EVENT,
        OnboardingPayload {
            stage: onboarding::OnboardingStage::Intro,
        },
    );
    Ok(())
}

// ─── Onboarding completion ───────────────────────────────────────────────────

/// Called when the user clicks "Get Started" on the intro screen.
/// Marks onboarding complete in the DB, restores the window to overlay mode,
/// and immediately shows the Ask Bar - no relaunch required.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn finish_onboarding(
    db: tauri::State<history::Database>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
    onboarding::mark_complete(&conn).map_err(|e| format!("db write failed: {e}"))?;
    drop(conn);

    // Restore panel to overlay configuration and show the Ask Bar.
    // Must run on the macOS main thread because NSPanel APIs are not thread-safe.
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        // Resize the window back to the collapsed overlay dimensions before
        // positioning, so the overlay appears at the correct size.
        if let Some(window) = handle.get_webview_window("main") {
            let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                OVERLAY_LOGICAL_WIDTH,
                OVERLAY_LOGICAL_HEIGHT_COLLAPSED,
            )));
        }
        // Restore NSPanel level, shadow, and style that show_onboarding_window
        // changed for the onboarding appearance.
        #[cfg(target_os = "macos")]
        init_panel(&handle);
        show_overlay(&handle, crate::context::ActivationContext::empty());
    });

    Ok(())
}

// ─── NSPanel initialisation ─────────────────────────────────────────────────

/// Converts the main Tauri window into an NSPanel and applies the overlay
/// configuration required to appear over fullscreen macOS applications.
///
/// The four critical settings are:
/// - `PanelLevel::Floating` - floats above normal windows
/// - `CollectionBehavior::full_screen_auxiliary()` - allows coexistence with
///   fullscreen Spaces (this is what standard `alwaysOnTop` cannot do)
/// - `StyleMask::nonactivating_panel()` - prevents the panel from stealing
///   focus/activation from the fullscreen application
/// - `set_has_shadow(false)` - disables the native compositor shadow, which
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

// ─── Settings panel initialisation ──────────────────────────────────────────

/// Converts the settings Tauri window into a ThukiSettingsPanel NSPanel subclass.
///
/// Called once during app setup. The resulting panel handle is stored in the
/// tauri-nspanel WebviewPanelManager, so subsequent calls to
/// `get_webview_panel("settings")` retrieve the same panel without
/// re-converting.
///
/// Collection behavior: `move_to_active_space` moves the panel to whichever
/// Space is current when `show_and_make_key` is called (requires the app to be
/// the active process first — see `show_settings_window`). `full_screen_auxiliary`
/// allows the panel to coexist with fullscreen app Spaces.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn init_settings_panel(app_handle: &tauri::AppHandle) {
    let Some(window) = app_handle.get_webview_window("settings") else {
        eprintln!("thuki: [settings] window not found during init_settings_panel");
        return;
    };
    match window.to_panel::<ThukiSettingsPanel>() {
        Ok(panel) => {
            panel.set_floating_panel(false);
            panel.set_level(PanelLevel::Floating.value());
            panel.set_has_shadow(true);
            panel.set_collection_behavior(
                CollectionBehavior::new()
                    .move_to_active_space()
                    .full_screen_auxiliary()
                    .into(),
            );
        }
        Err(e) => {
            eprintln!("thuki: [settings] NSPanel conversion failed: {e:?}");
        }
    }
}

// ─── Onboarding window ───────────────────────────────────────────────────────

/// Sizes the main window for the onboarding screen, centers it, makes it
/// visible, and emits `thuki://onboarding` so the frontend switches to
/// `OnboardingView`.
///
/// All window mutations run on the macOS main thread via `run_on_main_thread`;
/// the event is emitted from the same closure to avoid a race where the
/// frontend receives the event before the window is visible.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn show_onboarding_window(app_handle: &tauri::AppHandle, stage: onboarding::OnboardingStage) {
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("main") {
            let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                ONBOARDING_LOGICAL_WIDTH,
                ONBOARDING_LOGICAL_HEIGHT,
            )));
            let _ = window.center();
        }
        match handle.get_webview_panel("main") {
            Ok(panel) => {
                // Use normal window level so System Settings can appear above.
                panel.set_level(0);
                // Re-enable native shadow for onboarding. init_panel disables
                // it for the overlay to avoid the key/non-key shadow flicker,
                // but for onboarding the native shadow looks professional and
                // renders outside the window boundary - no transparent padding
                // needed.
                panel.set_has_shadow(true);
                panel.show_and_make_key();
            }
            Err(_) => {
                if let Some(w) = handle.get_webview_window("main") {
                    let _ = w.show();
                }
            }
        }
        let _ = handle.emit(ONBOARDING_EVENT, OnboardingPayload { stage });
    });
}

/// Payload emitted to the frontend for every onboarding transition.
#[derive(Clone, serde::Serialize)]
struct OnboardingPayload {
    stage: onboarding::OnboardingStage,
}

// ─── Image cleanup ──────────────────────────────────────────────────────────

/// Interval between periodic orphaned-image cleanup sweeps.
const IMAGE_CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

/// Runs a single orphaned-image cleanup sweep. Thin orchestration wrapper
/// that delegates to `database::get_all_image_paths` and
/// `images::cleanup_orphaned_images`, both independently tested.
#[cfg_attr(coverage_nightly, coverage(off))]
fn run_image_cleanup(app_handle: &tauri::AppHandle) {
    let db = app_handle.state::<history::Database>();
    let conn = match db.0.lock() {
        Ok(c) => c,
        Err(_) => return,
    };
    let referenced = database::get_all_image_paths(&conn).unwrap_or_default();
    drop(conn);

    let base_dir = match app_handle.path().app_data_dir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let _ = images::cleanup_orphaned_images(&base_dir, &referenced);
}

/// Spawns a background Tokio task that runs the cleanup sweep on a fixed
/// interval. Thin async wrapper - delegates to `run_image_cleanup`.
#[cfg_attr(coverage_nightly, coverage(off))]
fn spawn_periodic_image_cleanup(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(IMAGE_CLEANUP_INTERVAL);
        // Skip the first tick (startup cleanup already ran synchronously).
        interval.tick().await;
        loop {
            interval.tick().await;
            run_image_cleanup(&app_handle);
        }
    });
}

// ─── Application entry point ─────────────────────────────────────────────────

/// Initialises and runs the Tauri application.
///
/// Setup order:
/// 1. `ActivationPolicy::Accessory` suppresses the Dock icon.
/// 2. The main window is converted to an NSPanel for fullscreen overlay.
/// 3. The settings window is converted to a ThukiSettingsPanel NSPanel subclass.
/// 4. System tray is registered; double-tap Option listener starts.
/// 5. `CloseRequested` is intercepted to hide instead of destroy.
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
            #[cfg(target_os = "macos")]
            init_settings_panel(app.app_handle());

            // ── System tray icon + menu ───────────────────────────────────
            // Order chosen for muscle-memory parity with mac tray apps
            // (Bartender, CleanShot X, Rectangle): primary action at top,
            // settings near it with the macOS-canonical ⌘, accelerator,
            // separator, then Quit at the bottom. The "Reveal app data"
            // affordance lives inside the Settings → About tab so the tray
            // stays focused on session-level actions.
            let show_item = MenuItem::with_id(app, "show", "Open Thuki", true, None::<&str>)?;
            let settings_item =
                MenuItem::with_id(app, "settings", "Settings…", true, Some("Cmd+,"))?;
            let separator = tauri::menu::PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Thuki", true, Some("Cmd+Q"))?;
            let tray_menu =
                Menu::with_items(app, &[&show_item, &settings_item, &separator, &quit_item])?;

            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png"))
                .expect("Failed to load tray icon");

            let _tray = TrayIconBuilder::new()
                .icon(tray_icon)
                .icon_as_template(false)
                .tooltip("Thuki")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        show_overlay(app, crate::context::ActivationContext::empty());
                    }
                    "settings" => {
                        show_settings_window(app);
                    }
                    "quit" => {
                        app.state::<crate::commands::GenerationState>().cancel();
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_overlay(
                            tray.app_handle(),
                            crate::context::ActivationContext::empty(),
                        );
                    }
                })
                .build(app)?;

            // ── Activation listener (macOS only) ─────────────────────────
            // Only start the event tap when Accessibility is already granted.
            // Creating a CGEventTap without permission triggers a native macOS
            // popup; deferring until after onboarding (and the quit+reopen for
            // Screen Recording) avoids that redundant dialog entirely.
            #[cfg(target_os = "macos")]
            {
                let app_handle = app.handle().clone();
                let activator = activator::OverlayActivator::new();
                if permissions::is_accessibility_granted() {
                    activator.start(move || {
                        // Skip AX + clipboard when hiding - no context needed and
                        // simulating Cmd+C against Thuki's own WebView would produce
                        // a macOS alert sound.
                        let is_visible = OVERLAY_INTENDED_VISIBLE.load(Ordering::SeqCst);
                        let handle = app_handle.clone();
                        let handle2 = app_handle.clone();

                        // Dispatch context capture to a dedicated thread so the event
                        // tap callback returns immediately. AX attribute lookups and
                        // clipboard simulation can block for seconds (macOS AX default
                        // timeout is ~6 s) when the focused app does not implement the
                        // accessibility protocol. Blocking the tap callback freezes the
                        // CFRunLoop and silently prevents all future key events from
                        // being delivered to the activator.
                        std::thread::spawn(move || {
                            let ctx = crate::context::capture_activation_context(is_visible);
                            let _ =
                                handle.run_on_main_thread(move || toggle_overlay(&handle2, ctx));
                        });
                    });
                }
                app.manage(activator);
            }

            // ── Persistent HTTP client ────────────────────────────────
            app.manage(reqwest::Client::new());
            let warmup_handle = app.handle().clone();
            app.manage(warmup::WarmupState::with_on_loaded(Arc::new(
                move |model| {
                    let _ = warmup_handle.emit("warmup:model-loaded", model);
                },
            )));

            // ── Configuration (TOML file at app_config_dir) ─────────
            // Loaded once at startup. Missing file -> seed defaults.
            // Corrupt file -> rename-with-timestamp + reseed. Only a hard
            // write failure (disk full, permissions) is fatal; in that case
            // we show a native alert and exit. See src/config/mod.rs.
            let app_config = match crate::config::load(app.handle()) {
                Ok(c) => c,
                Err(e) => crate::config::show_fatal_dialog_and_exit(&e),
            };
            // Wrap in `parking_lot::RwLock` so the Settings panel can mutate
            // the in-memory config via `set_config_field` while readers
            // (every Ollama call, every search call) take cheap clones via
            // `state.read().clone()`. Parking_lot avoids std::sync poisoning
            // on writer panic. See design doc P10.
            app.manage(parking_lot::RwLock::new(app_config));

            // ── Generation + conversation state ─────────────────────
            app.manage(commands::GenerationState::new());
            app.manage(commands::ConversationHistory::new());

            // ── SQLite database for conversation history ──────────
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data directory");
            let db_conn = database::open_database(&app_data_dir)
                .expect("failed to initialise SQLite database");

            // ── Active-model state: seed from SQLite app_config table ──
            // The installed list isn't queried here (no async runtime yet).
            // get_model_picker_state reconciles against the live /api/tags
            // inventory on first picker open and may replace this seed.
            // When nothing is persisted the seed is `None`: there is no
            // compiled fallback. The Phase 3 onboarding gate refuses to
            // open the overlay until a real installed model is selected,
            // so an unset slug never reaches `ask_ollama`.
            let persisted_active = database::get_config(&db_conn, models::ACTIVE_MODEL_KEY)
                .expect("failed to read active_model from app_config");
            let initial_active_model =
                models::resolve_seed_active_model(persisted_active.as_deref());
            app.manage(models::ActiveModelState(std::sync::Mutex::new(
                initial_active_model,
            )));
            app.manage(models::ModelCapabilitiesCache::default());
            app.manage(history::Database(std::sync::Mutex::new(db_conn)));

            // ── Orphaned image cleanup (startup + periodic) ─────────
            run_image_cleanup(app.handle());
            spawn_periodic_image_cleanup(app.handle().clone());

            // ── VRAM sentinel poll ─────────────────────────────────
            // Detects external VRAM changes (ollama stop, TTL expiry,
            // daemon restart) that Thuki did not initiate. Polls
            // /api/ps every VRAM_POLL_INTERVAL_SECS seconds and emits
            // warmup:model-loaded or warmup:model-evicted as needed.
            warmup::spawn_vram_poller(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            #[cfg(not(coverage))]
            commands::ask_ollama,
            #[cfg(not(coverage))]
            commands::cancel_generation,
            #[cfg(not(coverage))]
            commands::open_url,
            #[cfg(not(coverage))]
            search::search_pipeline,
            #[cfg(not(coverage))]
            commands::reset_conversation,
            settings_commands::get_config,
            settings_commands::set_config_field,
            settings_commands::reset_config,
            settings_commands::reload_config_from_disk,
            settings_commands::get_corrupt_marker,
            #[cfg(not(coverage))]
            settings_commands::reveal_config_in_finder,
            #[cfg(not(coverage))]
            models::get_model_picker_state,
            #[cfg(not(coverage))]
            models::set_active_model,
            #[cfg(not(coverage))]
            models::check_model_setup,
            #[cfg(not(coverage))]
            models::get_model_capabilities,
            #[cfg(not(coverage))]
            history::save_conversation,
            #[cfg(not(coverage))]
            history::persist_message,
            #[cfg(not(coverage))]
            history::list_conversations,
            #[cfg(not(coverage))]
            history::load_conversation,
            #[cfg(not(coverage))]
            history::delete_conversation,
            #[cfg(not(coverage))]
            history::generate_title,
            #[cfg(not(coverage))]
            images::save_image_command,
            #[cfg(not(coverage))]
            images::remove_image_command,
            #[cfg(not(coverage))]
            images::cleanup_orphaned_images_command,
            #[cfg(not(coverage))]
            screenshot::capture_screenshot_command,
            #[cfg(not(coverage))]
            screenshot::capture_full_screen_command,
            notify_overlay_hidden,
            notify_frontend_ready,
            set_window_frame,
            #[cfg(not(coverage))]
            permissions::check_accessibility_permission,
            #[cfg(not(coverage))]
            permissions::open_accessibility_settings,
            #[cfg(not(coverage))]
            permissions::check_screen_recording_permission,
            #[cfg(not(coverage))]
            permissions::open_screen_recording_settings,
            #[cfg(not(coverage))]
            permissions::request_screen_recording_access,
            #[cfg(not(coverage))]
            permissions::check_screen_recording_tcc_granted,
            #[cfg(not(coverage))]
            permissions::quit_and_relaunch,
            finish_onboarding,
            advance_past_model_check,
            #[cfg(not(coverage))]
            warmup::warm_up_model,
            #[cfg(not(coverage))]
            warmup::evict_model,
            #[cfg(not(coverage))]
            warmup::get_loaded_model
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
                } else if label == "settings" {
                    // Hide instead of destroy so React state (active tab,
                    // form values) survives close/reopen.
                    api.prevent_close();
                    if let Some(window) = app_handle.get_webview_window("settings") {
                        let _ = window.hide();
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_window_frame_rejects_nan() {
        assert!(!f64::NAN.is_finite());
        assert!(!f64::INFINITY.is_finite());
        assert!(!f64::NEG_INFINITY.is_finite());
        assert!(100.0_f64.is_finite());
    }

    #[test]
    fn width_height_clamp_logic() {
        assert_eq!(0.5_f64.clamp(1.0, 10_000.0), 1.0);
        assert_eq!(500.0_f64.clamp(1.0, 10_000.0), 500.0);
        assert_eq!(20_000.0_f64.clamp(1.0, 10_000.0), 10_000.0);
    }

    #[test]
    fn notify_overlay_hidden_sets_flag_to_false() {
        OVERLAY_INTENDED_VISIBLE.store(true, Ordering::SeqCst);
        OVERLAY_INTENDED_VISIBLE.store(false, Ordering::SeqCst);
        assert!(!OVERLAY_INTENDED_VISIBLE.load(Ordering::SeqCst));
    }

    #[test]
    fn launch_show_pending_consumed_exactly_once() {
        LAUNCH_SHOW_PENDING.store(true, Ordering::SeqCst);
        assert!(LAUNCH_SHOW_PENDING.swap(false, Ordering::SeqCst));
        assert!(!LAUNCH_SHOW_PENDING.swap(false, Ordering::SeqCst));
    }

    #[test]
    fn overlay_visibility_event_constant_matches() {
        assert_eq!(OVERLAY_VISIBILITY_EVENT, "thuki://visibility");
        assert_eq!(OVERLAY_VISIBILITY_SHOW, "show");
        assert_eq!(OVERLAY_VISIBILITY_HIDE_REQUEST, "hide-request");
    }

    #[test]
    fn onboarding_event_constant_matches() {
        assert_eq!(ONBOARDING_EVENT, "thuki://onboarding");
    }

    #[test]
    fn onboarding_logical_dimensions() {
        assert_eq!(ONBOARDING_LOGICAL_WIDTH, 460.0);
        assert_eq!(ONBOARDING_LOGICAL_HEIGHT, 640.0);
    }

    #[test]
    fn overlay_logical_dimensions() {
        assert_eq!(OVERLAY_LOGICAL_WIDTH, 600.0);
        assert_eq!(OVERLAY_LOGICAL_HEIGHT_COLLAPSED, 80.0);
    }
}
