/*!
 * Active-model state module.
 *
 * Single source of truth for the locally-selected Ollama model. The "active"
 * model is whichever slug the user last picked via the picker popup,
 * persisted across launches in `app_config` under [`ACTIVE_MODEL_KEY`] and
 * mirrored in [`ActiveModelState`] for fast reads from Tauri commands.
 *
 * The backend treats Ollama's `/api/tags` response as authoritative: a
 * persisted model is only honored if it still appears in the live installed
 * list. If not, we fall back to the first installed model, then to the
 * bootstrap default from `THUKI_SUPPORTED_AI_MODELS`.
 */

use std::sync::Mutex;

use serde::Deserialize;

use crate::config::defaults::DEFAULT_OLLAMA_URL;
use crate::database::{get_config, set_config};
use crate::history::Database;

/// `app_config` key used to persist the user's selected model slug.
pub const ACTIVE_MODEL_KEY: &str = "active_model";

/// In-memory cache of the currently active model slug. Written once at
/// startup (after `resolve_active_model`) and updated every time the user
/// picks a new model via `set_active_model`.
#[derive(Default)]
pub struct ActiveModelState(pub Mutex<String>);

/// Top-level shape of the Ollama `/api/tags` response. Only the `models`
/// array is consumed; all other fields are ignored.
#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagsModel>,
}

/// A single entry in the `/api/tags` `models` array. Only the `name` slug
/// is needed; everything else (size, digest, modified_at, details) is
/// deliberately ignored to keep the schema surface small.
#[derive(Deserialize)]
struct TagsModel {
    name: String,
}

/// Chooses which model slug should be active given a persisted preference,
/// the live installed list from Ollama, and an env-derived bootstrap value.
///
/// Resolution rules, in order:
/// 1. If `persisted` is `Some` and still appears in `installed`, use it.
/// 2. Otherwise use the first entry in `installed`.
/// 3. Otherwise fall back to `bootstrap` (the compiled-in / env default).
pub fn resolve_active_model(
    persisted: Option<&str>,
    installed: &[String],
    bootstrap: &str,
) -> String {
    if let Some(p) = persisted {
        if installed.iter().any(|m| m == p) {
            return p.to_string();
        }
    }
    if let Some(first) = installed.first() {
        return first.clone();
    }
    bootstrap.to_string()
}

/// Verifies that `model` is present in `installed`. Returns an `Err` with
/// the exact error copy the frontend surfaces when a user somehow requests
/// a slug that is not pulled locally.
pub fn validate_model_installed(model: &str, installed: &[String]) -> Result<(), String> {
    if installed.iter().any(|m| m == model) {
        Ok(())
    } else {
        Err(format!("Model is not installed in Ollama: {model}"))
    }
}

/// GETs `{base_url}/api/tags` and returns the list of installed model slugs.
///
/// Every failure mode (transport error, non-2xx status, JSON decode error)
/// is translated to `Err(String)` so the Tauri command layer can propagate
/// it verbatim to the frontend without panicking.
pub async fn fetch_installed_model_names(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<Vec<String>, String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("failed to reach Ollama: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Ollama /api/tags returned HTTP {}",
            response.status().as_u16()
        ));
    }

    let body: TagsResponse = response
        .json()
        .await
        .map_err(|e| format!("failed to decode /api/tags response: {e}"))?;

    Ok(body.models.into_iter().map(|m| m.name).collect())
}

/// Returns the currently active model and the full list of installed models,
/// persisting the resolved active model so future launches see it.
///
/// Shape: `{ "active": "<slug>", "all": ["<slug>", ...] }`.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn get_model_picker_state(
    client: tauri::State<'_, reqwest::Client>,
    db: tauri::State<'_, Database>,
    active_model: tauri::State<'_, ActiveModelState>,
    app_config: tauri::State<'_, crate::config::AppConfig>,
) -> Result<serde_json::Value, String> {
    let installed = fetch_installed_model_names(&client, DEFAULT_OLLAMA_URL).await?;

    let persisted = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        get_config(&conn, ACTIVE_MODEL_KEY).map_err(|e| e.to_string())?
    };

    let resolved = resolve_active_model(persisted.as_deref(), &installed, app_config.model.active());

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        set_config(&conn, ACTIVE_MODEL_KEY, &resolved).map_err(|e| e.to_string())?;
    }

    {
        let mut guard = active_model.0.lock().map_err(|e| e.to_string())?;
        *guard = resolved.clone();
    }

    Ok(serde_json::json!({ "active": resolved, "all": installed }))
}

/// Persists `model` as the active model after validating that Ollama still
/// reports it as installed. Rejects uninstalled slugs with the exact error
/// copy `"Model is not installed in Ollama: {model}"`.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn set_active_model(
    model: String,
    client: tauri::State<'_, reqwest::Client>,
    db: tauri::State<'_, Database>,
    active_model: tauri::State<'_, ActiveModelState>,
) -> Result<(), String> {
    let installed = fetch_installed_model_names(&client, DEFAULT_OLLAMA_URL).await?;
    validate_model_installed(&model, &installed)?;

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        set_config(&conn, ACTIVE_MODEL_KEY, &model).map_err(|e| e.to_string())?;
    }

    {
        let mut guard = active_model.0.lock().map_err(|e| e.to_string())?;
        *guard = model;
    }

    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── resolve_active_model ─────────────────────────────────────────────────

    #[test]
    fn resolve_prefers_persisted_when_still_installed() {
        let installed = vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()];
        let result = resolve_active_model(Some("gemma4:e4b"), &installed, "gemma4:e2b");
        assert_eq!(result, "gemma4:e4b");
    }

    #[test]
    fn resolve_falls_back_to_first_installed_when_persisted_missing() {
        let installed = vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()];
        let result = resolve_active_model(Some("llama3:8b"), &installed, "bootstrap-model");
        assert_eq!(result, "gemma4:e2b");
    }

    #[test]
    fn resolve_falls_back_to_bootstrap_when_nothing_installed() {
        let installed: Vec<String> = vec![];
        let result = resolve_active_model(None, &installed, "bootstrap-model");
        assert_eq!(result, "bootstrap-model");
    }

    #[test]
    fn resolve_with_no_persisted_uses_first_installed() {
        let installed = vec!["gemma4:e2b".to_string()];
        let result = resolve_active_model(None, &installed, "bootstrap-model");
        assert_eq!(result, "gemma4:e2b");
    }

    #[test]
    fn resolve_with_empty_persisted_bootstrap_used_when_installed_empty() {
        let installed: Vec<String> = vec![];
        // Persisted is present but installed list is empty: bootstrap wins
        // because there's nothing to cross-check against.
        let result = resolve_active_model(Some("gemma4:e2b"), &installed, "fallback");
        assert_eq!(result, "fallback");
    }

    // ── validate_model_installed ─────────────────────────────────────────────

    #[test]
    fn validate_accepts_installed_model() {
        let installed = vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()];
        assert!(validate_model_installed("gemma4:e4b", &installed).is_ok());
    }

    #[test]
    fn validate_rejects_uninstalled_model_with_exact_message() {
        let installed = vec!["gemma4:e2b".to_string()];
        let err = validate_model_installed("llama3:8b", &installed).unwrap_err();
        assert_eq!(err, "Model is not installed in Ollama: llama3:8b");
    }

    #[test]
    fn validate_rejects_when_installed_list_empty() {
        let installed: Vec<String> = vec![];
        let err = validate_model_installed("gemma4:e2b", &installed).unwrap_err();
        assert_eq!(err, "Model is not installed in Ollama: gemma4:e2b");
    }

    // ── fetch_installed_model_names ──────────────────────────────────────────

    #[tokio::test]
    async fn fetch_parses_valid_tags_response() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{"models":[
            {"name":"gemma4:e2b"},
            {"name":"gemma4:e4b"}
        ]}"#;
        let mock = server
            .mock("GET", "/api/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let result = fetch_installed_model_names(&client, &server.url()).await;

        mock.assert_async().await;
        let names = result.unwrap();
        assert_eq!(
            names,
            vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()]
        );
    }

    #[tokio::test]
    async fn fetch_returns_empty_when_no_models_installed() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"models":[]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let result = fetch_installed_model_names(&client, &server.url()).await;

        mock.assert_async().await;
        assert_eq!(result.unwrap(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn fetch_maps_http_error_to_err_string() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/tags")
            .with_status(500)
            .with_body("server blew up")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let result = fetch_installed_model_names(&client, &server.url()).await;

        mock.assert_async().await;
        let err = result.unwrap_err();
        assert!(
            err.contains("500"),
            "expected status code in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn fetch_maps_invalid_json_to_err_string() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("not json at all")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let result = fetch_installed_model_names(&client, &server.url()).await;

        mock.assert_async().await;
        let err = result.unwrap_err();
        assert!(
            err.contains("failed to decode"),
            "expected decode error, got: {err}"
        );
    }

    #[tokio::test]
    async fn fetch_maps_transport_error_to_err_string() {
        // Port 1 is reserved and will refuse connections; tests the `send()`
        // error branch without a live server.
        let client = reqwest::Client::new();
        let result = fetch_installed_model_names(&client, "http://127.0.0.1:1").await;

        let err = result.unwrap_err();
        assert!(
            err.contains("failed to reach Ollama"),
            "expected transport error, got: {err}"
        );
    }

    #[tokio::test]
    async fn fetch_trims_trailing_slash_from_base_url() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"models":[{"name":"x"}]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        // Pass the URL with a trailing slash; the helper must strip it.
        let url_with_slash = format!("{}/", server.url());
        let result = fetch_installed_model_names(&client, &url_with_slash).await;

        mock.assert_async().await;
        assert_eq!(result.unwrap(), vec!["x".to_string()]);
    }

    // ── ActiveModelState ─────────────────────────────────────────────────────

    #[test]
    fn active_model_state_defaults_to_empty_string() {
        let state = ActiveModelState::default();
        assert_eq!(*state.0.lock().unwrap(), "");
    }

    #[test]
    fn active_model_state_round_trip_write_read() {
        let state = ActiveModelState::default();
        {
            let mut guard = state.0.lock().unwrap();
            *guard = "gemma4:e2b".to_string();
        }
        assert_eq!(*state.0.lock().unwrap(), "gemma4:e2b");
    }

    // ── Persistence round-trip through app_config ───────────────────────────

    #[test]
    fn active_model_key_persists_via_set_and_get_config() {
        let conn = crate::database::open_in_memory().unwrap();
        set_config(&conn, ACTIVE_MODEL_KEY, "gemma4:e4b").unwrap();
        let back = get_config(&conn, ACTIVE_MODEL_KEY).unwrap();
        assert_eq!(back.as_deref(), Some("gemma4:e4b"));
    }

    #[test]
    fn active_model_key_constant_matches_expected_value() {
        assert_eq!(ACTIVE_MODEL_KEY, "active_model");
    }
}
