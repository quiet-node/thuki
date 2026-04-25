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

/// Maximum accepted byte length for a model slug passed to `set_active_model`.
/// Real Ollama slugs are a handful of characters; 256 is generous while still
/// capping adversarial inputs long before any network or database work.
pub const MAX_MODEL_SLUG_LEN: usize = 256;

/// Shared error-message prefix used when a requested slug is not present in
/// the live Ollama inventory. Exported so the frontend and tests can match
/// against a stable constant instead of a prose string.
pub const MODEL_NOT_INSTALLED_ERR_PREFIX: &str = "Model is not installed in Ollama: ";

/// Maximum accepted body size for the `/api/tags` response. Guards against
/// a misbehaving or compromised localhost Ollama streaming an unbounded
/// response that would exhaust memory. 4 MiB comfortably fits thousands of
/// model entries.
const MAX_TAGS_BODY_BYTES: usize = 4 * 1024 * 1024;

/// In-memory cache of the currently active model slug. Written once at
/// startup (after `resolve_seed_active_model`) and updated every time the
/// user picks a new model via `set_active_model`.
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
///
/// This helper assumes `installed` reflects real Ollama ground truth. At
/// startup when no ground truth is available, use
/// [`resolve_seed_active_model`] instead so a valid persisted choice is
/// never overridden by the bootstrap default just because Ollama has not
/// been queried yet.
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

/// Startup-time resolver that never cross-checks against an installed list.
///
/// At process start we cannot call Ollama (no async runtime yet), so the
/// safe behavior is to trust the persisted value when present and only fall
/// back to the bootstrap default when nothing was ever persisted. The first
/// `get_model_picker_state` call from the frontend reconciles against the
/// real installed list and may replace this seed.
pub fn resolve_seed_active_model(persisted: Option<&str>, bootstrap: &str) -> String {
    match persisted {
        Some(slug) if !slug.is_empty() => slug.to_string(),
        _ => bootstrap.to_string(),
    }
}

/// Returns true when the resolved slug should be written back to persistent
/// storage. Only writes when Ollama actually reported some inventory AND the
/// resolved slug differs from the currently-persisted value. This prevents a
/// partially-up Ollama returning `models:[]` from clobbering a valid
/// persisted user preference with the bootstrap fallback.
pub fn should_persist_resolved(
    installed: &[String],
    persisted: Option<&str>,
    resolved: &str,
) -> bool {
    !installed.is_empty() && persisted != Some(resolved)
}

/// Verifies that `model` is present in `installed`. Returns an `Err` with
/// a stable prefix (see [`MODEL_NOT_INSTALLED_ERR_PREFIX`]) so the frontend
/// can match against a constant rather than a verbatim prose string.
pub fn validate_model_installed(model: &str, installed: &[String]) -> Result<(), String> {
    if installed.iter().any(|m| m == model) {
        Ok(())
    } else {
        Err(format!("{MODEL_NOT_INSTALLED_ERR_PREFIX}{model}"))
    }
}

/// Validates shape of a model slug coming across the IPC boundary before any
/// network work. Rejects empty, over-length, and out-of-charset inputs.
/// Accepted charset covers everything real Ollama slugs use:
/// `A-Z a-z 0-9 : . _ / -`.
pub fn validate_model_slug(model: &str) -> Result<(), String> {
    if model.is_empty() {
        return Err("Model name cannot be empty".to_string());
    }
    if model.len() > MAX_MODEL_SLUG_LEN {
        return Err(format!(
            "Model name exceeds maximum length of {MAX_MODEL_SLUG_LEN} bytes"
        ));
    }
    if !model
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '/' | '-'))
    {
        return Err("Model name contains invalid characters".to_string());
    }
    Ok(())
}

/// Per-request timeout for the Ollama `/api/tags` GET. Guards the IPC
/// boundary: if the daemon accepts the TCP connection but never responds
/// (hung socket, stuck process, network partition), `get_model_picker_state`
/// and `set_active_model` would otherwise block indefinitely and wedge the
/// UI. 5 seconds is generous for a localhost call.
const TAGS_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// GETs `{base_url}/api/tags` and returns the list of installed model slugs.
///
/// Every failure mode (transport error, non-2xx status, oversized body,
/// JSON decode error) is translated to `Err(String)` so the Tauri command
/// layer can propagate it verbatim to the frontend without panicking.
pub async fn fetch_installed_model_names(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<Vec<String>, String> {
    fetch_installed_model_names_with_timeout(client, base_url, TAGS_REQUEST_TIMEOUT).await
}

/// Internal variant of [`fetch_installed_model_names`] with a configurable
/// per-request timeout. Exists so tests can exercise the timeout branch
/// deterministically without waiting the production 5s.
async fn fetch_installed_model_names_with_timeout(
    client: &reqwest::Client,
    base_url: &str,
    timeout: std::time::Duration,
) -> Result<Vec<String>, String> {
    fetch_installed_model_names_inner(client, base_url, timeout, MAX_TAGS_BODY_BYTES).await
}

/// Innermost implementation of the tags fetcher with both timeout and body
/// size cap configurable. Exists so the size-cap branches can be exercised
/// deterministically in tests without allocating production-scale buffers.
async fn fetch_installed_model_names_inner(
    client: &reqwest::Client,
    base_url: &str,
    timeout: std::time::Duration,
    max_body_bytes: usize,
) -> Result<Vec<String>, String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = client
        .get(&url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| format!("failed to reach Ollama: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Ollama /api/tags returned HTTP {}",
            response.status().as_u16()
        ));
    }

    if let Some(declared_len) = response.content_length() {
        if declared_len as usize > max_body_bytes {
            return Err(format!(
                "/api/tags response exceeded {max_body_bytes} bytes"
            ));
        }
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("failed to read /api/tags body: {e}"))?;

    if bytes.len() > max_body_bytes {
        return Err(format!(
            "/api/tags response exceeded {max_body_bytes} bytes"
        ));
    }

    let body: TagsResponse = serde_json::from_slice(&bytes)
        .map_err(|e| format!("failed to decode /api/tags response: {e}"))?;

    Ok(body.models.into_iter().map(|m| m.name).collect())
}

/// Returns the currently active model and the full list of installed models,
/// persisting the resolved active model so future launches see it.
///
/// Shape: `{ "active": "<slug>", "all": ["<slug>", ...] }`.
///
/// Coalesces the read + conditional write into a single database critical
/// section to avoid a TOCTOU window where a concurrent `set_active_model`
/// could be clobbered, and refuses to persist when Ollama reports an empty
/// inventory so a partially-up daemon cannot corrupt the persisted choice.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn get_model_picker_state(
    client: tauri::State<'_, reqwest::Client>,
    db: tauri::State<'_, Database>,
    active_model: tauri::State<'_, ActiveModelState>,
) -> Result<serde_json::Value, String> {
    let installed = fetch_installed_model_names(&client, DEFAULT_OLLAMA_URL).await?;

    let resolved = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let persisted = get_config(&conn, ACTIVE_MODEL_KEY).map_err(|e| e.to_string())?;
        let resolved = resolve_active_model(
            persisted.as_deref(),
            &installed,
            crate::config::defaults::DEFAULT_MODEL_NAME,
        );
        if should_persist_resolved(&installed, persisted.as_deref(), &resolved) {
            set_config(&conn, ACTIVE_MODEL_KEY, &resolved).map_err(|e| e.to_string())?;
        }
        resolved
    };

    {
        let mut guard = active_model.0.lock().map_err(|e| e.to_string())?;
        *guard = resolved.clone();
    }

    Ok(serde_json::json!({ "active": resolved, "all": installed }))
}

/// Persists `model` as the active model after validating its shape and
/// confirming Ollama still reports it as installed. Rejects uninstalled
/// slugs with an error that starts with [`MODEL_NOT_INSTALLED_ERR_PREFIX`].
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn set_active_model(
    model: String,
    client: tauri::State<'_, reqwest::Client>,
    db: tauri::State<'_, Database>,
    active_model: tauri::State<'_, ActiveModelState>,
) -> Result<(), String> {
    validate_model_slug(&model)?;

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
        let result = resolve_active_model(Some("gemma4:e2b"), &installed, "fallback");
        assert_eq!(result, "fallback");
    }

    // ── resolve_seed_active_model ────────────────────────────────────────────

    #[test]
    fn seed_resolve_prefers_persisted() {
        let result = resolve_seed_active_model(Some("llama3:8b"), "bootstrap-model");
        assert_eq!(result, "llama3:8b");
    }

    #[test]
    fn seed_resolve_falls_back_to_bootstrap_when_none() {
        let result = resolve_seed_active_model(None, "bootstrap-model");
        assert_eq!(result, "bootstrap-model");
    }

    #[test]
    fn seed_resolve_falls_back_to_bootstrap_when_empty_persisted() {
        let result = resolve_seed_active_model(Some(""), "bootstrap-model");
        assert_eq!(result, "bootstrap-model");
    }

    // ── should_persist_resolved ─────────────────────────────────────────────

    #[test]
    fn should_persist_true_when_resolved_differs_and_inventory_present() {
        let installed = vec!["gemma4:e2b".to_string()];
        assert!(should_persist_resolved(
            &installed,
            Some("llama3:8b"),
            "gemma4:e2b"
        ));
    }

    #[test]
    fn should_persist_false_when_resolved_matches_persisted() {
        let installed = vec!["gemma4:e2b".to_string()];
        assert!(!should_persist_resolved(
            &installed,
            Some("gemma4:e2b"),
            "gemma4:e2b"
        ));
    }

    #[test]
    fn should_persist_false_when_inventory_empty() {
        let installed: Vec<String> = vec![];
        assert!(!should_persist_resolved(&installed, None, "bootstrap"));
    }

    #[test]
    fn should_persist_true_when_nothing_previously_persisted_but_resolved_available() {
        let installed = vec!["gemma4:e2b".to_string()];
        assert!(should_persist_resolved(&installed, None, "gemma4:e2b"));
    }

    // ── validate_model_installed ─────────────────────────────────────────────

    #[test]
    fn validate_accepts_installed_model() {
        let installed = vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()];
        assert!(validate_model_installed("gemma4:e4b", &installed).is_ok());
    }

    #[test]
    fn validate_rejects_uninstalled_model_with_stable_prefix() {
        let installed = vec!["gemma4:e2b".to_string()];
        let err = validate_model_installed("llama3:8b", &installed).unwrap_err();
        assert!(
            err.starts_with(MODEL_NOT_INSTALLED_ERR_PREFIX),
            "expected stable prefix, got: {err}"
        );
        assert!(err.ends_with("llama3:8b"));
    }

    #[test]
    fn validate_rejects_when_installed_list_empty() {
        let installed: Vec<String> = vec![];
        let err = validate_model_installed("gemma4:e2b", &installed).unwrap_err();
        assert_eq!(err, format!("{MODEL_NOT_INSTALLED_ERR_PREFIX}gemma4:e2b"));
    }

    // ── validate_model_slug ──────────────────────────────────────────────────

    #[test]
    fn validate_slug_accepts_valid_forms() {
        assert!(validate_model_slug("gemma4:e2b").is_ok());
        assert!(validate_model_slug("llama3.1:8b").is_ok());
        assert!(validate_model_slug("registry.example.com/user/model:tag").is_ok());
        assert!(validate_model_slug("my_model-v2").is_ok());
    }

    #[test]
    fn validate_slug_rejects_empty() {
        let err = validate_model_slug("").unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_slug_rejects_oversized() {
        let oversized = "a".repeat(MAX_MODEL_SLUG_LEN + 1);
        let err = validate_model_slug(&oversized).unwrap_err();
        assert!(err.contains("maximum length"));
    }

    #[test]
    fn validate_slug_accepts_max_length() {
        let at_limit = "a".repeat(MAX_MODEL_SLUG_LEN);
        assert!(validate_model_slug(&at_limit).is_ok());
    }

    #[test]
    fn validate_slug_rejects_shell_metacharacters() {
        assert!(validate_model_slug("bad; rm -rf /").is_err());
        assert!(validate_model_slug("../etc/passwd").is_ok()); // `.` `/` `-` allowed individually
        assert!(validate_model_slug("bad name").is_err()); // whitespace rejected
        assert!(validate_model_slug("bad\nname").is_err());
        assert!(validate_model_slug("bad$(whoami)").is_err());
        assert!(validate_model_slug("bad`whoami`").is_err());
    }

    #[test]
    fn validate_slug_rejects_non_ascii() {
        assert!(validate_model_slug("gëmma").is_err());
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
    async fn fetch_installed_model_names_times_out_when_ollama_hangs() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let _held = listener.accept().ok();
            std::thread::sleep(std::time::Duration::from_secs(10));
        });

        let client = reqwest::Client::new();
        let base = format!("http://{addr}");
        let result = fetch_installed_model_names_with_timeout(
            &client,
            &base,
            std::time::Duration::from_millis(100),
        )
        .await;

        let err = result.unwrap_err();
        assert!(
            err.contains("failed to reach Ollama"),
            "expected timeout to surface as transport error, got: {err}"
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
        let url_with_slash = format!("{}/", server.url());
        let result = fetch_installed_model_names(&client, &url_with_slash).await;

        mock.assert_async().await;
        assert_eq!(result.unwrap(), vec!["x".to_string()]);
    }

    #[tokio::test]
    async fn fetch_rejects_body_exceeding_size_cap_via_content_length() {
        let mut server = mockito::Server::new_async().await;
        // Tight cap (32 bytes) + a declared Content-Length that matches a
        // 100-byte payload; the pre-read guard on `content_length` must
        // reject before the bytes() call is issued.
        let body = "x".repeat(100);
        server
            .mock("GET", "/api/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let result = fetch_installed_model_names_inner(
            &client,
            &server.url(),
            std::time::Duration::from_secs(5),
            32,
        )
        .await;

        let err = result.unwrap_err();
        assert!(
            err.contains("exceeded"),
            "expected size-cap error, got: {err}"
        );
    }

    #[tokio::test]
    async fn fetch_maps_body_read_error_to_err_string() {
        // Headers advertise Content-Length but the server closes the socket
        // before sending any body bytes. reqwest's bytes() surfaces this as
        // a transport error; the helper must map it to the documented prose.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            // Promise 100 body bytes, then immediately hang up.
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n",
            );
        });

        let client = reqwest::Client::new();
        let base = format!("http://{addr}");
        let result = fetch_installed_model_names(&client, &base).await;

        let err = result.unwrap_err();
        assert!(
            err.contains("failed to read /api/tags body"),
            "expected body-read error, got: {err}"
        );
    }

    #[tokio::test]
    async fn fetch_rejects_body_exceeding_size_cap_when_no_content_length() {
        // Chunked-encoding response (no Content-Length); the post-read guard
        // on `bytes.len()` must still reject.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let body = "x".repeat(200);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        });

        let client = reqwest::Client::new();
        let base = format!("http://{addr}");
        let result = fetch_installed_model_names_inner(
            &client,
            &base,
            std::time::Duration::from_secs(5),
            32,
        )
        .await;

        let err = result.unwrap_err();
        assert!(
            err.contains("exceeded"),
            "expected post-read size-cap error, got: {err}"
        );
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

    #[test]
    fn model_not_installed_err_prefix_is_stable() {
        assert_eq!(
            MODEL_NOT_INSTALLED_ERR_PREFIX,
            "Model is not installed in Ollama: "
        );
    }
}
