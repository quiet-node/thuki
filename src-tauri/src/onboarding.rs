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

/// Relaunch-safety gate over the persisted stage.
///
/// The non-blocking download experience lets the user leave the picker while a
/// built-in model is still downloading: tapping "Continue setup" persists
/// `intro`, and finishing the intro tour persists `complete`, all before the
/// download completes. A quit + relaunch mid-download would otherwise strand
/// the user past model selection with no usable model.
///
/// So when the persisted stage is past model selection (`intro`/`complete`)
/// but the built-in engine is active with zero installed models AND a resumable
/// partial is on disk, force the user back to `model_check` to finish (resume)
/// the download. Every other case is returned unchanged.
///
/// The partial is the load-bearing signal: a deliberate delete-model-in-Settings
/// leaves no partial, so it does NOT re-trigger onboarding. Stages before model
/// selection (`permissions`/`model_check`) are never touched: the user has not
/// reached the picker yet, so there is nothing to relaunch-protect.
pub fn apply_model_gate(
    stage: OnboardingStage,
    is_builtin: bool,
    has_model: bool,
    has_partial: bool,
) -> OnboardingStage {
    let past_model_selection = matches!(stage, OnboardingStage::Intro | OnboardingStage::Complete);
    if past_model_selection && is_builtin && !has_model && has_partial {
        OnboardingStage::ModelCheck
    } else {
        stage
    }
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
    fn compute_startup_stage_shows_model_check_when_stage_is_model_check() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::ModelCheck).unwrap();
        assert_eq!(
            compute_startup_stage(&conn).unwrap(),
            Some(OnboardingStage::ModelCheck)
        );
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

    // ── apply_model_gate (relaunch safety) ───────────────────────────────────

    #[test]
    fn model_gate_forces_model_check_from_intro_mid_download() {
        // Quit while the model was still downloading after tapping Continue:
        // builtin, no model installed yet, a partial left on disk.
        assert_eq!(
            apply_model_gate(OnboardingStage::Intro, true, false, true),
            OnboardingStage::ModelCheck
        );
    }

    #[test]
    fn model_gate_forces_model_check_from_complete_mid_download() {
        // Quit after Get Started but before the download finished.
        assert_eq!(
            apply_model_gate(OnboardingStage::Complete, true, false, true),
            OnboardingStage::ModelCheck
        );
    }

    #[test]
    fn model_gate_keeps_stage_when_a_model_is_installed() {
        // Download completed: a model exists, so nothing to recover.
        assert_eq!(
            apply_model_gate(OnboardingStage::Intro, true, true, true),
            OnboardingStage::Intro
        );
        assert_eq!(
            apply_model_gate(OnboardingStage::Complete, true, true, false),
            OnboardingStage::Complete
        );
    }

    #[test]
    fn model_gate_ignores_a_deliberate_delete_with_no_partial() {
        // Model deleted in Settings (no partial left): must NOT re-onboard.
        assert_eq!(
            apply_model_gate(OnboardingStage::Intro, true, false, false),
            OnboardingStage::Intro
        );
        assert_eq!(
            apply_model_gate(OnboardingStage::Complete, true, false, false),
            OnboardingStage::Complete
        );
    }

    #[test]
    fn model_gate_ignores_non_builtin_providers() {
        // An Ollama/openai user with a stray partial is never re-gated.
        assert_eq!(
            apply_model_gate(OnboardingStage::Complete, false, false, true),
            OnboardingStage::Complete
        );
    }

    #[test]
    fn model_gate_leaves_pre_selection_stages_untouched() {
        // Before the picker there is nothing to relaunch-protect.
        assert_eq!(
            apply_model_gate(OnboardingStage::Permissions, true, false, true),
            OnboardingStage::Permissions
        );
        assert_eq!(
            apply_model_gate(OnboardingStage::ModelCheck, true, false, true),
            OnboardingStage::ModelCheck
        );
    }
}
