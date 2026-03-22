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

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, RunEvent, WebviewWindow,
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

/// Toggles the NSPanel between visible/hidden on macOS.
///
/// Uses `get_webview_panel` to operate directly on the NSPanel, which
/// correctly handles fullscreen Space visibility. Falls back to standard
/// window show/hide on non-macOS platforms.
#[cfg(target_os = "macos")]
fn toggle_panel(app_handle: &tauri::AppHandle) {
    if let Ok(panel) = app_handle.get_webview_panel("main") {
        if panel.is_visible() {
            panel.hide();
        } else {
            panel.show();
        }
    }
}

/// Toggles the chat window between visible/hidden states (non-macOS fallback).
#[cfg(not(target_os = "macos"))]
fn toggle_window(window: &WebviewWindow) {
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// ─── NSPanel initialisation ─────────────────────────────────────────────────

/// Converts the main Tauri window into an NSPanel and applies the overlay
/// configuration required to appear over fullscreen macOS applications.
///
/// The three critical settings are:
/// - `PanelLevel::Floating` — floats above normal windows
/// - `CollectionBehavior::full_screen_auxiliary()` — allows coexistence with
///   fullscreen Spaces (this is what standard `alwaysOnTop` cannot do)
/// - `StyleMask::nonactivating_panel()` — prevents the panel from stealing
///   focus/activation from the fullscreen application
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

            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
                .expect("Failed to load tray icon");

            let _tray = TrayIconBuilder::new()
                .icon(tray_icon)
                .icon_as_template(false)
                .tooltip("Thuki")
                .menu(&tray_menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        #[cfg(target_os = "macos")]
                        {
                            if let Ok(panel) = app.get_webview_panel("main") {
                                panel.show();
                            }
                        }
                        #[cfg(not(target_os = "macos"))]
                        {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                        }
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
                        #[cfg(target_os = "macos")]
                        toggle_panel(tray.app_handle());

                        #[cfg(not(target_os = "macos"))]
                        {
                            if let Some(win) = tray.app_handle().get_webview_window("main") {
                                toggle_window(&win);
                            }
                        }
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
                    let _ = app_handle.run_on_main_thread(move || toggle_panel(&handle));
                });
                app.manage(activator);
            }

            // ── Persistent HTTP client ────────────────────────────────
            app.manage(reqwest::Client::new());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::ask_ollama])
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

                    #[cfg(target_os = "macos")]
                    {
                        if let Ok(panel) = app_handle.get_webview_panel("main") {
                            panel.hide();
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        if let Some(win) = app_handle.get_webview_window("main") {
                            let _ = win.hide();
                        }
                    }
                }
            }
        });
}
