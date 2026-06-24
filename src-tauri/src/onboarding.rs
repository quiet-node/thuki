/*!
 * Onboarding stage management.
 *
 * Tracks the user's progress through the onboarding flow using a single
 * persisted value in the `app_config` table.
 *
 * Stages progress linearly for a brand-new install:
 *   "permissions" -> "model_check" -> "intro" -> "complete"
 *
 * Upgraders coming from a pre-built-in-engine version take one extra step
 * once permissions are satisfied:
 *   "permissions" -> "builtin_announcement" -> "model_check" -> "intro" -> ...
 *
 * "permissions" is the implicit default when no value has been written yet.
 * "builtin_announcement" tells a grandfathered Ollama user that Thuki now
 * ships its own engine; it is shown at most once, gated by the
 * `builtin_engine_announced` flag (see [`is_builtin_announced`]). "model_check"
 * gates the user on a usable model for the active provider. All pre-complete
 * stages are skipped on every subsequent launch once advanced past. Once
 * "complete", onboarding is never shown again regardless of permissions or
 * installed models.
 *
 * Backward compatibility: existing installs with persisted stages of
 * "permissions", "intro", or "complete" all parse correctly. Newer values
 * ("model_check", "builtin_announcement") are unknown to older installs but
 * the format is forward-compatible (unknown stages fall back to Permissions,
 * the safe default that re-runs the full flow).
 */

use rusqlite::Connection;

use crate::database::{get_config, set_config};

/// The config key used to store the onboarding stage.
const STAGE_KEY: &str = "onboarding_stage";

/// The config key used to store whether the built-in engine announcement has
/// been shown to (and answered by) this install. A one-way latch: once set to
/// `"true"` the announcement never returns.
const BUILTIN_ANNOUNCED_KEY: &str = "builtin_engine_announced";

/// The config key recording that an upgrader is owed the built-in engine
/// announcement. Latched the moment an upgrader is recognised (see
/// [`is_pre_builtin_upgrader`]) and consulted thereafter, so the notice
/// survives the permission flow no matter which stage that flow lands on.
const ANNOUNCEMENT_PENDING_KEY: &str = "builtin_engine_announcement_pending";

/// Provider kind that identifies a grandfathered upgrader: pre-built-in-engine
/// installs are pinned to Ollama by the config loader, so an Ollama-kind active
/// provider is the signal that the user predates the built-in engine.
const OLLAMA_PROVIDER_KIND: &str = "ollama";

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
    /// One-time notice shown to upgraders that the built-in engine now exists.
    /// Routed to by [`decide_startup_route`] when the announcement is pending.
    BuiltinAnnouncement,
    ModelCheck,
    Intro,
    Complete,
}

/// Reads the persisted onboarding stage. Returns `Permissions` if no value
/// has been written yet (first-ever launch) or if the persisted value is
/// not recognised (forward-compatible with future stage names).
pub fn get_stage(conn: &Connection) -> rusqlite::Result<OnboardingStage> {
    match get_config(conn, STAGE_KEY)?.as_deref() {
        Some("builtin_announcement") => Ok(OnboardingStage::BuiltinAnnouncement),
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
        OnboardingStage::BuiltinAnnouncement => "builtin_announcement",
        OnboardingStage::ModelCheck => "model_check",
        OnboardingStage::Intro => "intro",
        OnboardingStage::Complete => "complete",
    };
    set_config(conn, STAGE_KEY, value)
}

/// Returns `true` once the built-in engine announcement has been answered on
/// this install. Defaults to `false` (announcement still pending) when the flag
/// has never been written, which is exactly the state of every install that
/// predates the built-in engine.
pub fn is_builtin_announced(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(get_config(conn, BUILTIN_ANNOUNCED_KEY)?.as_deref() == Some("true"))
}

/// Latches the built-in engine announcement as shown so it never returns.
/// Called when the user answers it (either branch) and when a brand-new install
/// finishes onboarding, so a later provider switch to Ollama cannot resurface
/// an "upgrade" notice the user never needed.
pub fn mark_builtin_announced(conn: &Connection) -> rusqlite::Result<()> {
    set_config(conn, BUILTIN_ANNOUNCED_KEY, "true")
}

/// Returns `true` once an upgrader has been recognised and is owed the built-in
/// engine announcement. Defaults to `false`.
pub fn is_announcement_pending(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(get_config(conn, ANNOUNCEMENT_PENDING_KEY)?.as_deref() == Some("true"))
}

/// Latches the pending built-in engine announcement for an upgrader.
pub fn set_announcement_pending(conn: &Connection) -> rusqlite::Result<()> {
    set_config(conn, ANNOUNCEMENT_PENDING_KEY, "true")
}

/// Recognises a pre-built-in-engine upgrader at the one reliable moment: their
/// first launch on the new version, where the persisted stage is still
/// `Complete` (they finished onboarding on the old version) and the active
/// provider is Ollama (the config loader pins every pre-providers install to
/// Ollama), and the announcement has not been answered.
///
/// A fresh install is never `Complete` before it finishes onboarding, so it
/// never matches, even if the user later takes the in-picker "Use my existing
/// Ollama instead" hatch. The check must run before the permission flow can
/// clobber the stage; once it matches, the caller latches
/// [`set_announcement_pending`] so the decision survives that flow.
pub fn is_pre_builtin_upgrader(
    stage: &OnboardingStage,
    active_provider_kind: &str,
    announced: bool,
) -> bool {
    matches!(stage, OnboardingStage::Complete)
        && active_provider_kind == OLLAMA_PROVIDER_KIND
        && !announced
}

/// Where startup should route the user, decided purely from the persisted
/// stage, live permission grants, and the announcement latch. Keeping the
/// branching here (rather than in the side-effecting Tauri entry point) makes
/// the whole routing table unit-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupRoute {
    /// Re-run the permissions gate (a grant is missing or was revoked).
    ShowPermissions,
    /// Show the one-time built-in engine announcement to an upgrader.
    ShowAnnouncement,
    /// Show the model gate (built-in picker or Ollama setup).
    ShowModelCheck,
    /// Show the intro tour.
    ShowIntro,
    /// Onboarding is complete and permissions are intact: show the overlay.
    ShowOverlay,
}

/// Pure startup router. Inputs are the persisted `stage`, live accessibility
/// (`ax`) and screen-recording (`sr`) grants, and the two announcement latches
/// (`announced`, `pending`). Provider kind is not needed here: it only feeds the
/// upstream upgrader detection that sets `pending`.
///
/// Order matters and encodes two invariants:
/// - The `Intro` fast-path skips the live permission re-check, because
///   `CGPreflightScreenCaptureAccess` can return a stale false negative right
///   after a restart and would otherwise loop an intro-stage user back to
///   permissions. It is taken only when no announcement is owed.
/// - The announcement is shown only after permissions are confirmed granted, so
///   a pending notice waits behind a missing grant rather than rendering over a
///   half-permissioned app; the latch persists, so it re-shows next launch.
pub fn decide_startup_route(
    stage: &OnboardingStage,
    ax: bool,
    sr: bool,
    announced: bool,
    pending: bool,
) -> StartupRoute {
    let show_announcement = pending && !announced;

    if matches!(stage, OnboardingStage::Intro) && !show_announcement {
        return StartupRoute::ShowIntro;
    }
    if !ax || !sr {
        return StartupRoute::ShowPermissions;
    }
    if show_announcement {
        return StartupRoute::ShowAnnouncement;
    }
    if matches!(
        stage,
        OnboardingStage::Permissions
            | OnboardingStage::BuiltinAnnouncement
            | OnboardingStage::ModelCheck
    ) {
        return StartupRoute::ShowModelCheck;
    }
    StartupRoute::ShowOverlay
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
    fn set_and_get_stage_round_trips_builtin_announcement() {
        let conn = open_in_memory().unwrap();
        set_stage(&conn, &OnboardingStage::BuiltinAnnouncement).unwrap();
        assert_eq!(
            get_stage(&conn).unwrap(),
            OnboardingStage::BuiltinAnnouncement
        );
    }

    #[test]
    fn is_builtin_announced_defaults_to_false_for_upgraders() {
        let conn = open_in_memory().unwrap();
        assert!(!is_builtin_announced(&conn).unwrap());
    }

    #[test]
    fn mark_builtin_announced_latches_the_flag() {
        let conn = open_in_memory().unwrap();
        mark_builtin_announced(&conn).unwrap();
        assert!(is_builtin_announced(&conn).unwrap());
    }

    #[test]
    fn is_builtin_announced_false_for_unrecognised_value() {
        // Defensive: any non-"true" stored value reads as not-yet-announced,
        // so a corrupted flag re-shows the notice rather than silently hiding it.
        let conn = open_in_memory().unwrap();
        crate::database::set_config(&conn, BUILTIN_ANNOUNCED_KEY, "yes").unwrap();
        assert!(!is_builtin_announced(&conn).unwrap());
    }

    #[test]
    fn announcement_pending_defaults_false_and_latches() {
        let conn = open_in_memory().unwrap();
        assert!(!is_announcement_pending(&conn).unwrap());
        set_announcement_pending(&conn).unwrap();
        assert!(is_announcement_pending(&conn).unwrap());
    }

    #[test]
    fn is_pre_builtin_upgrader_only_for_completed_unannounced_ollama() {
        use OnboardingStage::*;
        // The upgrader: finished onboarding on the old version, still on Ollama,
        // never announced.
        assert!(is_pre_builtin_upgrader(&Complete, "ollama", false));
        // Already announced: not owed the notice.
        assert!(!is_pre_builtin_upgrader(&Complete, "ollama", true));
        // A fresh install is never Complete before it finishes, so a mid-flow
        // Ollama choice (any non-Complete stage) is not an upgrader.
        assert!(!is_pre_builtin_upgrader(&Permissions, "ollama", false));
        assert!(!is_pre_builtin_upgrader(&ModelCheck, "ollama", false));
        assert!(!is_pre_builtin_upgrader(&Intro, "ollama", false));
        // Built-in (or any non-Ollama) provider is never a grandfathered user.
        assert!(!is_pre_builtin_upgrader(&Complete, "builtin", false));
        assert!(!is_pre_builtin_upgrader(&Complete, "openai", false));
    }

    #[test]
    fn decide_startup_route_shows_announcement_regardless_of_landing_stage() {
        use OnboardingStage::*;
        // The bug this feature exists to avoid: an upgrader whose permission
        // flow left them at Intro must still get the announcement.
        assert_eq!(
            decide_startup_route(&Intro, true, true, false, true),
            StartupRoute::ShowAnnouncement
        );
        // No-reset upgrader sits at Complete.
        assert_eq!(
            decide_startup_route(&Complete, true, true, false, true),
            StartupRoute::ShowAnnouncement
        );
        // A session that quit on the announcement re-shows it.
        assert_eq!(
            decide_startup_route(&BuiltinAnnouncement, true, true, false, true),
            StartupRoute::ShowAnnouncement
        );
    }

    #[test]
    fn decide_startup_route_pending_waits_behind_missing_permissions() {
        use OnboardingStage::*;
        // A missing grant takes priority; the latch persists for next launch.
        assert_eq!(
            decide_startup_route(&Complete, false, true, false, true),
            StartupRoute::ShowPermissions
        );
        assert_eq!(
            decide_startup_route(&Permissions, true, false, false, true),
            StartupRoute::ShowPermissions
        );
    }

    #[test]
    fn decide_startup_route_answered_announcement_never_reshows() {
        use OnboardingStage::*;
        // pending stays latched, but `announced` suppresses the notice and the
        // user proceeds to the model gate.
        assert_eq!(
            decide_startup_route(&BuiltinAnnouncement, true, true, true, true),
            StartupRoute::ShowModelCheck
        );
    }

    #[test]
    fn decide_startup_route_leaves_new_users_untouched() {
        use OnboardingStage::*;
        // New user at the intro tour: untouched fast-path, no perm re-check.
        assert_eq!(
            decide_startup_route(&Intro, false, false, false, false),
            StartupRoute::ShowIntro
        );
        // New user, permissions granted, heading into the picker.
        assert_eq!(
            decide_startup_route(&Permissions, true, true, false, false),
            StartupRoute::ShowModelCheck
        );
        // Completed non-upgrader: straight to the overlay.
        assert_eq!(
            decide_startup_route(&Complete, true, true, false, false),
            StartupRoute::ShowOverlay
        );
        // Model gate stage routes back to the model gate.
        assert_eq!(
            decide_startup_route(&ModelCheck, true, true, false, false),
            StartupRoute::ShowModelCheck
        );
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
            serde_json::to_string(&OnboardingStage::BuiltinAnnouncement).unwrap(),
            "\"builtin_announcement\""
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
}
