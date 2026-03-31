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

/// Frontend event used to synchronize show/hide animations with native window visibility.
const OVERLAY_VISIBILITY_EVENT: &str = "thuki://visibility";
const OVERLAY_VISIBILITY_SHOW: &str = "show";
const OVERLAY_VISIBILITY_HIDE_REQUEST: &str = "hide-request";

/// Tracks the intended visibility state of the overlay, preventing race conditions
/// between the frontend exit animation and rapid activation toggles.
static OVERLAY_INTENDED_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Emits a visibility transition to the frontend animation controller.
fn emit_overlay_visibility(app_handle: &tauri::AppHandle, state: &str) {
    let _ = app_handle.emit(OVERLAY_VISIBILITY_EVENT, state);
}

/// Shows the overlay and requests the frontend to replay its entrance animation.
///
/// Uses `show_and_make_key()` to guarantee the NSPanel becomes the key window,
/// which is required for the WebView input to receive keyboard focus reliably.
#[cfg(target_os = "macos")]
fn show_overlay(app_handle: &tauri::AppHandle) {
    if OVERLAY_INTENDED_VISIBLE.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Ok(panel) = app_handle.get_webview_panel("main") {
        panel.show_and_make_key();
        emit_overlay_visibility(app_handle, OVERLAY_VISIBILITY_SHOW);
    }
}

/// Requests an animated hide sequence from the frontend. The actual native
/// window hide is deferred until the frontend exit animation completes.
fn request_overlay_hide(app_handle: &tauri::AppHandle) {
    if OVERLAY_INTENDED_VISIBLE.swap(false, Ordering::SeqCst) {
        emit_overlay_visibility(app_handle, OVERLAY_VISIBILITY_HIDE_REQUEST);
    }
}

/// Shows the overlay and requests the frontend to replay its entrance animation.
#[cfg(not(target_os = "macos"))]
fn show_overlay(app_handle: &tauri::AppHandle) {
    if OVERLAY_INTENDED_VISIBLE.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        emit_overlay_visibility(app_handle, OVERLAY_VISIBILITY_SHOW);
    }
}

/// Toggles the overlay between visible and hidden states.
///
/// Uses an atomic flag as the single source of truth for intended visibility,
/// which avoids race conditions with the native panel state during animations.
fn toggle_overlay(app_handle: &tauri::AppHandle) {
    if OVERLAY_INTENDED_VISIBLE.load(Ordering::SeqCst) {
        request_overlay_hide(app_handle);
    } else {
        show_overlay(app_handle);
    }
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
                        show_overlay(app);
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
                        toggle_overlay(tray.app_handle());
                    }
                })
                .build(app)?;

            // ── Activation listener (macOS only) ─────────────────────────
            #[cfg(target_os = "macos")]
            {
                let app_handle = app.handle().clone();
                let activator = activator::OverlayActivator::new();
                activator.start(move || {
                    let handle = app_handle.clone();
                    let _ = app_handle.run_on_main_thread(move || toggle_overlay(&handle));
                });
                app.manage(activator);
            }

            // ── Persistent HTTP client ────────────────────────────────
            app.manage(reqwest::Client::new());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ask_ollama,
            notify_overlay_hidden
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
