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
pub mod engine;
pub mod export;
pub mod history;
pub mod images;
pub mod models;
pub mod net;
pub mod ocr;
pub mod onboarding;
pub mod openai;
pub mod screenshot;
pub mod search;
pub mod settings_commands;
pub mod startup_guard;
pub mod subscribe;
pub mod trace;
pub mod updater;
pub mod warmup;

#[cfg(target_os = "macos")]
mod activator;
#[cfg(target_os = "macos")]
mod cg_displays;
pub mod context;
pub mod keychain;
pub mod permissions;
pub mod replace;

use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Listener, Manager, RunEvent, WebviewWindow,
};

#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;

#[cfg(target_os = "macos")]
use tauri_nspanel::{CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt};

// ─── NSPanel definition (macOS only) ────────────────────────────────────────

// Each tauri_panel! invocation emits `use` statements at its call-site
// module scope. Two calls in the same module cause name collisions, so
// each panel subclass lives in its own private module. The underscore
// prefix marks each module as an internal implementation detail; add
// any future panel subclass the same way.
//
// ThukiPanel - overlay NSPanel: floating, keyboard input for chat.
// ThukiSettingsPanel - settings NSPanel: floating + nonactivating so it
//   appears on the user's current Space; keyboard input; no
//   ActivationPolicy switch so the Dock icon never appears.
#[cfg(target_os = "macos")]
mod _thuki_panel {
    use tauri::Manager;
    use tauri_nspanel::TrackingAreaOptions;
    tauri_nspanel::tauri_panel! {
        panel!(ThukiPanel {
            config: {
                can_become_main_window: false,
                can_become_key_window: true,
                becomes_key_only_if_needed: true,
                is_floating_panel: true
            }
            with: {
                // A nonactivating panel under Accessory policy cannot
                // self-activate on modern macOS (cooperative activation), so
                // once the overlay is defocused a plain click can never regain
                // key/active: the webview then drops clicks, drag, and hover.
                // An `active_always` tracking area keeps mouse-move / enter /
                // exit / cursor-update events flowing to the webview even while
                // the app is inactive (revives `:hover` and the pointer
                // cursor), and the mouse-entered callback (wired in
                // `init_panel`) makes the panel key on cursor-enter so clicks
                // and drag land. None of this activates the app, so the overlay
                // still never yanks the user off another app's fullscreen Space.
                tracking_area: {
                    options: TrackingAreaOptions::new()
                        .active_always()
                        .mouse_entered_and_exited()
                        .mouse_moved()
                        .cursor_update(),
                    auto_resize: true
                }
            }
        })
        panel_event!(ThukiOverlayEventsInner {})
    }

    /// Constructs the mouse-event handler and attaches it to `panel`.
    ///
    /// `panel_event!` emits a private handler struct, so the wiring lives here
    /// where that type is in scope; callers only need the public `ThukiPanel`.
    /// The mouse-entered callback makes the overlay the key window the instant
    /// the cursor enters it, which is what restores clicks/drag after the
    /// overlay has been defocused (see the tracking-area comment on the panel).
    pub fn attach_overlay_event_handler(app_handle: tauri::AppHandle) {
        use tauri_nspanel::ManagerExt;
        let Ok(panel) = app_handle.get_webview_panel("main") else {
            return;
        };
        let cb_handle = app_handle.clone();
        let events = ThukiOverlayEventsInner::new();
        events.on_mouse_entered(move |_event| {
            if let Ok(p) = cb_handle.get_webview_panel("main") {
                p.make_key_window();
            }
        });
        panel.set_event_handler(Some(events.as_ref()));
    }
}
#[cfg(target_os = "macos")]
use _thuki_panel::ThukiPanel;

#[cfg(target_os = "macos")]
mod _settings_panel {
    use tauri::Manager;
    use tauri_nspanel::TrackingAreaOptions;
    tauri_nspanel::tauri_panel! {
        panel!(ThukiSettingsPanel {
            config: {
                can_become_key_window: true,
                is_floating_panel: true
            }
            with: {
                // Same hover-activate rationale as ThukiPanel. Settings is a
                // nonactivating panel with hides_on_deactivate(false), so once
                // it is defocused (the user clicks another app) a plain click
                // can never regain key on modern macOS and the webview drops
                // clicks, drag, and hover - the form inputs go dead. An
                // `active_always` tracking area keeps mouse events flowing while
                // the app is inactive, and the mouse-entered callback (wired in
                // `init_settings_panel`) makes the panel key on cursor-enter so
                // the inputs come back without activating the app.
                tracking_area: {
                    options: TrackingAreaOptions::new()
                        .active_always()
                        .mouse_entered_and_exited()
                        .mouse_moved()
                        .cursor_update(),
                    auto_resize: true
                }
            }
        })
        panel_event!(ThukiSettingsEventsInner {})
    }

    /// Constructs the mouse-event handler and attaches it to the Settings panel.
    ///
    /// Mirrors `attach_overlay_event_handler` for ThukiPanel: the mouse-entered
    /// callback makes the Settings overlay the key window the instant the cursor
    /// enters it, restoring clicks/drag/typing after the panel has been
    /// defocused (see the tracking-area comment on the panel).
    pub fn attach_settings_event_handler(app_handle: tauri::AppHandle) {
        use tauri_nspanel::ManagerExt;
        let Ok(panel) = app_handle.get_webview_panel("settings") else {
            return;
        };
        let cb_handle = app_handle.clone();
        let events = ThukiSettingsEventsInner::new();
        events.on_mouse_entered(move |_event| {
            if let Ok(p) = cb_handle.get_webview_panel("settings") {
                p.make_key_window();
            }
        });
        panel.set_event_handler(Some(events.as_ref()));
    }
}
#[cfg(target_os = "macos")]
use _settings_panel::ThukiSettingsPanel;

// ThukiUpdatePanel - "What's New" NSPanel. Modeled on the OVERLAY panel
//   (ThukiPanel), not settings: floating + nonactivating so it can appear
//   on whatever Space the user is on, including over another app's
//   fullscreen Space (the footer that opens it can be summoned there).
//   `can_become_key_window` stays true so the four action buttons still
//   receive clicks/keyboard. Separate subclass/module so the tauri_panel!
//   `use` emissions don't collide with the other two.
#[cfg(target_os = "macos")]
mod _update_panel {
    use tauri::Manager;
    use tauri_nspanel::TrackingAreaOptions;
    tauri_nspanel::tauri_panel! {
        panel!(ThukiUpdatePanel {
            config: {
                can_become_key_window: true,
                is_floating_panel: true
            }
            with: {
                // Same hover-activate rationale as ThukiPanel. The update panel
                // is nonactivating with hides_on_deactivate(false), so after it
                // is defocused a plain click can never regain key on modern
                // macOS and the webview drops clicks, drag, and hover - the four
                // action buttons go dead. An `active_always` tracking area keeps
                // mouse events flowing while the app is inactive, and the
                // mouse-entered callback (wired in `init_update_panel`) makes the
                // panel key on cursor-enter so the buttons come back without
                // activating the app.
                tracking_area: {
                    options: TrackingAreaOptions::new()
                        .active_always()
                        .mouse_entered_and_exited()
                        .mouse_moved()
                        .cursor_update(),
                    auto_resize: true
                }
            }
        })
        panel_event!(ThukiUpdateEventsInner {})
    }

    /// Constructs the mouse-event handler and attaches it to the update panel.
    ///
    /// Mirrors `attach_overlay_event_handler` for ThukiPanel: the mouse-entered
    /// callback makes the update overlay the key window the instant the cursor
    /// enters it, restoring clicks after the panel has been defocused (see the
    /// tracking-area comment on the panel).
    pub fn attach_update_event_handler(app_handle: tauri::AppHandle) {
        use tauri_nspanel::ManagerExt;
        let Ok(panel) = app_handle.get_webview_panel("update") else {
            return;
        };
        let cb_handle = app_handle.clone();
        let events = ThukiUpdateEventsInner::new();
        events.on_mouse_entered(move |_event| {
            if let Ok(p) = cb_handle.get_webview_panel("update") {
                p.make_key_window();
            }
        });
        panel.set_event_handler(Some(events.as_ref()));
    }
}
#[cfg(target_os = "macos")]
use _update_panel::ThukiUpdatePanel;

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
/// Emitted while the overlay is parked in the minimized icon and an
/// activation occurs. The frontend restores the chat without the
/// fresh-session wipe that OVERLAY_VISIBILITY_SHOW triggers.
const OVERLAY_VISIBILITY_RESTORE: &str = "restore";

/// Frontend event that triggers the onboarding screen when one or more
/// required permissions have not yet been granted.
const ONBOARDING_EVENT: &str = "thuki://onboarding";

/// Frontend event that asks the Settings window to jump to the Models tab's
/// Discover pane (the download picker). Emitted by `open_settings_window`, which
/// the in-overlay model picker calls from its "no model yet" empty state, so the
/// user lands on the model browser rather than the default Providers view.
const SETTINGS_SHOW_DISCOVER_EVENT: &str = "thuki://settings-show-discover";

/// Frontend event that asks the Settings window to jump to the Models tab's
/// Providers pane. Emitted by `open_settings_to_providers`, which the ask-bar
/// "Ollama isn't running" strip calls from its "switch to Built-in" link, so a
/// user who switched to Ollama without it running lands where they can flip the
/// active provider back to the built-in engine.
const SETTINGS_SHOW_PROVIDERS_EVENT: &str = "thuki://settings-show-providers";

/// Logical dimensions of the onboarding window (centered). The permission
/// and intro steps use the compact base size; the model-picker step widens
/// to fit the three-column comparison matrix. Steps smaller than the frame
/// they render in center their card against the transparent background, so
/// the per-stage size difference is invisible. Native macOS shadow is
/// re-enabled for onboarding so it renders outside the window boundary
/// without extra transparent padding.
const ONBOARDING_LOGICAL_WIDTH: f64 = 460.0;
const ONBOARDING_LOGICAL_HEIGHT: f64 = 640.0;
const ONBOARDING_PICKER_WIDTH: f64 = 860.0;
const ONBOARDING_PICKER_HEIGHT: f64 = 744.0;

/// Per-stage onboarding window size. The model-picker step needs a wide
/// frame for the comparison matrix; every other step keeps the compact base
/// size. Pure so the mapping is unit-tested even though the window mutation
/// it feeds runs on the macOS main thread.
fn onboarding_window_size(stage: &onboarding::OnboardingStage) -> (f64, f64) {
    match stage {
        onboarding::OnboardingStage::ModelCheck => {
            (ONBOARDING_PICKER_WIDTH, ONBOARDING_PICKER_HEIGHT)
        }
        // The intro tour is sized to its card by the frontend
        // (`useFitOnboardingWindow`) so the transparent window never blocks
        // background clicks and grows to fit the ambient download strip; the
        // compact base is only its pre-fit starting size.
        _ => (ONBOARDING_LOGICAL_WIDTH, ONBOARDING_LOGICAL_HEIGHT),
    }
}

/// Tracks the intended visibility state of the overlay, preventing race conditions
/// between the frontend exit animation and rapid activation toggles.
static OVERLAY_INTENDED_VISIBLE: AtomicBool = AtomicBool::new(false);

/// True while the overlay is collapsed into the floating minimized icon.
/// Read by the activator layer so any activation restores the parked
/// conversation instead of showing/hiding.
static OVERLAY_MINIMIZED: AtomicBool = AtomicBool::new(false);

/// True on first process launch; cleared when the frontend signals readiness.
/// Used to show the overlay automatically on startup without a race condition:
/// the frontend calls `notify_frontend_ready` after its event listener is
/// registered, so the show event is guaranteed to have a listener.
static LAUNCH_SHOW_PENDING: AtomicBool = AtomicBool::new(true);

/// True while the onboarding flow owns the main window (any stage:
/// permissions, model_check, intro). Set when `show_onboarding_window` puts
/// the window into its fixed 460x640 centered onboarding appearance, cleared
/// by `finish_onboarding` just before the first real overlay show. Read at the
/// top of `show_overlay` so an activation (tray "Open Thuki" / double-tap
/// Control) does not run the ask-bar show path while onboarding is up: doing so
/// would reposition the window and emit a `show` visibility event, and the
/// frontend's width/height sync would then collapse the still-onboarding window
/// to the ask-bar size.
static ONBOARDING_ACTIVE: AtomicBool = AtomicBool::new(false);

fn set_onboarding_active_impl(active: bool) {
    ONBOARDING_ACTIVE.store(active, Ordering::SeqCst);
}

/// True while the Settings window is open (shown, not yet closed/hidden). Set
/// in `show_settings_window`, cleared in the `settings` close handler. Combined
/// with `ONBOARDING_ACTIVE` to decide the app's activation policy: Thuki is a
/// menu-bar app and stays `Accessory` (no Dock icon, floating overlay) by
/// default, but flips to `Regular` while a real window (Settings or onboarding)
/// is open. Regular makes that window order and layer like a normal app window
/// (it opens on top, but another app clicked afterwards rises above it) and
/// surfaces a Dock icon so a user who clicks away can get back to it.
static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);

/// True while the "What's New" update window is open (shown, not yet
/// closed/hidden). Set in `show_update_window`, cleared in the `update` close
/// handler. Like `SETTINGS_OPEN`, it flips the app to `Regular` activation so the
/// update window behaves like a normal app window (it opens on top, another app
/// clicked afterwards rises above it, a Dock icon offers a way back) and so
/// activating pulls the user to the window's Space instead of floating it over
/// whatever Space they are on.
static UPDATE_OPEN: AtomicBool = AtomicBool::new(false);

/// Whether the app should currently present as a regular foreground app (Dock
/// icon + normal window layering) rather than a Dock-less `Accessory`. True
/// while Settings, the update window, or onboarding owns a real window; false
/// when only the floating overlay is around. Pure so the policy decision is
/// unit-tested without touching AppKit; `sync_activation_policy` applies it.
fn wants_regular_activation() -> bool {
    SETTINGS_OPEN.load(Ordering::SeqCst)
        || UPDATE_OPEN.load(Ordering::SeqCst)
        || ONBOARDING_ACTIVE.load(Ordering::SeqCst)
}

/// Set once the user confirms a quit (or quits with no download in flight), so
/// the re-entrant `ExitRequested` that `app.exit` raises is allowed straight
/// through instead of re-prompting the download warning forever.
static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);

/// True while the quit warning dialog is on screen. Cmd+Q reaches the warning
/// twice (the app-menu Quit event AND `RunEvent::ExitRequested`); this guard
/// keeps it to a single dialog instead of two stacked ones.
static QUIT_DIALOG_OPEN: AtomicBool = AtomicBool::new(false);

/// True while a model download is paused, set by the frontend via
/// `set_download_paused`. A paused download still has work left (the partial is
/// discarded on the next launch), so the quit warning must cover it too, not
/// only an actively-streaming download.
static DOWNLOAD_PAUSED: AtomicBool = AtomicBool::new(false);

/// Frontend hook so the quit warning fires for a paused download, not only an
/// actively-streaming one. The pause cancels the backend task, so the slot is
/// free and only the frontend knows a download is paused.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
fn set_download_paused(paused: bool) {
    DOWNLOAD_PAUSED.store(paused, Ordering::SeqCst);
}

/// Whether quitting now would discard an in-progress model download: one is
/// actively streaming, or one is paused.
fn should_warn_on_quit(app: &tauri::AppHandle) -> bool {
    models::download_in_flight(app.state::<models::DownloadState>().inner())
        || DOWNLOAD_PAUSED.load(Ordering::SeqCst)
}

/// Durably marks this session's launch record as a clean exit (issue #296).
///
/// The only place the launch circuit breaker's `clean_exit` marker is set to
/// true. Best-effort and idempotent: no-op when this process does not own the
/// session (another instance held the advisory lock), and any write failure is
/// logged rather than blocking shutdown.
#[cfg_attr(coverage_nightly, coverage(off))]
fn mark_session_clean_exit(app: &tauri::AppHandle) {
    if let Some(guard) = app.try_state::<startup_guard::SessionGuard>() {
        if let Some(writer) = guard.writer() {
            if let Err(e) = writer.mark_clean_exit() {
                eprintln!("thuki: [startup_guard] failed to mark clean exit: {e}");
            }
        }
    }
}

/// Kills the built-in engine sidecar under a bounded timeout (issue #296).
///
/// The `sigwait` shutdown thread runs on an ordinary thread and may block, but
/// it must not block forever: a wedged sidecar would otherwise keep the thread
/// from re-raising the caught signal, turning a polite `SIGTERM` at macOS
/// restart into an app that goes unresponsive past the OS kill deadline. Kill
/// the sidecar under `SHUTDOWN_SIGNAL_ENGINE_KILL_TIMEOUT_SECS`; on timeout the
/// caller proceeds to re-raise on schedule, and any sidecar that outlived the
/// bound is reaped at the next launch.
///
/// Runs AFTER the durable clean-exit write in the signal thread: that write is
/// the sole safe-mode input and has no backstop but itself, whereas this
/// shutdown has the next-launch orphan reaper as its backstop.
///
/// Coverage-off: thin runtime/FFI glue (`block_on` + `timeout` over the engine
/// actor), with no branch logic of its own to test. `block_on` here cannot
/// deadlock: the actor runs on Tauri's tokio runtime, mirroring `RunEvent::Exit`.
#[cfg_attr(coverage_nightly, coverage(off))]
fn shutdown_engine_bounded(app: &tauri::AppHandle) {
    let Some(engine) = app.try_state::<engine::runner::EngineHandle>() else {
        return;
    };
    let engine = engine.inner().clone();
    tauri::async_runtime::block_on(async move {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(
                crate::config::defaults::SHUTDOWN_SIGNAL_ENGINE_KILL_TIMEOUT_SECS,
            ),
            engine.shutdown(),
        )
        .await;
    });
}

/// Handles a quit request from the app menu or the tray: warn when a download
/// would be lost, otherwise quit immediately.
#[cfg_attr(coverage_nightly, coverage(off))]
fn request_quit(app: &tauri::AppHandle) {
    if should_warn_on_quit(app) {
        show_quit_dialog(app);
    } else {
        app.state::<crate::commands::GenerationState>().cancel();
        app.exit(0);
    }
}

/// Shows the native "quit while a model is downloading" warning. "Quit Anyway"
/// records the confirmation and exits; "Keep Downloading" cancels the quit.
/// Non-blocking, and deduplicated via `QUIT_DIALOG_OPEN` so the two quit paths
/// that both fire on Cmd+Q show a single dialog.
#[cfg_attr(coverage_nightly, coverage(off))]
fn show_quit_dialog(app: &tauri::AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    if QUIT_DIALOG_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    app.dialog()
        .message(
            "Quitting stops the model download and you'll have to start it over.\n\nTo keep it downloading in the background, just close Thuki instead (double-tap Control to reopen).",
        )
        .title("Quit while a model is downloading?")
        .kind(MessageDialogKind::Warning)
        // "Keep Downloading" is the primary/highlighted button (the default on
        // Enter): the safe choice for a destructive action. "Quit Anyway" is the
        // secondary. The callback's bool is true for the primary button.
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Keep Downloading".to_string(),
            "Quit Anyway".to_string(),
        ))
        .show(move |keep_downloading| {
            QUIT_DIALOG_OPEN.store(false, Ordering::SeqCst);
            if !keep_downloading {
                QUIT_CONFIRMED.store(true, Ordering::SeqCst);
                handle
                    .state::<crate::commands::GenerationState>()
                    .cancel();
                handle.exit(0);
            }
        });
}

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

/// Emits a restore request and marks the overlay intended-visible.
///
/// A restore makes the parked (minimized) overlay visible again, so
/// `OVERLAY_INTENDED_VISIBLE` must agree. If it were left `false` (it can be
/// after a hide that raced the minimized state), the next `toggle_overlay`
/// would read it and re-show instead of hiding the now-visible overlay.
fn emit_overlay_restore(app_handle: &tauri::AppHandle) {
    OVERLAY_INTENDED_VISIBLE.store(true, Ordering::SeqCst);
    emit_overlay_visibility(
        app_handle,
        OVERLAY_VISIBILITY_RESTORE,
        None,
        None,
        None,
        None,
    );
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
    // Onboarding owns the main window (fixed 460x640, centered). Running the
    // ask-bar show path here would reposition the window and emit a `show`
    // visibility event, and the frontend would then collapse the still-
    // onboarding window to the ask-bar size. Bring the onboarding window
    // forward instead so "Open Thuki" / the hotkey still surface it.
    if ONBOARDING_ACTIVE.load(Ordering::SeqCst) {
        if let Ok(panel) = app_handle.get_webview_panel("main") {
            panel.show_and_make_key();
        }
        return;
    }
    if take_minimized_for_restore() {
        emit_overlay_restore(app_handle);
        return;
    }
    let already_visible = OVERLAY_INTENDED_VISIBLE.swap(true, Ordering::SeqCst);
    if already_visible {
        return;
    }

    // Pre-load the active model so the user's first message does not pay
    // the cold-start penalty. Fires on all show paths: double-tap, tray,
    // and first-launch auto-show. Branches by the active provider's kind:
    // Ollama warms via its native /api/chat, the built-in engine starts
    // (or reuses) its sidecar and primes the KV cache, and openai providers
    // get no warmup (nothing local to warm).
    //
    // Launch circuit breaker (issue #296): after an unclean-launch streak we
    // are in safe mode, and this no-user-action model auto-load is exactly what
    // froze the machine. In safe mode we resolve an empty provider kind so the
    // match below falls through to its no-op arm, skipping BOTH the built-in
    // and Ollama auto-prime. This is broader than the literal "skip
    // warm_builtin" ask on purpose: both branches are no-user-action model
    // loads. Safe mode only DEFERS the load to user action, it does not disable
    // inference: the model still loads on the user's first message via
    // `ask_model`, which ensures the sidecar on demand. The rest of the
    // overlay-show path below runs unchanged. When safe_mode is false, behavior
    // is identical to before.
    let warmup_kind = if app_handle
        .state::<startup_guard::StartupSafety>()
        .safe_mode()
    {
        String::new()
    } else {
        app_handle
            .state::<parking_lot::RwLock<crate::config::AppConfig>>()
            .read()
            .inference
            .active_provider_kind()
            .to_string()
    };
    match warmup_kind.as_str() {
        crate::config::defaults::PROVIDER_KIND_OLLAMA => {
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
                    warmup_config
                        .inference
                        .active_provider_base_url()
                        .trim_end_matches('/')
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
        }
        crate::config::defaults::PROVIDER_KIND_BUILTIN => {
            let (model_id, num_ctx, system_prompt) = {
                let cfg_state = app_handle.state::<parking_lot::RwLock<crate::config::AppConfig>>();
                let cfg = cfg_state.read();
                (
                    cfg.inference.active_provider_model().to_string(),
                    cfg.inference.num_ctx,
                    cfg.prompt.resolved_system.clone(),
                )
            };
            // Run the pre-load memory gate (issue #296) before this
            // no-user-action auto-load: the shared helper resolves the target,
            // consults `preflight_memory_gate`, and spawns the warmup only when
            // it clears. This is the same gated path the `warm_up_model`
            // command uses, so the overlay-show trigger can never again slip
            // an oversized model into memory ungated.
            let engine = app_handle
                .state::<engine::runner::EngineHandle>()
                .inner()
                .clone();
            let client = app_handle.state::<reqwest::Client>().inner().clone();
            let store = app_handle.state::<models::storage::ModelStore>();
            let db = app_handle.state::<history::Database>();
            // why: `force = false`. Overlay-show is an automatic load with no
            // user action, so an oversized model must never be shoved into
            // memory here; only the user's explicit "Load anyway" (through the
            // `warm_up_model` command) passes `true` (issue #296).
            warmup::spawn_gated_builtin_warmup(
                app_handle.clone(),
                engine,
                &store,
                &db,
                model_id,
                num_ctx,
                system_prompt,
                client,
                false,
            );
        }
        _ => {}
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

/// Centers the settings window dead-center on its monitor (horizontally and
/// vertically). Called every time the settings window is shown so it spawns at
/// the center regardless of the OS-default spawn position or where a previous
/// session left it; it is not called again while open, so the user can drag it
/// freely without it snapping back.
#[cfg_attr(coverage_nightly, coverage(off))]
fn position_settings_window(window: &tauri::WebviewWindow) {
    const SETTINGS_WIDTH: f64 = 760.0;

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
        let logical_h = size.height as f64 / scale;
        let mon_x = pos.x as f64 / scale;
        let mon_y = pos.y as f64 / scale;
        // Use the window's actual height so the vertical center is exact; fall
        // back to top-aligned if the size query fails.
        let win_h = window
            .outer_size()
            .map(|s| s.height as f64 / scale)
            .unwrap_or(0.0);
        (
            mon_x + (logical_w - SETTINGS_WIDTH) / 2.0,
            mon_y + (logical_h - win_h) / 2.0,
        )
    } else {
        (100.0, 100.0)
    };

    let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
}

/// Shows or hides the Dock icon to match `wants_regular_activation`: visible
/// (Dock icon + foreground window layering) while Settings or onboarding owns a
/// real window, hidden (Dock-less floating overlay) otherwise. Runs on the macOS
/// main thread (AppKit is not thread-safe).
///
/// Uses Tauri's `set_dock_visibility`, which drives `TransformProcessType`
/// (foreground <-> UIElement) under the hood. That is the API that reliably
/// *removes* the Dock icon at runtime; a plain `setActivationPolicy(.accessory)`
/// downgrade does not drop the icon once the app has been foreground, which is
/// why the earlier attempts left it stuck on. `set_dock_visibility` also guards
/// the macOS multiple-icon bug by ignoring a hide within ~1s of a show.
///
/// On show it additionally activates the app so the just-opened window orders to
/// the front instead of appearing behind whatever was focused.
///
/// Thin wrapper, excluded from coverage; the decision it reads
/// (`wants_regular_activation`) is unit-tested directly.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn sync_activation_policy(app_handle: &tauri::AppHandle) {
    let regular = wants_regular_activation();
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        // `set_dock_visibility` drives TransformProcessType, the API that
        // reliably adds and removes the Dock icon at runtime.
        let _ = handle.set_dock_visibility(regular);
        if regular {
            activate_app();
        }
    });
}

/// Activates (foregrounds) the app so a newly opened Settings/onboarding window
/// orders front. `activateIgnoringOtherApps` is deprecated on macOS 14+ but
/// still functional and is the broadest-compatible call (Thuki supports macOS
/// 13.4+). Must be called on the macOS main thread.
///
/// Thin AppKit wrapper, excluded from coverage.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn activate_app() {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if !app.is_null() {
            let _: () = msg_send![app, activateIgnoringOtherApps: true];
        }
    }
}

/// Shows (or focuses, if already visible) the Settings window.
///
/// The settings window is converted to a ThukiSettingsPanel NSPanel subclass
/// (done once in `init_settings_panel` during setup). While it is open the app
/// switches to `Regular` activation (`sync_activation_policy`): the panel sits
/// at normal window level so it opens on top yet lets another app clicked
/// afterwards rise above it, and a Dock icon appears so a user who clicks away
/// can return. Closing the window restores `Accessory` (see the `settings`
/// close handler).
///
/// Idempotent: invoking while Settings is already visible re-focuses without
/// double-mounting the React tree (close handler hides instead of destroying).
///
/// Falls back to raw WebviewWindow show/focus if the panel handle is
/// unavailable (e.g., if init_settings_panel failed at startup).
fn show_settings_window(app_handle: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        // Flip to Regular activation (Dock icon + normal layering) and activate
        // so the window orders front. Queued on the main thread before the show
        // below, so the policy is in place as the panel appears.
        SETTINGS_OPEN.store(true, Ordering::SeqCst);
        sync_activation_policy(app_handle);
        let window = app_handle.get_webview_window("settings");
        match app_handle.get_webview_panel("settings") {
            Ok(panel) => {
                let _ = app_handle.run_on_main_thread(move || {
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

/// Frontend entry point for opening the Settings window. The tray menu reaches
/// `show_settings_window` directly; this exposes the same path to the UI (the
/// in-overlay model picker links "Settings" here when no model is installed yet).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn open_settings_window(app_handle: tauri::AppHandle) {
    show_settings_window(&app_handle);
    // The only caller is the picker's "no model yet" link, so always route to
    // the Discover download picker. The Settings window listens for this and
    // jumps to Models -> Discover (Staff picks is Discover's default).
    let _ = app_handle.emit(SETTINGS_SHOW_DISCOVER_EVENT, ());
}

/// Opens the Settings window straight on the Models tab's Providers pane. The
/// ask-bar "Ollama isn't running" strip links here from its "switch to Built-in"
/// action so the user lands on the provider switcher and can flip the active
/// provider back to the built-in engine themselves.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn open_settings_to_providers(app_handle: tauri::AppHandle) {
    show_settings_window(&app_handle);
    let _ = app_handle.emit(SETTINGS_SHOW_PROVIDERS_EVENT, ());
}

/// Closes (hides) the Settings window from the frontend and drops the Dock icon.
///
/// The Settings window is closed by the frontend (its close button and Cmd+W),
/// which calls this instead of `getCurrentWindow().hide()` directly. Routing the
/// close through the backend is what lets the Dock-icon state stay correct: a
/// raw `hide()` never reaches the Rust side, so `SETTINGS_OPEN` would stay set
/// and the Dock icon would never come back down. Clearing the flag here and
/// re-syncing hides the Dock icon (unless onboarding still owns a window). The
/// window is hidden rather than destroyed so its React state survives reopen.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn hide_settings_window(app_handle: tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    SETTINGS_OPEN.store(false, Ordering::SeqCst);
    if let Some(window) = app_handle.get_webview_window("settings") {
        let _ = window.hide();
    }
    #[cfg(target_os = "macos")]
    sync_activation_policy(&app_handle);
}

/// Centers the "What's New" update window horizontally on its monitor and
/// places it below the macOS menu bar, mirroring `position_settings_window`
/// but for the update window's 600 px width.
#[cfg_attr(coverage_nightly, coverage(off))]
fn position_update_window(window: &tauri::WebviewWindow) {
    const UPDATE_WIDTH: f64 = 600.0;
    const TOP_MARGIN: f64 = 72.0;

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
        (mon_x + (logical_w - UPDATE_WIDTH) / 2.0, mon_y + TOP_MARGIN)
    } else {
        (100.0, TOP_MARGIN)
    };

    let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
}

/// Shows (or focuses, if already visible) the "What's New" update window.
///
/// Mirrors `show_settings_window`: the window is converted to a
/// `ThukiUpdatePanel` NSPanel subclass once during setup (`init_update_panel`),
/// and while it is open the app flips to `Regular` activation
/// (`sync_activation_policy`, gated on `UPDATE_OPEN`) so it behaves like a
/// standard app window. The activation also pulls the user to the window's Space
/// when it is summoned from over a fullscreen app. Closing it restores
/// `Accessory` (see the `update` close handler).
///
/// Idempotent: invoking while the window is already visible re-focuses
/// without re-mounting the React tree (the close handler hides instead of
/// destroying).
///
/// Falls back to raw WebviewWindow show/focus if the panel handle is
/// unavailable (e.g., if `init_update_panel` failed at startup).
fn show_update_window(app_handle: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        // Flip to Regular activation (Dock icon + normal layering) and activate
        // so the window orders front and macOS pulls the user to its Space when
        // it is summoned from over a fullscreen app. Mirrors show_settings_window.
        UPDATE_OPEN.store(true, Ordering::SeqCst);
        sync_activation_policy(app_handle);
        let window = app_handle.get_webview_window("update");
        match app_handle.get_webview_panel("update") {
            Ok(panel) => {
                let _ = app_handle.run_on_main_thread(move || {
                    if let Some(win) = window {
                        position_update_window(&win);
                    }
                    panel.show_and_make_key();
                });
                return;
            }
            Err(e) => {
                eprintln!("thuki: [update] get_webview_panel failed: {e:?}");
            }
        }
    }
    let Some(window) = app_handle.get_webview_window("update") else {
        eprintln!("thuki: [update] window 'update' not found in app config");
        return;
    };
    position_update_window(&window);
    let _ = window.show();
    let _ = window.set_focus();
}

/// Requests an animated hide sequence from the frontend. The actual native
/// window hide is deferred until the frontend exit animation completes.
fn request_overlay_hide(app_handle: &tauri::AppHandle) {
    // A parked (minimized) conversation must survive a stray close request.
    // While minimized the icon is a small NSPanel that can still receive
    // Cmd+W / a system close, which routes here; hiding it would tear down the
    // background stream the user explicitly minimized to keep running. Ignore
    // the hide while minimized: the user restores first, then closes normally.
    if OVERLAY_MINIMIZED.load(Ordering::SeqCst) {
        return;
    }
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
    if ONBOARDING_ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    if take_minimized_for_restore() {
        emit_overlay_restore(app_handle);
        return;
    }
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
    if take_minimized_for_restore() {
        emit_overlay_restore(app_handle);
        return;
    }
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

/// Computes the AppKit target frame for a resize that keeps the window's
/// visual top-left corner fixed.
///
/// AppKit screen coordinates are bottom-left origin (Y grows upward), so the
/// current top edge is `origin.y + height`. To keep that top edge (and the
/// left edge) stationary while the size changes, the new origin's Y must be
/// `top - new_height`. This is purely relative to the current frame: no screen
/// lookup, no multi-monitor math, no absolute Y-flip. Robust by construction.
///
/// `cur` is `(origin_x, origin_y, width, height)`; the return is the same shape
/// for the target frame.
fn compute_top_left_anchored_target(
    cur: (f64, f64, f64, f64),
    w: f64,
    h: f64,
) -> (f64, f64, f64, f64) {
    let (cur_x, cur_y, _cur_w, cur_h) = cur;
    let top = cur_y + cur_h;
    let new_y = top - h;
    (cur_x, new_y, w, h)
}

/// Animates the main overlay NSPanel/NSWindow from its current native frame to
/// a new size, keeping the visual top-left corner fixed, using
/// `NSAnimationContext` so the OS compositor (Core Animation) drives the
/// animation. One IPC call per morph direction replaces the old per-frame
/// `setSize` storm.
///
/// Excluded from coverage: thin wrapper over AppKit `NSWindow`/
/// `NSAnimationContext` FFI that requires a real window and the macOS main
/// thread. The pure geometry is covered by
/// `compute_top_left_anchored_target`'s unit test.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn animate_overlay_frame(app_handle: tauri::AppHandle, width: f64, height: f64, duration_ms: f64) {
    // Never panic on bad input: reject non-finite / non-positive dimensions
    // and clamp the duration to a sane range. A missing window handle is a
    // silent no-op.
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return;
    }
    let duration_ms = if duration_ms.is_finite() {
        duration_ms.clamp(0.0, 2000.0)
    } else {
        0.0
    };

    #[cfg(target_os = "macos")]
    {
        use objc2::encode::{Encode, Encoding, RefEncode};
        use objc2::rc::autoreleasepool;
        use objc2::runtime::AnyObject;
        use objc2::{class, msg_send};

        // Local NSRect/NSPoint/NSSize. `core_graphics::geometry::CGRect` does
        // not implement objc2's `Encode`, so it cannot be a `msg_send!`
        // return/argument type. NSRect uses CGFloat = f64 on macOS and is
        // layout-compatible with the AppKit `NSWindow` frame ABI.
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct NSPoint {
            x: f64,
            y: f64,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct NSSize {
            width: f64,
            height: f64,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct NSRect {
            origin: NSPoint,
            size: NSSize,
        }

        unsafe impl Encode for NSPoint {
            const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
        }
        unsafe impl Encode for NSSize {
            const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
        }
        unsafe impl Encode for NSRect {
            const ENCODING: Encoding =
                Encoding::Struct("CGRect", &[NSPoint::ENCODING, NSSize::ENCODING]);
        }
        unsafe impl RefEncode for NSRect {
            const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
        }

        let handle = app_handle.clone();
        let _ = app_handle.run_on_main_thread(move || {
            let Some(window) = handle.get_webview_window("main") else {
                return;
            };
            let Ok(ns_window) = window.ns_window() else {
                return;
            };
            if ns_window.is_null() {
                return;
            }
            let win = ns_window as *mut AnyObject;

            autoreleasepool(|_| unsafe {
                let cur: NSRect = msg_send![win, frame];
                let (tx, ty, tw, th) = compute_top_left_anchored_target(
                    (cur.origin.x, cur.origin.y, cur.size.width, cur.size.height),
                    width,
                    height,
                );
                let target = NSRect {
                    origin: NSPoint { x: tx, y: ty },
                    size: NSSize {
                        width: tw,
                        height: th,
                    },
                };

                // duration 0 is the invisible endpoint snap used by the
                // in-page morph: the painted web content already matches the
                // target, so the OS frame must change instantly. The animator
                // proxy still tweens (briefly) even at duration 0, so bypass
                // NSAnimationContext entirely and set the frame directly on
                // the window for a true immediate, non-animated change.
                if duration_ms == 0.0 {
                    let _: () = msg_send![win, setFrame: target, display: true];
                } else {
                    let ctx_cls = class!(NSAnimationContext);
                    let _: () = msg_send![ctx_cls, beginGrouping];
                    let ctx: *mut AnyObject = msg_send![ctx_cls, currentContext];
                    let _: () = msg_send![ctx, setDuration: duration_ms / 1000.0];
                    let animator: *mut AnyObject = msg_send![win, animator];
                    let _: () = msg_send![animator, setFrame: target, display: true];
                    let _: () = msg_send![ctx_cls, endGrouping];
                }
            });
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_handle, width, height, duration_ms);
    }
}

/// Sets the alpha (opacity) of the main overlay NSPanel.
///
/// Used to temporarily hide Thuki while a foreign system dialog (the
/// `NSSavePanel` invoked by the export flow) is on screen. That dialog
/// ships with its own drop-shadow and `NSVisualEffectView` vibrancy
/// backdrop, both of which bleed onto anything behind them. Thuki's
/// transparent CSS shadow margin would otherwise show through as a
/// dark "ghost" rectangle around the card.
///
/// Driving alpha to 0 removes Thuki from the compositor for the
/// duration of the dialog without disturbing the NSPanel's state
/// machine, the activator, the trace recorder, or the React tree.
/// Restoring alpha to 1.0 paints the window again with the exact
/// same content it had before. Cheap, idempotent, and unrelated to
/// the window-resize path that the tighten-to-card approach broke.
///
/// When `duration_ms > 0` the transition is driven through
/// `NSAnimationContext` so the alpha change overlaps the dialog's
/// own fade-in / fade-out. With `duration_ms = 0` the alpha is set
/// instantly. Hiding the panel usually wants `0` (snap out so the
/// dialog's appearance is the only motion the user reads); restoring
/// usually wants a small duration so Thuki gracefully fades back in
/// instead of popping over the dialog dismiss animation.
///
/// Non-finite values are silently dropped and the magnitude is clamped
/// to `[0.0, 1.0]` so the IPC boundary stays forgiving. Duration is
/// clamped to `[0.0, 2000.0]` ms for the same reason.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn set_overlay_alpha(app_handle: tauri::AppHandle, alpha: f64, duration_ms: f64) {
    if !alpha.is_finite() {
        return;
    }
    let alpha = alpha.clamp(0.0, 1.0);
    let duration_ms = if duration_ms.is_finite() {
        duration_ms.clamp(0.0, 2000.0)
    } else {
        0.0
    };

    #[cfg(target_os = "macos")]
    {
        use objc2::class;
        use objc2::msg_send;
        use objc2::runtime::AnyObject;

        let handle = app_handle.clone();
        let _ = app_handle.run_on_main_thread(move || {
            let Some(window) = handle.get_webview_window("main") else {
                return;
            };
            let Ok(ns_window) = window.ns_window() else {
                return;
            };
            if ns_window.is_null() {
                return;
            }
            let win = ns_window as *mut AnyObject;
            unsafe {
                if duration_ms == 0.0 {
                    let _: () = msg_send![win, setAlphaValue: alpha];
                } else {
                    let ctx_cls = class!(NSAnimationContext);
                    let _: () = msg_send![ctx_cls, beginGrouping];
                    let ctx: *mut AnyObject = msg_send![ctx_cls, currentContext];
                    let _: () = msg_send![ctx, setDuration: duration_ms / 1000.0];
                    let animator: *mut AnyObject = msg_send![win, animator];
                    let _: () = msg_send![animator, setAlphaValue: alpha];
                    let _: () = msg_send![ctx_cls, endGrouping];
                }
            }
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_handle, alpha, duration_ms);
    }
}

/// Sets the main overlay NSPanel's alpha instantly. Unlike `set_overlay_alpha`
/// (a command that dispatches its own main-thread hop), this is a synchronous
/// helper meant to be called from code already running on the macOS main
/// thread. It is the onboarding -> overlay handoff's cover: dropping alpha to 0
/// before the window is resized to the ask bar lets the resize and the
/// not-yet-swapped intro card happen invisibly, so the card is never seen
/// squished into the 600x80 bar for a frame.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn set_main_window_alpha_now(window: &WebviewWindow, alpha: f64) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    let Ok(ns_window) = window.ns_window() else {
        return;
    };
    if ns_window.is_null() {
        return;
    }
    let win = ns_window as *mut AnyObject;
    unsafe {
        let _: () = msg_send![win, setAlphaValue: alpha];
    }
}

/// Generation counter for the onboarding-window reveal backstop. Every cover
/// bumps it; a backstop only reveals if its generation is still current, so a
/// stale backstop armed for an earlier transition can never reveal a later
/// cover (which would flash that later screen's intermediate frames).
#[cfg(target_os = "macos")]
static ONBOARDING_REVEAL_GEN: AtomicU64 = AtomicU64::new(0);

/// How long after a cover the onboarding panel is unconditionally faded back
/// in, even if the frontend's settle-based reveal never fires. Generous enough
/// that the frontend nice-path reveal normally wins; short enough that a missed
/// reveal can never leave the panel invisible for long.
#[cfg(target_os = "macos")]
const ONBOARDING_REVEAL_BACKSTOP: std::time::Duration = std::time::Duration::from_millis(800);

/// Covers the onboarding panel (alpha 0) for a transition and arms an
/// unconditional reveal backstop.
///
/// Each onboarding transition resizes and recenters the window while the React
/// tree is still mid-swap, which would flash the old card at the new size. The
/// fix is to drop the panel invisible across the swap; the frontend fades it
/// back in (`set_overlay_alpha`) once the new screen has settled (see
/// `useFitOnboardingWindow`). This backstop guarantees the panel is revealed
/// even if that frontend reveal is missed, so onboarding can never get stuck on
/// an invisible window. Gen-guarded so only the latest cover's backstop fires.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn arm_onboarding_reveal_backstop(app_handle: &tauri::AppHandle) {
    let generation = ONBOARDING_REVEAL_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(ONBOARDING_REVEAL_BACKSTOP).await;
        if ONBOARDING_REVEAL_GEN.load(Ordering::SeqCst) != generation {
            return;
        }
        let inner = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if let Some(window) = inner.get_webview_window("main") {
                set_main_window_alpha_now(&window, 1.0);
            }
        });
    });
}

/// Sets the default appearance of `NSSavePanel` (and its `NSOpenPanel`
/// sibling) to the **compact** layout — no sidebar, no file browser,
/// just the Save As field, a Where popup, and the action buttons.
///
/// macOS persists the expansion state of these panels per app under
/// the `NSNavPanelExpandedStateForSaveMode` key in `NSUserDefaults`.
/// On a fresh launch the panel inherits the system default, which is
/// the wide expanded layout most apps want. For a Spotlight-style
/// overlay like Thuki where export is a quick action invoked from a
/// floating bar, the compact layout reads as the right shape: the
/// user already picked the file in their head, they just need to
/// confirm the name and location.
///
/// Writing the key at startup means every save dialog opens compact
/// on a fresh launch. Within a session, macOS rewrites the key when
/// the user manually toggles the disclosure triangle, so their
/// per-save preference is respected until the next launch.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn apply_save_panel_compact_default() {
    use objc2::class;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_foundation::ns_string;

    let key = ns_string!("NSNavPanelExpandedStateForSaveMode");
    unsafe {
        let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
        if defaults.is_null() {
            return;
        }
        let _: () = msg_send![defaults, setBool: false, forKey: key];
    }
}

/// Synchronizes the Rust-side visibility tracking when the frontend
/// completes its exit animation and hides the native window.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn notify_overlay_hidden(generation: tauri::State<crate::commands::GenerationState>) {
    generation.cancel();
    OVERLAY_INTENDED_VISIBLE.store(false, Ordering::SeqCst);
    // The overlay is now fully hidden, so it can no longer be parked in the
    // minimized icon. Clearing the flag here prevents it leaking `true` across
    // a hide and routing the next activation to a restore of a gone window.
    OVERLAY_MINIMIZED.store(false, Ordering::SeqCst);
}

fn set_overlay_minimized_impl(minimized: bool) {
    OVERLAY_MINIMIZED.store(minimized, Ordering::SeqCst);
}

/// Returns true and clears the flag if the overlay was minimized. Used by
/// the activator layer to route any activation to a restore instead of a
/// show or hide while a conversation is parked.
fn take_minimized_for_restore() -> bool {
    OVERLAY_MINIMIZED.swap(false, Ordering::SeqCst)
}

#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn set_overlay_minimized(minimized: bool) {
    set_overlay_minimized_impl(minimized);
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
                // Use the persisted stage as-is. A built-in model download
                // still in flight no longer bounces the user back to the
                // picker: on a mid-download relaunch they stay where they left
                // off, and the DownloadProvider auto-resumes the partial in the
                // background so the ambient strip is the recovery surface (the
                // user is never stranded past selection with no usable model).
                let stage = onboarding::get_stage(&conn)
                    .unwrap_or(onboarding::OnboardingStage::Permissions);

                let active_kind = app_handle
                    .state::<parking_lot::RwLock<crate::config::AppConfig>>()
                    .read()
                    .inference
                    .active_provider_kind()
                    .to_string();
                let announced = onboarding::is_builtin_announced(&conn).unwrap_or(false);

                // Latch the built-in announcement at the one reliable moment: a
                // pre-built-in install that already finished onboarding is at
                // `Complete` on Ollama on its first launch of the new version,
                // before the permission flow can clobber the stage. The latch
                // then survives that flow, so the notice is shown wherever it
                // lands the user (Intro, Complete, or the model gate). A fresh
                // install is never `Complete` before finishing, so it never
                // latches.
                let mut pending = onboarding::is_announcement_pending(&conn).unwrap_or(false);
                if !pending && onboarding::is_pre_builtin_upgrader(&stage, &active_kind, announced)
                {
                    let _ = onboarding::set_announcement_pending(&conn);
                    pending = true;
                }

                // Live permission grants feed the pure router; reading them here
                // keeps the routing decision side-effect free and unit-tested.
                let ax = permissions::is_accessibility_granted();
                let sr = permissions::is_screen_recording_granted();

                match onboarding::decide_startup_route(&stage, ax, sr, announced, pending) {
                    onboarding::StartupRoute::ShowPermissions => {
                        let _ =
                            onboarding::set_stage(&conn, &onboarding::OnboardingStage::Permissions);
                        show_onboarding_window(
                            &app_handle,
                            onboarding::OnboardingStage::Permissions,
                        );
                        return;
                    }
                    onboarding::StartupRoute::ShowAnnouncement => {
                        let _ = onboarding::set_stage(
                            &conn,
                            &onboarding::OnboardingStage::BuiltinAnnouncement,
                        );
                        show_onboarding_window(
                            &app_handle,
                            onboarding::OnboardingStage::BuiltinAnnouncement,
                        );
                        return;
                    }
                    onboarding::StartupRoute::ShowModelCheck => {
                        let _ =
                            onboarding::set_stage(&conn, &onboarding::OnboardingStage::ModelCheck);
                        show_onboarding_window(
                            &app_handle,
                            onboarding::OnboardingStage::ModelCheck,
                        );
                        return;
                    }
                    onboarding::StartupRoute::ShowIntro => {
                        show_onboarding_window(&app_handle, onboarding::OnboardingStage::Intro);
                        return;
                    }
                    // Complete and permissions intact: fall through to the
                    // overlay show below.
                    onboarding::StartupRoute::ShowOverlay => {}
                }
            } else {
                // Mutex poisoned; safe fallback.
                show_onboarding_window(&app_handle, onboarding::OnboardingStage::Permissions);
                return;
            }
        }
        show_overlay(&app_handle, crate::context::ActivationContext::empty());
    }
}

/// Returns the persisted onboarding stage for the frontend's launch
/// auto-resume gate. The model-check picker owns the resume decision while it
/// is shown (its own Resume / Discard choice), so the `DownloadProvider` only
/// auto-resumes an interrupted built-in download once the user is past it (the
/// intro tour or the ask bar). Thin wrapper over the tested `get_stage`.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn onboarding_stage(
    db: tauri::State<history::Database>,
) -> Result<onboarding::OnboardingStage, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    onboarding::get_stage(&conn).map_err(|e| e.to_string())
}

/// Whether the built-in engine announcement has been latched. The starter
/// picker reads this to tell an upgrader (who reached the picker through the
/// announcement, which sets the flag) from a brand-new user (not yet announced):
/// the "use my existing Ollama instead" escape hatch is only offered to
/// brand-new users, since an upgrader already made that choice on the
/// announcement. Thin wrapper over the tested `is_builtin_announced`.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn is_builtin_announced(db: tauri::State<history::Database>) -> Result<bool, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    onboarding::is_builtin_announced(&conn).map_err(|e| e.to_string())
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

    // Cover the model_check -> intro swap (the picker shrinks to the intro card)
    // by dropping the panel invisible before the frontend switches, then letting
    // the intro step fade it back in once it has fitted. The emit fires inside
    // the same main-thread closure, after the cover, so the frontend never gets
    // the event before the panel is hidden.
    #[cfg(target_os = "macos")]
    {
        arm_onboarding_reveal_backstop(&app_handle);
        let handle = app_handle.clone();
        let _ = app_handle.run_on_main_thread(move || {
            if let Some(window) = handle.get_webview_window("main") {
                set_main_window_alpha_now(&window, 0.0);
            }
            let _ = handle.emit(
                ONBOARDING_EVENT,
                OnboardingPayload {
                    stage: onboarding::OnboardingStage::Intro,
                },
            );
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app_handle.emit(
            ONBOARDING_EVENT,
            OnboardingPayload {
                stage: onboarding::OnboardingStage::Intro,
            },
        );
    }
    Ok(())
}

/// Advances onboarding from the built-in engine announcement to the model
/// check, latching the announcement so it never returns.
///
/// Called by both branches of `BuiltinAnnouncementStep`: "Try Built-in Engine"
/// (after the frontend switches the active provider to the built-in engine via
/// `set_active_provider`) and "Keep using Ollama" (provider unchanged). The
/// resized `model_check` window then renders the built-in starter picker or the
/// Ollama setup gate based on the now-active provider, so no provider logic is
/// duplicated here.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn advance_past_builtin_announcement(
    db: tauri::State<history::Database>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
    onboarding::mark_builtin_announced(&conn).map_err(|e| format!("db write failed: {e}"))?;
    onboarding::set_stage(&conn, &onboarding::OnboardingStage::ModelCheck)
        .map_err(|e| format!("db write failed: {e}"))?;
    drop(conn);

    // Resize/recenter to the model-check window and emit the stage event so the
    // frontend swaps to the picker / Ollama gate.
    show_onboarding_window(&app_handle, onboarding::OnboardingStage::ModelCheck);
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
    // Latch the built-in engine announcement for fresh installs too: a new user
    // has already met the picker, so a later switch to Ollama in Settings must
    // not resurface an "upgrade" notice they never needed. Best-effort: a flag
    // write hiccup must not block onboarding completion.
    let _ = onboarding::mark_builtin_announced(&conn);
    drop(conn);

    // Onboarding no longer owns the window; release the gate before the
    // overlay show below (otherwise show_overlay would gate itself out).
    set_onboarding_active_impl(false);
    // Back to the Dock-less Accessory overlay now that no real window is open.
    #[cfg(target_os = "macos")]
    sync_activation_policy(&app_handle);

    // The handoff below covers the panel (alpha 0) and resizes it to the ask bar
    // under cover; IntroStep fades it back in once the ask bar has painted. Arm
    // the same backstop the step transitions use so the panel can never stay
    // invisible if that frontend reveal is missed.
    #[cfg(target_os = "macos")]
    arm_onboarding_reveal_backstop(&app_handle);

    // Restore panel to overlay configuration and show the Ask Bar.
    // Must run on the macOS main thread because NSPanel APIs are not thread-safe.
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        let window = handle.get_webview_window("main");
        if let Some(window) = &window {
            // Cover the swap: drop the panel to alpha 0 before resizing it to
            // the ask bar. The resize and the still-mounted intro card then
            // happen invisibly; the frontend swaps to the ask bar and fades the
            // panel back in once it has painted (see IntroStep), so the intro
            // card is never seen squished into the 600x80 bar for a frame. This
            // reuses the same alpha bracket the export flow uses to hide the
            // panel without disturbing its state machine or the React tree.
            #[cfg(target_os = "macos")]
            set_main_window_alpha_now(window, 0.0);
            // Resize the window back to the collapsed overlay dimensions before
            // positioning, so the overlay appears at the correct size.
            let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                OVERLAY_LOGICAL_WIDTH,
                OVERLAY_LOGICAL_HEIGHT_COLLAPSED,
            )));
        }
        // Restore NSPanel level, shadow, and style that show_onboarding_window
        // changed for the onboarding appearance.
        #[cfg(target_os = "macos")]
        init_panel(&handle);
        // `init_panel` re-converts the panel and rewrites its style mask, which
        // can reset window visual properties (it re-asserts clearColor for the
        // same reason). Re-assert the alpha cover so the swap stays hidden
        // through `init_panel` right up to the show below, regardless of whether
        // the resize and `init_panel` land in one compositor frame or several.
        #[cfg(target_os = "macos")]
        if let Some(window) = &window {
            set_main_window_alpha_now(window, 0.0);
        }
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

    // Hover-activate: take key focus the moment the cursor enters the overlay.
    // Pairs with the `active_always` tracking area declared on `ThukiPanel`.
    // A nonactivating panel cannot self-activate on modern macOS, so after the
    // overlay loses focus a click alone cannot regain key/active and the webview
    // drops clicks, drag, and hover. Making the panel key on mouse-enter (no app
    // activation) restores interaction without yanking the user off a fullscreen
    // Space. The handler is retained by `set_event_handler`, and the macro
    // forwards window-delegate events to wry's original delegate so window
    // resize/focus/close behavior is preserved.
    _thuki_panel::attach_overlay_event_handler(app_handle.clone());

    // Three NSPanel-layer assertions to keep the overlay visually clean
    // through the save-dialog flow, only one of which is strictly novel:
    //
    // 1. `setBackgroundColor: NSColor.clearColor` + `setOpaque: NO` -
    //    re-asserted because `to_panel::<ThukiPanel>()` plus the
    //    subsequent `set_style_mask` rewrite can leave the panel with
    //    `NSColor.windowBackgroundColor` painted into the backing layer.
    //
    // 2. `setWorksWhenModal: YES` - keeps the panel receiving keyboard
    //    and mouse events even while an application-modal session
    //    (NSSavePanel from `rfd`) is up. Per Apple docs this property
    //    controls event routing, NOT the AppKit modal dim - which is
    //    hardcoded on every non-modal window of the app and cannot be
    //    cleanly opted out of. Still worth setting so the panel stays
    //    interactive across the modal.
    //
    // 3. `contentView.layer.cornerRadius` + `masksToBounds` - the
    //    load-bearing fix for the visible halo around Thuki when the
    //    save dialog is up. AppKit's modal dim fills the entire NSPanel
    //    bounds, but the CSS chrome inside the WebView only paints a
    //    smaller rounded-rect (Tailwind `rounded-lg`, 8 px). The dim
    //    bleeds out from the dark CSS chrome and shows as a slate-gray
    //    annular halo. Clipping the content-view layer to the same
    //    rounded shape the CSS draws gives the dim no pixels to land on
    //    outside the chrome. Normal-state rendering is untouched: there
    //    is nothing to clip when the overlay is not being dimmed.
    //
    //    8 px matches `rounded-lg` used by the chat-mode chrome - the
    //    only state from which the save dialog can be launched (the
    //    export button only renders in chat mode and the chat-header
    //    handler gates on `messages.length > 0`). Ask-bar mode uses
    //    `rounded-2xl`
    //    (16 px), which produces a smaller visible CSS shape than this
    //    8 px content-view clip; the clip therefore has no visible
    //    effect in ask-bar mode (the smaller CSS shape is already
    //    inside the clip).
    if let Ok(ns_window) = window.ns_window() {
        if !ns_window.is_null() {
            use objc2::rc::autoreleasepool;
            use objc2::runtime::AnyObject;
            use objc2::{class, msg_send};
            let win = ns_window as *mut AnyObject;
            unsafe {
                autoreleasepool(|_| {
                    let clear: *mut AnyObject = msg_send![class!(NSColor), clearColor];
                    let _: () = msg_send![win, setBackgroundColor: clear];
                    let _: () = msg_send![win, setOpaque: false];
                    let _: () = msg_send![win, setWorksWhenModal: true];

                    let content_view: *mut AnyObject = msg_send![win, contentView];
                    if !content_view.is_null() {
                        let _: () = msg_send![content_view, setWantsLayer: true];
                        let layer: *mut AnyObject = msg_send![content_view, layer];
                        if !layer.is_null() {
                            let radius: f64 = 8.0;
                            let _: () = msg_send![layer, setCornerRadius: radius];
                            let _: () = msg_send![layer, setMasksToBounds: true];
                        }
                    }
                });
            }
        }
    }
}

// ─── Settings panel initialisation ──────────────────────────────────────────

/// Converts the settings Tauri window into a ThukiSettingsPanel NSPanel subclass.
///
/// Called once during app setup. The resulting panel handle is stored in the
/// tauri-nspanel WebviewPanelManager, so subsequent calls to
/// `get_webview_panel("settings")` retrieve the same panel without
/// re-converting.
///
/// While Settings is open the app runs under `Regular` activation
/// (`show_settings_window` -> `sync_activation_policy`), so the window behaves
/// like a normal macOS app window: it opens on top, another app the user clicks
/// afterwards rises above it, and a Dock icon offers a way back.
///
/// The collection behavior is `Managed` (the AppKit default for an ordinary app
/// window), deliberately NOT `can_join_all_spaces` + `full_screen_auxiliary`.
/// Those two are the overlay flags: they make a window appear on every Space,
/// follow the user across Space switches, and float over another app's
/// fullscreen Space. The ask-bar/chat overlay wants exactly that; Settings does
/// not. A `Managed` window is bound to a single Space, so it stays put when the
/// user swipes to a different fullscreen app, and the `Regular` activation above
/// pulls the user to the window's Space when Settings is summoned from over a
/// fullscreen app, matching how the system Settings app and other standard Mac
/// apps behave. `can_become_key_window` (set in the macro) keeps the Settings
/// form inputs focusable, and the `nonactivating_panel` style + the
/// hover-activate tracking area keep those inputs alive after the panel is
/// defocused without the app stealing focus. `hides_on_deactivate(false)` keeps
/// it open when the user clicks away.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn init_settings_panel(app_handle: &tauri::AppHandle) {
    let Some(window) = app_handle.get_webview_window("settings") else {
        eprintln!("thuki: [settings] window not found during init_settings_panel");
        return;
    };
    match window.to_panel::<ThukiSettingsPanel>() {
        Ok(panel) => {
            panel.set_floating_panel(true);
            // Normal window level (0), not Floating: while Settings is open the
            // app runs under Regular activation (see show_settings_window), so a
            // normal level lets another app the user clicks afterwards rise
            // above Settings instead of Settings staying pinned on top forever.
            panel.set_level(0);
            panel.set_style_mask(StyleMask::empty().nonactivating_panel().into());
            panel.set_has_shadow(true);
            panel.set_hides_on_deactivate(false);
            // `Managed`, not the overlay flags: bind Settings to a single Space
            // so it behaves like a standard app window (stays put when the user
            // swipes to another fullscreen app; the `Regular` activation in
            // show_settings_window pulls the user to its Space instead of
            // floating it over a fullscreen app). See the function doc comment.
            panel.set_collection_behavior(CollectionBehavior::new().managed().into());
            // Hover-activate: take key focus the moment the cursor enters the
            // Settings overlay, mirroring init_panel. Pairs with the
            // `active_always` tracking area on ThukiSettingsPanel so a defocused
            // nonactivating panel regains key without activating the app.
            _settings_panel::attach_settings_event_handler(app_handle.clone());
        }
        Err(e) => {
            eprintln!("thuki: [settings] NSPanel conversion failed: {e:?}");
        }
    }
}

/// Converts the update Tauri window into a ThukiUpdatePanel NSPanel
/// subclass. Called once during app setup.
///
/// Mirrors `init_settings_panel`, NOT `init_panel` (the overlay). The "What's
/// New" window is a window-style surface, so it should behave like a standard
/// macOS app window rather than a Space-following overlay. While it is open the
/// app runs under `Regular` activation (`show_update_window` ->
/// `sync_activation_policy`, gated on `UPDATE_OPEN`): it opens on top, another
/// app the user clicks afterwards rises above it (normal window level, not
/// floating), and a Dock icon offers a way back. The collection behavior is
/// `Managed` (bound to a single Space), so it stays put when the user swipes to
/// another fullscreen app, and the `Regular` activation pulls the user to the
/// window's Space when it is summoned from over a fullscreen app, instead of
/// floating over whatever Space they are on. `can_become_key_window` (set in the
/// macro) keeps the four action buttons clickable even though the panel is
/// nonactivating, and `hides_on_deactivate(false)` keeps it up if the user
/// clicks away without choosing an action.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn init_update_panel(app_handle: &tauri::AppHandle) {
    let Some(window) = app_handle.get_webview_window("update") else {
        eprintln!("thuki: [update] window not found during init_update_panel");
        return;
    };
    match window.to_panel::<ThukiUpdatePanel>() {
        Ok(panel) => {
            panel.set_floating_panel(true);
            // Normal window level (0), not Floating: while the window is open the
            // app runs under Regular activation (see show_update_window), so a
            // normal level lets another app the user clicks afterwards rise above
            // it instead of the window staying pinned on top forever.
            panel.set_level(0);
            panel.set_style_mask(StyleMask::empty().nonactivating_panel().into());
            panel.set_has_shadow(true);
            panel.set_hides_on_deactivate(false);
            // `Managed`, not the overlay flags: bind the update window to a single
            // Space so it behaves like a standard app window (stays put when the
            // user swipes to another fullscreen app; the Regular activation in
            // show_update_window pulls the user to its Space). See the doc comment.
            panel.set_collection_behavior(CollectionBehavior::new().managed().into());
            // Hover-activate: take key focus the moment the cursor enters the
            // update overlay, mirroring init_panel. Pairs with the
            // `active_always` tracking area on ThukiUpdatePanel so a defocused
            // nonactivating panel regains key without activating the app.
            _update_panel::attach_update_event_handler(app_handle.clone());
        }
        Err(e) => {
            eprintln!("thuki: [update] NSPanel conversion failed: {e:?}");
        }
    }
}

// ─── Onboarding window ───────────────────────────────────────────────────────

/// Given a monitor's origin and logical size, and the window's requested
/// logical size, returns the origin that centers the window on that monitor.
///
/// Pure arithmetic, no window handle, so it is unit-testable in isolation.
fn centered_origin(
    mon_x: f64,
    mon_y: f64,
    mon_w: f64,
    mon_h: f64,
    win_w: f64,
    win_h: f64,
) -> (f64, f64) {
    (mon_x + (mon_w - win_w) / 2.0, mon_y + (mon_h - win_h) / 2.0)
}

/// Resizes the onboarding window to the measured content size, and centers it
/// when `center` is set. The resize and the optional reposition run atomically
/// on the macOS main thread, so the frontend never positions the window itself.
///
/// `useFitOnboardingWindow` passes `center: true` only on the first fit after a
/// step spawns; later fits (content growing, the ambient strip appearing) pass
/// `center: false`, so a window the user has dragged is resized in place rather
/// than snapped back to the middle.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn fit_onboarding_window(app_handle: tauri::AppHandle, width: f64, height: f64, center: bool) {
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("main") {
            let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, height)));
            if center {
                // why: window.center() reads outer_size(), which the window server has
                // not yet updated to reflect the set_size() call above in this same
                // closure, so it centers against the PREVIOUS size and the window lands
                // off-center. Compute the origin from the size we just requested instead.
                let monitor = window
                    .current_monitor()
                    .ok()
                    .flatten()
                    .or_else(|| window.primary_monitor().ok().flatten());
                if let Some(mon) = monitor {
                    let scale = mon.scale_factor();
                    let pos = mon.position();
                    let size = mon.size();
                    let (x, y) = centered_origin(
                        pos.x as f64 / scale,
                        pos.y as f64 / scale,
                        size.width as f64 / scale,
                        size.height as f64 / scale,
                        width,
                        height,
                    );
                    let _ = window
                        .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
                }
            }
        }
    });
}

/// Repositions the overlay ("main") window to the ask bar's canonical default
/// origin: the same top-center placement `show_overlay` uses when there is no
/// text selection to anchor to (see `context::calculate_window_position`).
///
/// This is the single owner of "return the ask bar to its default position"
/// for surfaces that borrow the shared overlay window and move it: onboarding
/// and the safe-mode recovery card both re-center and resize the window for
/// themselves via `fit_onboarding_window`. When such a surface is dismissed
/// back to the ask bar there is no fresh native `show_overlay`, so nothing else
/// restores the ask bar's origin and the bar would render wherever the previous
/// surface left the window (issue #296).
///
/// why: POSITION only. The ask bar's SIZE is content-driven and owned by the
/// frontend `ResizeObserver`, so this command deliberately never touches size;
/// it owns just the one axis no other code path restores on a surface
/// hand-back. The frontend calls it only on the borrowing-surface -> ask-bar
/// dismiss edge, never on a normal show, so `show_overlay`'s selection-anchored
/// placement is never clobbered. The default origin is taken from
/// `calculate_window_position` with an empty context (its no-selection branch),
/// never re-derived here, keeping `top_center` the single source of the value.
///
/// The reposition runs on the macOS main thread so it is atomic with the window
/// server, mirroring `fit_onboarding_window` (positioning from JS is unreliable).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
fn position_overlay_ask_bar(app_handle: tauri::AppHandle) {
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("main") {
            let monitor = window
                .current_monitor()
                .ok()
                .flatten()
                .or_else(|| window.primary_monitor().ok().flatten());
            if let Some(mon) = monitor {
                let scale = mon.scale_factor();
                let pos = mon.position();
                let size = mon.size();
                let placement = crate::context::calculate_window_position(
                    &crate::context::ActivationContext::empty(),
                    size.width as f64 / scale,
                    size.height as f64 / scale,
                    OVERLAY_LOGICAL_WIDTH,
                    OVERLAY_LOGICAL_HEIGHT_COLLAPSED,
                );
                // Placement is monitor-local; shift to global screen coordinates
                // before setting the position, same conversion as show_overlay.
                let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                    placement.x + pos.x as f64 / scale,
                    placement.y + pos.y as f64 / scale,
                )));
            }
        }
    });
}

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
    // Mark onboarding as owning the main window so any activation that races in
    // (tray / double-tap Control) is gated out of the ask-bar show path.
    set_onboarding_active_impl(true);
    // Onboarding is a foreground task: run under Regular activation so it gets a
    // Dock icon (a lost user can click back to it) and normal window layering.
    sync_activation_policy(app_handle);
    // Cover the transition: the resize + recenter below happen while the React
    // tree is still showing the previous step, so do them on an invisible panel
    // and let the frontend fade it back in once the new screen has settled. The
    // backstop guarantees a reveal even if the frontend one is missed.
    arm_onboarding_reveal_backstop(app_handle);
    let handle = app_handle.clone();
    let (win_w, win_h) = onboarding_window_size(&stage);
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("main") {
            set_main_window_alpha_now(&window, 0.0);
            let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(win_w, win_h)));
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

// ─── Trace recorder bootstrap helpers ────────────────────────────────────────

/// Builds the inner recorder for the live trace wrapper based on the
/// current `[debug] trace_enabled` value.
///
/// Returns a `NoopRecorder` when off (zero-cost path), a
/// `RegistryRecorder` rooted at `app_data_dir()/traces/` when on. The
/// caller is responsible for installing the result either as the
/// initial state of a `LiveTraceRecorder` (at startup) or replacing
/// the live recorder's inner (on Settings save).
///
/// Emits a one-line stderr warning when transitioning to the on state
/// so a developer running `bun run dev` can see at a glance that
/// tracing is active and where the files are landing.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn build_trace_inner(
    app_handle: &tauri::AppHandle,
    enabled: bool,
) -> Arc<dyn trace::TraceRecorder> {
    if !enabled {
        return Arc::new(trace::NoopRecorder);
    }
    let traces_root = app_handle
        .path()
        .app_data_dir()
        .map(|d| d.join("traces"))
        .unwrap_or_else(|_| std::env::temp_dir().join("thuki").join("traces"));
    eprintln!(
        "thuki: [trace] trace_enabled = ON. Writing forensic JSONL to {}.",
        traces_root.display()
    );
    eprintln!(
        "thuki: [trace] Files may contain sensitive text. Disable in config.toml when not actively debugging."
    );
    Arc::new(trace::RegistryRecorder::new(traces_root))
}

// ─── Menu helpers ────────────────────────────────────────────────────────────

/// Custom macOS application menu, replacing Tauri's default. The Quit item is a
/// custom one (id "quit", Cmd+Q) so quitting routes through `show_quit_dialog`
/// instead of the predefined hard-quit that ignores an in-flight download. The
/// Edit submenu is kept so the ask bar's copy / paste / select-all shortcuts
/// (which the replaced default menu provided) keep working.
#[cfg_attr(coverage_nightly, coverage(off))]
fn build_app_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};

    let quit = MenuItem::with_id(app, "quit", "Quit Thuki", true, Some("Cmd+Q"))?;
    let app_menu = Submenu::with_items(
        app,
        "Thuki",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About Thuki"), None)?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit_menu])
}

// ─── Tray helpers ────────────────────────────────────────────────────────────

/// Builds the system-tray menu. When `update_version` is `Some`, a
/// "What's New in vX.Y.Z" item is injected between the separator and Quit.
/// It opens the "What's New" window (preview + explicit actions); it does
/// not install on click.
#[cfg_attr(coverage_nightly, coverage(off))]
fn build_tray_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    update_version: Option<&str>,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let show = MenuItem::with_id(app, "show", "Open Thuki", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("Cmd+,"))?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Thuki", true, Some("Cmd+Q"))?;

    if let Some(version) = update_version {
        let label = format!("What's New in v{version}");
        let update = MenuItem::with_id(app, "update", &label, true, None::<&str>)?;
        let sep2 = PredefinedMenuItem::separator(app)?;
        Menu::with_items(app, &[&show, &settings, &sep1, &update, &sep2, &quit])
    } else {
        Menu::with_items(app, &[&show, &settings, &sep1, &quit])
    }
}

/// Re-reads `UpdaterState` and atomically swaps the tray icon and menu to
/// reflect whether an update is available.
#[cfg_attr(coverage_nightly, coverage(off))]
fn refresh_tray(app: &tauri::AppHandle) {
    let state: tauri::State<updater::state::UpdaterState> = app.state();
    let snap = state.snapshot();
    let version = snap.update.as_ref().map(|u| u.version.clone());

    let Some(tray) = app.tray_by_id("main") else {
        return;
    };

    // Swap icon
    let bytes: &[u8] = if version.is_some() {
        include_bytes!("../icons/tray-update.png")
    } else {
        include_bytes!("../icons/128x128.png")
    };
    if let Ok(img) = tauri::image::Image::from_bytes(bytes) {
        let _ = tray.set_icon(Some(img));
    }

    // Swap menu
    if let Ok(menu) = build_tray_menu(app, version.as_deref()) {
        let _ = tray.set_menu(Some(menu));
    }
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
/// Thin filesystem wrapper: true when the on-disk config already carries an
/// `[[inference.providers]]` array (the new shape). Used by startup to decide
/// whether to perform the one-time old -> new shape upgrade write. The parse
/// logic it delegates to (`migrate::toml_has_providers`) is unit-tested; this
/// wrapper only does the file read and is excluded from coverage.
#[cfg_attr(coverage_nightly, coverage(off))]
fn config_file_has_providers(path: &std::path::Path) -> bool {
    std::fs::read_to_string(path)
        .map(|s| crate::config::migrate::toml_has_providers(&s))
        .unwrap_or(false)
}

/// Path to the bundled `llama-server` sidecar binary.
///
/// Debug builds run straight from the repo, so the target-triple-suffixed
/// binary in `src-tauri/binaries/` is used directly. Bundled builds rely on
/// Tauri's `externalBin` handling, which installs the sidecar next to the
/// app executable (`Contents/MacOS`) with the target-triple suffix stripped,
/// so it resolves relative to `current_exe()`. Verified manually against the
/// packaged app layout (see the release checklist).
#[cfg_attr(coverage_nightly, coverage(off))]
fn engine_sidecar_path() -> std::path::PathBuf {
    if cfg!(debug_assertions) {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join("llama-server-aarch64-apple-darwin")
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("llama-server")))
            .unwrap_or_else(|| std::path::PathBuf::from("llama-server"))
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ── Shutdown-signal mask (issue #296) ─────────────────────────────
    // Block SIGINT/SIGTERM on the main thread FIRST, before Tauri (or
    // anything else) spawns a single thread: every thread spawned afterward
    // inherits this block, so the polite-stop signal is delivered to the one
    // dedicated `sigwait` thread we start later (after the session record
    // exists) rather than terminating the process on an arbitrary thread.
    // Marking the clean exit itself happens on that ordinary thread, never in
    // async-signal context. See `startup_guard::block_shutdown_signals`.
    startup_guard::block_shutdown_signals();

    let mut builder = tauri::Builder::default();

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        // Replace Tauri's default macOS menu: its predefined Quit does a hard
        // quit on Cmd+Q that bypasses our handlers. Our custom Quit fires this
        // handler instead, so a download in flight gets the warning.
        .menu(build_app_menu)
        .on_menu_event(|app, event| {
            if event.id.as_ref() == "quit" {
                request_quit(app);
            }
        })
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(ActivationPolicy::Accessory);

            // ── NSPanel conversion (macOS only) ──────────────────────────
            #[cfg(target_os = "macos")]
            init_panel(app.app_handle());
            #[cfg(target_os = "macos")]
            init_settings_panel(app.app_handle());
            #[cfg(target_os = "macos")]
            init_update_panel(app.app_handle());

            // Default the export save dialog to the compact layout. The
            // user can still hit the disclosure triangle for a full
            // file browser on any individual save.
            #[cfg(target_os = "macos")]
            apply_save_panel_compact_default();

            // ── System tray icon + menu ───────────────────────────────────
            // Order chosen for muscle-memory parity with mac tray apps
            // (Bartender, CleanShot X, Rectangle): primary action at top,
            // settings near it with the macOS-canonical ⌘, accelerator,
            // separator, then Quit at the bottom. The "Reveal app data"
            // affordance lives inside the Settings → About tab so the tray
            // stays focused on session-level actions.
            let tray_menu = build_tray_menu(app.handle(), None)?;

            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png"))
                .expect("Failed to load tray icon");

            let _tray = TrayIconBuilder::with_id("main")
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
                    "update" => {
                        // Open the "What's New" window so the user previews
                        // the release notes and picks an action (Skip /
                        // Later / Install Update) instead of an install
                        // starting on a single click.
                        // The chat footer and Settings banner route through
                        // the same `open_update_window` command.
                        show_update_window(app);
                    }
                    "quit" => {
                        // Tray Quit click. Cmd+Q reaches the app menu + Exit
                        // Requested instead, all routed through request_quit so
                        // an in-progress download is never torn down silently.
                        request_quit(app);
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

            // ── Replace-target tracking ───────────────────────────────
            // Tracks the last-active non-Thuki app via an NSWorkspace
            // observer; this is the app the /rewrite & /refine Replace action
            // writes into. Managed so `replace_selection` can read the target,
            // and the observer is installed on the main thread here at setup.
            let last_active_app = crate::replace::LastActiveAppState::default();
            app.manage(last_active_app.clone());
            crate::replace::start_activation_tracking(last_active_app);
            let warmup_handle = app.handle().clone();
            app.manage(warmup::WarmupState::with_on_loaded(Arc::new(
                move |model| {
                    let _ = warmup_handle.emit("warmup:model-loaded", model);
                },
            )));
            // Port-keyed dedup + cue state for the built-in engine warm-up.
            app.manage(warmup::BuiltinWarmState::default());

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

            // ── Launch circuit breaker (issue #296) ───────────────────
            // Detect a previous session that died abnormally: the crash-loop
            // signature where Thuki froze the machine while auto-loading a
            // large model, macOS reopened the app after the forced power-off,
            // and it re-froze before the user could act. Runs BEFORE any
            // dangerous auto-op: it takes a process-lifetime advisory lock and
            // durably writes this launch's `clean_exit: false` record up front,
            // so a freeze anywhere in the dangerous window leaves the abnormal
            // marker on disk. The ONLY clear point is a genuine exit (see the
            // `mark_session_clean_exit` calls in the run loop below); the
            // renderer starting proves nothing about surviving that window.
            let startup = startup_guard::run_startup_guard(
                app.handle(),
                crate::config::defaults::DEFAULT_STARTUP_SAFE_MODE_THRESHOLD,
            );
            // This launch's immutable verdict (read by the auto-prime gate and
            // the `startup_safety` command) and the process-lifetime session
            // handle (holds the advisory lock open and owns the record writer)
            // are two separate managed values by design; never conflated.
            app.manage(startup.safety);
            app.manage(startup.guard);

            // ── Shutdown-signal → clean exit (issue #296) ─────────────
            // Ctrl+C (SIGINT) in a dev terminal and SIGTERM at macOS
            // shutdown/restart never run the Tauri RunEvent handlers, so
            // without this a polite stop would leave `clean_exit: false` and be
            // miscounted as abnormal: every normal Mac restart would inch the
            // streak toward a false safe mode. A controlling process asking us
            // to stop IS a clean exit.
            //
            // why HERE in the ordering: the mask was already installed at the
            // very top of `run()` before any thread was spawned, so this thread
            // (and all others) inherits the block and only this one consumes the
            // signal. We spawn it now, AFTER the launch routine durably wrote
            // this session's `clean_exit: false` record and AFTER `SessionGuard`
            // is managed, so `mark_session_clean_exit` has a writer to flip.
            //
            // why cry-wolf is still prevented: SIGKILL, a kernel panic, a power
            // cut, and a machine freeze remain uncatchable by construction: they
            // terminate the process without running this thread, so they still
            // leave `clean_exit: false` behind and correctly count as abnormal.
            // Only the polite-stop signals are reclassified as clean, and the
            // clean-exit write stays the single path in `mark_session_clean_exit`.
            //
            // why we ALSO shut the engine down here: a polite stop never runs the
            // Tauri RunEvent handlers, so without this the sidecar would outlive
            // Ctrl+C and, far more importantly, every macOS restart, reparenting
            // to launchd and holding ~2 GB (issue #296). The thread runs on an
            // ordinary thread, so it may block; the shutdown is bounded so a
            // wedged sidecar can never keep it from re-raising the signal.
            //
            // why THIS order (clean-exit write, then engine shutdown): the
            // clean-exit write is the sole safe-mode input and has no backstop but
            // itself, so it lands first and unconditionally. The engine shutdown
            // that follows can hang and is bounded; a sidecar that outlives the
            // bound is reaped at the next launch, so it is safe to run second and
            // behind a timeout.
            let signal_app = app.handle().clone();
            startup_guard::spawn_shutdown_signal_thread(move || {
                mark_session_clean_exit(&signal_app);
                shutdown_engine_bounded(&signal_app);
            });

            // Defense-in-depth on top of the session record: ask macOS not to
            // auto-reopen our overlay after an unclean shutdown, reducing the
            // chance the dangerous auto-startup path is re-entered with no
            // user action in the first place.
            #[cfg(target_os = "macos")]
            startup_guard::disable_quit_keeps_windows();

            // ── Orphaned sidecar reaper (issue #296) ──────────────────
            // SIGKILL, a kernel panic, or a machine freeze kills Thuki without
            // running any shutdown path, so a sidecar it spawned can reparent to
            // launchd (ppid 1) and linger holding ~2 GB. The next launch is the
            // only place to reap it. Runs on a detached thread so the (rare)
            // SIGTERM grace on an actual orphan never delays startup; it can
            // never touch a live Thuki's sidecar because that child's ppid is its
            // own Thuki's pid, not 1 (the load-bearing clause in the predicate).
            {
                let our_sidecar = engine_sidecar_path();
                std::thread::spawn(move || {
                    engine::orphan::reap_orphaned_sidecars(&our_sidecar);
                });
            }

            // ── Updater state + optional background poller ────────────
            {
                let updater_state = updater::UpdaterState::default();
                let running_version = app.package_info().version.to_string();

                let sidecar_path = app
                    .path()
                    .app_config_dir()
                    .ok()
                    .map(|d| d.join(crate::config::defaults::DEFAULT_UPDATER_STATE_FILENAME));

                let mut sidecar = updater::SnoozeSidecar::default();
                if let Some(path) = sidecar_path.as_ref() {
                    if let Ok(loaded) = updater::SnoozeSidecar::load(path) {
                        sidecar = loaded;
                    }
                }

                // Detect a fresh upgrade and clear the stale TCC grants
                // macOS keeps for the previous binary's code signature.
                // Without this, System Settings shows the toggle on but
                // the new binary cannot actually use the permission.
                let did_upgrade = updater::tcc_reset::should_reset_for_upgrade(
                    sidecar.last_launched_version.as_deref(),
                    &running_version,
                );
                if did_upgrade {
                    updater::tcc_reset::tccutil_reset(&app.config().identifier);
                    // Persist that the running version's csreq is what
                    // owns any TCC entries on disk now (or there are no
                    // entries, which is also fine). The click-time grant
                    // flow consults this so the user's first grant click
                    // after an upgrade does not trigger a second
                    // reset+relaunch on top of the one we are about to
                    // schedule below. Held in the sidecar (not memory)
                    // because the relaunch wipes any in-process state
                    // before the user could ever click.
                    sidecar.last_reset_for_version = Some(running_version.clone());
                }

                // Restore persisted snooze flags into the live state.
                updater_state.set_settings_snooze(sidecar.settings_snoozed_until);
                updater_state.set_chat_snooze(sidecar.chat_snoozed_until);
                // Seed the previously-seen available version so the first
                // poll after launch can correctly distinguish "user already
                // snoozed this version" from "new version arrived, clear
                // snooze." Without this, every cold start would see
                // None vs Some(v) and unconditionally clear the user's
                // snooze.
                updater_state
                    .set_last_seen_update_version(sidecar.last_seen_update_version.clone());
                // Mirror the on-disk reset marker so click-time decisions
                // don't have to re-read the sidecar.
                updater_state.set_last_reset_for_version(sidecar.last_reset_for_version.clone());
                // Seed the skip list so a version the user dismissed in a
                // previous session stays suppressed: the very first poll
                // after launch must already know it is skipped.
                updater_state.set_skipped_versions(sidecar.skipped_versions.clone());

                // Record the running version BEFORE any potential restart
                // so the post-restart launch reads a sidecar where the
                // recorded version matches the running version. Without
                // this, the next launch would see another "upgrade" and
                // restart-loop forever.
                sidecar.last_launched_version = Some(running_version);
                if let Some(path) = sidecar_path.as_ref() {
                    if let Err(e) = sidecar.save(path) {
                        eprintln!("thuki: [updater] failed to persist sidecar: {e}");
                    }
                }

                // After `tccutil reset` clears the TCC.db entry for Thuki,
                // the running process retains stale per-PID tracking inside
                // macOS's `tccd` daemon. Subsequent `AXIsProcessTrusted`
                // calls from THIS process do not register the new csreq, so
                // Thuki is missing from System Settings → Privacy &
                // Security → Accessibility and the user has no in-app path
                // to grant. Empirically (user-reproduced) the only fix is
                // a fresh process: `tccd` sees a brand new PID and
                // registers it normally on the first AX call from
                // onboarding. The restart is deferred so Tauri finishes
                // wiring up the rest of `setup` before we tear it down.
                if did_upgrade {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                        eprintln!(
                            "thuki: [updater] relaunching after TCC reset \
                             to refresh tccd PID tracking"
                        );
                        app_handle.restart();
                    });
                }

                let (interval, auto_check) = {
                    let cfg = app.state::<parking_lot::RwLock<crate::config::AppConfig>>();
                    let g = cfg.read();
                    (g.updater.check_interval_hours, g.updater.auto_check)
                };

                app.manage(updater_state);

                // Refresh the tray icon and menu whenever the poller finds a
                // new update. The listener must be registered after manage() so
                // refresh_tray can read UpdaterState from managed state.
                let tray_refresh_handle = app.handle().clone();
                app.listen("update-available", move |_event| {
                    refresh_tray(&tray_refresh_handle);
                });

                if auto_check {
                    updater::poller::spawn(app.handle().clone(), interval);
                }
            }

            // ── Generation + conversation state ─────────────────────
            app.manage(commands::GenerationState::new());
            app.manage(commands::ConversationHistory::new());

            // ── Unified trace recorder ─────────────────────────────
            // Off by default: when `[debug] trace_enabled = false` in
            // config.toml the live recorder wraps a `NoopRecorder` and
            // every chat / search / screenshot event is a constant-time
            // call. When on, it wraps a `RegistryRecorder` that routes
            // events to per-conversation JSONL files under
            // `app_data_dir()/traces/{chat,search}/`.
            //
            // Wrapped in a `LiveTraceRecorder` so toggling
            // `[debug] trace_enabled` from the Settings panel hot-swaps
            // the inner without requiring an app restart. See
            // `trace::live` for the swap contract and
            // `settings_commands::set_config_field` for the hook site.
            let trace_enabled = app
                .state::<parking_lot::RwLock<crate::config::AppConfig>>()
                .read()
                .debug
                .trace_enabled;
            let initial_inner = build_trace_inner(app.handle(), trace_enabled);
            app.manage(Arc::new(trace::LiveTraceRecorder::new(initial_inner)));

            // ── SQLite database for conversation history ──────────
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data directory");
            let db_conn = database::open_database(&app_data_dir)
                .expect("failed to initialise SQLite database");

            // ── Active model: migrate the legacy SQLite slug onto the active
            // provider, then seed the in-memory ActiveModelState ──────────
            // Pre-providers builds persisted the active model in SQLite under
            // ACTIVE_MODEL_KEY. It now lives on the active provider's `model`
            // field in config.toml. Read the legacy value once; if the active
            // provider has no model yet, fold it in, and for an old-shape file
            // upgrade the file to the providers shape (a one-time write).
            // After this the model persists to config and SQLite active_model
            // is never written again. The installed list isn't queried here
            // (no async runtime yet); get_model_picker_state reconciles against
            // the live /api/tags inventory on first picker open.
            let legacy_active_model = database::get_config(&db_conn, models::ACTIVE_MODEL_KEY)
                .expect("failed to read legacy active_model from app_config");
            let config_file_path = app
                .path()
                .app_config_dir()
                .expect("failed to resolve app config dir")
                .join(crate::config::CONFIG_FILE_NAME);
            let initial_active_model = {
                let state = app.state::<parking_lot::RwLock<crate::config::AppConfig>>();
                let mut cfg = state.write();
                let pre_providers = !config_file_has_providers(&config_file_path);
                let attached = crate::config::migrate::attach_legacy_active_model(
                    &mut cfg,
                    legacy_active_model.as_deref(),
                );
                if pre_providers || attached {
                    if let Err(e) = crate::config::writer::atomic_write(&config_file_path, &cfg) {
                        eprintln!("thuki: [config] active-model migration write failed: {e}");
                    }
                }
                models::resolve_seed_active_model(Some(cfg.inference.active_provider_model()))
            };
            app.manage(models::ActiveModelState(std::sync::Mutex::new(
                initial_active_model,
            )));
            app.manage(models::ModelCapabilitiesCache::default());

            // ── Model blob store + download slot for the built-in engine ──
            let model_store = models::storage::ModelStore::new(app_data_dir.join("models"))
                .expect("failed to initialise model blob store");

            // One-time heal: classify any installed models recorded before the
            // dynamic reasoning classifier existed (reasoning_always IS NULL),
            // reading each model's local GGUF, so the picker badge and /think
            // gate are correct without waiting for the first chat.
            models::heal_unclassified_reasoning(&db_conn, &model_store);

            app.manage(history::Database(std::sync::Mutex::new(db_conn)));
            app.manage(model_store);
            app.manage(models::DownloadState::default());

            // ── Keychain secret store ──────────────────────────────
            app.manage(keychain::Secrets(std::sync::Arc::new(
                keychain::KeyringStore,
            )));

            // ── Built-in inference engine runner ───────────────────
            // One actor owns the bundled llama-server lifecycle: at most one
            // process, kill-then-start on model switch, idle unload. Spawned
            // inside block_on so the actor task lands on Tauri's tokio
            // runtime (setup itself runs outside a runtime context).
            // The unified `keep_warm_inactivity_minutes` field governs both
            // local providers; translate its sentinel into the runner's own
            // `idle_minutes` convention through the shared boundary helper.
            let engine_idle_minutes = warmup::builtin_idle_minutes(
                app.state::<parking_lot::RwLock<crate::config::AppConfig>>()
                    .read()
                    .inference
                    .keep_warm_inactivity_minutes,
            );
            let engine_client = app.state::<reqwest::Client>().inner().clone();
            let engine = tauri::async_runtime::block_on(async move {
                engine::runner::EngineHandle::spawn(
                    std::sync::Arc::new(engine::process::TokioEngineProcess {
                        binary: engine_sidecar_path(),
                        client: engine_client,
                    }),
                    engine_idle_minutes,
                    std::time::Duration::from_secs(
                        crate::config::defaults::ENGINE_IDLE_CHECK_INTERVAL_SECS,
                    ),
                )
            });
            // Forward every engine lifecycle change to the frontend,
            // mirroring how warmup events are emitted.
            {
                let status_handle = app.handle().clone();
                let mut status_rx = engine.status();
                tauri::async_runtime::spawn(async move {
                    while status_rx.changed().await.is_ok() {
                        let status = status_rx.borrow_and_update().clone();
                        // A load left memory (idle-unload, model switch, crash):
                        // drop the built-in warm-up dedup so the next load primes
                        // fresh even when the OS reuses the same port. The dedup
                        // is keyed on port, so a stale primed record would
                        // otherwise skip the cold reload's prime.
                        if status.state != "loaded" {
                            status_handle.state::<warmup::BuiltinWarmState>().reset();
                        }
                        let _ = status_handle.emit("engine:status", status);
                    }
                });
            }
            app.manage(engine);

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
            commands::ask_model,
            #[cfg(not(coverage))]
            commands::cancel_generation,
            #[cfg(not(coverage))]
            commands::open_url,
            #[cfg(not(coverage))]
            search::search_pipeline,
            #[cfg(not(coverage))]
            commands::reset_conversation,
            #[cfg(not(coverage))]
            commands::record_conversation_end,
            settings_commands::get_config,
            settings_commands::set_config_field,
            settings_commands::set_ollama_url,
            settings_commands::set_active_provider,
            settings_commands::update_provider_field,
            settings_commands::add_openai_provider,
            settings_commands::remove_openai_provider,
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
            models::detect_ollama,
            #[cfg(not(coverage))]
            models::get_model_capabilities,
            #[cfg(not(coverage))]
            models::get_starter_options,
            #[cfg(not(coverage))]
            models::get_staff_picks,
            #[cfg(not(coverage))]
            models::get_system_ram_bytes,
            #[cfg(not(coverage))]
            models::memory::estimate_model_fit,
            #[cfg(not(coverage))]
            models::get_models_dir_free_bytes,
            #[cfg(not(coverage))]
            models::download_starter,
            #[cfg(not(coverage))]
            models::download_staff_pick,
            #[cfg(not(coverage))]
            models::download_repo_model,
            #[cfg(not(coverage))]
            models::list_hf_repo_ggufs,
            #[cfg(not(coverage))]
            models::search_hf_models,
            #[cfg(not(coverage))]
            models::list_openai_models,
            #[cfg(not(coverage))]
            models::cancel_model_download,
            #[cfg(not(coverage))]
            models::discard_partial_download,
            #[cfg(not(coverage))]
            models::get_active_downloads,
            #[cfg(not(coverage))]
            set_download_paused,
            #[cfg(not(coverage))]
            models::list_installed_models,
            #[cfg(not(coverage))]
            models::delete_installed_model,
            #[cfg(not(coverage))]
            models::reveal_model_in_finder,
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
            #[cfg(not(coverage))]
            ocr::extract_text_command,
            #[cfg(not(coverage))]
            export::prompt_and_save_chat_export,
            #[cfg(not(coverage))]
            replace::replace_selection,
            notify_overlay_hidden,
            set_overlay_minimized,
            notify_frontend_ready,
            set_window_frame,
            animate_overlay_frame,
            set_overlay_alpha,
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
            advance_past_builtin_announcement,
            fit_onboarding_window,
            position_overlay_ask_bar,
            onboarding_stage,
            is_builtin_announced,
            open_settings_window,
            open_settings_to_providers,
            hide_settings_window,
            #[cfg(not(coverage))]
            warmup::warm_up_model,
            #[cfg(not(coverage))]
            warmup::evict_model,
            #[cfg(not(coverage))]
            warmup::get_loaded_model,
            #[cfg(not(coverage))]
            warmup::get_engine_status,
            #[cfg(not(coverage))]
            warmup::get_builtin_warm_state,
            startup_guard::startup_safety,
            updater::commands::get_updater_state,
            #[cfg(not(coverage))]
            updater::commands::check_for_update,
            #[cfg(not(coverage))]
            updater::commands::install_update,
            #[cfg(not(coverage))]
            updater::commands::skip_update_version,
            #[cfg(not(coverage))]
            updater::commands::open_update_window,
            #[cfg(not(coverage))]
            updater::commands::snooze_update_chat,
            #[cfg(not(coverage))]
            updater::commands::snooze_update_settings,
            #[cfg(not(coverage))]
            updater::commands::reset_and_relaunch_for_grant,
            #[cfg(not(coverage))]
            updater::commands::consume_pending_grant_resume,
            #[cfg(not(coverage))]
            keychain::set_provider_api_key,
            #[cfg(not(coverage))]
            keychain::clear_provider_api_key,
            #[cfg(not(coverage))]
            keychain::has_provider_api_key,
            #[cfg(not(coverage))]
            subscribe::subscribe_email
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } => {
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
                    // Real window gone: drop the Dock icon and return to
                    // Accessory (unless onboarding still owns a window).
                    #[cfg(target_os = "macos")]
                    {
                        SETTINGS_OPEN.store(false, Ordering::SeqCst);
                        sync_activation_policy(app_handle);
                    }
                } else if label == "update" {
                    // Hide instead of destroy so the NSPanel handle from
                    // init_update_panel stays valid for the next open
                    // (cmd-W and the in-window buttons both close it).
                    api.prevent_close();
                    if let Some(window) = app_handle.get_webview_window("update") {
                        let _ = window.hide();
                    }
                    // Real window gone: drop the Dock icon and return to
                    // Accessory (unless Settings or onboarding still owns one).
                    #[cfg(target_os = "macos")]
                    {
                        UPDATE_OPEN.store(false, Ordering::SeqCst);
                        sync_activation_policy(app_handle);
                    }
                }
            }
            RunEvent::ExitRequested { api, .. } => {
                // Cmd+Q (and any app.exit issued before the user has confirmed)
                // lands here. If a download would be lost, hold the exit and
                // warn so the user can keep it running in the background. The
                // dialog itself is deduplicated against the app-menu path.
                if !QUIT_CONFIRMED.load(Ordering::SeqCst) && should_warn_on_quit(app_handle) {
                    api.prevent_exit();
                    show_quit_dialog(app_handle);
                } else {
                    // why: this is the ONLY place the session record is flipped
                    // to `clean_exit: true`, because the renderer starting
                    // proves nothing about surviving the dangerous startup
                    // window (issue #296). The exit is proceeding (not held for
                    // a download warning), so mark it clean here. `Exit` below
                    // marks it too, since on the macOS restart/shutdown
                    // termination path `ExitRequested` may not fire; the write
                    // is idempotent, so marking on both paths is safe.
                    mark_session_clean_exit(app_handle);
                }
            }
            RunEvent::Exit => {
                // why: also mark the clean exit here (idempotent) so the macOS
                // restart/shutdown termination path, which may skip
                // `ExitRequested`, still lands the clean marker durably.
                mark_session_clean_exit(app_handle);
                // Kill the built-in engine sidecar and confirm its exit so
                // no orphan llama-server survives quit. The actor runs on
                // the tokio runtime, so block_on here cannot deadlock.
                let engine = app_handle
                    .state::<engine::runner::EngineHandle>()
                    .inner()
                    .clone();
                tauri::async_runtime::block_on(async move { engine.shutdown().await });
                // The engine is now stopped, so no split-model load can be in
                // flight: remove the load-time symlink shims. The shard blobs
                // themselves stay in the store; only the symlink indirection is
                // reclaimed. Best-effort, a leftover dir is harmless.
                if let Some(store) = app_handle.try_state::<models::storage::ModelStore>() {
                    let _ = store.clear_split_shims();
                }
            }
            // Dock-icon click. The icon is only present while Settings, the
            // update window, or onboarding owns a window, so refocus that window
            // rather than summoning the ask-bar overlay (which is not what the
            // icon represents). macOS delivers Reopen on dock click / Cmd-Tab.
            //
            // Refocus only - deliberately does NOT call show_settings_window,
            // which would run position_settings_window and recenter a panel the
            // user has dragged. A dock click should bring the existing panel
            // forward in place, not snap it back to center.
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => {
                if SETTINGS_OPEN.load(Ordering::SeqCst) {
                    if let Ok(panel) = app_handle.get_webview_panel("settings") {
                        let _ = app_handle.run_on_main_thread(move || {
                            panel.show_and_make_key();
                        });
                    } else if let Some(w) = app_handle.get_webview_window("settings") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                } else if UPDATE_OPEN.load(Ordering::SeqCst) {
                    if let Ok(panel) = app_handle.get_webview_panel("update") {
                        let _ = app_handle.run_on_main_thread(move || {
                            panel.show_and_make_key();
                        });
                    } else if let Some(w) = app_handle.get_webview_window("update") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                } else if ONBOARDING_ACTIVE.load(Ordering::SeqCst) {
                    if let Some(w) = app_handle.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
            }
            _ => {}
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
    fn top_left_anchored_target_keeps_top_and_left_fixed() {
        // Current frame: origin (100, 200), size 600x700.
        // AppKit top edge = 200 + 700 = 900.
        let cur = (100.0_f64, 200.0_f64, 600.0_f64, 700.0_f64);

        // Shrink to a 48x48 square.
        let (x, y, w, h) = compute_top_left_anchored_target(cur, 48.0, 48.0);
        assert_eq!(x, 100.0, "left edge (origin.x) must not move");
        assert_eq!(w, 48.0, "width is set to the requested value");
        assert_eq!(h, 48.0, "height is set to the requested value");
        // Top edge after = new_y + new_h must equal the original top (900).
        assert_eq!(y + h, 900.0, "visual top edge must stay fixed");
        assert_eq!(y, 852.0);

        // Grow back to a larger box: top edge still fixed at 900.
        let (gx, gy, gw, gh) = compute_top_left_anchored_target(cur, 560.0, 648.0);
        assert_eq!(gx, 100.0);
        assert_eq!(gw, 560.0);
        assert_eq!(gh, 648.0);
        assert_eq!(gy + gh, 900.0, "visual top edge must stay fixed on grow");
        assert_eq!(gy, 252.0);
    }

    #[test]
    fn centered_origin_smaller_window_centers_exactly() {
        // 1000x800 monitor at origin (0, 0), a 420x300 window centers with
        // equal margins on both axes.
        let (x, y) = centered_origin(0.0, 0.0, 1000.0, 800.0, 420.0, 300.0);
        assert_eq!(x, 290.0);
        assert_eq!(y, 250.0);
    }

    #[test]
    fn centered_origin_equal_size_yields_monitor_origin() {
        // Window exactly filling the monitor: origin matches the monitor's.
        let (x, y) = centered_origin(50.0, 25.0, 1000.0, 800.0, 1000.0, 800.0);
        assert_eq!(x, 50.0);
        assert_eq!(y, 25.0);
    }

    #[test]
    fn centered_origin_nonzero_monitor_offset_shifts_result() {
        // Second display sitting to the right of the primary at (1920, 0).
        let (x, y) = centered_origin(1920.0, 0.0, 1000.0, 800.0, 420.0, 300.0);
        assert_eq!(x, 2210.0);
        assert_eq!(y, 250.0);
    }

    #[test]
    fn centered_origin_oversized_window_yields_negative_offset() {
        // Window larger than the monitor: origin goes negative, no panic.
        let (x, y) = centered_origin(0.0, 0.0, 800.0, 600.0, 1000.0, 800.0);
        assert_eq!(x, -100.0);
        assert_eq!(y, -100.0);
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
    fn set_overlay_minimized_toggles_flag() {
        OVERLAY_MINIMIZED.store(false, Ordering::SeqCst);
        set_overlay_minimized_impl(true);
        assert!(OVERLAY_MINIMIZED.load(Ordering::SeqCst));
        set_overlay_minimized_impl(false);
        assert!(!OVERLAY_MINIMIZED.load(Ordering::SeqCst));
    }

    #[test]
    fn onboarding_active_gate_toggles_flag() {
        ONBOARDING_ACTIVE.store(false, Ordering::SeqCst);
        set_onboarding_active_impl(true);
        assert!(ONBOARDING_ACTIVE.load(Ordering::SeqCst));
        set_onboarding_active_impl(false);
        assert!(!ONBOARDING_ACTIVE.load(Ordering::SeqCst));
    }

    #[test]
    fn wants_regular_activation_when_settings_update_or_onboarding_open() {
        SETTINGS_OPEN.store(false, Ordering::SeqCst);
        UPDATE_OPEN.store(false, Ordering::SeqCst);
        ONBOARDING_ACTIVE.store(false, Ordering::SeqCst);
        assert!(!wants_regular_activation());

        SETTINGS_OPEN.store(true, Ordering::SeqCst);
        assert!(wants_regular_activation());
        SETTINGS_OPEN.store(false, Ordering::SeqCst);

        UPDATE_OPEN.store(true, Ordering::SeqCst);
        assert!(wants_regular_activation());
        UPDATE_OPEN.store(false, Ordering::SeqCst);

        ONBOARDING_ACTIVE.store(true, Ordering::SeqCst);
        assert!(wants_regular_activation());

        SETTINGS_OPEN.store(true, Ordering::SeqCst);
        UPDATE_OPEN.store(true, Ordering::SeqCst);
        assert!(wants_regular_activation());

        SETTINGS_OPEN.store(false, Ordering::SeqCst);
        UPDATE_OPEN.store(false, Ordering::SeqCst);
        ONBOARDING_ACTIVE.store(false, Ordering::SeqCst);
        assert!(!wants_regular_activation());
    }

    #[test]
    fn minimized_guard_clears_flag() {
        OVERLAY_MINIMIZED.store(true, Ordering::SeqCst);
        let consumed = take_minimized_for_restore();
        assert!(consumed);
        assert!(!OVERLAY_MINIMIZED.load(Ordering::SeqCst));
        assert!(!take_minimized_for_restore());
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
    fn restore_visibility_constant_is_distinct() {
        assert_ne!(OVERLAY_VISIBILITY_RESTORE, OVERLAY_VISIBILITY_SHOW);
        assert_ne!(OVERLAY_VISIBILITY_RESTORE, OVERLAY_VISIBILITY_HIDE_REQUEST);
        assert_eq!(OVERLAY_VISIBILITY_RESTORE, "restore");
    }

    #[test]
    fn onboarding_event_constant_matches() {
        assert_eq!(ONBOARDING_EVENT, "thuki://onboarding");
    }

    #[test]
    fn onboarding_logical_dimensions() {
        assert_eq!(ONBOARDING_LOGICAL_WIDTH, 460.0);
        assert_eq!(ONBOARDING_LOGICAL_HEIGHT, 640.0);
        assert_eq!(ONBOARDING_PICKER_WIDTH, 860.0);
        assert_eq!(ONBOARDING_PICKER_HEIGHT, 744.0);
    }

    #[test]
    fn onboarding_window_size_widens_for_picker() {
        assert_eq!(
            onboarding_window_size(&onboarding::OnboardingStage::ModelCheck),
            (ONBOARDING_PICKER_WIDTH, ONBOARDING_PICKER_HEIGHT),
        );
        assert_eq!(
            onboarding_window_size(&onboarding::OnboardingStage::Permissions),
            (ONBOARDING_LOGICAL_WIDTH, ONBOARDING_LOGICAL_HEIGHT),
        );
        // Intro falls back to the compact base; the frontend fits it to its
        // card at runtime via `useFitOnboardingWindow`.
        assert_eq!(
            onboarding_window_size(&onboarding::OnboardingStage::Intro),
            (ONBOARDING_LOGICAL_WIDTH, ONBOARDING_LOGICAL_HEIGHT),
        );
    }

    #[test]
    fn overlay_logical_dimensions() {
        assert_eq!(OVERLAY_LOGICAL_WIDTH, 600.0);
        assert_eq!(OVERLAY_LOGICAL_HEIGHT_COLLAPSED, 80.0);
    }
}
