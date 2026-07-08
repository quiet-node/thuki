//! Unit tests for the pure launch-circuit-breaker logic.
//!
//! Only the pure decision surface is exercised here: `decide`, `healthy_state`,
//! and the `StartupSafety` managed state. The `coverage(off)` I/O and FFI
//! wrappers (`read_state`, `write_state`, `run_startup_guard`, `mark_healthy`,
//! `disable_quit_keeps_windows`) are thin and exempt by contract.

use super::*;

/// A clean prior sentinel produces no safe mode and a dirty-count-zero next
/// state: the normal first-run / healthy-restart path.
#[test]
fn clean_prior_does_not_trip_safe_mode() {
    let prior = PersistedGuardState {
        launch_dirty: false,
        consecutive_unclean: 0,
    };
    let decision = decide(prior, 1);
    assert!(!decision.safe_mode);
    assert_eq!(decision.unclean_count, 0);
    assert_eq!(
        decision.next_state,
        PersistedGuardState {
            launch_dirty: true,
            consecutive_unclean: 0,
        }
    );
}

/// A dirty prior sentinel at threshold 1 trips safe mode on the very next
/// launch: the previous launch never became healthy.
#[test]
fn dirty_prior_trips_safe_mode_at_threshold_one() {
    let prior = PersistedGuardState {
        launch_dirty: true,
        consecutive_unclean: 0,
    };
    let decision = decide(prior, 1);
    assert!(decision.safe_mode);
    assert_eq!(decision.unclean_count, 1);
    assert_eq!(
        decision.next_state,
        PersistedGuardState {
            launch_dirty: true,
            consecutive_unclean: 1,
        }
    );
}

/// The unclean streak increments across repeated dirty launches.
#[test]
fn dirty_prior_increments_streak() {
    let prior = PersistedGuardState {
        launch_dirty: true,
        consecutive_unclean: 7,
    };
    let decision = decide(prior, 1);
    assert!(decision.safe_mode);
    assert_eq!(decision.unclean_count, 8);
    assert_eq!(decision.next_state.consecutive_unclean, 8);
}

/// With threshold 2, a single dirty launch (count 1) is below threshold, so
/// safe mode stays off. Exercises the `>=`-false-while-dirty branch that
/// threshold 1 can never hit.
#[test]
fn below_threshold_does_not_trip_safe_mode() {
    let prior = PersistedGuardState {
        launch_dirty: true,
        consecutive_unclean: 0,
    };
    let decision = decide(prior, 2);
    assert!(!decision.safe_mode);
    assert_eq!(decision.unclean_count, 1);
}

/// `consecutive_unclean` saturates rather than overflowing on a pathological
/// streak, so the guard can never panic in release-overflow-checked builds.
#[test]
fn streak_saturates_at_u32_max() {
    let prior = PersistedGuardState {
        launch_dirty: true,
        consecutive_unclean: u32::MAX,
    };
    let decision = decide(prior, 1);
    assert_eq!(decision.unclean_count, u32::MAX);
}

/// The default sentinel is the clean first-run value.
#[test]
fn default_state_is_clean() {
    assert_eq!(
        PersistedGuardState::default(),
        PersistedGuardState {
            launch_dirty: false,
            consecutive_unclean: 0,
        }
    );
}

/// The healthy sentinel clears both dirty flag and streak.
#[test]
fn healthy_state_is_clean() {
    assert_eq!(
        healthy_state(),
        PersistedGuardState {
            launch_dirty: false,
            consecutive_unclean: 0,
        }
    );
}

/// `StartupSafety` mirrors the decision it was built from.
#[test]
fn startup_safety_reflects_decision() {
    let decision = StartupDecision {
        safe_mode: true,
        unclean_count: 3,
        next_state: PersistedGuardState {
            launch_dirty: true,
            consecutive_unclean: 3,
        },
    };
    let safety = StartupSafety::from_decision(&decision);
    assert!(safety.safe_mode());
    assert_eq!(safety.unclean_count(), 3);
}

/// `clear` resets the managed state to healthy.
#[test]
fn startup_safety_clear_resets() {
    let decision = StartupDecision {
        safe_mode: true,
        unclean_count: 5,
        next_state: PersistedGuardState {
            launch_dirty: true,
            consecutive_unclean: 5,
        },
    };
    let safety = StartupSafety::from_decision(&decision);
    safety.clear();
    assert!(!safety.safe_mode());
    assert_eq!(safety.unclean_count(), 0);
}

/// `snapshot` exposes the current verdict for the frontend.
#[test]
fn startup_safety_snapshot_matches_state() {
    let decision = StartupDecision {
        safe_mode: true,
        unclean_count: 2,
        next_state: PersistedGuardState {
            launch_dirty: true,
            consecutive_unclean: 2,
        },
    };
    let safety = StartupSafety::from_decision(&decision);
    let snap = safety.snapshot();
    assert_eq!(
        snap,
        StartupSafetySnapshot {
            safe_mode: true,
            unclean_count: 2,
        }
    );
}
