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
 * list. If not, we fall back to the first installed model. There is no
 * compiled fallback: when nothing is installed and nothing is persisted,
 * the active model is `None` and the user is prompted to pick one.
 */

use std::collections::HashMap;
use std::sync::Mutex;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::config::defaults::{
    DEFAULT_OLLAMA_SHOW_REQUEST_TIMEOUT_SECS, DEFAULT_OLLAMA_TAGS_REQUEST_TIMEOUT_SECS,
    MAX_MODEL_SLUG_LEN, MAX_OLLAMA_SHOW_BODY_BYTES, MAX_OLLAMA_TAGS_BODY_BYTES,
};
use crate::config::AppConfig;
use crate::database::{get_config, set_config};
use crate::history::Database;

/// `app_config` key used to persist the user's selected model slug.
pub const ACTIVE_MODEL_KEY: &str = "active_model";

/// Shared error-message prefix used when a requested slug is not present in
/// the live Ollama inventory. Exported so the frontend and tests can match
/// against a stable constant instead of a prose string.
pub const MODEL_NOT_INSTALLED_ERR_PREFIX: &str = "Model is not installed in Ollama: ";

/// In-memory cache of the currently active model slug. Written once at
/// startup (after `resolve_seed_active_model`) and updated every time the
/// user picks a new model via `set_active_model`.
///
/// `None` means no model has been chosen yet: either the user has never
/// picked one and Ollama has nothing installed, or the user removed every
/// model with `ollama rm` between launches. Consumers must treat `None` as
/// "refuse the request and steer the user to the picker", never as a
/// trigger to invent a default.
#[derive(Default)]
pub struct ActiveModelState(pub Mutex<Option<String>>);

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

/// Chooses which model slug should be active given a persisted preference
/// and the live installed list from Ollama.
///
/// Resolution rules, in order:
/// 1. If `persisted` is `Some` and still appears in `installed`, use it.
/// 2. Otherwise use the first entry in `installed`.
/// 3. Otherwise return `None`: nothing is installed and nothing is persisted,
///    so there is no honest answer.
///
/// This helper assumes `installed` reflects real Ollama ground truth. At
/// startup when no ground truth is available, use
/// [`resolve_seed_active_model`] instead so a valid persisted choice is
/// never lost just because Ollama has not been queried yet.
pub fn resolve_active_model(persisted: Option<&str>, installed: &[String]) -> Option<String> {
    if let Some(p) = persisted {
        if installed.iter().any(|m| m == p) {
            return Some(p.to_string());
        }
    }
    installed.first().cloned()
}

/// Startup-time resolver that never cross-checks against an installed list.
///
/// At process start we cannot call Ollama (no async runtime yet), so the
/// safe behavior is to trust the persisted value when present and otherwise
/// return `None`. The first `get_model_picker_state` call from the frontend
/// reconciles against the real installed list and may replace this seed.
pub fn resolve_seed_active_model(persisted: Option<&str>) -> Option<String> {
    match persisted {
        Some(slug) if !slug.is_empty() => Some(slug.to_string()),
        _ => None,
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

/// GETs `{base_url}/api/tags` and returns the list of installed model slugs.
///
/// Every failure mode (transport error, non-2xx status, oversized body,
/// JSON decode error) is translated to `Err(String)` so the Tauri command
/// layer can propagate it verbatim to the frontend without panicking.
pub async fn fetch_installed_model_names(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<Vec<String>, String> {
    fetch_installed_model_names_with_timeout(
        client,
        base_url,
        std::time::Duration::from_secs(DEFAULT_OLLAMA_TAGS_REQUEST_TIMEOUT_SECS),
    )
    .await
}

/// Internal variant of [`fetch_installed_model_names`] with a configurable
/// per-request timeout. Exists so tests can exercise the timeout branch
/// deterministically without waiting the production 5s.
async fn fetch_installed_model_names_with_timeout(
    client: &reqwest::Client,
    base_url: &str,
    timeout: std::time::Duration,
) -> Result<Vec<String>, String> {
    fetch_installed_model_names_inner(client, base_url, timeout, MAX_OLLAMA_TAGS_BODY_BYTES).await
}

/// Innermost implementation of the tags fetcher with both timeout and body
/// size cap configurable. Exists so the size-cap branches can be exercised
/// deterministically in tests without allocating production-scale buffers.
///
/// The cap is enforced incrementally during the streaming read: each chunk
/// is checked before being appended, so the connection is aborted the moment
/// the running total would exceed `max_body_bytes` rather than after the full
/// body has been buffered.
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

    let mut stream = response.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("failed to read /api/tags body: {e}"))?;
        if buf.len() + chunk.len() > max_body_bytes {
            return Err(format!(
                "/api/tags response exceeded {max_body_bytes} bytes"
            ));
        }
        buf.extend_from_slice(&chunk);
    }

    let body: TagsResponse = serde_json::from_slice(&buf)
        .map_err(|e| format!("failed to decode /api/tags response: {e}"))?;

    Ok(body.models.into_iter().map(|m| m.name).collect())
}

/// Returns the currently active model, the full list of installed models, and
/// a flag telling the frontend whether Ollama itself is reachable.
///
/// Shape: `{ "active": "<slug>" | null, "all": ["<slug>", ...], "ollamaReachable": bool }`.
///
/// The command intentionally never propagates a transport / fetch error to
/// the frontend. Instead, an unreachable Ollama collapses into a structured
/// `{ active: null, all: [], ollamaReachable: false }` payload so the UI can
/// distinguish "Ollama is down" from "Ollama is up but has no models" without
/// parsing error strings. The Ok branch coalesces the read + conditional
/// write into a single database critical section to avoid a TOCTOU window
/// where a concurrent `set_active_model` could be clobbered, and refuses to
/// persist when Ollama reports an empty inventory so a partially-up daemon
/// cannot corrupt the persisted choice.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn get_model_picker_state(
    client: tauri::State<'_, reqwest::Client>,
    db: tauri::State<'_, Database>,
    active_model: tauri::State<'_, ActiveModelState>,
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
) -> Result<serde_json::Value, String> {
    let ollama_url = config.read().inference.ollama_url.clone();
    let fetch_result = fetch_installed_model_names(&client, &ollama_url).await;

    let installed = match fetch_result {
        Ok(installed) => installed,
        Err(_) => {
            // Mirror the `None` active into the in-memory state so downstream
            // callers (ask_ollama, search_pipeline) see the same truth as the
            // frontend: with Ollama unreachable, no model is active.
            let mut guard = active_model.0.lock().map_err(|e| e.to_string())?;
            *guard = None;
            return Ok(build_picker_state_payload(None, &[], false));
        }
    };

    let resolved = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let persisted = get_config(&conn, ACTIVE_MODEL_KEY).map_err(|e| e.to_string())?;
        let resolved = resolve_active_model(persisted.as_deref(), &installed);
        if let Some(slug) = resolved.as_deref() {
            if should_persist_resolved(&installed, persisted.as_deref(), slug) {
                set_config(&conn, ACTIVE_MODEL_KEY, slug).map_err(|e| e.to_string())?;
            }
        }
        resolved
    };

    {
        let mut guard = active_model.0.lock().map_err(|e| e.to_string())?;
        *guard = resolved.clone();
    }

    Ok(build_picker_state_payload(
        resolved.as_deref(),
        &installed,
        true,
    ))
}

/// Pure helper that shapes the `get_model_picker_state` payload. Extracted so
/// the three states (unreachable, reachable + empty, reachable + populated)
/// can be unit-tested without spinning up a Tauri runtime or an HTTP server.
pub fn build_picker_state_payload(
    active: Option<&str>,
    installed: &[String],
    ollama_reachable: bool,
) -> serde_json::Value {
    let active_value = match active {
        Some(slug) => serde_json::Value::String(slug.to_string()),
        None => serde_json::Value::Null,
    };
    serde_json::json!({
        "active": active_value,
        "all": installed,
        "ollamaReachable": ollama_reachable,
    })
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
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
) -> Result<(), String> {
    validate_model_slug(&model)?;

    let ollama_url = config.read().inference.ollama_url.clone();
    let installed = fetch_installed_model_names(&client, &ollama_url).await?;
    validate_model_installed(&model, &installed)?;

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        set_config(&conn, ACTIVE_MODEL_KEY, &model).map_err(|e| e.to_string())?;
    }

    {
        let mut guard = active_model.0.lock().map_err(|e| e.to_string())?;
        *guard = Some(model);
    }

    Ok(())
}

// ─── Model setup gate (Phase 3 onboarding) ──────────────────────────────────

/// Result of probing the local Ollama daemon for setup readiness.
///
/// Drives the Phase 3 onboarding gate that fires after the user grants
/// macOS permissions but before the chat overlay is allowed to open.
/// Variants are emitted to the frontend in `snake_case` with an
/// internally-tagged `state` discriminator so the React side can route
/// on a single string field without inspecting payload shape.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelSetupState {
    /// `/api/tags` could not be reached. Treat as "Ollama is not installed
    /// or not running"; the UI must guide the user to install or start it.
    OllamaUnreachable,
    /// `/api/tags` responded successfully but the installed list is empty.
    /// The UI must guide the user to `ollama pull <slug>`.
    NoModelsInstalled,
    /// Ollama is running with at least one installed model. `active_slug`
    /// is the slug we resolved (persisted preference if still installed,
    /// else first installed) and `installed` is the live list for the
    /// frontend to render in the picker.
    Ready {
        active_slug: String,
        installed: Vec<String>,
    },
}

/// Pure state-machine derivation: maps the result of probing `/api/tags`
/// plus the persisted active-slug preference into a [`ModelSetupState`].
///
/// Exists as a free function so the three branches can be unit-tested
/// without spinning up an HTTP server or a Tauri runtime. The fetch
/// result and persisted preference are the only inputs; no I/O happens
/// here. The Tauri command is a thin wrapper that calls the fetcher,
/// reads the persisted slug from SQLite, then delegates here.
///
/// Resolution rules for the Ready arm match [`resolve_active_model`]:
/// prefer the persisted slug when it is still installed; otherwise fall
/// back to the first installed slug. Ready is gated on `!installed.is_empty()`
/// so `installed.first()` is always `Some`; the unwrap is therefore total.
pub fn derive_model_setup_state(
    installed_result: Result<Vec<String>, String>,
    persisted: Option<&str>,
) -> ModelSetupState {
    match installed_result {
        Err(_) => ModelSetupState::OllamaUnreachable,
        Ok(installed) if installed.is_empty() => ModelSetupState::NoModelsInstalled,
        Ok(installed) => {
            // The empty-list arm above guarantees `installed` has at least
            // one entry. Mirror `resolve_active_model`'s logic inline so
            // every branch is statically reachable from tests: when the
            // persisted slug is still installed we keep it, otherwise we
            // fall through to the first installed slug. This avoids a
            // dead `unwrap_or_else` arm that coverage cannot exercise.
            let active_slug = match persisted {
                Some(p) if installed.iter().any(|m| m == p) => p.to_string(),
                _ => installed[0].clone(),
            };
            ModelSetupState::Ready {
                active_slug,
                installed,
            }
        }
    }
}

/// Probes Ollama for setup readiness and returns the typed
/// [`ModelSetupState`] for the frontend onboarding gate.
///
/// Idempotent: safe to call on every overlay open. The Ready arm also
/// commits two side effects, both intentionally bounded:
///
/// 1. If the resolved slug differs from the persisted slug AND the live
///    installed list is non-empty, persist the resolved slug. This heals
///    the case where a user removed their previously-selected model with
///    `ollama rm` between launches.
/// 2. Mirror the resolved slug into the in-memory [`ActiveModelState`] so
///    `ask_ollama` and `search_pipeline` see it on the next request
///    without an extra DB read.
///
/// Both writes are gated through [`should_persist_resolved`] which
/// refuses to persist when Ollama reports an empty inventory (i.e.
/// daemon is up but mid-restart), so a transient empty response cannot
/// clobber a valid persisted choice.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn check_model_setup(
    client: tauri::State<'_, reqwest::Client>,
    db: tauri::State<'_, Database>,
    active_model: tauri::State<'_, ActiveModelState>,
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
) -> Result<ModelSetupState, String> {
    let ollama_url = config.read().inference.ollama_url.clone();
    let installed_result = fetch_installed_model_names(&client, &ollama_url).await;

    let persisted = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        get_config(&conn, ACTIVE_MODEL_KEY).map_err(|e| e.to_string())?
    };

    let state = derive_model_setup_state(installed_result, persisted.as_deref());

    if let ModelSetupState::Ready {
        ref active_slug,
        ref installed,
    } = state
    {
        if should_persist_resolved(installed, persisted.as_deref(), active_slug) {
            let conn = db.0.lock().map_err(|e| e.to_string())?;
            set_config(&conn, ACTIVE_MODEL_KEY, active_slug).map_err(|e| e.to_string())?;
        }
        let mut guard = active_model.0.lock().map_err(|e| e.to_string())?;
        *guard = Some(active_slug.clone());
    }

    Ok(state)
}

// ─── Model capabilities (vision, thinking) ──────────────────────────────────

/// Per-model capability flags surfaced to the frontend so the picker can
/// label rows and the submit-time gate can refuse mismatched messages
/// (image attached + text-only model). Booleans are derived from Ollama's
/// `/api/show` `capabilities` array; unknown strings are ignored so future
/// Ollama additions cannot break the schema.
///
/// Thuki surfaces exactly two capability flags. `completion` is implicit
/// (every chat model supports it; absence is rendered as the "text" tag
/// on the frontend). `tools`, embedding, and any future Ollama additions
/// are intentionally dropped so the picker stays focused on the
/// distinctions Thuki actually drives behavior off of.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Model accepts image inputs alongside text prompts. Drives the
    /// submit-time vision gate.
    #[serde(default)]
    pub vision: bool,
    /// Model emits explicit reasoning tokens that Thuki renders in the
    /// ThinkingBlock UI.
    #[serde(default)]
    pub thinking: bool,
    /// Maximum number of images the model accepts in a single request, when
    /// known. `None` means "unknown / unbounded by Thuki" and the gate lets
    /// the request through. Today this is keyed off the model architecture
    /// reported by `/api/show` (e.g. `mllama` → 1) because Ollama does not
    /// surface a declarative max-image count anywhere in its metadata.
    #[serde(default)]
    pub max_images: Option<u32>,
}

/// Architecture-keyed cap on the number of images accepted per request.
/// Ollama runners enforce these limits internally and answer with an HTTP
/// 500 when violated; mirroring them here lets the frontend gate refuse
/// the submit before the round-trip.
///
/// Unknown architectures fall through to `None`, which the gate interprets
/// as "no Thuki-side cap", trusting Ollama's runner as the final authority.
/// New architectures only need to be added when we observe a hard,
/// model-specific limit (today: `mllama`, used by llama3.2-vision).
pub fn max_images_for_architecture(arch: &str) -> Option<u32> {
    match arch {
        "mllama" => Some(1),
        _ => None,
    }
}

/// Subset of the `/api/show` response that Thuki consumes. All other fields
/// (modelfile, parameters, template, etc.) are ignored.
#[derive(Deserialize)]
struct ShowResponse {
    #[serde(default)]
    capabilities: Vec<String>,
    /// `details.family` (e.g. "mllama", "gemma4"). Older Ollama versions
    /// omit this; the field stays optional so decoding never fails on a
    /// model that pre-dates the field.
    #[serde(default)]
    details: Option<ShowDetails>,
    /// Detailed `model_info` map. We only read `general.architecture` from
    /// it. Stored as raw JSON so the rest of the (sometimes tens of fields,
    /// arbitrary types) payload does not have to be modelled.
    #[serde(default)]
    model_info: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Subset of `details` from `/api/show`. Only `family` is consumed today;
/// the rest of the object (parameter_size, quantization_level, etc.) is
/// ignored so unrelated changes upstream cannot break decoding.
#[derive(Deserialize)]
struct ShowDetails {
    #[serde(default)]
    family: Option<String>,
}

/// Reads the model architecture string from a parsed `/api/show` payload.
/// Prefers `model_info["general.architecture"]` (the canonical source);
/// falls back to `details.family` for older Ollama builds that did not
/// surface the structured `model_info` map. Returns `None` when neither
/// source is populated.
fn architecture_from_show(body: &ShowResponse) -> Option<&str> {
    if let Some(mi) = &body.model_info {
        if let Some(arch) = mi.get("general.architecture").and_then(|v| v.as_str()) {
            if !arch.is_empty() {
                return Some(arch);
            }
        }
    }
    body.details
        .as_ref()
        .and_then(|d| d.family.as_deref())
        .filter(|s| !s.is_empty())
}

/// Pure mapping from Ollama's capability strings into the typed
/// [`Capabilities`] struct. Unknown strings are silently dropped so a
/// future Ollama version that adds e.g. `"audio"` does not poison the
/// frontend payload. The `max_images` field is left at `None` here and
/// populated by the caller once the architecture is known.
pub fn capabilities_from_strings(items: &[String]) -> Capabilities {
    let mut caps = Capabilities::default();
    for c in items {
        match c.as_str() {
            "vision" => caps.vision = true,
            "thinking" => caps.thinking = true,
            _ => {}
        }
    }
    caps
}

/// POSTs `{base_url}/api/show {"name": "<slug>"}` and returns the parsed
/// [`Capabilities`] for that model.
///
/// Every failure mode (transport error, non-2xx status, oversized body,
/// JSON decode error) is translated to `Err(String)` so the Tauri command
/// layer can propagate it verbatim without panicking.
pub async fn fetch_model_capabilities(
    client: &reqwest::Client,
    base_url: &str,
    name: &str,
) -> Result<Capabilities, String> {
    fetch_model_capabilities_with_timeout(
        client,
        base_url,
        name,
        std::time::Duration::from_secs(DEFAULT_OLLAMA_SHOW_REQUEST_TIMEOUT_SECS),
    )
    .await
}

/// Internal variant of [`fetch_model_capabilities`] with a configurable
/// per-request timeout. Exists so tests can exercise the timeout branch
/// deterministically without waiting the production 5s.
async fn fetch_model_capabilities_with_timeout(
    client: &reqwest::Client,
    base_url: &str,
    name: &str,
    timeout: std::time::Duration,
) -> Result<Capabilities, String> {
    fetch_model_capabilities_inner(client, base_url, name, timeout, MAX_OLLAMA_SHOW_BODY_BYTES)
        .await
}

/// Innermost implementation of the `/api/show` fetcher. Both timeout and
/// body size cap are configurable so the size-cap branches can be
/// exercised in tests without allocating production-scale buffers.
///
/// The cap is enforced incrementally during the streaming read: each chunk
/// is checked before being appended, so the connection is aborted the moment
/// the running total would exceed `max_body_bytes` rather than after the full
/// body has been buffered.
async fn fetch_model_capabilities_inner(
    client: &reqwest::Client,
    base_url: &str,
    name: &str,
    timeout: std::time::Duration,
    max_body_bytes: usize,
) -> Result<Capabilities, String> {
    let url = format!("{}/api/show", base_url.trim_end_matches('/'));
    let response = client
        .post(&url)
        .json(&serde_json::json!({ "name": name }))
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| format!("failed to reach Ollama: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Ollama /api/show returned HTTP {}",
            response.status().as_u16()
        ));
    }

    if let Some(declared_len) = response.content_length() {
        if declared_len as usize > max_body_bytes {
            return Err(format!(
                "/api/show response exceeded {max_body_bytes} bytes"
            ));
        }
    }

    let mut stream = response.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("failed to read /api/show body: {e}"))?;
        if buf.len() + chunk.len() > max_body_bytes {
            return Err(format!(
                "/api/show response exceeded {max_body_bytes} bytes"
            ));
        }
        buf.extend_from_slice(&chunk);
    }

    let body: ShowResponse = serde_json::from_slice(&buf)
        .map_err(|e| format!("failed to decode /api/show response: {e}"))?;

    let mut caps = capabilities_from_strings(&body.capabilities);
    // Only attach max_images for vision models. There is no point capping a
    // text-only model on an image count; the vision gate refuses those
    // submits before the count check ever runs.
    if caps.vision {
        if let Some(arch) = architecture_from_show(&body) {
            caps.max_images = max_images_for_architecture(arch);
        }
    }
    Ok(caps)
}

/// In-memory cache of capabilities keyed by model slug. Populated lazily
/// the first time a model is queried. Cleared on app restart, which is
/// the simplest valid invalidation strategy: re-pulling a model under the
/// same slug requires a process restart anyway because Tauri's reqwest
/// client is process-scoped, and capabilities for a given (slug, digest)
/// pair never change.
#[derive(Default)]
pub struct ModelCapabilitiesCache(pub Mutex<HashMap<String, Capabilities>>);

/// Fetches `/api/tags` for the installed list, then returns a map of
/// `model name -> Capabilities` covering every installed model. Uses the
/// cache for hits and POSTs `/api/show` sequentially for misses, writing
/// results through to the cache.
///
/// Sequential fetch is intentional: localhost Ollama responds in tens of
/// milliseconds, the typical user has fewer than ten models installed,
/// and sequential keeps lifetime / borrow plumbing simple. Per-model
/// fetch failures are skipped (the offending entry is just absent from
/// the result map) so a single bad model cannot blank out the whole
/// picker.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn get_model_capabilities(
    client: tauri::State<'_, reqwest::Client>,
    cache: tauri::State<'_, ModelCapabilitiesCache>,
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
) -> Result<HashMap<String, Capabilities>, String> {
    let base_url = config.read().inference.ollama_url.clone();
    let installed = fetch_installed_model_names(&client, &base_url).await?;
    Ok(reconcile_capabilities(&client, &cache, &base_url, &installed).await)
}

/// Pure-ish helper extracted so tests can drive the cache + fetch loop
/// against a `mockito` server without going through the Tauri command
/// boundary. Honors the cache for already-known slugs and fetches the
/// rest from `base_url`.
///
/// Defense-in-depth: every miss is shape-checked via [`validate_model_slug`]
/// before being sent in the `/api/show` JSON body. Slugs that come from
/// `/api/tags` should already be well-formed, but a compromised or
/// misbehaving Ollama could return a slug containing control characters
/// or shell metacharacters; this guard keeps such inputs out of the
/// request entirely. Invalid slugs are silently dropped so they are
/// simply absent from the result map.
///
/// Concurrency: the read snapshot, the per-miss fetch, and the
/// write-back each take their own short-lived `Mutex` guard. Two
/// concurrent calls for the same miss may both fetch and both write the
/// same value. This is benign because the operation is idempotent (the
/// same `(slug, /api/show)` always yields the same `Capabilities`); the
/// only cost is a duplicate POST.
async fn reconcile_capabilities(
    client: &reqwest::Client,
    cache: &ModelCapabilitiesCache,
    base_url: &str,
    installed: &[String],
) -> HashMap<String, Capabilities> {
    let mut hits: HashMap<String, Capabilities> = HashMap::new();
    let mut misses: Vec<String> = Vec::new();
    match cache.0.lock() {
        Ok(guard) => {
            for name in installed {
                if let Some(c) = guard.get(name) {
                    hits.insert(name.clone(), c.clone());
                } else {
                    misses.push(name.clone());
                }
            }
        }
        Err(_) => {
            // Poisoned lock: treat every requested slug as a miss so the
            // caller still gets a best-effort result.
            misses.extend(installed.iter().cloned());
        }
    }
    for name in &misses {
        if validate_model_slug(name).is_err() {
            continue;
        }
        if let Ok(caps) = fetch_model_capabilities(client, base_url, name).await {
            if let Ok(mut guard) = cache.0.lock() {
                guard.insert(name.clone(), caps.clone());
            }
            hits.insert(name.clone(), caps);
        }
    }
    hits
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_picker_state_payload ───────────────────────────────────────────

    #[test]
    fn picker_payload_unreachable_emits_null_active_empty_list_and_flag() {
        // S1 mirrors the unreachable case: no model can be resolved, the
        // installed list is empty by definition, and the flag is false so
        // the frontend can pick the right strip copy.
        let payload = build_picker_state_payload(None, &[], false);
        assert_eq!(payload["active"], serde_json::Value::Null);
        assert_eq!(payload["all"], serde_json::json!([]));
        assert_eq!(payload["ollamaReachable"], serde_json::Value::Bool(false));
    }

    #[test]
    fn picker_payload_reachable_but_empty_keeps_flag_true_and_null_active() {
        // S2: Ollama responded but installed list is empty. Active is null
        // (nothing to resolve to) yet ollamaReachable is true so the strip
        // can tell the user to pull a model rather than start the daemon.
        let payload = build_picker_state_payload(None, &[], true);
        assert_eq!(payload["active"], serde_json::Value::Null);
        assert_eq!(payload["all"], serde_json::json!([]));
        assert_eq!(payload["ollamaReachable"], serde_json::Value::Bool(true));
    }

    #[test]
    fn picker_payload_reachable_with_models_carries_active_slug() {
        // S4 (normal): active slug is present and ollamaReachable is true.
        // The frontend renders the chip with the slug and skips the strip.
        let installed = vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()];
        let payload = build_picker_state_payload(Some("gemma4:e4b"), &installed, true);
        assert_eq!(payload["active"], serde_json::json!("gemma4:e4b"));
        assert_eq!(
            payload["all"],
            serde_json::json!(["gemma4:e2b", "gemma4:e4b"])
        );
        assert_eq!(payload["ollamaReachable"], serde_json::Value::Bool(true));
    }

    // ── resolve_active_model ─────────────────────────────────────────────────

    #[test]
    fn resolve_prefers_persisted_when_still_installed() {
        let installed = vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()];
        let result = resolve_active_model(Some("gemma4:e4b"), &installed);
        assert_eq!(result, Some("gemma4:e4b".to_string()));
    }

    #[test]
    fn resolve_falls_back_to_first_installed_when_persisted_missing() {
        let installed = vec!["gemma4:e2b".to_string(), "gemma4:e4b".to_string()];
        let result = resolve_active_model(Some("llama3:8b"), &installed);
        assert_eq!(result, Some("gemma4:e2b".to_string()));
    }

    #[test]
    fn resolve_returns_none_when_nothing_installed_and_nothing_persisted() {
        let installed: Vec<String> = vec![];
        let result = resolve_active_model(None, &installed);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_with_no_persisted_uses_first_installed() {
        let installed = vec!["gemma4:e2b".to_string()];
        let result = resolve_active_model(None, &installed);
        assert_eq!(result, Some("gemma4:e2b".to_string()));
    }

    #[test]
    fn resolve_returns_none_when_persisted_present_but_installed_empty() {
        // The persisted slug names a model the user removed with `ollama rm`
        // and Ollama now reports an empty inventory. There is no honest
        // answer here; refuse to invent one.
        let installed: Vec<String> = vec![];
        let result = resolve_active_model(Some("gemma4:e2b"), &installed);
        assert_eq!(result, None);
    }

    // ── resolve_seed_active_model ────────────────────────────────────────────

    #[test]
    fn seed_resolve_prefers_persisted() {
        let result = resolve_seed_active_model(Some("llama3:8b"));
        assert_eq!(result, Some("llama3:8b".to_string()));
    }

    #[test]
    fn seed_resolve_returns_none_when_nothing_persisted() {
        let result = resolve_seed_active_model(None);
        assert_eq!(result, None);
    }

    #[test]
    fn seed_resolve_returns_none_when_empty_persisted() {
        let result = resolve_seed_active_model(Some(""));
        assert_eq!(result, None);
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
        // Chunked-encoding response (no Content-Length); the incremental stream
        // cap must reject when the running total exceeds the limit.
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
            "expected incremental stream cap error, got: {err}"
        );
    }

    #[tokio::test]
    async fn fetch_tags_chunked_early_abort_incremental() {
        // Explicit test of the incremental streaming abort: the response has NO
        // Content-Length header and sends chunks whose cumulative size exceeds
        // the cap. The abort must fire during the streaming read, not after.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut request_buf = [0u8; 1024];
            let _ = conn.read(&mut request_buf);
            // Send two small chunks without Content-Length (chunked encoding).
            // Each chunk alone is under the cap of 20 bytes, but together
            // they exceed it, exercising the incremental buf.len() + chunk.len()
            // check inside the stream loop.
            let _ = conn.write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                  0a\r\n0123456789\r\n\
                  0a\r\n0123456789\r\n\
                  0a\r\n0123456789\r\n\
                  0\r\n\r\n",
            );
        });
        let client = reqwest::Client::new();
        let base = format!("http://{addr}");
        let err = fetch_installed_model_names_inner(
            &client,
            &base,
            std::time::Duration::from_secs(5),
            20,
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("exceeded"),
            "expected incremental abort error, got: {err}"
        );
    }

    // ── ActiveModelState ─────────────────────────────────────────────────────

    #[test]
    fn active_model_state_defaults_to_none() {
        let state = ActiveModelState::default();
        assert_eq!(*state.0.lock().unwrap(), None);
    }

    #[test]
    fn active_model_state_round_trip_write_read() {
        let state = ActiveModelState::default();
        {
            let mut guard = state.0.lock().unwrap();
            *guard = Some("gemma4:e2b".to_string());
        }
        assert_eq!(*state.0.lock().unwrap(), Some("gemma4:e2b".to_string()));
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

    // ── derive_model_setup_state (Phase 3 onboarding gate) ──────────────────

    #[test]
    fn derive_setup_state_returns_unreachable_on_fetch_error() {
        let state = derive_model_setup_state(Err("connection refused".to_string()), None);
        assert_eq!(state, ModelSetupState::OllamaUnreachable);
    }

    #[test]
    fn derive_setup_state_returns_unreachable_even_when_persisted_choice_exists() {
        // Past selection must NOT mask a current outage. The user needs to
        // see the "Ollama not detected" screen even if SQLite remembers a slug.
        let state = derive_model_setup_state(Err("timeout".to_string()), Some("gemma4:e4b"));
        assert_eq!(state, ModelSetupState::OllamaUnreachable);
    }

    #[test]
    fn derive_setup_state_returns_no_models_when_inventory_empty() {
        let state = derive_model_setup_state(Ok(vec![]), None);
        assert_eq!(state, ModelSetupState::NoModelsInstalled);
    }

    #[test]
    fn derive_setup_state_returns_no_models_even_with_stale_persisted_slug() {
        // Daemon up but the user removed every model with `ollama rm`. The
        // persisted slug is no longer valid; the gate must re-engage.
        let state = derive_model_setup_state(Ok(vec![]), Some("removed-model:7b"));
        assert_eq!(state, ModelSetupState::NoModelsInstalled);
    }

    #[test]
    fn derive_setup_state_ready_keeps_persisted_when_still_installed() {
        let state = derive_model_setup_state(
            Ok(vec!["gemma4:e2b".to_string(), "llama3:8b".to_string()]),
            Some("llama3:8b"),
        );
        assert_eq!(
            state,
            ModelSetupState::Ready {
                active_slug: "llama3:8b".to_string(),
                installed: vec!["gemma4:e2b".to_string(), "llama3:8b".to_string()],
            }
        );
    }

    #[test]
    fn derive_setup_state_ready_falls_back_to_first_when_persisted_gone() {
        let state = derive_model_setup_state(
            Ok(vec!["gemma4:e4b".to_string(), "llama3:8b".to_string()]),
            Some("removed-model:7b"),
        );
        assert_eq!(
            state,
            ModelSetupState::Ready {
                active_slug: "gemma4:e4b".to_string(),
                installed: vec!["gemma4:e4b".to_string(), "llama3:8b".to_string()],
            }
        );
    }

    #[test]
    fn derive_setup_state_ready_uses_first_when_no_persisted_choice() {
        // First-time user who somehow has models installed already (rare:
        // they used Ollama for something else first). Pick the first.
        let state = derive_model_setup_state(Ok(vec!["qwen2.5:7b".to_string()]), None);
        assert_eq!(
            state,
            ModelSetupState::Ready {
                active_slug: "qwen2.5:7b".to_string(),
                installed: vec!["qwen2.5:7b".to_string()],
            }
        );
    }

    #[test]
    fn model_setup_state_serializes_with_state_tag_for_frontend() {
        // Wire format must be discriminated on a `state` field so the
        // React side can route on a single string before pattern-matching
        // payload shape. Drift here breaks the frontend dispatch.
        let unreachable = serde_json::to_value(ModelSetupState::OllamaUnreachable).unwrap();
        assert_eq!(
            unreachable,
            serde_json::json!({"state": "ollama_unreachable"})
        );

        let none = serde_json::to_value(ModelSetupState::NoModelsInstalled).unwrap();
        assert_eq!(none, serde_json::json!({"state": "no_models_installed"}));

        let ready = serde_json::to_value(ModelSetupState::Ready {
            active_slug: "gemma4:e2b".to_string(),
            installed: vec!["gemma4:e2b".to_string()],
        })
        .unwrap();
        assert_eq!(
            ready,
            serde_json::json!({
                "state": "ready",
                "active_slug": "gemma4:e2b",
                "installed": ["gemma4:e2b"],
            })
        );
    }

    // ── capabilities_from_strings ────────────────────────────────────────────

    #[test]
    fn capabilities_from_strings_recognises_all_known_flags() {
        let caps = capabilities_from_strings(&["vision".to_string(), "thinking".to_string()]);
        assert!(caps.vision);
        assert!(caps.thinking);
    }

    #[test]
    fn capabilities_from_strings_defaults_to_all_false_on_empty() {
        let caps = capabilities_from_strings(&[]);
        assert!(!caps.vision);
        assert!(!caps.thinking);
    }

    #[test]
    fn capabilities_from_strings_drops_unknown_flags_silently() {
        let caps = capabilities_from_strings(&[
            "vision".to_string(),
            "tools".to_string(),
            "audio".to_string(),
            "completion".to_string(),
            "future-thing".to_string(),
        ]);
        assert!(caps.vision);
        assert!(!caps.thinking);
    }

    #[test]
    fn capabilities_serialize_uses_camel_case_field_names() {
        let caps = Capabilities {
            vision: true,
            thinking: false,
            max_images: Some(1),
        };
        let v = serde_json::to_value(&caps).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "vision": true,
                "thinking": false,
                "maxImages": 1,
            })
        );
    }

    #[test]
    fn capabilities_serialize_emits_null_max_images_when_unknown() {
        let caps = Capabilities {
            vision: true,
            thinking: false,
            max_images: None,
        };
        let v = serde_json::to_value(&caps).unwrap();
        assert_eq!(v["maxImages"], serde_json::Value::Null);
    }

    #[test]
    fn capabilities_deserialize_tolerates_missing_fields() {
        let caps: Capabilities = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(caps, Capabilities::default());
    }

    #[test]
    fn capabilities_deserialize_round_trips_max_images() {
        let caps: Capabilities = serde_json::from_value(serde_json::json!({
            "vision": true,
            "thinking": false,
            "maxImages": 3
        }))
        .unwrap();
        assert!(caps.vision);
        assert_eq!(caps.max_images, Some(3));
    }

    // ── max_images_for_architecture ─────────────────────────────────────────

    #[test]
    fn max_images_caps_mllama_at_one() {
        assert_eq!(max_images_for_architecture("mllama"), Some(1));
    }

    #[test]
    fn max_images_returns_none_for_unknown_arch() {
        assert_eq!(max_images_for_architecture("gemma4"), None);
        assert_eq!(max_images_for_architecture(""), None);
        assert_eq!(max_images_for_architecture("future-arch"), None);
    }

    // ── architecture_from_show ──────────────────────────────────────────────

    #[test]
    fn architecture_prefers_model_info_general_architecture() {
        let body: ShowResponse = serde_json::from_value(serde_json::json!({
            "capabilities": ["completion","vision"],
            "details": {"family": "fallback-family"},
            "model_info": {"general.architecture": "mllama"}
        }))
        .unwrap();
        assert_eq!(architecture_from_show(&body), Some("mllama"));
    }

    #[test]
    fn architecture_falls_back_to_details_family_when_model_info_absent() {
        let body: ShowResponse = serde_json::from_value(serde_json::json!({
            "capabilities": ["completion","vision"],
            "details": {"family": "mllama"}
        }))
        .unwrap();
        assert_eq!(architecture_from_show(&body), Some("mllama"));
    }

    #[test]
    fn architecture_falls_back_when_model_info_arch_is_blank() {
        let body: ShowResponse = serde_json::from_value(serde_json::json!({
            "capabilities": [],
            "details": {"family": "mllama"},
            "model_info": {"general.architecture": ""}
        }))
        .unwrap();
        assert_eq!(architecture_from_show(&body), Some("mllama"));
    }

    #[test]
    fn architecture_returns_none_when_neither_source_populated() {
        let body: ShowResponse = serde_json::from_value(serde_json::json!({
            "capabilities": []
        }))
        .unwrap();
        assert_eq!(architecture_from_show(&body), None);
    }

    #[test]
    fn architecture_returns_none_when_details_family_blank() {
        let body: ShowResponse = serde_json::from_value(serde_json::json!({
            "capabilities": [],
            "details": {"family": ""}
        }))
        .unwrap();
        assert_eq!(architecture_from_show(&body), None);
    }

    #[test]
    fn architecture_ignores_non_string_general_architecture() {
        let body: ShowResponse = serde_json::from_value(serde_json::json!({
            "capabilities": [],
            "details": {"family": "mllama"},
            "model_info": {"general.architecture": 7}
        }))
        .unwrap();
        // Non-string in model_info falls through; details.family wins.
        assert_eq!(architecture_from_show(&body), Some("mllama"));
    }

    // ── fetch_model_capabilities ─────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_capabilities_parses_full_response() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{"capabilities":["completion","vision","thinking"],"modelfile":"…"}"#;
        let _m = server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let caps = fetch_model_capabilities(&client, &server.url(), "llama3.2-vision")
            .await
            .unwrap();
        assert!(caps.vision);
        assert!(caps.thinking);
    }

    #[tokio::test]
    async fn fetch_capabilities_attaches_max_images_for_mllama_vision_models() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{
            "capabilities":["completion","vision"],
            "details":{"family":"mllama"},
            "model_info":{"general.architecture":"mllama"}
        }"#;
        let _m = server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let caps = fetch_model_capabilities(&client, &server.url(), "llama3.2-vision:11b")
            .await
            .unwrap();
        assert!(caps.vision);
        assert_eq!(caps.max_images, Some(1));
    }

    #[tokio::test]
    async fn fetch_capabilities_leaves_max_images_unset_for_unknown_arch() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{
            "capabilities":["completion","vision","thinking"],
            "details":{"family":"gemma4"},
            "model_info":{"general.architecture":"gemma4"}
        }"#;
        let _m = server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let caps = fetch_model_capabilities(&client, &server.url(), "gemma4:e2b")
            .await
            .unwrap();
        assert!(caps.vision);
        assert!(caps.thinking);
        assert_eq!(caps.max_images, None);
    }

    #[tokio::test]
    async fn fetch_capabilities_skips_max_images_for_text_only_models() {
        // No point capping a text-only model on image count; vision gate
        // will refuse the submit before max_images is consulted anyway.
        let mut server = mockito::Server::new_async().await;
        let body = r#"{
            "capabilities":["completion"],
            "details":{"family":"mllama"},
            "model_info":{"general.architecture":"mllama"}
        }"#;
        let _m = server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let caps = fetch_model_capabilities(&client, &server.url(), "x")
            .await
            .unwrap();
        assert!(!caps.vision);
        assert_eq!(caps.max_images, None);
    }

    #[tokio::test]
    async fn fetch_capabilities_handles_missing_array() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_body(r#"{"modelfile":"…"}"#)
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let caps = fetch_model_capabilities(&client, &server.url(), "x")
            .await
            .unwrap();
        assert_eq!(caps, Capabilities::default());
    }

    #[tokio::test]
    async fn fetch_capabilities_returns_err_on_non_2xx() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/api/show")
            .with_status(404)
            .with_body("not found")
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let err = fetch_model_capabilities(&client, &server.url(), "missing")
            .await
            .unwrap_err();
        assert!(err.contains("404"));
    }

    #[tokio::test]
    async fn fetch_capabilities_returns_err_on_invalid_json() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_body("not json")
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let err = fetch_model_capabilities(&client, &server.url(), "x")
            .await
            .unwrap_err();
        assert!(err.contains("decode"));
    }

    #[tokio::test]
    async fn fetch_capabilities_returns_err_on_unreachable() {
        let client = reqwest::Client::new();
        let err = fetch_model_capabilities(&client, "http://127.0.0.1:1", "x")
            .await
            .unwrap_err();
        assert!(err.contains("failed to reach Ollama"));
    }

    #[tokio::test]
    async fn fetch_capabilities_rejects_oversized_via_content_length() {
        // Tight cap + 100-byte body; mockito sets Content-Length: 100, the
        // pre-read guard on `content_length` must reject before bytes() is
        // issued.
        let mut server = mockito::Server::new_async().await;
        let body = "x".repeat(100);
        server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let err = fetch_model_capabilities_inner(
            &client,
            &server.url(),
            "x",
            std::time::Duration::from_secs(5),
            32,
        )
        .await
        .unwrap_err();
        assert!(err.contains("exceeded"));
    }

    #[tokio::test]
    async fn fetch_capabilities_rejects_oversized_when_no_content_length() {
        // Chunked-encoding response (no Content-Length); the incremental stream
        // cap must reject when the running total exceeds the limit.
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
        let err = fetch_model_capabilities_inner(
            &client,
            &base,
            "x",
            std::time::Duration::from_secs(5),
            32,
        )
        .await
        .unwrap_err();
        assert!(err.contains("exceeded"));
    }

    #[tokio::test]
    async fn fetch_show_chunked_early_abort_incremental() {
        // Explicit test of the incremental streaming abort for /api/show: the
        // response has NO Content-Length header and sends chunks whose
        // cumulative size exceeds the cap. The abort must fire during the
        // streaming read, not after.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut request_buf = [0u8; 1024];
            let _ = conn.read(&mut request_buf);
            // Send three 10-byte chunks without Content-Length (chunked
            // encoding). Each chunk alone is under the cap of 20 bytes, but
            // together they exceed it, exercising the incremental check.
            let _ = conn.write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                  0a\r\n0123456789\r\n\
                  0a\r\n0123456789\r\n\
                  0a\r\n0123456789\r\n\
                  0\r\n\r\n",
            );
        });
        let client = reqwest::Client::new();
        let base = format!("http://{addr}");
        let err = fetch_model_capabilities_inner(
            &client,
            &base,
            "x",
            std::time::Duration::from_secs(5),
            20,
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("exceeded"),
            "expected incremental abort error, got: {err}"
        );
    }

    #[tokio::test]
    async fn fetch_capabilities_maps_body_read_error_to_err_string() {
        // Headers promise body but the server hangs up.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n",
            );
        });
        let client = reqwest::Client::new();
        let base = format!("http://{addr}");
        let err = fetch_model_capabilities(&client, &base, "x")
            .await
            .unwrap_err();
        assert!(err.contains("failed to read /api/show body"));
    }

    #[tokio::test]
    async fn fetch_capabilities_with_custom_timeout_branch_runs() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_body(r#"{"capabilities":["vision"]}"#)
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let caps = fetch_model_capabilities_with_timeout(
            &client,
            &server.url(),
            "x",
            std::time::Duration::from_millis(500),
        )
        .await
        .unwrap();
        assert!(caps.vision);
    }

    // ── reconcile_capabilities ───────────────────────────────────────────────

    /// `reconcile_capabilities` calls `DEFAULT_OLLAMA_URL` directly which
    /// points at 127.0.0.1:11434. To keep the test deterministic without a
    /// running Ollama we exercise the helper in cache-only mode: pre-seed
    /// every requested name into the cache so no network call is issued.
    #[tokio::test]
    async fn reconcile_returns_cached_entries_without_network() {
        let cache = ModelCapabilitiesCache::default();
        cache.0.lock().unwrap().insert(
            "a".to_string(),
            Capabilities {
                vision: true,
                ..Default::default()
            },
        );
        cache.0.lock().unwrap().insert(
            "b".to_string(),
            Capabilities {
                thinking: true,
                ..Default::default()
            },
        );
        let client = reqwest::Client::new();
        let installed = vec!["a".to_string(), "b".to_string()];
        let result = reconcile_capabilities(&client, &cache, "http://unused", &installed).await;
        assert_eq!(result.len(), 2);
        assert!(result["a"].vision);
        assert!(result["b"].thinking);
    }

    #[tokio::test]
    async fn reconcile_with_empty_installed_returns_empty_map() {
        let cache = ModelCapabilitiesCache::default();
        let client = reqwest::Client::new();
        let result = reconcile_capabilities(&client, &cache, "http://unused", &[]).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn reconcile_fetches_misses_and_writes_through_cache() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_body(r#"{"capabilities":["completion","vision"]}"#)
            .expect_at_least(1)
            .create_async()
            .await;
        let cache = ModelCapabilitiesCache::default();
        let client = reqwest::Client::new();
        let installed = vec!["fresh".to_string()];
        let result = reconcile_capabilities(&client, &cache, &server.url(), &installed).await;
        assert!(result["fresh"].vision);
        // Cache must now hold the fetched entry.
        let guard = cache.0.lock().unwrap();
        assert!(guard.contains_key("fresh"));
        assert!(guard["fresh"].vision);
    }

    #[tokio::test]
    async fn reconcile_drops_unreachable_misses_without_failing() {
        let cache = ModelCapabilitiesCache::default();
        cache.0.lock().unwrap().insert(
            "cached".to_string(),
            Capabilities {
                vision: true,
                ..Default::default()
            },
        );
        let client = reqwest::Client::new();
        let installed = vec!["cached".to_string(), "missing".to_string()];
        // Point base_url at a port nothing listens on so misses fail fast.
        let result =
            reconcile_capabilities(&client, &cache, "http://127.0.0.1:1", &installed).await;
        assert!(result.contains_key("cached"));
        assert!(!result.contains_key("missing"));
    }

    #[tokio::test]
    async fn reconcile_skips_misses_with_invalid_slugs() {
        // Defense in depth: a compromised Ollama returning a slug with
        // shell metacharacters or whitespace must be dropped before any
        // network work, never make it into the `/api/show` request.
        let mut server = mockito::Server::new_async().await;
        let m = server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_body(r#"{"capabilities":["vision"]}"#)
            .expect(0)
            .create_async()
            .await;
        let cache = ModelCapabilitiesCache::default();
        let client = reqwest::Client::new();
        let installed = vec!["bad name".to_string(), "bad$(whoami)".to_string()];
        let result = reconcile_capabilities(&client, &cache, &server.url(), &installed).await;
        assert!(result.is_empty());
        m.assert_async().await;
    }

    #[tokio::test]
    async fn reconcile_when_cache_poisoned_still_attempts_fetches() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/api/show")
            .with_status(200)
            .with_body(r#"{"capabilities":["vision"]}"#)
            .create_async()
            .await;
        let cache = ModelCapabilitiesCache::default();
        // Poison the mutex so the read-snapshot branch falls back to
        // treating every slug as a miss.
        let cache_ref = std::panic::AssertUnwindSafe(&cache.0);
        let _ = std::panic::catch_unwind(|| {
            let _guard = cache_ref.0.lock().unwrap();
            panic!("poison");
        });
        let client = reqwest::Client::new();
        let installed = vec!["x".to_string()];
        let result = reconcile_capabilities(&client, &cache, &server.url(), &installed).await;
        // Cache writes silently fail on the poisoned lock, but the
        // result map still carries the freshly-fetched value.
        assert!(result["x"].vision);
    }
}
