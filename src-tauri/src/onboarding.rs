/*!
 * Onboarding stage management.
 *
 * Tracks the user's progress through the onboarding flow using a single
 * persisted value in the `app_config` table.
 *
 * Stages progress linearly:
 *   "permissions" -> "intro" -> "complete"
 *
 * "permissions" is the implicit default when no value has been written yet.
 * Once "complete", onboarding is never shown again regardless of permissions.
 */

use rusqlite::Connection;

use crate::database::{get_config, set_config};

/// The config key used to store the onboarding stage.
const STAGE_KEY: &str = "onboarding_stage";

/// Serializable stage value sent to the frontend via the onboarding event.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OnboardingStage {
    Permissions,
    Intro,
    Complete,
}

/// Reads the persisted onboarding stage. Returns `Permissions` if no value
/// has been written yet (i.e. first-ever launch).
pub fn get_stage(conn: &Connection) -> rusqlite::Result<OnboardingStage> {
    match get_config(conn, STAGE_KEY)?.as_deref() {
        Some("intro") => Ok(OnboardingStage::Intro),
        Some("complete") => Ok(OnboardingStage::Complete),
        _ => Ok(OnboardingStage::Permissions),
    }
}

/// Persists the onboarding stage.
pub fn set_stage(conn: &Connection, stage: &OnboardingStage) -> rusqlite::Result<()> {
    let value = match stage {
        OnboardingStage::Permissions => "permissions",
        OnboardingStage::Intro => "intro",
        OnboardingStage::Complete => "complete",
    };
    set_config(conn, STAGE_KEY, value)
}

/// Returns which onboarding stage to show at startup, or `None` if onboarding
/// is complete.
///
/// Reads only the persisted stage: no permission API calls. Permission APIs
/// (CGPreflightScreenCaptureAccess) can return stale results immediately after
/// a process restart on macOS 15+. PermissionsStep owns live permission
/// detection via its own polling checks. quit_and_relaunch writes "intro" to
/// the DB before restarting so this path sees the correct stage on next launch.
pub fn compute_startup_stage(conn: &Connection) -> rusqlite::Result<Option<OnboardingStage>> {
    match get_stage(conn)? {
        OnboardingStage::Complete => Ok(None),
        stage => Ok(Some(stage)),
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
    fn set_and_get_stage_round_trips_complete() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::Complete).unwrap();
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::Complete);
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
    fn compute_startup_stage_returns_none_when_complete() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::Complete).unwrap();
        assert_eq!(compute_startup_stage(&conn).unwrap(), None);
    }

    #[test]
    fn compute_startup_stage_shows_permissions_when_not_granted() {
        let conn = open_in_memory().unwrap();
        // Default stage is "permissions" on first launch.
        let result = compute_startup_stage(&conn).unwrap();
        assert_eq!(result, Some(OnboardingStage::Permissions));
        // Stage must not have been modified.
        assert_eq!(get_stage(&conn).unwrap(), OnboardingStage::Permissions);
    }

    #[test]
    fn compute_startup_stage_shows_intro_when_stage_is_intro() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::Intro).unwrap();
        let result = compute_startup_stage(&conn).unwrap();
        assert_eq!(result, Some(OnboardingStage::Intro));
    }

    #[test]
    fn compute_startup_stage_trusts_intro_stage_even_if_permissions_check_fails() {
        let conn = open_in_memory().unwrap();
        // Startup trusts the persisted stage entirely. No permission API is
        // called. CGPreflightScreenCaptureAccess can return false on macOS 15
        // even after a successful grant+restart, so startup never gates on it.
        set_stage(&conn, &OnboardingStage::Intro).unwrap();
        let result = compute_startup_stage(&conn).unwrap();
        assert_eq!(result, Some(OnboardingStage::Intro));
    }
}
