//! Unified activation and visibility management for the Thuki overlay.
//!
//! This module coordinates the interaction between system-level input events
//! and the application's visibility state. It provides a non-intrusive monitoring
//! layer that detects specific user intent (via a primary activation trigger)
//! to toggle the overlay.
//!
//! The implementation uses a high-performance background listener with its own
//! event loop, ensuring zero latency impact on the main application or the
//! host system's responsiveness.
//!
//! **macOS Permissions**: This module requires Accessibility permission to
//! monitor system-wide modifier key transitions. It includes self-diagnostic
//! checks and automated permission prompting.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation::string::CFString;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};

/// Maximum temporal proximity between trigger events to qualify as an activation signal.
const ACTIVATION_WINDOW: Duration = Duration::from_millis(400);

/// Minimum interval between successive activations to prevent accidental double-toggles.
const ACTIVATION_COOLDOWN: Duration = Duration::from_millis(600);

/// Primary keycodes used for the activation sequence (macOS Control keys).
const KC_PRIMARY_L: i64 = 0x3b;
const KC_PRIMARY_R: i64 = 0x3e;

/// Maximum number of attempts to establish the event tap while waiting for system permissions.
const MAX_PERMISSION_ATTEMPTS: u32 = 6;

/// Interval between permission check cycles.
const PERMISSION_POLL_INTERVAL: Duration = Duration::from_secs(5);

// ─── Native Framework Interop (macOS ApplicationServices) ──────────────────

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    /// Returns true if the current process is trusted for Accessibility access.
    fn AXIsProcessTrusted() -> bool;

    /// Checks for Accessibility trust, optionally triggering the system-level privacy prompt.
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
}

/// Verifies and optionally requests Accessibility authorization from the OS.
///
/// Under development builds launched via terminal, macOS attributes this
/// permission to the terminal emulator. In production `.app` bundles, the
/// permission is correctly attributed to the application identity.
#[cfg_attr(coverage_nightly, coverage(off))]
fn request_authorization(prompt: bool) -> bool {
    unsafe {
        if AXIsProcessTrusted() {
            return true;
        }

        if prompt {
            // "AXTrustedCheckOptionPrompt" key is the standard mechanism to
            // trigger the macOS Privacy & Security dialog.
            let key = CFString::new("AXTrustedCheckOptionPrompt");
            let value = CFBoolean::true_value();
            let dict = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
            AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef() as *const c_void);
        }

        false
    }
}

// ─── Activation Logic ────────────────────────────────────────────────────────

/// Internal state tracking for the activation sequence.
struct ActivationState {
    /// Timestamp of the last verified event in the sequence.
    last_trigger: Option<Instant>,
    /// Tracks the current physical state of the trigger key.
    is_pressed: bool,
    /// Timestamp of the last successful activation to enforce cooldown.
    last_activation: Option<Instant>,
}

/// Evaluates a raw input event to determine if the activation sequence is complete.
///
/// Implements a state machine that filters for state transitions (press/release)
/// and enforces temporal constraints defined by [`ACTIVATION_WINDOW`].
fn evaluate_activation(state: &mut ActivationState, is_press: bool) -> bool {
    if is_press && !state.is_pressed {
        state.is_pressed = true;
        let now = Instant::now();

        // Enforce cooldown period after a successful activation to prevent
        // rapid tapping from triggering multiple toggles.
        if let Some(last_act) = state.last_activation {
            if now.duration_since(last_act) < ACTIVATION_COOLDOWN {
                return false;
            }
        }

        if let Some(last) = state.last_trigger {
            if now.duration_since(last) < ACTIVATION_WINDOW {
                state.last_trigger = None;
                state.last_activation = Some(now);
                return true;
            }
        }
        state.last_trigger = Some(now);
    } else if !is_press {
        state.is_pressed = false;
    }

    false
}

// ─── Public Interface ────────────────────────────────────────────────────────

/// Orchestrates the lifecycle and threading of the background activation listener.
pub struct OverlayActivator {
    is_active: Arc<AtomicBool>,
}

impl OverlayActivator {
    /// Creates a new, inactive instance of the activator.
    pub fn new() -> Self {
        Self {
            is_active: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Spawns the background monitoring thread and initializes the event loop.
    ///
    /// The method handles initial authorization checks and enters a retry loop
    /// if permissions are not yet available, allowing the user to interact
    /// with system prompts without needing to restart the application.
    ///
    /// # Arguments
    ///
    /// * `on_activation` - A thread-safe closure executed whenever the activation
    ///   sequence is detected.
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn start<F>(&self, on_activation: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        if self.is_active.load(Ordering::SeqCst) {
            return;
        }
        self.is_active.store(true, Ordering::SeqCst);

        // Check authorization without prompting. The onboarding screen owns
        // the responsibility of directing the user to System Settings when
        // Accessibility is not yet granted.
        request_authorization(false);

        let is_active = self.is_active.clone();
        let on_activation = Arc::new(on_activation);

        std::thread::spawn(move || {
            run_loop_with_retry(is_active, on_activation);
        });
    }
}

/// Persistence layer that maintains the event loop through permission cycles.
#[cfg_attr(coverage_nightly, coverage(off))]
fn run_loop_with_retry<F>(is_active: Arc<AtomicBool>, on_activation: Arc<F>)
where
    F: Fn() + Send + Sync + 'static,
{
    for attempt in 1..=MAX_PERMISSION_ATTEMPTS {
        if attempt > 1 {
            std::thread::sleep(PERMISSION_POLL_INTERVAL);
            if !request_authorization(false) {
                continue;
            }
        }

        if try_initialize_tap(&is_active, &on_activation) {
            return; // Successfully established and running
        }
    }

    eprintln!("thuki: [error] activation listener failed after maximum retries. check system permissions.");
}

/// Core initialization of the Mach event tap.
#[cfg_attr(coverage_nightly, coverage(off))]
fn try_initialize_tap<F>(is_active: &Arc<AtomicBool>, on_activation: &Arc<F>) -> bool
where
    F: Fn() + Send + Sync + 'static,
{
    let state = Arc::new(Mutex::new(ActivationState {
        last_trigger: None,
        is_pressed: false,
        last_activation: None,
    }));

    let cb_active = is_active.clone();
    let cb_on_activation = on_activation.clone();
    let cb_state = state.clone();

    // Create the event tap at the Session level. This requires Accessibility
    // permission but does not require root/superuser privileges unlike HID level.
    let tap_result = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
        vec![CGEventType::FlagsChanged],
        move |_proxy, _event_type, event: &CGEvent| -> CallbackResult {
            if !cb_active.load(Ordering::SeqCst) {
                CFRunLoop::get_current().stop();
                return CallbackResult::Keep;
            }

            let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
            let flags = event.get_flags();

            // Filter for primary triggers (Modifier keys)
            if keycode != KC_PRIMARY_L && keycode != KC_PRIMARY_R {
                return CallbackResult::Keep;
            }

            // Check specific bitmask for the Control key state
            let is_press = flags.contains(CGEventFlags::CGEventFlagControl);

            let mut s = cb_state.lock().unwrap();
            if evaluate_activation(&mut s, is_press) {
                cb_on_activation();
            }

            CallbackResult::Keep
        },
    );

    match tap_result {
        Ok(tap) => {
            unsafe {
                let loop_source = tap
                    .mach_port()
                    .create_runloop_source(0)
                    .expect("failed to create run loop source");

                let run_loop = CFRunLoop::get_current();
                run_loop.add_source(&loop_source, kCFRunLoopCommonModes);
                tap.enable();

                CFRunLoop::run_current();
            }
            true
        }
        Err(()) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_activator_is_inactive() {
        let activator = OverlayActivator::new();
        assert!(!activator
            .is_active
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn validates_activation_sequence() {
        let mut state = ActivationState {
            last_trigger: None,
            is_pressed: false,
            last_activation: None,
        };

        // First event
        assert!(!evaluate_activation(&mut state, true));
        evaluate_activation(&mut state, false);

        // Sequence completion
        assert!(evaluate_activation(&mut state, true));
    }

    #[test]
    fn rejects_stale_sequence() {
        let mut state = ActivationState {
            last_trigger: None,
            is_pressed: false,
            last_activation: None,
        };

        evaluate_activation(&mut state, true);
        evaluate_activation(&mut state, false);

        // Simulate temporal drift beyond window
        state.last_trigger = Some(Instant::now() - Duration::from_millis(500));

        assert!(!evaluate_activation(&mut state, true));
    }

    #[test]
    fn cooldown_rejects_activation_within_window() {
        let mut state = ActivationState {
            last_trigger: None,
            is_pressed: false,
            last_activation: None,
        };

        // Complete first activation
        evaluate_activation(&mut state, true);
        evaluate_activation(&mut state, false);
        assert!(evaluate_activation(&mut state, true));
        evaluate_activation(&mut state, false);

        // Try to activate again immediately — within 600ms cooldown
        evaluate_activation(&mut state, true);
        evaluate_activation(&mut state, false);
        // This should be rejected by cooldown
        assert!(!evaluate_activation(&mut state, true));
    }

    #[test]
    fn cooldown_allows_activation_after_expiry() {
        let mut state = ActivationState {
            last_trigger: None,
            is_pressed: false,
            last_activation: None,
        };

        // Complete first activation
        evaluate_activation(&mut state, true);
        evaluate_activation(&mut state, false);
        assert!(evaluate_activation(&mut state, true));
        evaluate_activation(&mut state, false);

        // Simulate cooldown expiry
        state.last_activation = Some(Instant::now() - Duration::from_millis(700));

        // Should work now
        evaluate_activation(&mut state, true);
        evaluate_activation(&mut state, false);
        assert!(evaluate_activation(&mut state, true));
    }

    #[test]
    fn boundary_timing_at_exactly_400ms_is_rejected() {
        let mut state = ActivationState {
            last_trigger: Some(Instant::now() - Duration::from_millis(400)),
            is_pressed: false,
            last_activation: None,
        };

        assert!(!evaluate_activation(&mut state, true));
    }

    #[test]
    fn boundary_timing_at_399ms_is_accepted() {
        let mut state = ActivationState {
            last_trigger: Some(Instant::now() - Duration::from_millis(399)),
            is_pressed: false,
            last_activation: None,
        };

        assert!(evaluate_activation(&mut state, true));
    }

    #[test]
    fn first_tap_records_timestamp() {
        let mut state = ActivationState {
            last_trigger: None,
            is_pressed: false,
            last_activation: None,
        };

        assert!(!evaluate_activation(&mut state, true));
        assert!(state.last_trigger.is_some());
    }

    #[test]
    fn state_resets_after_successful_activation() {
        let mut state = ActivationState {
            last_trigger: None,
            is_pressed: false,
            last_activation: None,
        };

        evaluate_activation(&mut state, true);
        evaluate_activation(&mut state, false);
        assert!(evaluate_activation(&mut state, true));

        assert!(state.last_trigger.is_none());
        assert!(state.last_activation.is_some());
    }

    #[test]
    fn repeated_press_without_release_is_ignored() {
        let mut state = ActivationState {
            last_trigger: None,
            is_pressed: false,
            last_activation: None,
        };

        evaluate_activation(&mut state, true);
        assert!(!evaluate_activation(&mut state, true));
    }

    #[test]
    fn release_without_press_does_nothing() {
        let mut state = ActivationState {
            last_trigger: None,
            is_pressed: false,
            last_activation: None,
        };

        assert!(!evaluate_activation(&mut state, false));
        assert!(state.last_trigger.is_none());
    }
}
