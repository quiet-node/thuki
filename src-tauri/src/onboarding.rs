/*!
 * Onboarding stage management.
 *
 * Tracks the user's progress through the onboarding flow using a single
 * persisted value in the `app_config` table.
 *
 * Stages progress linearly:
 *   "permissions" -> "model_check" -> "intro" -> "complete"
 *
 * "permissions" is the implicit default when no value has been written yet.
 * "model_check" gates the user on having Ollama running with at least one
 * installed model. Both stages are skipped on every subsequent launch once
 * advanced past. Once "complete", onboarding is never shown again regardless
 * of permissions or installed models.
 *
 * Backward compatibility: existing installs with persisted stages of
 * "permissions", "intro", or "complete" all parse correctly. The new
 * "model_check" value is unknown to older installs but the file format is
 * forward-compatible (unknown stages fall back to Permissions, the safe
 * default that re-runs the full flow).
 */

use rusqlite::Connection;

use crate::database::{get_config, set_config};

/// The config key used to store the onboarding stage.
const STAGE_KEY: &str = "onboarding_stage";

/// Serializable stage value sent to the frontend via the onboarding event.
///
/// Variants are emitted in `snake_case` for the frontend to match the
/// `OnboardingStage` TypeScript union exactly. The persisted SQLite value
/// uses the same string form, so the on-disk format is identical to the
/// wire format.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStage {
    Permissions,
    ModelCheck,
    Intro,
    Complete,
}

/// Reads the persisted onboarding stage. Returns `Permissions` if no value
/// has been written yet (first-ever launch) or if the persisted value is
/// not recognised (forward-compatible with future stage names).
pub fn get_stage(conn: &Connection) -> rusqlite::Result<OnboardingStage> {
    match get_config(conn, STAGE_KEY)?.as_deref() {
        Some("model_check") => Ok(OnboardingStage::ModelCheck),
        Some("intro") => Ok(OnboardingStage::Intro),
        Some("complete") => Ok(OnboardingStage::Complete),
        _ => Ok(OnboardingStage::Permissions),
    }
}

/// Persists the onboarding stage.
pub fn set_stage(conn: &Connection, stage: &OnboardingStage) -> rusqlite::Result<()> {
    let value = match stage {
        OnboardingStage::Permissions => "permissions",
        OnboardingStage::ModelCheck => "model_check",
        OnboardingStage::Intro => "intro",
        OnboardingStage::Complete => "complete",
    };
    set_config(conn, STAGE_KEY, value)
}

/// What `notify_frontend_ready` should do at startup.
#[derive(Debug, PartialEq)]
pub enum StartupAction {
    /// Show the given onboarding step.
    ShowOnboarding(OnboardingStage),
    /// Onboarding is finished and both permissions are intact: show the overlay.
    ShowOverlay,
}

/// Decides what to show at startup from the persisted stage and the live
/// permission grants.
///
/// In-progress stages (`Permissions`, `ModelCheck`, `Intro`) are trusted as-is
/// and never re-gated on the live permission APIs. `AXIsProcessTrusted` and
/// `CGPreflightScreenCaptureAccess` return false negatives for a short settle
/// window after a process restart on macOS 15+, so re-checking them here would
/// bounce a user who just granted a permission and restarted back to the start
/// of onboarding. This is exactly what happens when the user accepts macOS's
/// own "Quit & Reopen" prompt after toggling Screen Recording on: the restart
/// does not advance the stage, and a flaky post-restart check would loop them
/// to step 1. The step components own live permission detection via their own
/// polling, and `advance_past_permissions` / `advance_past_model_check` persist
/// the forward progress, so trusting the stage here is safe.
///
/// Only `Complete` consults the live grants, to catch a genuine permission
/// revocation after onboarding has finished and re-run the flow.
pub fn decide_startup_action(
    stage: OnboardingStage,
    accessibility: bool,
    screen_recording: bool,
) -> StartupAction {
    match stage {
        OnboardingStage::Complete => {
            if crate::permissions::needs_onboarding(accessibility, screen_recording) {
                StartupAction::ShowOnboarding(OnboardingStage::Permissions)
            } else {
                StartupAction::ShowOverlay
            }
        }
        in_progress => StartupAction::ShowOnboarding(in_progress),
    }
}

/// Persists the `Complete` stage, marking onboarding as finished.
///
/// Called by the `finish_onboarding` Tauri command after the user clicks
/// "Get Started". Extracted so the DB write is covered by tests independently
/// of the Tauri command wrapper.
pub fn mark_complete(conn: &Connection) -> rusqlite::Result<()> {
    set_stage(conn, &OnboardingStage::Complete)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::open_in_memory;

    #[test]
    fn get_stage_defaults_to_permissions_on_first_launch() {
        let conn = open_in_memory().unwrap();
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::Permissions);
    }

    #[test]
    fn set_and_get_stage_round_trips_permissions() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::Permissions).unwrap();
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::Permissions);
    }

    #[test]
    fn set_and_get_stage_round_trips_intro() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::Intro).unwrap();
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::Intro);
    }

    #[test]
    fn set_and_get_stage_round_trips_model_check() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::ModelCheck).unwrap();
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::ModelCheck);
    }

    #[test]
    fn set_and_get_stage_round_trips_complete() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::Complete).unwrap();
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::Complete);
    }

    #[test]
    fn get_stage_falls_back_to_permissions_on_unknown_value() {
        // Forward-compat guard: if a future build wrote an unrecognised
        // stage and the user downgrades, we must safely re-run the flow
        // rather than panic or pick an arbitrary stage.
        let conn = open_in_memory().unwrap();
        crate::database::set_config(&conn, STAGE_KEY, "future_stage").unwrap();
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::Permissions);
    }

    #[test]
    fn stage_serializes_to_snake_case_for_frontend() {
        // Wire format must match the TypeScript OnboardingStage union exactly.
        // Frontend routes on these strings, so any drift breaks the dispatch.
        assert_eq!(
            serde_json::to_string(&OnboardingStage::Permissions).unwrap(),
            "\"permissions\""
        );
        assert_eq!(
            serde_json::to_string(&OnboardingStage::ModelCheck).unwrap(),
            "\"model_check\""
        );
        assert_eq!(
            serde_json::to_string(&OnboardingStage::Intro).unwrap(),
            "\"intro\""
        );
        assert_eq!(
            serde_json::to_string(&OnboardingStage::Complete).unwrap(),
            "\"complete\""
        );
    }

    #[test]
    fn set_stage_overwrites_previous_value() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::Intro).unwrap();
        set_stage(&conn, &OnboardingStage::Complete).unwrap();
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::Complete);
    }

    #[test]
    fn mark_complete_sets_stage_to_complete() {
        let conn = open_in_memory().unwrap();
        mark_complete(&conn).unwrap();
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::Complete);
    }

    #[test]
    fn mark_complete_overwrites_any_prior_stage() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::Intro).unwrap();
        mark_complete(&conn).unwrap();
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::Complete);
    }

    #[test]
    fn decide_startup_action_shows_overlay_when_complete_and_granted() {
        assert_eq!(
            decide_startup_action(OnboardingStage::Complete, true, true),
            StartupAction::ShowOverlay
        );
    }

    #[test]
    fn decide_startup_action_reruns_onboarding_when_complete_but_accessibility_revoked() {
        // Genuine post-onboarding revocation: the only stage that consults the
        // live grants. A revoked permission must restart the flow at step 1.
        assert_eq!(
            decide_startup_action(OnboardingStage::Complete, false, true),
            StartupAction::ShowOnboarding(OnboardingStage::Permissions)
        );
    }

    #[test]
    fn decide_startup_action_reruns_onboarding_when_complete_but_screen_recording_revoked() {
        assert_eq!(
            decide_startup_action(OnboardingStage::Complete, true, false),
            StartupAction::ShowOnboarding(OnboardingStage::Permissions)
        );
    }

    #[test]
    fn decide_startup_action_shows_permissions_on_first_launch() {
        assert_eq!(
            decide_startup_action(OnboardingStage::Permissions, false, false),
            StartupAction::ShowOnboarding(OnboardingStage::Permissions)
        );
    }

    #[test]
    fn decide_startup_action_trusts_model_check_stage_even_if_live_checks_fail() {
        // The macOS-initiated "Quit & Reopen" after a Screen Recording grant
        // restarts us without advancing the stage, and the live permission
        // APIs read stale-false in the settle window right after. Trusting the
        // persisted stage is what stops the user being bounced back to step 1.
        assert_eq!(
            decide_startup_action(OnboardingStage::ModelCheck, false, false),
            StartupAction::ShowOnboarding(OnboardingStage::ModelCheck)
        );
    }

    #[test]
    fn decide_startup_action_trusts_intro_stage_even_if_live_checks_fail() {
        assert_eq!(
            decide_startup_action(OnboardingStage::Intro, false, false),
            StartupAction::ShowOnboarding(OnboardingStage::Intro)
        );
    }

    #[test]
    fn decide_startup_action_trusts_permissions_stage_even_when_both_granted() {
        // When perms are already granted at the permissions stage we still show
        // PermissionsStep; it auto-advances via `advance_past_permissions`
        // rather than the startup path re-deriving the advance from a flaky
        // live check.
        assert_eq!(
            decide_startup_action(OnboardingStage::Permissions, true, true),
            StartupAction::ShowOnboarding(OnboardingStage::Permissions)
        );
    }
}
