//! Writes rewritten text back into the user's source app for `/rewrite` and
//! `/refine`.
//!
//! Two parts:
//!
//! 1. **Target tracking.** An `NSWorkspace` activation observer records the PID
//!    of the last application the user activated that is *not* Thuki. Because
//!    Thuki's overlay is a non-activating panel, switching into it never fires
//!    an activation, so this reliably holds "the app you were last really in" —
//!    whether that is the app you summoned Thuki from (in-place) or a different
//!    app you clicked into afterwards (retarget).
//!
//! 2. **Writing (synthetic paste).** The target app is activated and, only once
//!    it is confirmed frontmost, the clipboard is saved (every type, so an
//!    image or file copy survives), the rewrite written to it (tagged transient
//!    so clipboard-history managers skip it), and a synthetic Cmd+V posted
//!    directly to the target process with `CGEventPostToPid`. Posting to the
//!    process rather than the system key window means the paste reaches the
//!    source app even though Thuki's nonactivating panel still holds the key
//!    window, so the overlay stays open instead of having to be dismissed
//!    first. The clipboard is then restored. If the target never becomes
//!    frontmost the write is skipped entirely and the clipboard left untouched.
//!    Paste is used rather than an Accessibility write
//!    because Cmd+V reliably *replaces* the selection. An AX selected-text
//!    write does not: the selection range collapses when the app loses focus,
//!    so the AX write inserts at the caret instead. Secure input (a focused
//!    password field) suppresses the write entirely.

use std::sync::{Arc, Mutex};

use serde::Serialize;

/// Managed state: PID of the last non-Thuki application to activate, kept
/// current by the `NSWorkspace` activation observer. This is the app a
/// `/rewrite` or `/refine` Replace writes into.
#[derive(Default, Clone)]
pub struct LastActiveAppState(pub Arc<Mutex<Option<i32>>>);

impl LastActiveAppState {
    /// The current target app PID, if one has been observed.
    pub fn get(&self) -> Option<i32> {
        *self.0.lock().unwrap()
    }

    /// Records `pid` as the latest target app.
    pub fn set(&self, pid: i32) {
        *self.0.lock().unwrap() = Some(pid);
    }
}

/// Outcome of a replace attempt, surfaced to the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplaceOutcome {
    /// Text was pasted into the target app.
    Replaced,
    /// No-op: empty text, Accessibility not granted, no target app observed,
    /// the target app could not be brought to the foreground, or secure input
    /// was active.
    Skipped,
}

/// Whether an activation of `activated_pid` should be recorded as the write
/// target: any app other than Thuki itself (`own_pid`). Thuki's own
/// activations are ignored so summoning the overlay never overwrites the
/// remembered target.
pub fn should_record_activation(activated_pid: i32, own_pid: i32) -> bool {
    activated_pid != own_pid
}

/// Starts tracking the last-active external app. Seeds the state with the
/// current frontmost app, then installs the `NSWorkspace` activation observer.
/// Must be called once, on the main thread, during app setup. No-op off macOS.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn start_activation_tracking(state: LastActiveAppState) {
    #[cfg(target_os = "macos")]
    {
        let own = std::process::id() as i32;
        if let Some(pid) = macos::frontmost_app_pid() {
            if should_record_activation(pid, own) {
                state.set(pid);
            }
        }
        macos::install_activation_observer(state, own);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
    }
}

/// Pastes `text` into the last-active app, replacing its selection. The paste
/// is posted directly to the target process, so the overlay does not need to be
/// dismissed first. Returns [`ReplaceOutcome::Skipped`] without side effects
/// when there is nothing safe to write into.
///
/// Runs the macOS clipboard / event work on a blocking pool thread: the paste
/// path sleeps while the target activates, so it must not run on the main
/// thread.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn replace_selection(
    text: String,
    last_active: tauri::State<'_, LastActiveAppState>,
) -> Result<ReplaceOutcome, ()> {
    if text.is_empty() {
        return Ok(ReplaceOutcome::Skipped);
    }

    #[cfg(target_os = "macos")]
    {
        if !crate::permissions::is_accessibility_granted() {
            return Ok(ReplaceOutcome::Skipped);
        }
        let Some(pid) = last_active.get() else {
            return Ok(ReplaceOutcome::Skipped);
        };
        let outcome = tokio::task::spawn_blocking(move || macos::paste_into(pid, &text))
            .await
            .unwrap_or(ReplaceOutcome::Skipped);
        Ok(outcome)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = &last_active;
        Ok(ReplaceOutcome::Skipped)
    }
}

// ─── macOS clipboard + event implementation ──────────────────────────────────

#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
mod macos {
    use std::ffi::c_void;

    use block2::RcBlock;
    use core_foundation::base::CFTypeRef;
    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::{ns_string, NSArray, NSData, NSString};

    use super::{should_record_activation, LastActiveAppState, ReplaceOutcome};

    /// macOS virtual keycode for 'v'.
    const KEY_V: u16 = 0x09;
    /// CGEventFlags::kCGEventFlagMaskCommand.
    const K_CG_EVENT_FLAG_MASK_COMMAND: u64 = 0x0010_0000;
    /// NSApplicationActivationOptions::NSApplicationActivateIgnoringOtherApps.
    const NS_ACTIVATE_IGNORING_OTHER_APPS: usize = 1 << 1;
    /// Milliseconds to wait after the synthetic paste before restoring the
    /// clipboard, giving the target app time to read the pasteboard first.
    const PASTE_SETTLE_MS: u64 = 200;

    // CoreGraphics is already linked by activator.rs.
    extern "C" {
        fn CFRelease(cf: CFTypeRef);
        fn CGEventCreateKeyboardEvent(
            source: *const c_void,
            virtual_key: u16,
            key_down: bool,
        ) -> CFTypeRef;
        fn CGEventSetFlags(event: CFTypeRef, flags: u64);
        // Posts an event directly to a target process, bypassing the key-window
        // routing that `CGEventPost` uses. This delivers the paste to the
        // source app even though Thuki's panel still holds the system key
        // window, so the overlay does not need to be dismissed first.
        fn CGEventPostToPid(pid: i32, event: CFTypeRef);
    }

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn IsSecureEventInputEnabled() -> u8;
    }

    /// UTI for plain UTF-8 text on the pasteboard (`NSPasteboardTypeString`).
    fn plain_text_type() -> &'static NSString {
        ns_string!("public.utf8-plain-text")
    }

    /// Returns the PID of the frontmost application, or `None`.
    pub fn frontmost_app_pid() -> Option<i32> {
        autoreleasepool(|_| unsafe {
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            if workspace.is_null() {
                return None;
            }
            let app: *mut AnyObject = msg_send![workspace, frontmostApplication];
            if app.is_null() {
                return None;
            }
            let pid: i32 = msg_send![app, processIdentifier];
            Some(pid)
        })
    }

    /// Installs an `NSWorkspace` observer that records every non-Thuki app
    /// activation into `state`. The observer block is leaked deliberately: it
    /// lives for the entire process, so there is never a point at which it
    /// should be torn down.
    pub fn install_activation_observer(state: LastActiveAppState, own: i32) {
        autoreleasepool(|_| unsafe {
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let center: *mut AnyObject = msg_send![workspace, notificationCenter];
            let name = ns_string!("NSWorkspaceDidActivateApplicationNotification");
            let nil: *mut AnyObject = std::ptr::null_mut();

            let block = RcBlock::new(move |notification: *mut AnyObject| {
                if notification.is_null() {
                    return;
                }
                let user_info: *mut AnyObject = msg_send![notification, userInfo];
                if user_info.is_null() {
                    return;
                }
                let app: *mut AnyObject =
                    msg_send![user_info, objectForKey: ns_string!("NSWorkspaceApplicationKey")];
                if app.is_null() {
                    return;
                }
                let pid: i32 = msg_send![app, processIdentifier];
                if should_record_activation(pid, own) {
                    state.set(pid);
                }
            });

            let _token: *mut AnyObject = msg_send![
                center,
                addObserverForName: name,
                object: nil,
                queue: nil,
                usingBlock: &*block,
            ];
            std::mem::forget(block);
        });
    }

    /// Whether secure input is active (a focused password field, or iTerm
    /// "Secure Keyboard Entry").
    unsafe fn is_secure_input() -> bool {
        IsSecureEventInputEnabled() != 0
    }

    /// Brings the app with `pid` to the foreground and waits, with bounded
    /// backoff, for the activation to take effect. Returns whether `pid` became
    /// frontmost: only then is its focused text field first responder and able
    /// to handle the synthetic Cmd+V as a paste over the selection. A `false`
    /// return means the paste must not be posted (the app quit, is blocked by a
    /// Spaces switch, or the PID no longer maps to a foregroundable app).
    unsafe fn activate_and_settle(pid: i32) -> bool {
        let app: *mut AnyObject =
            msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
        if !app.is_null() {
            let _: bool = msg_send![app, activateWithOptions: NS_ACTIVATE_IGNORING_OTHER_APPS];
        }
        for delay_ms in [0u64, 20, 30, 50, 80] {
            if delay_ms != 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }
            if frontmost_app_pid() == Some(pid) {
                return true;
            }
        }
        false
    }

    /// Posts a synthetic Cmd+V directly to the process `pid`. Targeting the
    /// process rather than the system key window is what lets the paste land in
    /// the source app while Thuki's panel remains key and visible.
    unsafe fn post_cmd_v_to_pid(pid: i32) {
        let down = CGEventCreateKeyboardEvent(std::ptr::null(), KEY_V, true);
        if !down.is_null() {
            CGEventSetFlags(down, K_CG_EVENT_FLAG_MASK_COMMAND);
            CGEventPostToPid(pid, down);
            CFRelease(down);
        }
        let up = CGEventCreateKeyboardEvent(std::ptr::null(), KEY_V, false);
        if !up.is_null() {
            CGEventSetFlags(up, K_CG_EVENT_FLAG_MASK_COMMAND);
            CGEventPostToPid(pid, up);
            CFRelease(up);
        }
    }

    /// Snapshots every type currently on the general pasteboard as `(UTI, data)`
    /// pairs so the user's clipboard can be restored after the synthetic paste
    /// in full — text, image, file reference, RTF — not just its plain-text
    /// representation. The returned pointers are autoreleased and stay valid for
    /// the lifetime of the enclosing autorelease pool (the whole `paste_into`
    /// call), which is where the matching [`restore_pasteboard`] runs.
    unsafe fn snapshot_pasteboard() -> Vec<(*mut NSString, *mut NSData)> {
        let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let types: *mut AnyObject = msg_send![pb, types];
        if types.is_null() {
            return Vec::new();
        }
        let count: usize = msg_send![types, count];
        let mut saved = Vec::with_capacity(count);
        for i in 0..count {
            let ty: *mut NSString = msg_send![types, objectAtIndex: i];
            if ty.is_null() {
                continue;
            }
            let data: *mut NSData = msg_send![pb, dataForType: ty];
            if !data.is_null() {
                saved.push((ty, data));
            }
        }
        saved
    }

    /// Writes `text` to the general pasteboard, tagged transient +
    /// auto-generated so clipboard-history managers (Maccy, Alfred, Raycast,
    /// 1Password, ...) skip recording it.
    unsafe fn write_pasteboard_transient(text: &str) {
        let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let plain = plain_text_type();
        let transient = ns_string!("org.nspasteboard.TransientType");
        let autogen = ns_string!("org.nspasteboard.AutoGeneratedType");
        let types = NSArray::from_slice(&[plain, transient, autogen]);
        let nil: *mut AnyObject = std::ptr::null_mut();
        let _: isize = msg_send![pb, clearContents];
        let _: isize = msg_send![pb, declareTypes: &*types, owner: nil];
        let value = NSString::from_str(text);
        let _: bool = msg_send![pb, setString: &*value, forType: plain];
        let empty: *mut AnyObject = msg_send![class!(NSData), data];
        let _: bool = msg_send![pb, setData: empty, forType: transient];
        let _: bool = msg_send![pb, setData: empty, forType: autogen];
    }

    /// Restores the general pasteboard from a [`snapshot_pasteboard`] snapshot,
    /// rewriting every saved type. Clears the pasteboard when the snapshot was
    /// empty (nothing was on it to begin with).
    unsafe fn restore_pasteboard(saved: Vec<(*mut NSString, *mut NSData)>) {
        let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let _: isize = msg_send![pb, clearContents];
        if saved.is_empty() {
            return;
        }
        let type_refs: Vec<&NSString> = saved.iter().map(|(ty, _)| &**ty).collect();
        let types = NSArray::from_slice(&type_refs);
        let nil: *mut AnyObject = std::ptr::null_mut();
        let _: isize = msg_send![pb, declareTypes: &*types, owner: nil];
        for (ty, data) in saved {
            let _: bool = msg_send![pb, setData: data, forType: ty];
        }
    }

    /// Pastes `text` into the app identified by `pid`, replacing its selection.
    /// Refuses to write while secure input is active.
    pub fn paste_into(pid: i32, text: &str) -> ReplaceOutcome {
        autoreleasepool(|_| unsafe {
            if is_secure_input() {
                return ReplaceOutcome::Skipped;
            }
            // Activate the target and confirm it actually became frontmost
            // before doing anything else. If it never does, posting Cmd+V would
            // land nowhere or in the wrong app, so skip without touching the
            // clipboard and report it honestly so the UI does not show a false
            // success (and auto-close does not dismiss over a no-op).
            if !activate_and_settle(pid) {
                return ReplaceOutcome::Skipped;
            }
            // Re-check after the activation delay: secure input is a moving
            // target (a password field may have taken focus meanwhile), and the
            // paste must never fire into one.
            if is_secure_input() {
                return ReplaceOutcome::Skipped;
            }
            let saved = snapshot_pasteboard();
            write_pasteboard_transient(text);
            post_cmd_v_to_pid(pid);
            std::thread::sleep(std::time::Duration::from_millis(PASTE_SETTLE_MS));
            restore_pasteboard(saved);
            ReplaceOutcome::Replaced
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_activations_of_other_apps() {
        assert!(should_record_activation(412, 7));
    }

    #[test]
    fn ignores_thukis_own_activations() {
        assert!(!should_record_activation(7, 7));
    }

    #[test]
    fn last_active_state_round_trips() {
        let state = LastActiveAppState::default();
        assert_eq!(state.get(), None);
        state.set(412);
        assert_eq!(state.get(), Some(412));
    }

    #[test]
    fn outcome_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&ReplaceOutcome::Replaced).unwrap(),
            "\"replaced\""
        );
        assert_eq!(
            serde_json::to_string(&ReplaceOutcome::Skipped).unwrap(),
            "\"skipped\""
        );
    }
}
