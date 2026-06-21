/*!
 * Active-model state module.
 *
 * The "active" model is whichever slug the user last picked via the picker
 * popup. It is persisted across launches on the active provider's `model`
 * field in `config.toml` (see [`crate::config::schema::Provider`]) and mirrored
 * in [`ActiveModelState`] for fast reads from Tauri commands. The legacy SQLite
 * [`ACTIVE_MODEL_KEY`] is read once at startup and folded onto the active
 * provider by `crate::config::migrate`; it is no longer written.
 *
 * The backend treats Ollama's `/api/tags` response as authoritative: a
 * persisted model is only honored if it still appears in the live installed
 * list. If not, we fall back to the first installed model. There is no
 * compiled fallback: when nothing is installed and nothing is persisted,
 * the active model is `None` and the user is prompted to pick one.
 */

pub mod download;
pub mod gguf;
pub mod manifest;
pub mod reasoning;
pub mod registry;
pub mod storage;

use std::collections::HashMap;
use std::sync::Mutex;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::config::defaults::{
    DEFAULT_OLLAMA_SHOW_REQUEST_TIMEOUT_SECS, DEFAULT_OLLAMA_TAGS_REQUEST_TIMEOUT_SECS,
    HF_API_TIMEOUT_SECS, HF_BASE_URL, HF_SEARCH_LIMIT_MAX, MAX_HF_API_BODY_BYTES,
    MAX_HF_SEARCH_QUERY_LEN, MAX_MODEL_CONTEXT_LENGTH, MAX_MODEL_SLUG_LEN,
    MAX_OLLAMA_SHOW_BODY_BYTES, MAX_OLLAMA_TAGS_BODY_BYTES, OPENAI_MODELS_TIMEOUT_SECS,
    PROVIDER_ID_BUILTIN, PROVIDER_KIND_BUILTIN, PROVIDER_KIND_OLLAMA, PROVIDER_KIND_OPENAI,
    RUNTIME_OVERHEAD_GB,
};
use crate::config::AppConfig;

/// Legacy SQLite `app_config` key that older builds used to persist the
/// selected model slug. Now read once at startup and folded onto the active
/// provider's `model` field by `crate::config::migrate`; never written anymore.
pub const ACTIVE_MODEL_KEY: &str = "active_model";

/// Shared error-message prefix used when a requested slug is not present in
/// the active provider's inventory (the live Ollama tags, the builtin
/// manifest, or the openai configured model). Exported so the frontend and
/// tests can match against a stable constant instead of a prose string.
pub const MODEL_NOT_INSTALLED_ERR_PREFIX: &str = "Model is not installed: ";

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

/// Installed-model inventory for the active provider, plus a reachability
/// flag, routed by provider kind:
///
/// - `builtin`: the manifest ids passed in by the caller, no network probe.
///   The engine starts on demand per request, so the inventory is always
///   trustworthy and `reachable` is always `true`.
/// - `openai`: the provider's configured model as a single-element list
///   (empty when none is configured yet). No probe either: errors surface
///   at request time, and model management lives in Settings.
/// - anything else (Ollama): probes `{base_url}/api/tags`. A fetch failure
///   collapses into `(empty, false)` so the caller can emit the structured
///   unreachable payload instead of an error string.
///
/// Extracted from `get_model_picker_state` so the kind routing is testable
/// without a Tauri runtime; the command wrapper only does state plumbing.
pub async fn picker_inventory_for_kind(
    client: &reqwest::Client,
    kind: &str,
    base_url: &str,
    provider_model: Option<&str>,
    builtin_installed: &[String],
) -> (Vec<String>, bool) {
    match kind {
        PROVIDER_KIND_BUILTIN => (builtin_installed.to_vec(), true),
        PROVIDER_KIND_OPENAI => (
            provider_model
                .map(|m| vec![m.to_string()])
                .unwrap_or_default(),
            true,
        ),
        _ => match fetch_installed_model_names(client, base_url).await {
            Ok(installed) => (installed, true),
            Err(_) => (Vec::new(), false),
        },
    }
}

/// Reads every installed-model id from the manifest. Thin DB wrapper shared
/// by the commands that need the builtin inventory (`get_model_picker_state`,
/// `set_active_model`, `check_model_setup`); the underlying `manifest::list`
/// carries the tested logic.
#[cfg_attr(coverage_nightly, coverage(off))]
fn manifest_model_ids(db: &crate::history::Database) -> Result<Vec<String>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    Ok(manifest::list(&conn)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|m| m.id)
        .collect())
}

/// Returns the currently active model, the full list of installed models, and
/// a flag telling the frontend whether the active provider's inventory could
/// be read.
///
/// Shape: `{ "active": "<slug>" | null, "all": ["<slug>", ...], "ollamaReachable": bool }`.
/// The wire key stays the legacy camelCase `ollamaReachable` even though the
/// flag is provider-generic now: renaming it would churn the frontend
/// contract for zero behavioral gain. For `builtin` and `openai` providers
/// the flag is always `true` (see [`picker_inventory_for_kind`]).
///
/// The command intentionally never propagates a transport / fetch error to
/// the frontend. Instead, an unreachable Ollama collapses into a structured
/// `{ active: null, all: [], ollamaReachable: false }` payload so the UI can
/// distinguish "Ollama is down" from "Ollama is up but has no models" without
/// parsing error strings. Resolution + conditional persist go through
/// [`resolve_active_model`] and [`should_persist_resolved`], which refuse to
/// persist when the provider reports an empty inventory so a partially-up
/// daemon cannot corrupt the persisted choice. The resolved value (possibly
/// `None` when unreachable or empty) is always mirrored into the in-memory
/// [`ActiveModelState`] so downstream callers (ask_model, search_pipeline)
/// see the same truth as the frontend.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn get_model_picker_state(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    active_model: tauri::State<'_, ActiveModelState>,
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
    db: tauri::State<'_, crate::history::Database>,
) -> Result<serde_json::Value, String> {
    let (base_url, active_id, persisted, kind) = read_provider_model_context(&config);
    let manifest_rows = if kind == PROVIDER_KIND_BUILTIN {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        manifest::list(&conn).map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    let manifest_ids: Vec<String> = manifest_rows.iter().map(|m| m.id.clone()).collect();
    let display_names = manifest_displays_map(&manifest_rows);
    let (installed, reachable) = picker_inventory_for_kind(
        &client,
        &kind,
        &base_url,
        persisted.as_deref(),
        &manifest_ids,
    )
    .await;

    let resolved = resolve_active_model(persisted.as_deref(), &installed);
    if let Some(slug) = resolved.as_deref() {
        if should_persist_resolved(&installed, persisted.as_deref(), slug) {
            persist_active_provider_model(&app, &config, &active_id, slug)?;
        }
    }

    {
        let mut guard = active_model.0.lock().map_err(|e| e.to_string())?;
        *guard = resolved.clone();
    }

    Ok(build_picker_state_payload(
        resolved.as_deref(),
        &installed,
        reachable,
        &display_names,
    ))
}

/// Snapshots the active provider's base URL, id, selected model, and kind
/// from the shared config under a single lock read so a concurrent provider
/// switch can never pair fields from different providers. Returns the model
/// as `Option<String>` (empty -> `None`) so callers can feed it straight into
/// the resolve helpers.
#[cfg_attr(coverage_nightly, coverage(off))]
fn read_provider_model_context(
    config: &parking_lot::RwLock<AppConfig>,
) -> (String, String, Option<String>, String) {
    let c = config.read();
    (
        c.inference.active_provider_base_url().to_string(),
        c.inference.active_provider.clone(),
        c.inference.active_provider_model_opt().map(str::to_string),
        c.inference.active_provider_kind().to_string(),
    )
}

/// Writes `slug` onto the active provider's `model` field in config.toml and
/// swaps the resolved result into the shared in-memory config. Replaces the
/// former SQLite `set_config(ACTIVE_MODEL_KEY, ...)` persistence. When the
/// written provider is the active one, also refreshes the managed
/// [`ActiveModelState`] mirror so chat sees the new selection without a
/// restart (e.g. a builtin download finishing via `finalize_install`).
#[cfg_attr(coverage_nightly, coverage(off))]
fn persist_active_provider_model(
    app: &tauri::AppHandle,
    config: &parking_lot::RwLock<AppConfig>,
    provider_id: &str,
    slug: &str,
) -> Result<(), String> {
    let path = crate::settings_commands::config_path(app).map_err(|e| e.to_string())?;
    let mirror = persist_provider_model_locked(&path, config, provider_id, slug)?;
    if let Some(mirror) = mirror {
        let active = app.state::<ActiveModelState>();
        let mut guard = active.0.lock().map_err(|e| e.to_string())?;
        *guard = mirror;
    }
    // Broadcast the same config-change event every settings_commands writer
    // emits, so the other webview (the overlay's picker, or the Settings panel)
    // resyncs live. set_active_model is otherwise the only model-write path
    // that left other windows stale; this also covers finalize_install's
    // auto-select and the delete-clear path. The listeners refresh via the
    // read-only get_config, never reload_config_from_disk, so this cannot loop.
    let _ = app.emit(crate::settings_commands::CONFIG_UPDATED_EVENT, ());
    Ok(())
}

/// Serialized core of [`persist_active_provider_model`]: takes the config
/// write guard BEFORE the on-disk read-modify-write and holds it until the
/// in-memory snapshot is replaced. Every config disk writer serializes on
/// this same lock (see the `settings_commands` module docs), so a background
/// persist (e.g. a download finalizing) can never interleave with a
/// Settings-UI write: the loser of an unserialized race would re-read a
/// stale file and revert the other writer's change. The disk I/O is
/// synchronous `std::fs`, so holding the `parking_lot` guard across it is
/// safe (no `.await` runs under the guard). Returns the
/// [`should_refresh_active_model`] decision for the caller to apply to the
/// [`ActiveModelState`] mirror outside the guard.
pub(crate) fn persist_provider_model_locked(
    path: &std::path::Path,
    config: &parking_lot::RwLock<AppConfig>,
    provider_id: &str,
    slug: &str,
) -> Result<Option<Option<String>>, String> {
    let mut guard = config.write();
    let resolved =
        crate::settings_commands::write_provider_field_to_disk(path, provider_id, "model", slug)
            .map_err(|e| e.to_string())?;
    let mirror = should_refresh_active_model(provider_id, &resolved);
    *guard = resolved;
    Ok(mirror)
}

/// Decides whether a provider-model write must be mirrored into the managed
/// [`ActiveModelState`]. Returns `Some(new_value)` only when `provider_id` is
/// the resolved config's active provider (the mirror tracks the active
/// provider only); the value is the resolved model with empty mapped to
/// `None` (the delete-model clear path writes ""). Pure so the decision is
/// unit-tested even though the persisting wrapper is coverage-off.
pub(crate) fn should_refresh_active_model(
    provider_id: &str,
    resolved: &AppConfig,
) -> Option<Option<String>> {
    if resolved.inference.active_provider != provider_id {
        return None;
    }
    Some(
        resolved
            .inference
            .active_provider_model_opt()
            .map(str::to_string),
    )
}

/// Pure helper that shapes the `get_model_picker_state` payload. Extracted so
/// the three states (unreachable, reachable + empty, reachable + populated)
/// can be unit-tested without spinning up a Tauri runtime or an HTTP server.
pub fn build_picker_state_payload(
    active: Option<&str>,
    installed: &[String],
    ollama_reachable: bool,
    display_names: &HashMap<String, String>,
) -> serde_json::Value {
    let active_value = match active {
        Some(slug) => serde_json::Value::String(slug.to_string()),
        None => serde_json::Value::Null,
    };
    serde_json::json!({
        "active": active_value,
        "all": installed,
        "ollamaReachable": ollama_reachable,
        // id -> friendly display name; populated for built-in models (whose ids
        // are "repo:file.gguf"), empty for Ollama/OpenAI whose ids already read
        // cleanly. The frontend falls back to the id when an entry is missing.
        "displayNames": display_names,
    })
}

/// Maps each installed model's id to its recorded display name, for the picker
/// to show "Qwen3.5 9B" instead of the raw "repo:file.gguf" id.
fn manifest_displays_map(rows: &[manifest::InstalledModel]) -> HashMap<String, String> {
    rows.iter()
        .map(|m| (m.id.clone(), m.display_name.clone()))
        .collect()
}

/// Persists `model` as the active model after validating its shape and
/// confirming the active provider still serves it. The validation source is
/// routed by provider kind exactly like [`picker_inventory_for_kind`]: the
/// builtin manifest and the openai configured model never touch the network,
/// while the Ollama arm keeps probing `/api/tags` and propagating fetch
/// errors verbatim. Rejects unserved slugs with an error that starts with
/// [`MODEL_NOT_INSTALLED_ERR_PREFIX`].
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn set_active_model(
    model: String,
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    active_model: tauri::State<'_, ActiveModelState>,
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
    db: tauri::State<'_, crate::history::Database>,
) -> Result<(), String> {
    validate_model_slug(&model)?;

    let (ollama_url, active_id, persisted, kind) = read_provider_model_context(&config);
    let installed: Vec<String> = match kind.as_str() {
        PROVIDER_KIND_BUILTIN => manifest_model_ids(&db)?,
        PROVIDER_KIND_OPENAI => persisted.into_iter().collect(),
        _ => fetch_installed_model_names(&client, &ollama_url).await?,
    };
    validate_model_installed(&model, &installed)?;

    persist_active_provider_model(&app, &config, &active_id, &model)?;

    {
        let mut guard = active_model.0.lock().map_err(|e| e.to_string())?;
        *guard = Some(model);
    }

    Ok(())
}

// ─── Model setup gate (onboarding) ──────────────────────────────────────────

/// Result of probing the local Ollama daemon for setup readiness.
///
/// Drives the onboarding model-setup gate that fires after the user grants
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
    /// The active provider has no usable model yet (built-in engine with no
    /// downloaded starter, or an `openai` provider with no model configured).
    /// The UI must offer the starter download picker.
    NeedsDownload,
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

/// Pure setup gate for the built-in engine: Ready when the provider has a
/// model selected AND that model is recorded in the installed manifest;
/// NeedsDownload otherwise (no model chosen yet, or the manifest row was
/// removed out from under a stale provider pointer).
///
/// `installed` carries every manifest id so the Ready payload mirrors the
/// Ollama arm's shape (active slug + full inventory).
pub fn derive_builtin_setup_state(
    provider_model: Option<&str>,
    manifest_ids: &[String],
) -> ModelSetupState {
    match provider_model {
        Some(model) if manifest_ids.iter().any(|id| id == model) => ModelSetupState::Ready {
            active_slug: model.to_string(),
            installed: manifest_ids.to_vec(),
        },
        _ => ModelSetupState::NeedsDownload,
    }
}

/// Defensive setup gate for an `openai`-kind active provider. Onboarding never
/// sets one active, but if a hand-edited config does, a configured model is
/// treated as Ready (there is no probe surface to verify against) and an
/// unconfigured one falls back to the download picker.
pub fn derive_openai_setup_state(provider_model: Option<&str>) -> ModelSetupState {
    match provider_model {
        Some(model) => ModelSetupState::Ready {
            active_slug: model.to_string(),
            installed: vec![model.to_string()],
        },
        None => ModelSetupState::NeedsDownload,
    }
}

/// Base URL of the configured Ollama provider, regardless of which provider
/// is active. Empty when no Ollama-kind provider exists (the loader always
/// seeds one, so the fallback is defensive).
pub fn ollama_provider_base_url(config: &AppConfig) -> String {
    config
        .inference
        .providers
        .iter()
        .find(|p| p.kind == PROVIDER_KIND_OLLAMA)
        .map(|p| p.base_url.clone())
        .unwrap_or_default()
}

/// True when a local Ollama daemon answered `/api/tags` on the configured
/// Ollama provider's base URL, regardless of how many models it reports.
/// Backs onboarding's "Use my existing Ollama instead" escape hatch while
/// the built-in provider is active (so `get_model_picker_state`, which
/// probes the ACTIVE provider and mutates the active-model mirror, cannot
/// be reused here).
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn detect_ollama(
    client: tauri::State<'_, reqwest::Client>,
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
) -> Result<bool, String> {
    let base_url = ollama_provider_base_url(&config.read());
    Ok(fetch_installed_model_names(&client, &base_url)
        .await
        .is_ok())
}

/// Probes the active provider for setup readiness and returns the typed
/// [`ModelSetupState`] for the frontend onboarding gate.
///
/// Routing is by provider kind: `builtin` consults the installed-model
/// manifest, `openai` trusts its configured model, and Ollama probes
/// `/api/tags` exactly as before.
///
/// Idempotent: safe to call on every overlay open. The Ready arm also
/// commits two side effects, both intentionally bounded:
///
/// 1. If the resolved slug differs from the persisted slug AND the live
///    installed list is non-empty, persist the resolved slug. This heals
///    the case where a user removed their previously-selected model with
///    `ollama rm` between launches.
/// 2. Mirror the resolved slug into the in-memory [`ActiveModelState`] so
///    `ask_model` and `search_pipeline` see it on the next request
///    without an extra DB read.
///
/// Both writes are gated through [`should_persist_resolved`] which
/// refuses to persist when Ollama reports an empty inventory (i.e.
/// daemon is up but mid-restart), so a transient empty response cannot
/// clobber a valid persisted choice.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn check_model_setup(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    active_model: tauri::State<'_, ActiveModelState>,
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
    db: tauri::State<'_, crate::history::Database>,
) -> Result<ModelSetupState, String> {
    let (ollama_url, active_id, persisted, kind) = read_provider_model_context(&config);

    let state = match kind.as_str() {
        PROVIDER_KIND_BUILTIN => {
            let ids = manifest_model_ids(&db)?;
            derive_builtin_setup_state(persisted.as_deref(), &ids)
        }
        PROVIDER_KIND_OPENAI => derive_openai_setup_state(persisted.as_deref()),
        _ => {
            let installed_result = fetch_installed_model_names(&client, &ollama_url).await;
            derive_model_setup_state(installed_result, persisted.as_deref())
        }
    };

    if let ModelSetupState::Ready {
        ref active_slug,
        ref installed,
    } = state
    {
        if should_persist_resolved(installed, persisted.as_deref(), active_slug) {
            persist_active_provider_model(&app, &config, &active_id, active_slug)?;
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
    /// Reasoning is structural and cannot be turned off (gpt-oss/Harmony,
    /// DeepSeek-R1, QwQ, ...). Thuki still shows such a model's reasoning
    /// cleanly and marks it in the picker so the user is not surprised by the
    /// latency. `false` when reasoning is optional (off by default) or absent.
    #[serde(default)]
    pub reasoning_always: bool,
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

/// In-memory cache of capabilities keyed by `(provider_id, model)`. The same
/// model slug can resolve to different capabilities on different providers, so
/// the provider id is part of the key. Populated lazily the first time a model
/// is queried; cleared on app restart, which is the simplest valid invalidation
/// strategy (capabilities for a given provider+slug pair never change during a
/// process lifetime).
#[derive(Default)]
pub struct ModelCapabilitiesCache(pub Mutex<HashMap<(String, String), Capabilities>>);

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
///
/// Non-Ollama kinds never touch the network: the built-in provider reads the
/// curated vision/thinking flags from the installed-model manifest and an
/// `openai` provider maps its manual vision flag onto its configured model.
/// Both write through to the cache under the same `(provider_id, model)` keys
/// as the Ollama path so `ask_model`'s per-request capability filter sees one
/// cache shape for every kind.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn get_model_capabilities(
    client: tauri::State<'_, reqwest::Client>,
    cache: tauri::State<'_, ModelCapabilitiesCache>,
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
    db: tauri::State<'_, crate::history::Database>,
) -> Result<HashMap<String, Capabilities>, String> {
    let (provider_id, base_url, kind, provider_model, provider_vision) = {
        let c = config.read();
        (
            c.inference.active_provider.clone(),
            c.inference.active_provider_base_url().to_string(),
            c.inference.active_provider_kind().to_string(),
            c.inference.active_provider_model().to_string(),
            c.inference.active().map(|p| p.vision).unwrap_or(false),
        )
    };
    match kind.as_str() {
        PROVIDER_KIND_BUILTIN => {
            let rows = {
                let conn = db.0.lock().map_err(|e| e.to_string())?;
                manifest::list(&conn).map_err(|e| e.to_string())?
            };
            let caps = builtin_capabilities_from_manifest(&rows);
            cache_capabilities(&cache, &provider_id, &caps);
            Ok(caps)
        }
        PROVIDER_KIND_OPENAI => {
            let caps = openai_capabilities(&provider_model, provider_vision);
            cache_capabilities(&cache, &provider_id, &caps);
            Ok(caps)
        }
        _ => {
            let installed = fetch_installed_model_names(&client, &base_url).await?;
            Ok(reconcile_capabilities(&client, &cache, &provider_id, &base_url, &installed).await)
        }
    }
}

/// Capability map for the built-in provider. For a curated starter the flags
/// come from the current registry, not the manifest row: the row freezes the
/// flags recorded at download time, so a later flag correction (e.g. a
/// reasoning model previously recorded as non-thinking) would otherwise stay
/// wrong for already-installed models. Reading the registry heals those rows on
/// every read with no manifest migration. A pasted (non-curated) repo has no
/// registry entry and keeps the flags its row recorded. `max_images` stays
/// `None` because llama-server imposes no fixed per-request image cap.
pub(crate) fn builtin_capabilities_from_manifest(
    rows: &[manifest::InstalledModel],
) -> HashMap<String, Capabilities> {
    rows.iter()
        .map(|row| {
            // Curated starters heal `vision`/`thinking`/`reasoning_always` from
            // the registry (highest confidence). A pasted repo has no registry
            // entry and keeps its row's classified flags: the install-time GGUF
            // classifier populates them, and the runtime backstop corrects them.
            let (vision, thinking, reasoning_always) =
                registry::by_repo_file(&row.repo, &row.file_name)
                    .map(|s| (s.vision, s.thinking, s.reasoning_always))
                    .unwrap_or((row.vision, row.thinking, row.reasoning_always));
            (
                row.id.clone(),
                Capabilities {
                    vision,
                    thinking,
                    reasoning_always,
                    max_images: None,
                },
            )
        })
        .collect()
}

/// Capability map for an `openai`-kind provider: a single entry for the
/// configured model, driven by the provider's manual vision flag (generic
/// `/v1` servers expose no capability probe). Thinking stays `false`: there
/// is no declared reasoning-token contract to honor. An empty model (none
/// configured yet) yields an empty map.
pub(crate) fn openai_capabilities(model: &str, vision: bool) -> HashMap<String, Capabilities> {
    if model.is_empty() {
        return HashMap::new();
    }
    HashMap::from([(
        model.to_string(),
        Capabilities {
            vision,
            thinking: false,
            reasoning_always: false,
            max_images: None,
        },
    )])
}

/// Writes a resolved capability map through to the cache under
/// `(provider_id, model)` keys, mirroring the Ollama reconcile path's
/// write-through so `ask_model`'s per-request filter finds the entries.
/// Best-effort: a poisoned lock skips the write (the map is still returned
/// to the caller).
pub(crate) fn cache_capabilities(
    cache: &ModelCapabilitiesCache,
    provider_id: &str,
    caps: &HashMap<String, Capabilities>,
) {
    if let Ok(mut guard) = cache.0.lock() {
        for (model, c) in caps {
            guard.insert((provider_id.to_string(), model.clone()), c.clone());
        }
    }
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
    provider_id: &str,
    base_url: &str,
    installed: &[String],
) -> HashMap<String, Capabilities> {
    let mut hits: HashMap<String, Capabilities> = HashMap::new();
    let mut misses: Vec<String> = Vec::new();
    match cache.0.lock() {
        Ok(guard) => {
            for name in installed {
                if let Some(c) = guard.get(&(provider_id.to_string(), name.clone())) {
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
                guard.insert((provider_id.to_string(), name.clone()), caps.clone());
            }
            hits.insert(name.clone(), caps);
        }
    }
    hits
}

// ─── Model library (built-in engine downloads) ──────────────────────────────

/// Stable error returned when a repo id fails [`is_valid_repo_id`].
const INVALID_REPO_ID_ERR: &str = "invalid Hugging Face repo id";

/// Cancellation handles for the in-flight model downloads, keyed by the
/// caller-supplied download key (the frontend's stable per-row identity, e.g. a
/// Staff Picks id or `repo\0file`). Empty when nothing is downloading. Distinct
/// keys download concurrently; a duplicate key is rejected via [`claim_download`]
/// so the same row cannot start twice.
///
/// Parallelism never corrupts the content-addressed blob store, and this map is
/// not what protects it: [`download::run_download`] verifies each blob's sha256
/// before renaming its partial into the store, and [`download::download_one`]
/// skips a blob whose final file already exists. With per-key dedupe here and
/// the distinct blob shas the registry guarantees (asserted in
/// `registry::tests`), no two concurrent downloads target the same blob, so no
/// per-blob lock is needed.
#[derive(Default)]
pub struct DownloadState(
    pub std::sync::Mutex<std::collections::HashMap<String, tokio_util::sync::CancellationToken>>,
);

/// Atomically claims a download slot for `key`. Returns a fresh cancellation
/// token on success; an error when `key` already has an in-flight download (or
/// the lock is poisoned).
pub fn claim_download(
    state: &DownloadState,
    key: &str,
) -> Result<tokio_util::sync::CancellationToken, String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if guard.contains_key(key) {
        return Err("a download is already in progress".to_string());
    }
    let token = tokio_util::sync::CancellationToken::new();
    guard.insert(key.to_string(), token.clone());
    Ok(token)
}

/// Releases the slot held by `key`. Best-effort: a poisoned lock is ignored
/// because release runs on the task teardown path where there is nothing left
/// to do.
pub fn release_download(state: &DownloadState, key: &str) {
    if let Ok(mut guard) = state.0.lock() {
        guard.remove(key);
    }
}

/// True while any model download holds a slot. Read before quitting so the app
/// can warn that quitting discards the in-flight download(s).
pub fn download_in_flight(state: &DownloadState) -> bool {
    state
        .0
        .lock()
        .map(|guard| !guard.is_empty())
        .unwrap_or(false)
}

/// Cancels the download held by `key`, if one is in flight. Does NOT remove the
/// slot: the download task notices the cancellation, emits `Cancelled`, and
/// releases its own slot. A missing key is a harmless no-op.
pub fn cancel_download(state: &DownloadState, key: &str) {
    if let Ok(guard) = state.0.lock() {
        if let Some(token) = guard.get(key) {
            token.cancel();
        }
    }
}

/// One starter row for the download picker: the compile-time registry entry
/// plus the machine-specific runtime facts the UI renders next to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StarterOption {
    /// The curated registry entry (tier, repo, sizes, license).
    pub starter: registry::Starter,
    /// RAM-fit badge for this machine.
    pub fit: registry::RamFit,
    /// Whether the starter is already recorded in the installed manifest.
    pub installed: bool,
    /// Length of an interrupted download's partial file, when one exists.
    pub partial_bytes: Option<u64>,
}

/// Annotates one registry entry with the machine-specific facts the picker
/// renders next to it: RAM fit, installed state, and resumable-partial size. A
/// manifest read error degrades to "not installed" rather than failing the row.
fn annotate_starter(
    s: &registry::Starter,
    conn: &rusqlite::Connection,
    store: &storage::ModelStore,
    ram_bytes: u64,
) -> StarterOption {
    StarterOption {
        starter: s.clone(),
        fit: registry::ram_fit(s.est_runtime_gb, ram_bytes),
        installed: matches!(
            manifest::get(conn, &registry::installed_model_id(s)),
            Ok(Some(_))
        ),
        partial_bytes: store.existing_partial_len(s.sha256),
    }
}

/// The onboarding starter picker rows: exactly the three tier heroes, annotated
/// for this machine. Onboarding's 3-up comparison is fixed at one model per
/// tier, so it draws only the heroes even as the Staff Picks catalog grows.
pub fn build_starter_options(
    conn: &rusqlite::Connection,
    store: &storage::ModelStore,
    ram_bytes: u64,
) -> Vec<StarterOption> {
    registry::onboarding_heroes()
        .into_iter()
        .map(|s| annotate_starter(s, conn, store, ram_bytes))
        .collect()
}

/// The full Staff Picks catalog: every curated registry entry annotated for
/// this machine. The frontend groups the rows by `starter.category`; unlike
/// [`build_starter_options`] this is not capped at one model per tier.
pub fn build_staff_picks(
    conn: &rusqlite::Connection,
    store: &storage::ModelStore,
    ram_bytes: u64,
) -> Vec<StarterOption> {
    registry::STARTERS
        .iter()
        .map(|s| annotate_starter(s, conn, store, ram_bytes))
        .collect()
}

/// Maps a Staff Picks `id` onto its curated registry entry. An unknown id
/// yields an error rather than a panic, so a stale frontend id can never crash
/// the download path.
pub fn starter_for_id(id: &str) -> Result<&'static registry::Starter, String> {
    registry::by_id(id).ok_or_else(|| format!("unknown staff pick id: {id}"))
}

/// Maps a frontend tier string (`"fast" | "balanced" | "smartest"`) onto the
/// onboarding hero for that tier. The hero is resolved by id from
/// [`registry::ONBOARDING_HERO_IDS`], so adding more models of the same tier to
/// the Staff Picks catalog never changes which model onboarding downloads.
pub fn starter_for_tier(tier: &str) -> Result<&'static registry::Starter, String> {
    let idx = match tier {
        "fast" => 0,
        "balanced" => 1,
        "smartest" => 2,
        other => return Err(format!("unknown starter tier: {other}")),
    };
    starter_for_id(registry::ONBOARDING_HERO_IDS[idx])
}

/// The builtin provider's currently configured model id (empty when none).
pub fn builtin_provider_model(config: &AppConfig) -> String {
    config
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_BUILTIN)
        .map(|p| p.model.clone())
        .unwrap_or_default()
}

/// True when `repo` is a well-formed Hugging Face repo id: exactly two
/// non-empty segments of `[A-Za-z0-9_.-]` joined by one `/`. Validated before
/// the id is embedded in any URL so it cannot smuggle path or query syntax.
pub fn is_valid_repo_id(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(org), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let segment_ok = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes().any(|b| b.is_ascii_alphanumeric())
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    };
    segment_ok(org) && segment_ok(name)
}

/// Quantisation token extracted from a GGUF file name: the first `-`/`.`
/// separated token that contains `Q` and is made of uppercase letters,
/// digits, and underscores (e.g. `Q4_K_M`, `IQ4_XS`). Empty when none.
pub fn quant_from_filename(file: &str) -> String {
    let stem = file.strip_suffix(".gguf").unwrap_or(file);
    stem.split(['-', '.'])
        .find(|t| {
            !t.is_empty()
                && t.contains('Q')
                && t.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        })
        .map(str::to_string)
        .unwrap_or_default()
}

/// Marker substrings that flag a GGUF model as emitting explicit reasoning
/// tokens (rendered in the ThinkingBlock UI). There is no machine-readable
/// thinking signal in GGUF metadata or the Hugging Face API, so detection reads
/// the publisher's own naming: an explicit reasoning self-label
/// (`thinking`/`reasoning`/`reasoner`) or a known reasoning-first family. The
/// list is kept narrow to avoid false positives; curated starters set the flag
/// explicitly in the registry and never consult it, and a user override is the
/// authority whenever the guess is wrong.
const THINKING_MARKERS: &[&str] = &[
    "thinking",
    "reasoning",
    "reasoner",
    "deepseek-r1",
    "qwq",
    "gpt-oss",
    "magistral",
];

/// Best-effort detection of whether an arbitrary GGUF model is a reasoning
/// model, matching [`THINKING_MARKERS`] case-insensitively against both the
/// repo id and the file name. Returns `false` when nothing matches.
pub fn detect_thinking(repo: &str, file: &str) -> bool {
    let repo = repo.to_ascii_lowercase();
    let file = file.to_ascii_lowercase();
    THINKING_MARKERS
        .iter()
        .any(|marker| repo.contains(marker) || file.contains(marker))
}

/// A `.gguf` entry in a Hugging Face repo listing, for the paste-a-repo UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HfGgufFile {
    /// File name within the repo (`rfilename`).
    pub file: String,
    /// File size in bytes; 0 when the API reports no size.
    pub size_bytes: u64,
    /// LFS content digest: the blob key used to resume or discard the partial.
    /// Empty when the repo file is not LFS-backed (rare for GGUF weights).
    pub sha256: String,
    /// Length of an interrupted partial for this file on disk, or `None` when
    /// there is none. Drives the Browse-all Paused / Resume / Discard row.
    pub partial_bytes: Option<u64>,
}

/// Subset of the HF `/api/models/<repo>?blobs=true` response Thuki consumes.
#[derive(Deserialize)]
struct HfRepoInfo {
    /// Current commit SHA of the repo's default branch; pinned as the
    /// manifest revision so later repo pushes cannot change what was vetted.
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    siblings: Vec<HfSibling>,
}

/// The slice of HF's parsed `gguf` metadata block Thuki reads. Present on a
/// search row when the query requests `expand[]=gguf`. Untrusted external input:
/// the context window is sanitized before use, and the chat template is only fed
/// to the never-panicking [`reasoning::classify_reasoning`] classifier.
#[derive(Deserialize)]
struct HfGgufMeta {
    #[serde(default)]
    context_length: Option<u64>,
    /// The model's embedded chat template, the highest-signal reasoning class
    /// source. Already carried by `expand[]=gguf` (the same block that holds the
    /// context window), so reading it costs no extra request.
    #[serde(default)]
    chat_template: Option<String>,
    /// `general.architecture`, a secondary reasoning signal (e.g. gpt-oss is
    /// always-on even when its template omits the channel marker).
    #[serde(default)]
    architecture: Option<String>,
}

/// Trust check for an externally-reported context window. Accepts a positive
/// value no larger than [`MAX_MODEL_CONTEXT_LENGTH`] and narrows it to `u32`;
/// anything missing, zero, or implausibly large is dropped to `None` so a
/// hand-edited or malicious GGUF cannot inject an absurd figure into the UI.
pub fn sanitize_context_length(raw: Option<u64>) -> Option<u32> {
    raw.filter(|&n| n >= 1 && n <= MAX_MODEL_CONTEXT_LENGTH as u64)
        .map(|n| n as u32)
}

/// One repo file in the HF listing. Only LFS-backed `.gguf` files matter.
#[derive(Deserialize)]
struct HfSibling {
    #[serde(default)]
    rfilename: String,
    /// Plain (non-LFS) size; fallback for the file browser listing.
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<HfLfs>,
}

/// LFS pointer metadata: the digest the downloader verifies against.
#[derive(Deserialize)]
struct HfLfs {
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

/// The sibling's LFS digest and size, when both are present.
fn lfs_digest(s: &HfSibling) -> Option<(String, u64)> {
    let lfs = s.lfs.as_ref()?;
    Some((lfs.sha256.clone()?, lfs.size?))
}

/// What a pasted repo id + file resolves to: the pinned commit, the weights
/// digest, and the vision companion when the repo ships an mmproj file.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoResolved {
    /// 40-hex commit SHA reported by the API at resolve time.
    pub revision: String,
    /// Lowercase hex SHA-256 of the weights blob.
    pub weights_sha256: String,
    /// Weights file size in bytes.
    pub weights_size_bytes: u64,
    /// Vision projection companion, when present in the repo.
    pub mmproj: Option<MmprojCompanion>,
}

/// An `mmproj*.gguf` sibling shipped next to the weights file.
#[derive(Debug, Clone, PartialEq)]
pub struct MmprojCompanion {
    pub file: String,
    pub sha256: String,
    pub size_bytes: u64,
}

/// True when `name` is an `mmproj*.gguf` vision projection companion. The
/// presence of one is Thuki's ground-truth vision signal: llama.cpp cannot do
/// image input without it, regardless of how the base model is tagged.
fn is_mmproj(name: &str) -> bool {
    name.starts_with("mmproj") && name.ends_with(".gguf")
}

/// Pure parse of an HF repo listing into the spec for one target `file`.
/// Capability rule for pasted repos: vision = an `mmproj*.gguf` sibling with
/// complete LFS metadata exists. The reasoning class is recorded in two stages:
/// [`repo_installed_model`] seeds `thinking` from the model name via
/// [`detect_thinking`], then `finalize_install` refines `thinking` and sets
/// `reasoning_always` from the downloaded GGUF's chat template (falling back to
/// the name guess when the template cannot be read).
pub fn resolve_listing(body: &[u8], file: &str) -> Result<RepoResolved, String> {
    let info: HfRepoInfo = serde_json::from_slice(body)
        .map_err(|e| format!("failed to decode Hugging Face API response: {e}"))?;
    let revision = info.sha.unwrap_or_default();
    if !(revision.len() == 40
        && revision
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
    {
        return Err("Hugging Face API response carries no valid commit sha".to_string());
    }
    let target = info
        .siblings
        .iter()
        .find(|s| s.rfilename == file)
        .ok_or_else(|| format!("file not found in repo: {file}"))?;
    let (weights_sha256, weights_size_bytes) =
        lfs_digest(target).ok_or_else(|| format!("file has no LFS digest metadata: {file}"))?;
    let mmproj = info
        .siblings
        .iter()
        .filter(|s| is_mmproj(&s.rfilename))
        .find_map(|s| {
            lfs_digest(s).map(|(sha256, size_bytes)| MmprojCompanion {
                file: s.rfilename.clone(),
                sha256,
                size_bytes,
            })
        });
    Ok(RepoResolved {
        revision,
        weights_sha256,
        weights_size_bytes,
        mmproj,
    })
}

/// Pure parse of an HF repo listing into the `.gguf` file browser rows.
/// Excludes `mmproj*` companions: they download alongside their weights file
/// and are never picked directly.
pub fn parse_gguf_listing(body: &[u8]) -> Result<Vec<HfGgufFile>, String> {
    let info: HfRepoInfo = serde_json::from_slice(body)
        .map_err(|e| format!("failed to decode Hugging Face API response: {e}"))?;
    Ok(info
        .siblings
        .into_iter()
        .filter(|s| s.rfilename.ends_with(".gguf") && !is_mmproj(&s.rfilename))
        .map(|s| {
            let size_bytes = s.lfs.as_ref().and_then(|l| l.size).or(s.size).unwrap_or(0);
            let sha256 = s
                .lfs
                .as_ref()
                .and_then(|l| l.sha256.clone())
                .unwrap_or_default();
            HfGgufFile {
                file: s.rfilename,
                size_bytes,
                sha256,
                partial_bytes: None,
            }
        })
        .collect())
}

/// GETs `<base>/api/models/<repo>?blobs=true` with the production timeout and
/// body cap and returns the raw body bytes.
async fn fetch_hf_repo_listing(
    client: &reqwest::Client,
    base_url: &str,
    repo: &str,
) -> Result<Vec<u8>, String> {
    fetch_hf_repo_listing_inner(
        client,
        base_url,
        repo,
        std::time::Duration::from_secs(HF_API_TIMEOUT_SECS),
        MAX_HF_API_BODY_BYTES,
    )
    .await
}

/// Innermost HF metadata fetcher with timeout and body cap configurable so
/// the cap branches are testable. The cap is enforced incrementally during
/// the streaming read, mirroring [`fetch_installed_model_names_inner`].
async fn fetch_hf_repo_listing_inner(
    client: &reqwest::Client,
    base_url: &str,
    repo: &str,
    timeout: std::time::Duration,
    max_body_bytes: usize,
) -> Result<Vec<u8>, String> {
    let url = format!(
        "{}/api/models/{}?blobs=true",
        base_url.trim_end_matches('/'),
        repo
    );
    let response = client
        .get(&url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| format!("failed to reach Hugging Face: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Hugging Face API returned HTTP {}",
            response.status().as_u16()
        ));
    }

    if let Some(declared_len) = response.content_length() {
        if declared_len as usize > max_body_bytes {
            return Err(format!(
                "Hugging Face API response exceeded {max_body_bytes} bytes"
            ));
        }
    }

    let mut stream = response.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("failed to read Hugging Face API body: {e}"))?;
        if buf.len() + chunk.len() > max_body_bytes {
            return Err(format!(
                "Hugging Face API response exceeded {max_body_bytes} bytes"
            ));
        }
        buf.extend_from_slice(&chunk);
    }

    Ok(buf)
}

/// Validates `repo`, fetches its listing from `base_url`, and resolves the
/// download spec for `file` (plus the mmproj companion when present).
/// `base_url` is parameterized so tests point at a mock server; production
/// passes [`HF_BASE_URL`].
pub async fn resolve_repo_spec(
    client: &reqwest::Client,
    base_url: &str,
    repo: &str,
    file: &str,
) -> Result<RepoResolved, String> {
    if !is_valid_repo_id(repo) {
        return Err(INVALID_REPO_ID_ERR.to_string());
    }
    let body = fetch_hf_repo_listing(client, base_url, repo).await?;
    resolve_listing(&body, file)
}

/// Validates `repo` and returns its `.gguf` file rows for the paste-a-repo
/// browser. Same API call as [`resolve_repo_spec`].
pub async fn fetch_repo_gguf_listing(
    client: &reqwest::Client,
    base_url: &str,
    repo: &str,
) -> Result<Vec<HfGgufFile>, String> {
    if !is_valid_repo_id(repo) {
        return Err(INVALID_REPO_ID_ERR.to_string());
    }
    let body = fetch_hf_repo_listing(client, base_url, repo).await?;
    parse_gguf_listing(&body)
}

// ─── Hugging Face model search ───────────────────────────────────────────────

/// One repo row from a Hugging Face model search, trimmed to the fields the
/// in-app browser needs to identify, rank, gate, and label a model.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HfModelSummary {
    /// Repo id, e.g. `unsloth/Qwen3.5-9B-GGUF`; the install target.
    pub id: String,
    /// Lifetime download count. The search is sorted by it and the UI shows it
    /// as a trust signal; `0` when the API omits the field.
    pub downloads: u64,
    /// True when the repo is access-gated (license click-through or manual
    /// approval). Gated repos cannot be fetched anonymously, so the UI can flag
    /// them instead of offering a download that would fail.
    pub gated: bool,
    /// Model's trained context window in tokens, from the repo's parsed GGUF
    /// `context_length` metadata (a per-repo property, identical across quants).
    /// `None` when the API omits it or the value fails [`sanitize_context_length`].
    pub context_length: Option<u32>,
    /// True when the repo ships an `mmproj*.gguf` vision companion (see
    /// [`is_mmproj`]). A capability of the model, shared by every quant, so the
    /// pill belongs on the repo row, not the per-quant list.
    pub vision: bool,
    /// True when the model emits reasoning tokens, from its chat template via
    /// [`reasoning::classify_reasoning`], or the repo name via [`detect_thinking`]
    /// when the template is absent. Also a per-repo capability.
    pub thinking: bool,
}

/// A page of search rows plus whether the Hub holds more. The flag is derived
/// from the raw entry count, not the kept-row count, so the per-row pipeline
/// allowlist (which drops non-chat repos) cannot prematurely end pagination.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HfSearchPage {
    pub rows: Vec<HfModelSummary>,
    pub has_more: bool,
}

/// HF `pipeline_tag`s Thuki surfaces in Browse-all: plain text chat and
/// multimodal (image+text) chat. Every other tag (embeddings, translation,
/// text-to-video, ...) is not a usable chat model and is dropped. This is an
/// allowlist applied per row, replacing a single server-side `pipeline_tag`
/// filter that could not express "text OR image-text" and so hid vision repos.
const SEARCHABLE_PIPELINE_TAGS: &[&str] = &["text-generation", "image-text-to-text"];

/// One entry in the Hugging Face `/api/models` search response. Only the fields
/// surfaced by [`HfModelSummary`] (and the `pipeline_tag` allowlist gate) are
/// decoded; everything else is ignored so upstream additions cannot break it.
#[derive(Deserialize)]
struct HfSearchEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    downloads: u64,
    /// HF reports `gated` as `false` or a strategy string (`"auto"`/`"manual"`);
    /// [`deserialize_gated`] normalizes it to a bool. Absent on some rows, so it
    /// defaults to `false`.
    #[serde(default, deserialize_with = "deserialize_gated")]
    gated: bool,
    /// HF pipeline tag, present because the search requests `expand[]=pipeline_tag`.
    /// Gated against [`SEARCHABLE_PIPELINE_TAGS`]; an absent tag drops the row.
    #[serde(default)]
    pipeline_tag: Option<String>,
    /// HF-parsed GGUF metadata, present because the search requests
    /// `expand[]=gguf`: the context window and the chat template / architecture.
    #[serde(default)]
    gguf: Option<HfGgufMeta>,
    /// Repo file listing, present because the search requests `expand[]=siblings`.
    /// Scanned for an `mmproj*.gguf` companion to derive the vision flag.
    #[serde(default)]
    siblings: Vec<HfSibling>,
}

/// Projects one raw search entry onto a summary row, or `None` when the row is
/// not a usable chat model (empty id, or a `pipeline_tag` outside
/// [`SEARCHABLE_PIPELINE_TAGS`]).
fn search_entry_to_summary(entry: HfSearchEntry) -> Option<HfModelSummary> {
    let HfSearchEntry {
        id,
        downloads,
        gated,
        pipeline_tag,
        gguf,
        siblings,
    } = entry;
    if id.is_empty() {
        return None;
    }
    if !pipeline_tag
        .as_deref()
        .is_some_and(|tag| SEARCHABLE_PIPELINE_TAGS.contains(&tag))
    {
        return None;
    }
    let vision = siblings.iter().any(|s| is_mmproj(&s.rfilename));
    // Reasoning runs through the one shared derivation so a search row and the
    // install it leads to can never disagree. A search row has no chosen file,
    // so the name fallback (used only when no template ships) sees the repo only.
    let chat_template = gguf.as_ref().and_then(|g| g.chat_template.as_deref());
    let architecture = gguf.as_ref().and_then(|g| g.architecture.as_deref());
    let (thinking, _) = reasoning_flags_from_metadata(chat_template, architecture, &id, "");
    let context_length = sanitize_context_length(gguf.and_then(|g| g.context_length));
    Some(HfModelSummary {
        id,
        downloads,
        gated,
        context_length,
        vision,
        thinking,
    })
}

/// Normalizes Hugging Face's polymorphic `gated` field (a bool `false` or a
/// strategy string like `"manual"`) into a plain bool: any string means gated,
/// `true` means gated, everything else (including `null`) means not gated.
fn deserialize_gated<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::String(_) => true,
        _ => false,
    })
}

/// Pure parse of an `/api/models` search body into a page of summary rows.
/// Non-chat and empty-id rows are dropped per [`search_entry_to_summary`];
/// `has_more` is set from the raw entry count against `limit` so dropped rows
/// never cut pagination short.
pub fn parse_search_results(body: &[u8], limit: usize) -> Result<HfSearchPage, String> {
    let entries: Vec<HfSearchEntry> = serde_json::from_slice(body)
        .map_err(|e| format!("failed to decode Hugging Face search response: {e}"))?;
    let has_more = entries.len() >= limit;
    let rows = entries
        .into_iter()
        .filter_map(search_entry_to_summary)
        .collect();
    Ok(HfSearchPage { rows, has_more })
}

// ─── RAM-fit estimation + annotated view rows ────────────────────────────────
//
// The model-settings UI surfaces a "will this fit in your Mac's RAM" hint in
// both Discover and Library. The authoritative per-starter estimate lives in
// the registry; for arbitrary downloaded/searched models there is no curated
// number, so these helpers estimate the resident footprint (weights + a fixed
// KV/runtime overhead) and reuse `registry::ram_fit` for the threshold. They
// are deliberately approximate: the result is a hint, never a hard gate.

/// A repo `.gguf` file annotated with the accurate per-quant RAM-fit computed
/// from its real file size. `fit` is `None` when host RAM or the file size is
/// unknown (both are required to judge fit).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HfGgufFileRow {
    #[serde(flatten)]
    pub file: HfGgufFile,
    pub fit: Option<registry::RamFit>,
    /// Whether this exact repo file is already recorded in the installed
    /// manifest. Lets Browse-all show an "Installed" marker instead of a
    /// download button once a quant finishes downloading.
    pub installed: bool,
}

/// An installed model annotated with its RAM-fit on the host, computed from the
/// recorded weights size. `fit` is `None` when host RAM or the size is unknown.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstalledModelView {
    #[serde(flatten)]
    pub model: manifest::InstalledModel,
    pub fit: Option<registry::RamFit>,
    /// Trained context window in tokens, healed from the curated registry by
    /// repo + file. `None` for a pasted model with no registry entry (its
    /// context is not recorded in the manifest).
    pub context_length: Option<u32>,
    /// Vision projector size in bytes, healed from the registry so the listed
    /// total (weights + mmproj) matches Discover's. `0` for a text model or a
    /// pasted repo with no registry entry (the manifest records only weights).
    pub mmproj_bytes: u64,
    /// Model maker (e.g. "Google"), healed from the registry. `None` for a
    /// pasted repo with no entry, where the UI falls back to the repo id.
    pub origin: Option<String>,
}

/// Estimated resident memory (GiB) for a GGUF weights blob of `size_bytes`:
/// the on-disk size plus the fixed [`RUNTIME_OVERHEAD_GB`].
pub fn estimate_runtime_gb_from_bytes(size_bytes: u64) -> f64 {
    size_bytes as f64 / (1u64 << 30) as f64 + RUNTIME_OVERHEAD_GB
}

/// Clamps a requested search page size to `1..=`[`HF_SEARCH_LIMIT_MAX`] so a
/// runaway page count cannot request an unbounded result set.
pub fn clamp_search_limit(limit: usize) -> usize {
    limit.clamp(1, HF_SEARCH_LIMIT_MAX)
}

/// Annotates repo `.gguf` rows with the accurate per-quant RAM-fit from each
/// file's real size. A row gets `None` when host RAM or the file size is 0.
pub fn annotate_gguf_rows(files: Vec<HfGgufFile>, ram_bytes: u64) -> Vec<HfGgufFileRow> {
    files
        .into_iter()
        .map(|file| {
            let fit = if ram_bytes > 0 && file.size_bytes > 0 {
                Some(registry::ram_fit(
                    estimate_runtime_gb_from_bytes(file.size_bytes),
                    ram_bytes,
                ))
            } else {
                None
            };
            HfGgufFileRow {
                file,
                fit,
                installed: false,
            }
        })
        .collect()
}

/// Fills each row's `partial_bytes` from the blob store so Browse-all can offer
/// Resume / Discard for any file with an interrupted partial on disk. A row
/// whose `sha256` is empty (a non-LFS file) has no content-addressed partial
/// and stays `None`.
pub fn attach_partials(
    rows: Vec<HfGgufFileRow>,
    store: &storage::ModelStore,
) -> Vec<HfGgufFileRow> {
    rows.into_iter()
        .map(|mut row| {
            if !row.file.sha256.is_empty() {
                row.file.partial_bytes = store.existing_partial_len(&row.file.sha256);
            }
            row
        })
        .collect()
}

/// Marks each row whose `<repo>:<file>` is already recorded in the installed
/// manifest, so Browse-all shows an "Installed" marker rather than a download
/// button once a quant finishes. A manifest read error degrades to "not
/// installed" rather than failing the listing, mirroring [`annotate_starter`].
pub fn attach_installed(
    rows: Vec<HfGgufFileRow>,
    repo: &str,
    conn: &rusqlite::Connection,
) -> Vec<HfGgufFileRow> {
    rows.into_iter()
        .map(|mut row| {
            let id = format!("{repo}:{}", row.file.file);
            row.installed = matches!(manifest::get(conn, &id), Ok(Some(_)));
            row
        })
        .collect()
}

/// Annotates installed models with their RAM-fit on the host, from the recorded
/// weights size. A model gets `None` when host RAM or the size is 0.
pub fn build_installed_views(
    models: Vec<manifest::InstalledModel>,
    ram_bytes: u64,
) -> Vec<InstalledModelView> {
    models
        .into_iter()
        .map(|model| {
            let fit = if ram_bytes > 0 && model.size_bytes > 0 {
                Some(registry::ram_fit(
                    estimate_runtime_gb_from_bytes(model.size_bytes),
                    ram_bytes,
                ))
            } else {
                None
            };
            // Curated models heal their context window, vision-projector size,
            // and maker from the registry so the Library row reads the same
            // facts Discover does; a pasted repo has no entry, so those stay
            // absent (the UI falls back to the repo id for the maker).
            let starter = registry::by_repo_file(&model.repo, &model.file_name);
            let context_length = starter.map(|s| s.context_length);
            let mmproj_bytes = starter.map_or(0, |s| s.mmproj_bytes);
            let origin = starter.map(|s| s.origin.to_string());
            InstalledModelView {
                model,
                fit,
                context_length,
                mmproj_bytes,
                origin,
            }
        })
        .collect()
}

/// Validates the query length, runs the Hugging Face GGUF model search against
/// `base_url`, and parses the result. `base_url` is parameterized so tests
/// point at a mock server; production passes [`HF_BASE_URL`].
pub async fn fetch_hf_search(
    client: &reqwest::Client,
    base_url: &str,
    query: &str,
    limit: usize,
) -> Result<HfSearchPage, String> {
    let query = query.trim();
    if query.len() > MAX_HF_SEARCH_QUERY_LEN {
        return Err(format!(
            "search query exceeds maximum length of {MAX_HF_SEARCH_QUERY_LEN} bytes"
        ));
    }
    let body = fetch_hf_search_inner(
        client,
        base_url,
        query,
        std::time::Duration::from_secs(HF_API_TIMEOUT_SECS),
        MAX_HF_API_BODY_BYTES,
        limit,
    )
    .await?;
    parse_search_results(&body, limit)
}

/// Innermost search fetcher with timeout, body cap, and result limit
/// configurable so the cap branches are testable. Every query parameter is
/// percent-encoded by `Url::parse_with_params` (no manual string building) so a
/// query cannot smuggle URL syntax, and the host stays fixed to `base_url` so
/// there is no SSRF surface. The body cap is enforced incrementally during the
/// streaming read, mirroring [`fetch_hf_repo_listing_inner`].
async fn fetch_hf_search_inner(
    client: &reqwest::Client,
    base_url: &str,
    query: &str,
    timeout: std::time::Duration,
    max_body_bytes: usize,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let endpoint = format!("{}/api/models", base_url.trim_end_matches('/'));
    let limit = limit.to_string();
    // `filter=gguf` matches repos *tagged* gguf (the dedicated quant repos that
    // actually ship `.gguf` files). `library=gguf` is deliberately NOT used: it
    // also matches base repos that merely link to GGUF quants elsewhere, so the
    // rows would have no downloadable `.gguf` files of their own. The chat-model
    // gate is NOT a server `pipeline_tag` filter: that param takes a single value
    // and so cannot express "text-generation OR image-text-to-text", which hid
    // every multimodal repo. Instead each row's `pipeline_tag` is expanded and
    // checked against `SEARCHABLE_PIPELINE_TAGS` in `search_entry_to_summary`.
    let mut params: Vec<(&str, &str)> = vec![
        ("filter", "gguf"),
        ("sort", "downloads"),
        ("direction", "-1"),
        ("limit", &limit),
        // One expand set carries everything a row needs in a single request, so
        // there is no per-repo follow-up call: `gguf` (context window + chat
        // template + architecture), `siblings` (the file list, scanned for an
        // mmproj vision companion), and `pipeline_tag` (the chat-model allowlist).
        ("expand[]", "gguf"),
        ("expand[]", "siblings"),
        ("expand[]", "pipeline_tag"),
    ];
    // An empty query browses the most-downloaded GGUF repos; only attach the
    // search term when the user actually typed one.
    if !query.is_empty() {
        params.push(("search", query));
    }
    let url = reqwest::Url::parse_with_params(&endpoint, params)
        .map_err(|e| format!("failed to build Hugging Face search URL: {e}"))?;
    let response = client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| format!("failed to reach Hugging Face: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Hugging Face API returned HTTP {}",
            response.status().as_u16()
        ));
    }

    if let Some(declared_len) = response.content_length() {
        if declared_len as usize > max_body_bytes {
            return Err(format!(
                "Hugging Face search response exceeded {max_body_bytes} bytes"
            ));
        }
    }

    let mut stream = response.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("failed to read Hugging Face search body: {e}"))?;
        if buf.len() + chunk.len() > max_body_bytes {
            return Err(format!(
                "Hugging Face search response exceeded {max_body_bytes} bytes"
            ));
        }
        buf.extend_from_slice(&chunk);
    }

    Ok(buf)
}

// ─── OpenAI-compatible model listing ─────────────────────────────────────────

/// Subset of an OpenAI-compatible `/v1/models` response Thuki consumes.
#[derive(Deserialize)]
struct OpenAiModelsResponse {
    #[serde(default)]
    data: Vec<OpenAiModelEntry>,
}

/// One model row in the `/v1/models` listing.
#[derive(Deserialize)]
struct OpenAiModelEntry {
    #[serde(default)]
    id: String,
}

/// Pure parse of a `/v1/models` body into model ids. Rows with an empty or
/// missing `id` are dropped rather than surfaced as blank dropdown entries.
pub fn parse_openai_models(body: &[u8]) -> Result<Vec<String>, String> {
    let parsed: OpenAiModelsResponse = serde_json::from_slice(body)
        .map_err(|e| format!("failed to decode /v1/models response: {e}"))?;
    Ok(parsed
        .data
        .into_iter()
        .map(|m| m.id)
        .filter(|id| !id.is_empty())
        .collect())
}

/// The configured OpenAI-compatible provider's `(id, base_url)`. Errors when
/// no `openai`-kind provider exists so the UI shows a stable message instead
/// of probing an empty URL.
pub fn openai_provider_target(config: &AppConfig) -> Result<(String, String), String> {
    config
        .inference
        .providers
        .iter()
        .find(|p| p.kind == PROVIDER_KIND_OPENAI)
        .map(|p| (p.id.clone(), p.base_url.clone()))
        .ok_or_else(|| "no OpenAI-compatible provider is configured".to_string())
}

/// GETs `<base_url>/v1/models` with the production timeout and body cap and
/// returns the listed model ids. `api_key` is sent as a bearer token when
/// present (keyless local servers are common, so it is optional).
pub async fn fetch_openai_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, String> {
    fetch_openai_models_inner(
        client,
        base_url,
        api_key,
        std::time::Duration::from_secs(OPENAI_MODELS_TIMEOUT_SECS),
        MAX_HF_API_BODY_BYTES,
    )
    .await
}

/// Innermost `/v1/models` fetcher with timeout and body cap configurable so
/// the cap branches are testable. The cap is enforced incrementally during
/// the streaming read, mirroring [`fetch_installed_model_names_inner`].
async fn fetch_openai_models_inner(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    timeout: std::time::Duration,
    max_body_bytes: usize,
) -> Result<Vec<String>, String> {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let mut request = client.get(&url).timeout(timeout);
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("failed to reach the server: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "/v1/models returned HTTP {}",
            response.status().as_u16()
        ));
    }

    if let Some(declared_len) = response.content_length() {
        if declared_len as usize > max_body_bytes {
            return Err(format!(
                "/v1/models response exceeded {max_body_bytes} bytes"
            ));
        }
    }

    let mut stream = response.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("failed to read /v1/models body: {e}"))?;
        if buf.len() + chunk.len() > max_body_bytes {
            return Err(format!(
                "/v1/models response exceeded {max_body_bytes} bytes"
            ));
        }
        buf.extend_from_slice(&chunk);
    }

    parse_openai_models(&buf)
}

/// Download specs for a resolved repo model: weights first, then the mmproj
/// companion. URL shape matches [`registry::download_specs`]:
/// `<base>/<repo>/resolve/<revision>/<file>`.
pub fn repo_download_specs(
    base_url: &str,
    repo: &str,
    file: &str,
    resolved: &RepoResolved,
) -> Vec<download::DownloadSpec> {
    let url = |f: &str| {
        format!(
            "{}/{}/resolve/{}/{}",
            base_url.trim_end_matches('/'),
            repo,
            resolved.revision,
            f
        )
    };
    let mut specs = vec![download::DownloadSpec {
        url: url(file),
        file: file.to_string(),
        sha256: resolved.weights_sha256.clone(),
        total_bytes: resolved.weights_size_bytes,
    }];
    if let Some(mm) = &resolved.mmproj {
        specs.push(download::DownloadSpec {
            url: url(&mm.file),
            file: mm.file.clone(),
            sha256: mm.sha256.clone(),
            total_bytes: mm.size_bytes,
        });
    }
    specs
}

/// Manifest row for a resolved repo model. id = `"<repo>:<file>"`;
/// display name = the file stem; revision pins the resolve-time commit.
pub fn repo_installed_model(
    repo: &str,
    file: &str,
    resolved: &RepoResolved,
) -> manifest::InstalledModel {
    manifest::InstalledModel {
        id: format!("{repo}:{file}"),
        display_name: file.strip_suffix(".gguf").unwrap_or(file).to_string(),
        repo: repo.to_string(),
        revision: resolved.revision.clone(),
        file_name: file.to_string(),
        sha256: resolved.weights_sha256.clone(),
        size_bytes: resolved.weights_size_bytes,
        quant: quant_from_filename(file),
        vision: resolved.mmproj.is_some(),
        // Name-based first guess; finalize_install refines `thinking` and sets
        // `reasoning_always` from the downloaded GGUF's chat template, falling
        // back to this guess when the template cannot be read.
        thinking: detect_thinking(repo, file),
        reasoning_always: false,
        mmproj_file: resolved.mmproj.as_ref().map(|m| m.file.clone()),
        mmproj_sha256: resolved.mmproj.as_ref().map(|m| m.sha256.clone()),
    }
}

/// The curated `(thinking, reasoning_always)` flags for a model, when it is a
/// registry starter. `None` for a pasted/arbitrary repo. Curated flags are the
/// highest-confidence source, so both the installer and the heal prefer them
/// over a GGUF scan.
pub(crate) fn curated_reasoning_flags(repo: &str, file_name: &str) -> Option<(bool, bool)> {
    registry::STARTERS
        .iter()
        .find(|s| s.repo == repo && s.file_name == file_name)
        .map(|s| (s.thinking, s.reasoning_always))
}

/// Derives `(thinking, reasoning_always)` for a non-curated model from its GGUF
/// metadata: a readable chat template is classified by
/// [`reasoning::classify_reasoning`]; an absent or empty template falls back to
/// the repo/file name via [`detect_thinking`], leaving `reasoning_always` off
/// for the runtime backstop to correct an always-reasoning model from real
/// output. The single reasoning-derivation point: the Browse-all search rows and
/// the install/heal path both route through it, so identical metadata always
/// yields identical flags. The name is only the inputs here; for a repo-level
/// search row with no chosen file, pass an empty `file`.
pub(crate) fn reasoning_flags_from_metadata(
    chat_template: Option<&str>,
    architecture: Option<&str>,
    repo: &str,
    file: &str,
) -> (bool, bool) {
    match chat_template {
        Some(t) if !t.is_empty() => reasoning::classify_reasoning(t, architecture).flags(),
        _ => (detect_thinking(repo, file), false),
    }
}

/// Resolves the final reasoning flags for a model: curated registry flags when
/// it is a starter, otherwise the class derived from the on-disk GGUF blob's
/// chat template (with the name fallback baked into
/// [`reasoning_flags_from_metadata`]). Coverage-off: the registry lookup and the
/// derivation are tested through [`curated_reasoning_flags`] /
/// [`reasoning_flags_from_metadata`]; this wrapper only adds the blob read.
#[cfg_attr(coverage_nightly, coverage(off))]
fn resolve_reasoning_flags(
    store: &storage::ModelStore,
    repo: &str,
    file_name: &str,
    sha256: &str,
) -> (bool, bool) {
    if let Some(curated) = curated_reasoning_flags(repo, file_name) {
        return curated;
    }
    let meta = gguf::read_gguf_metadata_from_file(&store.blob_path(sha256));
    let template = meta.as_ref().and_then(|m| m.chat_template.as_deref());
    let architecture = meta.as_ref().and_then(|m| m.architecture.as_deref());
    reasoning_flags_from_metadata(template, architecture, repo, file_name)
}

/// Re-classifies installed built-in rows whose `reasoning_always` is `NULL`
/// (rows written before the classifier existed) and persists the result so they
/// stop appearing in [`manifest::list_unclassified`]. Best-effort: any list,
/// blob-read, or write failure is logged and skipped, never fatal. Coverage-off:
/// orchestration over tested helpers (`list_unclassified`, `resolve_reasoning_flags`,
/// `update_classification`).
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn heal_unclassified_reasoning(conn: &rusqlite::Connection, store: &storage::ModelStore) {
    let pending = match manifest::list_unclassified(conn) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("thuki: [models] reasoning heal: failed to list rows: {e}");
            return;
        }
    };
    for row in pending {
        let (thinking, reasoning_always) =
            resolve_reasoning_flags(store, &row.repo, &row.file_name, &row.sha256);
        if let Err(e) = manifest::update_classification(conn, &row.id, thinking, reasoning_always) {
            eprintln!(
                "thuki: [models] reasoning heal: failed to persist {}: {e}",
                row.id
            );
        }
    }
}

/// Deletion outcome consumed by the thin Tauri wrapper.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeleteOutcome {
    /// True when the deleted model was the builtin provider's configured
    /// model, so the wrapper must clear that provider's `model` field.
    pub clear_builtin: bool,
}

/// Deletes a model from the manifest and removes the blobs no other row
/// references. `builtin_model` is the builtin provider's currently configured
/// model id; deleting it flags `clear_builtin` for the caller. Refuses while
/// a download is in flight (it may be about to insert or share the very blobs
/// being refcounted), holding the download-state lock across the removal so a
/// concurrent claim cannot race the delete (mirrors `discard_partial_inner`).
pub fn delete_installed_model_inner(
    state: &DownloadState,
    conn: &rusqlite::Connection,
    store: &storage::ModelStore,
    id: &str,
    builtin_model: &str,
) -> Result<DeleteOutcome, String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if !guard.is_empty() {
        return Err("a download is already in progress".to_string());
    }
    let orphans = manifest::delete(conn, id).map_err(|e| e.to_string())?;
    store.remove_blobs(&orphans).map_err(|e| e.to_string())?;
    Ok(DeleteOutcome {
        clear_builtin: builtin_model == id,
    })
}

/// Removes the partial file for `sha256` so the next download starts fresh.
/// Refuses malformed digests (the digest doubles as a file name) and refuses
/// while a download is running (it may be writing that very partial). Holds
/// the download-state lock across the removal so a concurrent claim cannot
/// race the delete.
pub fn discard_partial_inner(
    state: &DownloadState,
    store: &storage::ModelStore,
    sha256: &str,
) -> Result<(), String> {
    if !download::is_valid_sha256(sha256) {
        return Err("invalid sha256".to_string());
    }
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if !guard.is_empty() {
        return Err("a download is already in progress".to_string());
    }
    match std::fs::remove_file(store.partial_path(sha256)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("failed to remove partial download: {e}")),
    }
}

/// Total physical RAM in bytes via `sysctlbyname("hw.memsize")`; 0 when the
/// syscall fails.
///
/// Not covered by the cargo coverage gate: this is a direct OS syscall with
/// no branching logic beyond error propagation, making instrumentation
/// meaningless here (mirrors `storage::free_disk_bytes`).
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn system_ram_bytes() -> u64 {
    let mut value: u64 = 0;
    let mut len: libc::size_t = std::mem::size_of::<u64>();
    // SAFETY: `value` is a valid 8-byte buffer and `len` carries its exact
    // size; `sysctlbyname` writes at most `len` bytes into it on success
    // (return value 0). The name is a static NUL-terminated literal.
    unsafe {
        if libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            &mut value as *mut u64 as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        ) == 0
        {
            value
        } else {
            0
        }
    }
}

// ─── Model library Tauri commands (thin wrappers) ───────────────────────────

/// Returns the starter picker rows: registry entries annotated with RAM fit,
/// installed state, and resumable-partial size.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn get_starter_options(
    db: tauri::State<'_, crate::history::Database>,
    store: tauri::State<'_, storage::ModelStore>,
) -> Result<Vec<StarterOption>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    Ok(build_starter_options(&conn, &store, system_ram_bytes()))
}

/// Returns the full Staff Picks catalog: every curated registry entry annotated
/// with RAM fit, installed state, and resumable-partial size. The frontend
/// groups the rows by `starter.category` into use-case sections.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn get_staff_picks(
    db: tauri::State<'_, crate::history::Database>,
    store: tauri::State<'_, storage::ModelStore>,
) -> Result<Vec<StarterOption>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    Ok(build_staff_picks(&conn, &store, system_ram_bytes()))
}

/// Total physical RAM in bytes, for frontend sizing copy.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn get_system_ram_bytes() -> u64 {
    system_ram_bytes()
}

/// Free bytes on the volume holding the models directory, for the
/// pre-download disk-space line. `None` means unknown; the UI skips the line.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn get_models_dir_free_bytes(store: tauri::State<'_, storage::ModelStore>) -> Option<u64> {
    store.free_bytes()
}

/// Starts downloading a curated starter (`tier` = "fast" | "balanced" |
/// "smartest"). Progress streams over `on_event`; on success the model is
/// recorded in the manifest and set as the builtin provider's model.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn download_starter(
    tier: String,
    key: String,
    on_event: tauri::ipc::Channel<download::DownloadEvent>,
    app: tauri::AppHandle,
    download_state: tauri::State<'_, DownloadState>,
) -> Result<(), String> {
    let starter = starter_for_tier(&tier)?;
    let token = claim_download(&download_state, &key)?;
    spawn_model_download(
        app,
        registry::download_specs(starter),
        registry::to_installed_model(starter),
        key,
        token,
        on_event,
    );
    Ok(())
}

/// Starts downloading a Staff Picks catalog entry by its stable `id`. Same
/// verified path as [`download_starter`] (pinned revision + sha256, manifest
/// record on success), but keyed by id so a category can hold any number of
/// models. Progress streams over `on_event`.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn download_staff_pick(
    id: String,
    key: String,
    on_event: tauri::ipc::Channel<download::DownloadEvent>,
    app: tauri::AppHandle,
    download_state: tauri::State<'_, DownloadState>,
) -> Result<(), String> {
    let starter = starter_for_id(&id)?;
    let token = claim_download(&download_state, &key)?;
    spawn_model_download(
        app,
        registry::download_specs(starter),
        registry::to_installed_model(starter),
        key,
        token,
        on_event,
    );
    Ok(())
}

/// Starts downloading a pasted-repo model after resolving its digest, size,
/// pinned revision, and optional mmproj companion from the Hugging Face API.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn download_repo_model(
    repo: String,
    file: String,
    key: String,
    on_event: tauri::ipc::Channel<download::DownloadEvent>,
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    download_state: tauri::State<'_, DownloadState>,
) -> Result<(), String> {
    let resolved = resolve_repo_spec(&client, HF_BASE_URL, &repo, &file).await?;
    let token = claim_download(&download_state, &key)?;
    spawn_model_download(
        app,
        repo_download_specs(HF_BASE_URL, &repo, &file, &resolved),
        repo_installed_model(&repo, &file, &resolved),
        key,
        token,
        on_event,
    );
    Ok(())
}

/// Lists the `.gguf` files in a Hugging Face repo for the paste-a-repo UI.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn list_hf_repo_ggufs(
    repo: String,
    client: tauri::State<'_, reqwest::Client>,
    store: tauri::State<'_, storage::ModelStore>,
    db: tauri::State<'_, crate::history::Database>,
) -> Result<Vec<HfGgufFileRow>, String> {
    let files = fetch_repo_gguf_listing(&client, HF_BASE_URL, &repo).await?;
    let rows = attach_partials(annotate_gguf_rows(files, system_ram_bytes()), &store);
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    Ok(attach_installed(rows, &repo, &conn))
}

/// Searches Hugging Face for GGUF model repos matching `query`, most-downloaded
/// first. Backs the in-app model browser; an empty query returns the most
/// popular GGUF repos.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn search_hf_models(
    query: String,
    limit: usize,
    client: tauri::State<'_, reqwest::Client>,
) -> Result<HfSearchPage, String> {
    fetch_hf_search(&client, HF_BASE_URL, &query, clamp_search_limit(limit)).await
}

/// Lists the models served by the configured OpenAI-compatible provider via
/// its `/v1/models` endpoint, using the Keychain API key when one is stored.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn list_openai_models(
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
    secrets: tauri::State<'_, crate::keychain::Secrets>,
    client: tauri::State<'_, reqwest::Client>,
) -> Result<Vec<String>, String> {
    let (provider_id, base_url) = openai_provider_target(&config.read())?;
    // A Keychain read failure degrades to "no key": keyless local servers
    // must keep listing even when the Keychain is unavailable.
    let api_key = secrets.0.get(&provider_id).ok().flatten();
    fetch_openai_models(&client, &base_url, api_key.as_deref()).await
}

/// Cancels the in-flight model download identified by `key`, if any. The
/// download task emits `Cancelled` and keeps the partial for a later resume.
/// Other concurrent downloads are unaffected.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn cancel_model_download(key: String, download_state: tauri::State<'_, DownloadState>) {
    cancel_download(&download_state, &key);
}

/// Removes the partial file for `sha256` (the user chose Discard over Resume).
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn discard_partial_download(
    sha256: String,
    download_state: tauri::State<'_, DownloadState>,
    store: tauri::State<'_, storage::ModelStore>,
) -> Result<(), String> {
    discard_partial_inner(&download_state, &store, &sha256)
}

/// Returns every installed model from the manifest.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn list_installed_models(
    db: tauri::State<'_, crate::history::Database>,
) -> Result<Vec<InstalledModelView>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let models = manifest::list(&conn).map_err(|e| e.to_string())?;
    Ok(build_installed_views(models, system_ram_bytes()))
}

/// Deletes an installed model: manifest row, orphaned blobs, and (when it was
/// the builtin provider's selected model) the provider's `model` field.
/// Refused while a download is in flight.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn delete_installed_model(
    id: String,
    app: tauri::AppHandle,
    db: tauri::State<'_, crate::history::Database>,
    store: tauri::State<'_, storage::ModelStore>,
    config: tauri::State<'_, parking_lot::RwLock<AppConfig>>,
    download_state: tauri::State<'_, DownloadState>,
) -> Result<(), String> {
    let builtin_model = builtin_provider_model(&config.read());
    let outcome = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        delete_installed_model_inner(&download_state, &conn, &store, &id, &builtin_model)?
    };
    if outcome.clear_builtin {
        persist_active_provider_model(&app, &config, PROVIDER_ID_BUILTIN, "")?;
    }
    Ok(())
}

/// Reveals an installed model's weights blob in Finder. Thin FFI wrapper
/// (excluded from coverage) over `open -R`, mirroring
/// [`crate::settings_commands::reveal_config_in_finder`]; the manifest lookup
/// and content-addressed path are covered through `manifest::get` and
/// `storage::ModelStore::blob_path`.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn reveal_model_in_finder(
    id: String,
    db: tauri::State<'_, crate::history::Database>,
    store: tauri::State<'_, storage::ModelStore>,
) -> Result<(), String> {
    let model = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        manifest::get(&conn, &id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("model not installed: {id}"))?
    };
    std::process::Command::new("open")
        .arg("-R")
        .arg(store.blob_path(&model.sha256))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Maps the `finalize_install` outcome onto the terminal download event:
/// `AllDone` once the install is recorded, `Failed` otherwise. AllDone is
/// emitted here (after finalize) rather than from `run_download` so the
/// frontend can never advance past an install that was not recorded. Pure
/// function; testable without Tauri state.
pub(crate) fn finalize_outcome_event(result: Result<(), String>) -> download::DownloadEvent {
    match result {
        Ok(()) => download::DownloadEvent::AllDone,
        Err(message) => download::DownloadEvent::Failed {
            kind: download::DownloadFailKind::Other,
            message,
        },
    }
}

/// Runs the claimed download on the async runtime: streams events to the
/// channel, records the manifest row + builtin provider model on success
/// (then emits AllDone, or Failed when recording fails), and releases the
/// download slot in every outcome. A cancellation that lands after the run
/// already succeeded is too late to mean anything: every byte is verified
/// and installed, so the install is recorded unconditionally.
#[cfg_attr(coverage_nightly, coverage(off))]
fn spawn_model_download(
    app: tauri::AppHandle,
    specs: Vec<download::DownloadSpec>,
    model: manifest::InstalledModel,
    key: String,
    token: tokio_util::sync::CancellationToken,
    on_event: tauri::ipc::Channel<download::DownloadEvent>,
) {
    tauri::async_runtime::spawn(async move {
        let client = app.state::<reqwest::Client>().inner().clone();
        let on_event_finalize = on_event.clone();
        let result = {
            let store = app.state::<storage::ModelStore>();
            let emit = move |event: download::DownloadEvent| {
                let _ = on_event.send(event);
            };
            download::run_download(&specs, store.inner(), &client, token, emit).await
        };
        if result.is_ok() {
            let finalized = finalize_install(&app, &model);
            if let Err(e) = &finalized {
                eprintln!("thuki: [models] failed to record installed model: {e}");
            }
            let _ = on_event_finalize.send(finalize_outcome_event(finalized));
        }
        release_download(&app.state::<DownloadState>(), &key);
    });
}

/// Records a completed download: manifest insert, removal of blobs the
/// replaced row no longer references (a re-download whose upstream content
/// changed must not strand the old multi-GB blob), then adopts the model as the
/// builtin provider's selection ONLY when none is chosen yet (the active
/// provider is never changed here). A later install must not steal the active
/// model from a model the user already selected.
#[cfg_attr(coverage_nightly, coverage(off))]
fn finalize_install(
    app: &tauri::AppHandle,
    model: &manifest::InstalledModel,
) -> Result<(), String> {
    let store = app.state::<storage::ModelStore>();
    // Classify reasoning from the just-downloaded GGUF's chat template so the
    // picker badge and `/think` gate are correct the instant the install lands.
    // Curated starters keep their registry flags; a template that cannot be read
    // keeps the placeholder flags for the runtime backstop to correct.
    let (thinking, reasoning_always) =
        resolve_reasoning_flags(store.inner(), &model.repo, &model.file_name, &model.sha256);
    let model = manifest::InstalledModel {
        thinking,
        reasoning_always,
        ..model.clone()
    };
    let orphans = {
        let db = app.state::<crate::history::Database>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        manifest::insert(&conn, &model).map_err(|e| e.to_string())?
    };
    // Best-effort: the install itself succeeded, so a failure to reclaim the
    // superseded blobs must not fail the download; it only leaks disk space.
    if let Err(e) = store.remove_blobs(&orphans) {
        eprintln!("thuki: [models] failed to remove superseded blobs: {e}");
    }
    let config = app.state::<parking_lot::RwLock<AppConfig>>();
    // Auto-select only the first model: adopt this download as the built-in
    // model when the provider has none yet; otherwise a completed download just
    // installs and leaves the user's active choice alone. Parallel downloads
    // finish in arbitrary order, so a last-one-wins overwrite would be
    // unpredictable.
    if adopt_as_builtin_model(&builtin_provider_model(&config.read())) {
        persist_active_provider_model(app, &config, PROVIDER_ID_BUILTIN, &model.id)
    } else {
        Ok(())
    }
}

/// Whether a freshly installed model should become the built-in provider's
/// active model: only when the provider has no model selected yet (empty id).
/// Keeps "auto-select the first model" predictable under parallel downloads.
fn adopt_as_builtin_model(current_builtin_model: &str) -> bool {
    current_builtin_model.is_empty()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    // The generic SQLite config helpers are no longer used by the production
    // commands (model selection persists to config.toml), but the DB layer
    // itself is still covered here via the ACTIVE_MODEL_KEY round-trip test.
    use crate::database::{get_config, set_config};

    // ── build_picker_state_payload ───────────────────────────────────────────

    #[test]
    fn picker_payload_unreachable_emits_null_active_empty_list_and_flag() {
        // S1 mirrors the unreachable case: no model can be resolved, the
        // installed list is empty by definition, and the flag is false so
        // the frontend can pick the right strip copy.
        let payload = build_picker_state_payload(None, &[], false, &HashMap::new());
        assert_eq!(payload["active"], serde_json::Value::Null);
        assert_eq!(payload["all"], serde_json::json!([]));
        assert_eq!(payload["ollamaReachable"], serde_json::Value::Bool(false));
        assert_eq!(payload["displayNames"], serde_json::json!({}));
    }

    #[test]
    fn picker_payload_reachable_but_empty_keeps_flag_true_and_null_active() {
        // S2: Ollama responded but installed list is empty. Active is null
        // (nothing to resolve to) yet ollamaReachable is true so the strip
        // can tell the user to pull a model rather than start the daemon.
        let payload = build_picker_state_payload(None, &[], true, &HashMap::new());
        assert_eq!(payload["active"], serde_json::Value::Null);
        assert_eq!(payload["all"], serde_json::json!([]));
        assert_eq!(payload["ollamaReachable"], serde_json::Value::Bool(true));
        assert_eq!(payload["displayNames"], serde_json::json!({}));
    }

    #[test]
    fn picker_payload_reachable_with_models_carries_active_slug_and_display_names() {
        // S4 (normal): active slug is present and ollamaReachable is true.
        // Built-in ids carry a friendly display name so the picker shows
        // "Qwen3.5 9B" rather than the raw "repo:file.gguf" slug.
        let installed = vec!["org/repo:a.gguf".to_string(), "org/repo:b.gguf".to_string()];
        let displays = HashMap::from([
            ("org/repo:a.gguf".to_string(), "Model A".to_string()),
            ("org/repo:b.gguf".to_string(), "Model B".to_string()),
        ]);
        let payload =
            build_picker_state_payload(Some("org/repo:b.gguf"), &installed, true, &displays);
        assert_eq!(payload["active"], serde_json::json!("org/repo:b.gguf"));
        assert_eq!(
            payload["all"],
            serde_json::json!(["org/repo:a.gguf", "org/repo:b.gguf"])
        );
        assert_eq!(payload["ollamaReachable"], serde_json::Value::Bool(true));
        assert_eq!(payload["displayNames"]["org/repo:a.gguf"], "Model A");
        assert_eq!(payload["displayNames"]["org/repo:b.gguf"], "Model B");
    }

    #[test]
    fn manifest_displays_map_keys_ids_to_display_names() {
        let rows = vec![
            manifest_row("org/repo:a.gguf", true, false),
            manifest_row("org/repo:b.gguf", false, false),
        ];
        let map = manifest_displays_map(&rows);
        assert_eq!(
            map.get("org/repo:a.gguf").map(String::as_str),
            Some("Model org/repo:a.gguf")
        );
        assert_eq!(
            map.get("org/repo:b.gguf").map(String::as_str),
            Some("Model org/repo:b.gguf")
        );
    }

    // ── picker_inventory_for_kind ────────────────────────────────────────────

    #[tokio::test]
    async fn picker_inventory_builtin_serves_manifest_without_probing() {
        // The base URL is unroutable on purpose: if the builtin arm ever
        // probed the network it would collapse into the unreachable shape.
        // Getting the manifest back with reachable=true proves the builtin
        // inventory never leaves the process.
        let client = reqwest::Client::new();
        let ids = vec!["tinyllama-1.1b".to_string(), "qwen2.5-0.5b".to_string()];
        let (installed, reachable) = picker_inventory_for_kind(
            &client,
            PROVIDER_KIND_BUILTIN,
            "http://127.0.0.1:1",
            Some("tinyllama-1.1b"),
            &ids,
        )
        .await;
        assert_eq!(installed, ids);
        assert!(reachable);
    }

    #[tokio::test]
    async fn picker_inventory_builtin_empty_manifest_stays_reachable() {
        // Zero downloaded models is a "go download one" state, never an
        // "engine down" state: the frontend routes on the flag.
        let client = reqwest::Client::new();
        let (installed, reachable) =
            picker_inventory_for_kind(&client, PROVIDER_KIND_BUILTIN, "", None, &[]).await;
        assert!(installed.is_empty());
        assert!(reachable);
    }

    #[tokio::test]
    async fn picker_inventory_openai_lists_configured_model() {
        // The unroutable base URL doubles as the no-probe assertion for the
        // openai arm too.
        let client = reqwest::Client::new();
        let (installed, reachable) = picker_inventory_for_kind(
            &client,
            PROVIDER_KIND_OPENAI,
            "http://127.0.0.1:1",
            Some("gpt-4o-mini"),
            &[],
        )
        .await;
        assert_eq!(installed, vec!["gpt-4o-mini".to_string()]);
        assert!(reachable);
    }

    #[tokio::test]
    async fn picker_inventory_openai_empty_when_no_model_configured() {
        let client = reqwest::Client::new();
        let (installed, reachable) =
            picker_inventory_for_kind(&client, PROVIDER_KIND_OPENAI, "", None, &[]).await;
        assert!(installed.is_empty());
        assert!(reachable);
    }

    #[tokio::test]
    async fn picker_inventory_ollama_probes_tags_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"models":[{"name":"gemma4:e2b"}]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let (installed, reachable) =
            picker_inventory_for_kind(&client, PROVIDER_KIND_OLLAMA, &server.url(), None, &[])
                .await;

        mock.assert_async().await;
        assert_eq!(installed, vec!["gemma4:e2b".to_string()]);
        assert!(reachable);
    }

    #[tokio::test]
    async fn picker_inventory_ollama_unreachable_collapses_to_empty_and_false() {
        // Port 1 refuses connections. The persisted model must not leak into
        // the inventory: with the daemon down nothing can be trusted.
        let client = reqwest::Client::new();
        let (installed, reachable) = picker_inventory_for_kind(
            &client,
            PROVIDER_KIND_OLLAMA,
            "http://127.0.0.1:1",
            Some("gemma4:e2b"),
            &[],
        )
        .await;
        assert!(installed.is_empty());
        assert!(!reachable);
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

    // ── OpenAI-compatible model listing ──────────────────────────────────────

    #[test]
    fn parse_openai_models_extracts_ids_and_drops_blank_rows() {
        let body = br#"{"object":"list","data":[
            {"id":"llama-3.1-8b","object":"model"},
            {"id":"","object":"model"},
            {"object":"model"},
            {"id":"qwen2.5-7b"}
        ]}"#;
        assert_eq!(
            parse_openai_models(body).unwrap(),
            vec!["llama-3.1-8b".to_string(), "qwen2.5-7b".to_string()]
        );
    }

    #[test]
    fn parse_openai_models_tolerates_missing_data_field() {
        assert_eq!(parse_openai_models(b"{}").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn parse_openai_models_maps_malformed_json_to_err() {
        let err = parse_openai_models(b"not json").unwrap_err();
        assert!(err.contains("failed to decode /v1/models response"));
    }

    #[test]
    fn openai_provider_target_returns_id_and_base_url() {
        let mut cfg = AppConfig::default();
        cfg.inference
            .providers
            .push(crate::config::schema::openai_provider(
                "openai",
                "LM Studio",
                "http://127.0.0.1:1234",
            ));
        assert_eq!(
            openai_provider_target(&cfg).unwrap(),
            ("openai".to_string(), "http://127.0.0.1:1234".to_string())
        );
    }

    #[test]
    fn openai_provider_target_errors_when_absent() {
        let cfg = AppConfig::default();
        let err = openai_provider_target(&cfg).unwrap_err();
        assert!(err.contains("no OpenAI-compatible provider"));
    }

    #[tokio::test]
    async fn fetch_openai_models_sends_bearer_key_and_parses_ids() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/models")
            .match_header("authorization", "Bearer sk-test")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":[{"id":"m1"},{"id":"m2"}]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let result = fetch_openai_models(&client, &server.url(), Some("sk-test")).await;

        mock.assert_async().await;
        assert_eq!(result.unwrap(), vec!["m1".to_string(), "m2".to_string()]);
    }

    #[tokio::test]
    async fn fetch_openai_models_omits_authorization_without_key() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/models")
            .match_header("authorization", mockito::Matcher::Missing)
            .with_status(200)
            .with_body(r#"{"data":[{"id":"m1"}]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        // Trailing slash also exercises the base-url trim.
        let base = format!("{}/", server.url());
        let result = fetch_openai_models(&client, &base, None).await;

        mock.assert_async().await;
        assert_eq!(result.unwrap(), vec!["m1".to_string()]);
    }

    #[tokio::test]
    async fn fetch_openai_models_maps_http_error_to_err_string() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/models")
            .with_status(401)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let err = fetch_openai_models(&client, &server.url(), None)
            .await
            .unwrap_err();
        assert!(err.contains("/v1/models returned HTTP 401"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_openai_models_maps_transport_error_to_err_string() {
        // Bind then drop a listener so the port is closed.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let client = reqwest::Client::new();
        let err = fetch_openai_models(&client, &format!("http://{addr}"), None)
            .await
            .unwrap_err();
        assert!(err.contains("failed to reach the server"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_openai_models_rejects_body_exceeding_cap_via_content_length() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/models")
            .with_status(200)
            .with_body("x".repeat(100))
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let err = fetch_openai_models_inner(
            &client,
            &server.url(),
            None,
            std::time::Duration::from_secs(5),
            32,
        )
        .await
        .unwrap_err();
        assert!(err.contains("exceeded"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_openai_models_rejects_body_exceeding_cap_when_no_content_length() {
        // Chunked response (no Content-Length); the incremental stream cap
        // must reject when the running total exceeds the limit.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                  0a\r\n0123456789\r\n\
                  0a\r\n0123456789\r\n\
                  0a\r\n0123456789\r\n\
                  0\r\n\r\n",
            );
        });

        let client = reqwest::Client::new();
        let err = fetch_openai_models_inner(
            &client,
            &format!("http://{addr}"),
            None,
            std::time::Duration::from_secs(5),
            20,
        )
        .await
        .unwrap_err();
        assert!(err.contains("exceeded"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_openai_models_maps_body_read_error_to_err_string() {
        // Headers advertise Content-Length but the server hangs up before
        // sending the body, so the streaming read fails mid-flight.
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
        let err = fetch_openai_models(&client, &format!("http://{addr}"), None)
            .await
            .unwrap_err();
        assert!(err.contains("failed to read /v1/models body"), "got: {err}");
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
        // Provider-neutral: reachable on builtin (chip click racing a model
        // delete) and openai providers, not only Ollama.
        assert_eq!(MODEL_NOT_INSTALLED_ERR_PREFIX, "Model is not installed: ");
    }

    // ── derive_model_setup_state (onboarding model-setup gate) ──────────────

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

        let needs_download = serde_json::to_value(ModelSetupState::NeedsDownload).unwrap();
        assert_eq!(
            needs_download,
            serde_json::json!({"state": "needs_download"})
        );
    }

    // ── derive_builtin_setup_state / derive_openai_setup_state ───────────────

    #[test]
    fn builtin_ready_when_model_and_manifest() {
        // Round-trip through a real in-memory manifest so the ids carry
        // exactly what a finished download recorded.
        let conn = crate::database::open_in_memory().unwrap();
        manifest::insert(&conn, &manifest_row("org/repo:w.gguf", false, false)).unwrap();
        manifest::insert(&conn, &manifest_row("org/repo:x.gguf", false, false)).unwrap();
        let ids: Vec<String> = manifest::list(&conn)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();

        let state = derive_builtin_setup_state(Some("org/repo:w.gguf"), &ids);
        assert_eq!(
            state,
            ModelSetupState::Ready {
                active_slug: "org/repo:w.gguf".to_string(),
                installed: ids,
            }
        );
    }

    #[test]
    fn builtin_needs_download_when_no_model() {
        // Fresh install: nothing selected, nothing downloaded.
        let conn = crate::database::open_in_memory().unwrap();
        let ids: Vec<String> = manifest::list(&conn)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            derive_builtin_setup_state(None, &ids),
            ModelSetupState::NeedsDownload
        );
    }

    #[test]
    fn builtin_needs_download_when_manifest_row_missing() {
        // The provider points at a model whose manifest row is gone (e.g.
        // deleted between launches). The gate must re-engage, not trust the
        // stale pointer.
        let conn = crate::database::open_in_memory().unwrap();
        manifest::insert(&conn, &manifest_row("org/repo:other.gguf", false, false)).unwrap();
        let ids: Vec<String> = manifest::list(&conn)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            derive_builtin_setup_state(Some("org/repo:gone.gguf"), &ids),
            ModelSetupState::NeedsDownload
        );
    }

    #[test]
    fn openai_ready_when_model_configured() {
        assert_eq!(
            derive_openai_setup_state(Some("gpt-4o")),
            ModelSetupState::Ready {
                active_slug: "gpt-4o".to_string(),
                installed: vec!["gpt-4o".to_string()],
            }
        );
    }

    #[test]
    fn openai_needs_download_when_no_model_configured() {
        assert_eq!(
            derive_openai_setup_state(None),
            ModelSetupState::NeedsDownload
        );
    }

    // ── ollama_provider_base_url (detect_ollama's config read) ──────────────

    #[test]
    fn ollama_provider_base_url_reads_ollama_kind_entry() {
        // The default config seeds builtin first, Ollama second; the lookup
        // must key on kind, not position or active_provider.
        let cfg = AppConfig::default();
        assert_eq!(
            ollama_provider_base_url(&cfg),
            crate::config::defaults::DEFAULT_OLLAMA_URL
        );
    }

    #[test]
    fn ollama_provider_base_url_empty_when_no_ollama_provider() {
        let mut cfg = AppConfig::default();
        cfg.inference.providers.retain(|p| p.kind != "ollama");
        assert_eq!(ollama_provider_base_url(&cfg), "");
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
            reasoning_always: false,
            max_images: Some(1),
        };
        let v = serde_json::to_value(&caps).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "vision": true,
                "thinking": false,
                "reasoningAlways": false,
                "maxImages": 1,
            })
        );
    }

    #[test]
    fn capabilities_serialize_emits_null_max_images_when_unknown() {
        let caps = Capabilities {
            vision: true,
            thinking: false,
            reasoning_always: false,
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
            ("ollama".to_string(), "a".to_string()),
            Capabilities {
                vision: true,
                ..Default::default()
            },
        );
        cache.0.lock().unwrap().insert(
            ("ollama".to_string(), "b".to_string()),
            Capabilities {
                thinking: true,
                ..Default::default()
            },
        );
        let client = reqwest::Client::new();
        let installed = vec!["a".to_string(), "b".to_string()];
        let result =
            reconcile_capabilities(&client, &cache, "ollama", "http://unused", &installed).await;
        assert_eq!(result.len(), 2);
        assert!(result["a"].vision);
        assert!(result["b"].thinking);
    }

    #[tokio::test]
    async fn reconcile_with_empty_installed_returns_empty_map() {
        let cache = ModelCapabilitiesCache::default();
        let client = reqwest::Client::new();
        let result = reconcile_capabilities(&client, &cache, "ollama", "http://unused", &[]).await;
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
        let result =
            reconcile_capabilities(&client, &cache, "ollama", &server.url(), &installed).await;
        assert!(result["fresh"].vision);
        // Cache must now hold the fetched entry.
        let guard = cache.0.lock().unwrap();
        assert!(guard.contains_key(&("ollama".to_string(), "fresh".to_string())));
        assert!(guard[&("ollama".to_string(), "fresh".to_string())].vision);
    }

    #[tokio::test]
    async fn reconcile_drops_unreachable_misses_without_failing() {
        let cache = ModelCapabilitiesCache::default();
        cache.0.lock().unwrap().insert(
            ("ollama".to_string(), "cached".to_string()),
            Capabilities {
                vision: true,
                ..Default::default()
            },
        );
        let client = reqwest::Client::new();
        let installed = vec!["cached".to_string(), "missing".to_string()];
        // Point base_url at a port nothing listens on so misses fail fast.
        let result =
            reconcile_capabilities(&client, &cache, "ollama", "http://127.0.0.1:1", &installed)
                .await;
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
        let result =
            reconcile_capabilities(&client, &cache, "ollama", &server.url(), &installed).await;
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
        let result =
            reconcile_capabilities(&client, &cache, "ollama", &server.url(), &installed).await;
        // Cache writes silently fail on the poisoned lock, but the
        // result map still carries the freshly-fetched value.
        assert!(result["x"].vision);
    }

    // ── Non-Ollama capability resolution ─────────────────────────────────────

    /// Manifest row literal with the given capability flags.
    fn manifest_row(id: &str, vision: bool, thinking: bool) -> manifest::InstalledModel {
        manifest::InstalledModel {
            id: id.to_string(),
            display_name: format!("Model {id}"),
            repo: "org/repo".to_string(),
            revision: "a".repeat(40),
            file_name: format!("{id}.gguf"),
            sha256: "b".repeat(64),
            size_bytes: 1_000_000,
            quant: "Q4_K_M".to_string(),
            vision,
            thinking,
            reasoning_always: false,
            mmproj_file: None,
            mmproj_sha256: None,
        }
    }

    #[test]
    fn builtin_capabilities_come_from_manifest() {
        // Round-trip through a real in-memory manifest so the rows carry
        // exactly what the download recorded.
        let conn = crate::database::open_in_memory().unwrap();
        manifest::insert(&conn, &manifest_row("org/repo:vis.gguf", true, false)).unwrap();
        manifest::insert(&conn, &manifest_row("org/repo:think.gguf", false, true)).unwrap();
        let rows = manifest::list(&conn).unwrap();

        let caps = builtin_capabilities_from_manifest(&rows);

        assert_eq!(caps.len(), 2);
        assert!(caps["org/repo:vis.gguf"].vision);
        assert!(!caps["org/repo:vis.gguf"].thinking);
        assert!(!caps["org/repo:think.gguf"].vision);
        assert!(caps["org/repo:think.gguf"].thinking);
        assert!(caps.values().all(|c| c.max_images.is_none()));
    }

    /// A curated starter installed before its `thinking` flag was corrected
    /// still carries the stale flag in its manifest row. The capability view
    /// heals it from the current registry, so the model is no longer wrongly
    /// told it "does not emit thinking tokens" without a manifest migration.
    #[test]
    fn builtin_capabilities_heal_curated_flags_from_registry() {
        let fast = registry::STARTERS
            .iter()
            .find(|s| s.tier == registry::Tier::Fast)
            .unwrap();
        // Simulate a row written before the flag fix: capabilities recorded
        // as the old, wrong values.
        let mut stale = registry::to_installed_model(fast);
        stale.thinking = false;
        stale.vision = false;

        let caps = builtin_capabilities_from_manifest(&[stale]);

        let healed = &caps[&registry::to_installed_model(fast).id];
        assert!(
            healed.thinking,
            "registry heals the corrected reasoning flag"
        );
        assert!(
            healed.vision,
            "registry capabilities win for curated models"
        );
    }

    /// gpt-oss (curated Smartest) reasons unstoppably; its `reasoning_always`
    /// capability is healed from the registry so the picker can badge it. A
    /// pasted (non-curated) row defaults to not-always (runtime detection is a
    /// follow-up).
    #[test]
    fn builtin_capabilities_reasoning_always_from_registry() {
        let smartest = registry::STARTERS
            .iter()
            .find(|s| s.tier == registry::Tier::Smartest)
            .unwrap();
        let caps = builtin_capabilities_from_manifest(&[registry::to_installed_model(smartest)]);
        assert!(caps[&registry::to_installed_model(smartest).id].reasoning_always);

        let pasted =
            builtin_capabilities_from_manifest(&[manifest_row("org/repo:x.gguf", false, true)]);
        assert!(!pasted["org/repo:x.gguf"].reasoning_always);
    }

    #[test]
    fn builtin_capabilities_empty_manifest_yields_empty_map() {
        assert!(builtin_capabilities_from_manifest(&[]).is_empty());
    }

    #[test]
    fn openai_capabilities_use_provider_vision_flag() {
        let with_vision = openai_capabilities("gpt-4o", true);
        assert_eq!(with_vision.len(), 1);
        assert!(with_vision["gpt-4o"].vision);
        assert!(!with_vision["gpt-4o"].thinking);
        assert_eq!(with_vision["gpt-4o"].max_images, None);

        let without_vision = openai_capabilities("local-llm", false);
        assert!(!without_vision["local-llm"].vision);

        assert!(
            openai_capabilities("", true).is_empty(),
            "no configured model means nothing to report"
        );
    }

    #[test]
    fn cache_capabilities_writes_through_under_provider_key() {
        let cache = ModelCapabilitiesCache::default();
        let caps =
            builtin_capabilities_from_manifest(&[manifest_row("org/repo:vis.gguf", true, true)]);

        cache_capabilities(&cache, "builtin", &caps);

        let guard = cache.0.lock().unwrap();
        let entry = &guard[&("builtin".to_string(), "org/repo:vis.gguf".to_string())];
        assert!(entry.vision);
        assert!(entry.thinking);
    }

    #[test]
    fn cache_capabilities_poisoned_lock_is_best_effort() {
        let cache = ModelCapabilitiesCache::default();
        let cache_ref = std::panic::AssertUnwindSafe(&cache.0);
        let _ = std::panic::catch_unwind(|| {
            let _guard = cache_ref.0.lock().unwrap();
            panic!("poison");
        });
        // Must not panic; the write is silently skipped.
        cache_capabilities(&cache, "builtin", &openai_capabilities("m", true));
    }

    // ── Model library: starter options ───────────────────────────────────────

    /// Build a fresh store rooted at a temporary directory.
    fn make_store() -> (tempfile::TempDir, storage::ModelStore) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = storage::ModelStore::new(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    #[test]
    fn build_starter_options_returns_annotated_onboarding_heroes() {
        let conn = crate::database::open_in_memory().unwrap();
        let (_dir, store) = make_store();

        // Onboarding draws exactly the three tier heroes, in tier order. First
        // hero is installed (manifest row present); second has an in-flight
        // partial; third is untouched.
        let heroes = registry::onboarding_heroes();
        manifest::insert(&conn, &registry::to_installed_model(heroes[0])).unwrap();
        std::fs::write(store.partial_path(heroes[1].sha256), [0u8; 10]).unwrap();

        const GIB: u64 = 1 << 30;
        let opts = build_starter_options(&conn, &store, 16 * GIB);

        assert_eq!(opts.len(), heroes.len());
        assert_eq!(
            opts.iter().map(|o| o.starter.id).collect::<Vec<_>>(),
            registry::ONBOARDING_HERO_IDS.to_vec()
        );
        assert_eq!(&opts[0].starter, heroes[0]);
        assert!(opts[0].installed);
        assert_eq!(opts[0].partial_bytes, None);
        assert!(!opts[1].installed);
        assert_eq!(opts[1].partial_bytes, Some(10));
        assert!(!opts[2].installed);
        assert_eq!(opts[2].partial_bytes, None);
        // Fit hints come straight from registry::ram_fit at the given RAM.
        for (opt, s) in opts.iter().zip(heroes) {
            assert_eq!(opt.fit, registry::ram_fit(s.est_runtime_gb, 16 * GIB));
        }
    }

    #[test]
    fn build_starter_options_treats_sql_error_as_not_installed() {
        let conn = crate::database::open_in_memory().unwrap();
        conn.execute_batch("DROP TABLE installed_models;").unwrap();
        let (_dir, store) = make_store();
        let opts = build_starter_options(&conn, &store, 16 * (1 << 30));
        assert!(opts.iter().all(|o| !o.installed));
    }

    #[test]
    fn starter_option_serializes_for_frontend() {
        let opt = StarterOption {
            starter: registry::STARTERS[0].clone(),
            fit: registry::RamFit::Fits,
            installed: false,
            partial_bytes: Some(42),
        };
        let v = serde_json::to_value(&opt).unwrap();
        assert_eq!(v["fit"], serde_json::json!("fits"));
        assert_eq!(v["installed"], serde_json::json!(false));
        assert_eq!(v["partial_bytes"], serde_json::json!(42));
        assert_eq!(v["starter"]["tier"], serde_json::json!("fast"));
    }

    // ── Model library: tier parsing ──────────────────────────────────────────

    #[test]
    fn starter_for_tier_parses_and_rejects() {
        assert_eq!(starter_for_tier("fast").unwrap().tier, registry::Tier::Fast);
        assert_eq!(
            starter_for_tier("balanced").unwrap().tier,
            registry::Tier::Balanced
        );
        assert_eq!(
            starter_for_tier("smartest").unwrap().tier,
            registry::Tier::Smartest
        );
        assert!(starter_for_tier("Fast").is_err());
        assert!(starter_for_tier("").is_err());
        assert!(starter_for_tier("turbo").is_err());
    }

    #[test]
    fn starter_for_id_resolves_and_rejects() {
        // The id-keyed Staff Picks download path resolves a real slug and
        // rejects an unknown one with an error rather than a panic.
        assert_eq!(starter_for_id("qwen3.5-9b").unwrap().id, "qwen3.5-9b");
        assert_eq!(starter_for_id("gpt-oss-20b").unwrap().id, "gpt-oss-20b");
        assert!(starter_for_id("not-a-real-id").is_err());
    }

    #[test]
    fn build_staff_picks_covers_every_registry_entry() {
        let conn = crate::database::open_in_memory().unwrap();
        let (_dir, store) = make_store();
        // Install the first catalog entry; only it must read back as installed.
        manifest::insert(&conn, &registry::to_installed_model(&registry::STARTERS[0])).unwrap();

        const GIB: u64 = 1 << 30;
        let opts = build_staff_picks(&conn, &store, 16 * GIB);

        // Every registry entry is present, in registry order.
        assert_eq!(opts.len(), registry::STARTERS.len());
        assert_eq!(
            opts.iter().map(|o| o.starter.id).collect::<Vec<_>>(),
            registry::STARTERS.iter().map(|s| s.id).collect::<Vec<_>>()
        );
        assert!(opts[0].installed);
        assert!(opts[1..].iter().all(|o| !o.installed));
        // Fit comes straight from registry::ram_fit at the given RAM.
        for (opt, s) in opts.iter().zip(registry::STARTERS) {
            assert_eq!(opt.fit, registry::ram_fit(s.est_runtime_gb, 16 * GIB));
        }
    }

    // ── Model library: download claim ────────────────────────────────────────

    #[test]
    fn download_claim_allows_distinct_keys_and_rejects_a_duplicate() {
        let state = DownloadState::default();
        let token = claim_download(&state, "model-a").unwrap();
        assert!(!token.is_cancelled());
        // A different model downloads concurrently: its own slot is granted.
        assert!(claim_download(&state, "model-b").is_ok());
        // The same key cannot start twice while it is in flight.
        let err = claim_download(&state, "model-a").unwrap_err();
        assert_eq!(err, "a download is already in progress");
        // Releasing one key frees only that slot.
        release_download(&state, "model-a");
        assert!(claim_download(&state, "model-a").is_ok());
    }

    #[test]
    fn download_in_flight_tracks_any_claim() {
        let state = DownloadState::default();
        assert!(!download_in_flight(&state));
        let _a = claim_download(&state, "a").unwrap();
        let _b = claim_download(&state, "b").unwrap();
        assert!(download_in_flight(&state));
        // One release leaves the other download in flight.
        release_download(&state, "a");
        assert!(download_in_flight(&state));
        release_download(&state, "b");
        assert!(!download_in_flight(&state));
    }

    #[test]
    fn cancel_download_cancels_only_the_keyed_token_and_tolerates_idle() {
        let state = DownloadState::default();
        // No such key: cancelling is a harmless no-op.
        cancel_download(&state, "missing");
        let a = claim_download(&state, "a").unwrap();
        let b = claim_download(&state, "b").unwrap();
        cancel_download(&state, "a");
        assert!(a.is_cancelled());
        // Cancelling one download leaves the others running.
        assert!(!b.is_cancelled());
    }

    #[test]
    fn poisoned_download_state_surfaces_errors_and_tolerates_best_effort_ops() {
        let state = DownloadState::default();
        let state_ref = std::panic::AssertUnwindSafe(&state.0);
        let _ = std::panic::catch_unwind(|| {
            let _guard = state_ref.0.lock().unwrap();
            panic!("poison");
        });
        assert!(claim_download(&state, "k").is_err());
        let (_dir, store) = make_store();
        assert!(discard_partial_inner(&state, &store, &"a".repeat(64)).is_err());
        let conn = crate::database::open_in_memory().unwrap();
        assert!(delete_installed_model_inner(&state, &conn, &store, "x:y.gguf", "").is_err());
        // Best-effort operations must not panic on the poisoned lock.
        cancel_download(&state, "k");
        release_download(&state, "k");
    }

    #[test]
    fn adopt_as_builtin_model_only_for_the_first_model() {
        // No model selected yet: the first completed download is adopted.
        assert!(adopt_as_builtin_model(""));
        // A model is already active: a later parallel completion does not steal
        // the active slot.
        assert!(!adopt_as_builtin_model("google/gemma:gemma-q4.gguf"));
    }

    #[test]
    fn finalize_outcome_event_maps_ok_to_all_done_and_err_to_failed() {
        assert_eq!(
            finalize_outcome_event(Ok(())),
            download::DownloadEvent::AllDone
        );
        assert_eq!(
            finalize_outcome_event(Err("disk full".to_string())),
            download::DownloadEvent::Failed {
                kind: download::DownloadFailKind::Other,
                message: "disk full".to_string(),
            }
        );
    }

    // ── Model library: repo id validation ────────────────────────────────────

    #[test]
    fn repo_id_validation_accepts_two_clean_segments_only() {
        assert!(is_valid_repo_id("ggml-org/gemma-3-4b-it-GGUF"));
        assert!(is_valid_repo_id("bartowski/phi-4-GGUF"));
        assert!(is_valid_repo_id("a_b.c-d/e.f_g-h"));
        assert!(!is_valid_repo_id(""));
        assert!(!is_valid_repo_id("no-slash"));
        assert!(!is_valid_repo_id("a/b/c"));
        assert!(!is_valid_repo_id("/name"));
        assert!(!is_valid_repo_id("org/"));
        assert!(!is_valid_repo_id("org/na me"));
        assert!(!is_valid_repo_id("org/$(whoami)"));
        assert!(!is_valid_repo_id("org/name?x=1"));
        assert!(!is_valid_repo_id("örg/name"));
        // dot and dotdot segments are path-traversal risks; reject them
        assert!(!is_valid_repo_id("org/.."));
        assert!(!is_valid_repo_id("../repo"));
        assert!(!is_valid_repo_id("org/."));
        assert!(!is_valid_repo_id("./repo"));
    }

    // ── Model library: quant extraction ──────────────────────────────────────

    #[test]
    fn quant_from_filename_variants() {
        assert_eq!(quant_from_filename("phi-4-Q4_K_M.gguf"), "Q4_K_M");
        assert_eq!(quant_from_filename("gemma-3-4b-it-Q4_K_M.gguf"), "Q4_K_M");
        assert_eq!(quant_from_filename("model.Q8_0.gguf"), "Q8_0");
        assert_eq!(quant_from_filename("model-IQ4_XS.gguf"), "IQ4_XS");
        assert_eq!(quant_from_filename("model-f16.gguf"), "");
        assert_eq!(quant_from_filename("model-q4_k_m.gguf"), "");
        assert_eq!(quant_from_filename("no-extension-Q4_0"), "Q4_0");
        assert_eq!(quant_from_filename(""), "");
    }

    // ── Model library: HF listing parse ──────────────────────────────────────

    /// Canonical HF `/api/models/<repo>?blobs=true` fixture used across the
    /// resolve/listing tests. `c…` is the pinned commit; `a…`/`b…` are the
    /// weights and mmproj digests.
    fn hf_fixture() -> serde_json::Value {
        serde_json::json!({
            "sha": "c".repeat(40),
            "siblings": [
                {"rfilename": "README.md", "size": 10},
                {"rfilename": "model-Q4_K_M.gguf",
                 "lfs": {"sha256": "a".repeat(64), "size": 1000}},
                {"rfilename": "mmproj-model-f16.gguf",
                 "lfs": {"sha256": "b".repeat(64), "size": 200}},
                {"rfilename": "extra.gguf", "size": 7},
                {"rfilename": "bare.gguf"}
            ]
        })
    }

    #[test]
    fn parse_gguf_listing_filters_mmproj_and_non_gguf() {
        let body = hf_fixture().to_string();
        let files = parse_gguf_listing(body.as_bytes()).unwrap();
        assert_eq!(
            files,
            vec![
                HfGgufFile {
                    file: "model-Q4_K_M.gguf".to_string(),
                    size_bytes: 1000,
                    sha256: "a".repeat(64),
                    partial_bytes: None,
                },
                HfGgufFile {
                    file: "extra.gguf".to_string(),
                    size_bytes: 7,
                    sha256: String::new(),
                    partial_bytes: None,
                },
                HfGgufFile {
                    file: "bare.gguf".to_string(),
                    size_bytes: 0,
                    sha256: String::new(),
                    partial_bytes: None,
                },
            ]
        );
    }

    #[test]
    fn sanitize_context_length_trusts_only_sane_values() {
        assert_eq!(sanitize_context_length(None), None);
        assert_eq!(sanitize_context_length(Some(0)), None);
        assert_eq!(sanitize_context_length(Some(131_072)), Some(131_072));
        assert_eq!(
            sanitize_context_length(Some(MAX_MODEL_CONTEXT_LENGTH as u64)),
            Some(MAX_MODEL_CONTEXT_LENGTH)
        );
        assert_eq!(
            sanitize_context_length(Some(MAX_MODEL_CONTEXT_LENGTH as u64 + 1)),
            None
        );
    }

    #[test]
    fn attach_partials_reports_planted_and_skips_empty_sha() {
        let (_dir, store) = make_store();
        let sha = "a".repeat(64);
        // Plant a 9-byte partial for the LFS-backed row.
        let path = store.partial_path(&sha);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [0u8; 9]).unwrap();

        let rows = vec![
            HfGgufFileRow {
                file: HfGgufFile {
                    file: "weights.gguf".to_string(),
                    size_bytes: 100,
                    sha256: sha.clone(),
                    partial_bytes: None,
                },
                fit: None,
                installed: false,
            },
            HfGgufFileRow {
                file: HfGgufFile {
                    file: "no-lfs.gguf".to_string(),
                    size_bytes: 50,
                    sha256: String::new(),
                    partial_bytes: None,
                },
                fit: None,
                installed: false,
            },
        ];
        let out = attach_partials(rows, &store);
        // The LFS-backed row reflects the planted partial; the empty-sha row is
        // skipped entirely.
        assert_eq!(out[0].file.partial_bytes, Some(9));
        assert_eq!(out[1].file.partial_bytes, None);
    }

    #[test]
    fn attach_installed_marks_only_manifest_rows() {
        let conn = crate::database::open_in_memory().unwrap();
        // Record one of the two files in the manifest under "<repo>:<file>".
        manifest::insert(&conn, &manifest_row("org/repo:in.gguf", false, false)).unwrap();

        let row = |name: &str| HfGgufFileRow {
            file: HfGgufFile {
                file: name.to_string(),
                size_bytes: 100,
                sha256: String::new(),
                partial_bytes: None,
            },
            fit: None,
            installed: false,
        };
        let out = attach_installed(vec![row("in.gguf"), row("out.gguf")], "org/repo", &conn);

        // Only the file recorded in the manifest is marked installed.
        assert!(out[0].installed);
        assert!(!out[1].installed);
    }

    #[test]
    fn parse_gguf_listing_rejects_invalid_json() {
        let err = parse_gguf_listing(b"not json").unwrap_err();
        assert!(err.contains("failed to decode"), "got: {err}");
    }

    #[test]
    fn hf_gguf_file_serializes_for_frontend() {
        let v = serde_json::to_value(HfGgufFile {
            file: "x.gguf".to_string(),
            size_bytes: 5,
            sha256: "a".repeat(64),
            partial_bytes: Some(3),
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "file": "x.gguf",
                "size_bytes": 5,
                "sha256": "a".repeat(64),
                "partial_bytes": 3,
            })
        );
    }

    // ── Model library: resolve_listing (pure) ───────────────────────────────

    #[test]
    fn resolve_listing_extracts_weights_revision_and_mmproj() {
        let body = hf_fixture().to_string();
        let r = resolve_listing(body.as_bytes(), "model-Q4_K_M.gguf").unwrap();
        assert_eq!(r.revision, "c".repeat(40));
        assert_eq!(r.weights_sha256, "a".repeat(64));
        assert_eq!(r.weights_size_bytes, 1000);
        let mm = r.mmproj.unwrap();
        assert_eq!(mm.file, "mmproj-model-f16.gguf");
        assert_eq!(mm.sha256, "b".repeat(64));
        assert_eq!(mm.size_bytes, 200);
    }

    #[test]
    fn resolve_listing_rejects_invalid_json() {
        let err = resolve_listing(b"not json", "f.gguf").unwrap_err();
        assert!(err.contains("failed to decode"), "got: {err}");
    }

    #[test]
    fn resolve_listing_errors_when_file_missing() {
        let body = hf_fixture().to_string();
        let err = resolve_listing(body.as_bytes(), "nope.gguf").unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn resolve_listing_errors_when_file_has_no_lfs_digest() {
        let body = hf_fixture().to_string();
        // `extra.gguf` exists but carries no lfs block.
        let err = resolve_listing(body.as_bytes(), "extra.gguf").unwrap_err();
        assert!(err.contains("LFS"), "got: {err}");
    }

    #[test]
    fn resolve_listing_errors_on_missing_or_malformed_revision() {
        for sha in [serde_json::Value::Null, serde_json::json!("main")] {
            let mut fixture = hf_fixture();
            fixture["sha"] = sha;
            let body = fixture.to_string();
            let err = resolve_listing(body.as_bytes(), "model-Q4_K_M.gguf").unwrap_err();
            assert!(err.contains("commit"), "got: {err}");
        }
    }

    #[test]
    fn resolve_listing_skips_mmproj_without_lfs_and_non_gguf_mmproj() {
        let body = serde_json::json!({
            "sha": "c".repeat(40),
            "siblings": [
                {"rfilename": "w.gguf", "lfs": {"sha256": "a".repeat(64), "size": 9}},
                {"rfilename": "mmproj-no-lfs.gguf", "size": 5},
                {"rfilename": "mmproj-wrong-ext.bin",
                 "lfs": {"sha256": "b".repeat(64), "size": 5}}
            ]
        })
        .to_string();
        let r = resolve_listing(body.as_bytes(), "w.gguf").unwrap();
        assert_eq!(r.mmproj, None);
    }

    #[test]
    fn resolve_listing_errors_when_lfs_lacks_sha256() {
        let body = serde_json::json!({
            "sha": "c".repeat(40),
            "siblings": [
                {"rfilename": "w.gguf", "lfs": {"size": 9}}
            ]
        })
        .to_string();
        let err = resolve_listing(body.as_bytes(), "w.gguf").unwrap_err();
        assert!(err.contains("LFS"), "got: {err}");
    }

    // ── Model library: resolve_repo_spec (HTTP) ──────────────────────────────

    #[tokio::test]
    async fn resolve_repo_spec_finds_file_and_mmproj() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/models/test-org/test-repo?blobs=true")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(hf_fixture().to_string())
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let r = resolve_repo_spec(
            &client,
            &server.url(),
            "test-org/test-repo",
            "model-Q4_K_M.gguf",
        )
        .await
        .unwrap();

        mock.assert_async().await;
        assert_eq!(r.revision, "c".repeat(40));
        assert_eq!(r.weights_sha256, "a".repeat(64));
        assert!(r.mmproj.is_some());
    }

    #[tokio::test]
    async fn resolve_repo_spec_missing_file_errors() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/api/models/test-org/test-repo?blobs=true")
            .with_status(200)
            .with_body(hf_fixture().to_string())
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let err = resolve_repo_spec(&client, &server.url(), "test-org/test-repo", "nope.gguf")
            .await
            .unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_repo_spec_rejects_bad_repo_id() {
        // Validation fires before any network work: the bogus base URL would
        // fail loudly if a request were issued.
        let client = reqwest::Client::new();
        let err = resolve_repo_spec(&client, "http://127.0.0.1:9", "no-slash", "w.gguf")
            .await
            .unwrap_err();
        assert!(err.contains("repo id"), "got: {err}");
    }

    // ── Model library: HF fetch failure modes ────────────────────────────────

    #[tokio::test]
    async fn hf_fetch_maps_http_error_to_err_string() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/api/models/o/r?blobs=true")
            .with_status(500)
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let err = fetch_hf_repo_listing(&client, &server.url(), "o/r")
            .await
            .unwrap_err();
        assert!(err.contains("500"), "got: {err}");
    }

    #[tokio::test]
    async fn hf_fetch_maps_transport_error_to_err_string() {
        let client = reqwest::Client::new();
        let err = fetch_hf_repo_listing(&client, "http://127.0.0.1:1", "o/r")
            .await
            .unwrap_err();
        assert!(err.contains("failed to reach Hugging Face"), "got: {err}");
    }

    #[tokio::test]
    async fn hf_fetch_rejects_body_exceeding_size_cap_via_content_length() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/api/models/o/r?blobs=true")
            .with_status(200)
            .with_body("x".repeat(100))
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let err = fetch_hf_repo_listing_inner(
            &client,
            &server.url(),
            "o/r",
            std::time::Duration::from_secs(5),
            32,
        )
        .await
        .unwrap_err();
        assert!(err.contains("exceeded"), "got: {err}");
    }

    #[tokio::test]
    async fn hf_fetch_rejects_body_exceeding_size_cap_when_no_content_length() {
        // Chunked-encoding response (no Content-Length); the incremental
        // stream cap must reject when the running total exceeds the limit.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut request_buf = [0u8; 1024];
            let _ = conn.read(&mut request_buf);
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
        let err = fetch_hf_repo_listing_inner(
            &client,
            &base,
            "o/r",
            std::time::Duration::from_secs(5),
            20,
        )
        .await
        .unwrap_err();
        assert!(err.contains("exceeded"), "got: {err}");
    }

    #[tokio::test]
    async fn hf_fetch_maps_body_read_error_to_err_string() {
        // Headers promise 100 body bytes, then the server hangs up.
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
        let err = fetch_hf_repo_listing(&client, &base, "o/r")
            .await
            .unwrap_err();
        assert!(
            err.contains("failed to read Hugging Face API body"),
            "got: {err}"
        );
    }

    // ── Model library: repo listing wrapper ──────────────────────────────────

    #[tokio::test]
    async fn fetch_repo_gguf_listing_validates_then_lists() {
        let client = reqwest::Client::new();
        // Invalid repo id: rejected before any network work.
        let err = fetch_repo_gguf_listing(&client, "http://127.0.0.1:9", "no-slash")
            .await
            .unwrap_err();
        assert!(err.contains("repo id"), "got: {err}");

        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/api/models/o/r?blobs=true")
            .with_status(200)
            .with_body(hf_fixture().to_string())
            .create_async()
            .await;
        let files = fetch_repo_gguf_listing(&client, &server.url(), "o/r")
            .await
            .unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].file, "model-Q4_K_M.gguf");
    }

    // ── Model library: Hugging Face search ───────────────────────────────────

    /// Search fixture exercising the capability derivation and the pipeline
    /// allowlist: each `gated` shape (bool, strategy string, absent, null),
    /// vision from an mmproj sibling, thinking from the chat template (template
    /// class wins over a reasoning-y name) with a name fallback when no template
    /// is present, a non-chat pipeline that is dropped, an untagged repo that is
    /// dropped, and an empty-id row that is dropped.
    fn search_fixture() -> serde_json::Value {
        serde_json::json!([
            // alpha: chat model that ships an mmproj companion and an optional
            // (`enable_thinking`) template -> vision + thinking, with context.
            {"id": "org/alpha-GGUF", "downloads": 1000, "gated": false,
             "pipeline_tag": "text-generation",
             "gguf": {"context_length": 131072,
                      "chat_template": "{%- if enable_thinking %}<think>{% endif %}",
                      "architecture": "qwen3"},
             "siblings": [{"rfilename": "alpha-Q4_K_M.gguf"},
                          {"rfilename": "mmproj-f16.gguf"}]},
            // beta: a multimodal pipeline tag is allowlisted; no mmproj sibling
            // means no vision, and an always-on `<think>` template means thinking.
            {"id": "org/beta-GGUF", "downloads": 500, "gated": "manual",
             "pipeline_tag": "image-text-to-text",
             "gguf": {"chat_template": "<|im_start|>assistant\\n<think>\\n"},
             "siblings": [{"rfilename": "beta.gguf"}]},
            // gamma: no expanded gguf at all, so thinking falls back to the name
            // (`QwQ` is a known reasoning family); no mmproj means no vision.
            {"id": "org/QwQ-32B-GGUF", "downloads": 7,
             "pipeline_tag": "text-generation"},
            // delta: a non-chat pipeline (embeddings) is dropped by the allowlist
            // even though it is the most downloaded.
            {"id": "org/embed-GGUF", "downloads": 99999,
             "pipeline_tag": "feature-extraction"},
            // epsilon: a plain instruct template classifies as non-thinking and
            // overrides the reasoning-y repo name; its context is implausibly
            // large so it is dropped; no mmproj means no vision.
            {"id": "org/Reasoner-GGUF", "downloads": 2, "gated": null,
             "pipeline_tag": "text-generation",
             "gguf": {"context_length": 9000000000u64,
                      "chat_template": "<|user|>{{x}}<|assistant|>",
                      "architecture": "llama"},
             "siblings": [{"rfilename": "r.gguf"}]},
            // zeta: no pipeline tag at all is dropped (the allowlist requires an
            // explicit chat-capable tag).
            {"id": "org/untagged-GGUF", "downloads": 3},
            // empty id is dropped.
            {"id": "", "downloads": 9, "pipeline_tag": "text-generation"}
        ])
    }

    #[test]
    fn parse_search_results_maps_capabilities_and_drops_non_chat_rows() {
        let body = search_fixture().to_string();
        // A generous limit keeps `has_more` false so this case stays about rows.
        let page = parse_search_results(body.as_bytes(), 100).unwrap();
        assert!(!page.has_more);
        assert_eq!(
            page.rows,
            vec![
                HfModelSummary {
                    id: "org/alpha-GGUF".to_string(),
                    downloads: 1000,
                    gated: false,
                    context_length: Some(131072),
                    vision: true,
                    thinking: true,
                },
                HfModelSummary {
                    id: "org/beta-GGUF".to_string(),
                    downloads: 500,
                    gated: true,
                    context_length: None,
                    vision: false,
                    thinking: true,
                },
                // gamma: thinking healed from the `QwQ` name when no template ships.
                HfModelSummary {
                    id: "org/QwQ-32B-GGUF".to_string(),
                    downloads: 7,
                    gated: false,
                    context_length: None,
                    vision: false,
                    thinking: true,
                },
                // epsilon: the plain template wins over the reasoning-y name.
                HfModelSummary {
                    id: "org/Reasoner-GGUF".to_string(),
                    downloads: 2,
                    gated: false,
                    context_length: None,
                    vision: false,
                    thinking: false,
                },
            ]
        );
    }

    #[test]
    fn parse_search_results_flags_has_more_when_the_page_is_full() {
        let body = serde_json::json!([
            {"id": "org/a-GGUF", "downloads": 2, "pipeline_tag": "text-generation"},
            {"id": "org/b-GGUF", "downloads": 1, "pipeline_tag": "text-generation"}
        ])
        .to_string();
        // Two raw entries: a page of two is full (more may exist on the Hub)...
        assert!(parse_search_results(body.as_bytes(), 2).unwrap().has_more);
        // ...but a page asking for three was not filled, so the Hub is exhausted.
        assert!(!parse_search_results(body.as_bytes(), 3).unwrap().has_more);
    }

    #[test]
    fn parse_search_results_rejects_invalid_json() {
        let err = parse_search_results(b"not json", 30).unwrap_err();
        assert!(err.contains("failed to decode"), "got: {err}");
    }

    #[test]
    fn hf_model_summary_serializes_snake_case() {
        let v = serde_json::to_value(HfModelSummary {
            id: "o/r".to_string(),
            downloads: 7,
            gated: true,
            context_length: Some(131072),
            vision: true,
            thinking: false,
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "id": "o/r", "downloads": 7, "gated": true, "context_length": 131072,
                "vision": true, "thinking": false,
            })
        );
    }

    #[test]
    fn hf_search_page_serializes_snake_case() {
        let v = serde_json::to_value(HfSearchPage {
            rows: vec![],
            has_more: true,
        })
        .unwrap();
        assert_eq!(v, serde_json::json!({ "rows": [], "has_more": true }));
    }

    // ── RAM-fit estimation + annotated views ─────────────────────────────────

    #[test]
    fn estimate_runtime_gb_from_bytes_adds_overhead() {
        // 1 GiB weights + 2.0 overhead.
        assert!((estimate_runtime_gb_from_bytes(1 << 30) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn clamp_search_limit_bounds_the_page_size() {
        assert_eq!(clamp_search_limit(0), 1);
        assert_eq!(clamp_search_limit(50), 50);
        assert_eq!(clamp_search_limit(10_000), HF_SEARCH_LIMIT_MAX);
    }

    #[test]
    fn annotate_gguf_rows_uses_real_sizes() {
        let files = vec![
            HfGgufFile {
                file: "a.gguf".to_string(),
                size_bytes: 1 << 30,
                sha256: String::new(),
                partial_bytes: None,
            },
            HfGgufFile {
                file: "b.gguf".to_string(),
                size_bytes: 0,
                sha256: String::new(),
                partial_bytes: None,
            },
        ];
        let rows = annotate_gguf_rows(files.clone(), 64 << 30);
        assert_eq!(rows[0].fit, Some(registry::RamFit::Fits));
        // A zero size cannot be judged.
        assert_eq!(rows[1].fit, None);
        // Unknown host RAM drops every verdict.
        let rows = annotate_gguf_rows(files, 0);
        assert_eq!(rows[0].fit, None);
    }

    #[test]
    fn build_installed_views_annotates_fit() {
        let model = manifest::InstalledModel {
            id: "org/Repo:weights.gguf".to_string(),
            display_name: "Repo".to_string(),
            repo: "org/Repo".to_string(),
            revision: "0".repeat(40),
            file_name: "weights.gguf".to_string(),
            sha256: "a".repeat(64),
            size_bytes: 1 << 30,
            quant: "Q4_K_M".to_string(),
            vision: false,
            thinking: false,
            reasoning_always: false,
            mmproj_file: None,
            mmproj_sha256: None,
        };
        let views = build_installed_views(vec![model.clone()], 64 << 30);
        assert_eq!(views[0].fit, Some(registry::RamFit::Fits));
        // A pasted repo has no registry entry, so its context window, vision
        // projector size, and maker are all unknown.
        assert_eq!(views[0].context_length, None);
        assert_eq!(views[0].mmproj_bytes, 0);
        assert_eq!(views[0].origin, None);
        // Unknown host RAM drops the verdict.
        let views = build_installed_views(vec![model], 0);
        assert_eq!(views[0].fit, None);

        // A curated model heals its context window, projector size, and maker
        // from the registry.
        let curated = registry::to_installed_model(&registry::STARTERS[0]);
        let views = build_installed_views(vec![curated], 64 << 30);
        assert_eq!(
            views[0].context_length,
            Some(registry::STARTERS[0].context_length)
        );
        assert_eq!(views[0].mmproj_bytes, registry::STARTERS[0].mmproj_bytes);
        assert_eq!(
            views[0].origin,
            Some(registry::STARTERS[0].origin.to_string())
        );
    }

    #[test]
    fn gguf_file_row_serializes_with_flattened_base_and_fit() {
        let file_row = HfGgufFileRow {
            file: HfGgufFile {
                file: "w.gguf".to_string(),
                size_bytes: 42,
                sha256: String::new(),
                partial_bytes: None,
            },
            fit: None,
            installed: false,
        };
        assert_eq!(
            serde_json::to_value(file_row).unwrap(),
            serde_json::json!({
                "file": "w.gguf",
                "size_bytes": 42,
                "sha256": "",
                "partial_bytes": serde_json::Value::Null,
                "fit": serde_json::Value::Null,
                "installed": false,
            })
        );
    }

    #[tokio::test]
    async fn fetch_hf_search_returns_rows_and_sends_widened_query() {
        let mut server = mockito::Server::new_async().await;
        // The query no longer pins `pipeline_tag=text-generation` (that excluded
        // multimodal `image-text-to-text` repos); chat-vs-non-chat is now an
        // allowlist applied to each row's expanded `pipeline_tag`. The expand set
        // carries the gguf block (context + chat template), the file list (mmproj
        // -> vision), and the pipeline tag (the allowlist) in one request.
        let mock = server
            .mock("GET", "/api/models")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("filter".into(), "gguf".into()),
                mockito::Matcher::UrlEncoded("search".into(), "qwen".into()),
                mockito::Matcher::UrlEncoded("sort".into(), "downloads".into()),
                mockito::Matcher::UrlEncoded("limit".into(), "60".into()),
                // The widened query expands the gguf block, the file list, and
                // the pipeline tag, and (critically) no longer pins
                // `pipeline_tag=text-generation`, which had hidden vision repos.
                mockito::Matcher::Regex("expand%5B%5D=gguf".into()),
                mockito::Matcher::Regex("expand%5B%5D=siblings".into()),
                mockito::Matcher::Regex("expand%5B%5D=pipeline_tag".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(search_fixture().to_string())
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let page = fetch_hf_search(&client, &server.url(), "qwen", 60)
            .await
            .unwrap();
        mock.assert_async().await;
        // Four chat rows survive the allowlist; the multimodal beta row proves
        // the widened query surfaces a repo the old filter would have dropped.
        assert_eq!(page.rows.len(), 4);
        assert_eq!(page.rows[0].id, "org/alpha-GGUF");
        assert!(page.rows.iter().any(|r| r.id == "org/beta-GGUF"));
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn fetch_hf_search_omits_blank_query() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/api/models")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body("[]")
            .create_async()
            .await;
        let client = reqwest::Client::new();
        // Whitespace-only query trims to empty and the search param is dropped.
        let page = fetch_hf_search(
            &client,
            &server.url(),
            "   ",
            crate::config::defaults::HF_SEARCH_LIMIT,
        )
        .await
        .unwrap();
        assert!(page.rows.is_empty());
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn fetch_hf_search_maps_http_error() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/api/models")
            .match_query(mockito::Matcher::Any)
            .with_status(503)
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let err = fetch_hf_search(
            &client,
            &server.url(),
            "q",
            crate::config::defaults::HF_SEARCH_LIMIT,
        )
        .await
        .unwrap_err();
        assert!(err.contains("503"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_hf_search_maps_transport_error() {
        let client = reqwest::Client::new();
        let err = fetch_hf_search(
            &client,
            "http://127.0.0.1:1",
            "q",
            crate::config::defaults::HF_SEARCH_LIMIT,
        )
        .await
        .unwrap_err();
        assert!(err.contains("failed to reach Hugging Face"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_hf_search_rejects_overlong_query() {
        let client = reqwest::Client::new();
        let long = "x".repeat(crate::config::defaults::MAX_HF_SEARCH_QUERY_LEN + 1);
        let err = fetch_hf_search(
            &client,
            "http://127.0.0.1:9",
            &long,
            crate::config::defaults::HF_SEARCH_LIMIT,
        )
        .await
        .unwrap_err();
        assert!(err.contains("maximum length"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_hf_search_inner_rejects_body_over_cap_via_content_length() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/api/models")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body("x".repeat(100))
            .create_async()
            .await;
        let client = reqwest::Client::new();
        let err = fetch_hf_search_inner(
            &client,
            &server.url(),
            "q",
            std::time::Duration::from_secs(5),
            32,
            30,
        )
        .await
        .unwrap_err();
        assert!(err.contains("exceeded"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_hf_search_inner_rejects_body_over_cap_when_chunked() {
        // Chunked response (no Content-Length): the incremental cap must reject.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut request_buf = [0u8; 1024];
            let _ = conn.read(&mut request_buf);
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
        let err = fetch_hf_search_inner(
            &client,
            &base,
            "q",
            std::time::Duration::from_secs(5),
            20,
            30,
        )
        .await
        .unwrap_err();
        assert!(err.contains("exceeded"), "got: {err}");
    }

    #[tokio::test]
    async fn fetch_hf_search_inner_maps_body_read_error() {
        // Headers promise 100 body bytes, then the server hangs up.
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
        let err = fetch_hf_search_inner(
            &client,
            &base,
            "q",
            std::time::Duration::from_secs(5),
            4 * 1024 * 1024,
            30,
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("failed to read Hugging Face search body"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn fetch_hf_search_inner_rejects_unparseable_base_url() {
        let client = reqwest::Client::new();
        let err = fetch_hf_search_inner(
            &client,
            "not a url",
            "q",
            std::time::Duration::from_secs(5),
            4 * 1024 * 1024,
            30,
        )
        .await
        .unwrap_err();
        assert!(err.contains("failed to build"), "got: {err}");
    }

    // ── Model library: repo spec/model mapping ───────────────────────────────

    fn sample_resolved(with_mmproj: bool) -> RepoResolved {
        RepoResolved {
            revision: "c".repeat(40),
            weights_sha256: "a".repeat(64),
            weights_size_bytes: 1000,
            mmproj: with_mmproj.then(|| MmprojCompanion {
                file: "mmproj-model-f16.gguf".to_string(),
                sha256: "b".repeat(64),
                size_bytes: 200,
            }),
        }
    }

    #[test]
    fn repo_download_specs_builds_urls_and_optional_mmproj() {
        let r = sample_resolved(true);
        let specs = repo_download_specs("https://huggingface.co/", "o/r", "w-Q4_K_M.gguf", &r);
        assert_eq!(specs.len(), 2);
        assert_eq!(
            specs[0].url,
            format!(
                "https://huggingface.co/o/r/resolve/{}/w-Q4_K_M.gguf",
                r.revision
            )
        );
        assert_eq!(specs[0].file, "w-Q4_K_M.gguf");
        assert_eq!(specs[0].sha256, r.weights_sha256);
        assert_eq!(specs[0].total_bytes, 1000);
        assert_eq!(
            specs[1].url,
            format!(
                "https://huggingface.co/o/r/resolve/{}/mmproj-model-f16.gguf",
                r.revision
            )
        );
        assert_eq!(specs[1].sha256, "b".repeat(64));
        assert_eq!(specs[1].total_bytes, 200);

        let text_only = sample_resolved(false);
        let specs = repo_download_specs("https://huggingface.co", "o/r", "w.gguf", &text_only);
        assert_eq!(specs.len(), 1);
    }

    #[test]
    fn repo_installed_model_maps_fields() {
        let r = sample_resolved(true);
        let m = repo_installed_model("o/r", "w-Q4_K_M.gguf", &r);
        assert_eq!(m.id, "o/r:w-Q4_K_M.gguf");
        assert_eq!(m.display_name, "w-Q4_K_M");
        assert_eq!(m.repo, "o/r");
        assert_eq!(m.revision, r.revision);
        assert_eq!(m.file_name, "w-Q4_K_M.gguf");
        assert_eq!(m.sha256, r.weights_sha256);
        assert_eq!(m.size_bytes, 1000);
        assert_eq!(m.quant, "Q4_K_M");
        assert!(m.vision);
        assert!(!m.thinking);
        // Pasted rows record placeholder reasoning flags; the real class is
        // resolved from the GGUF in finalize_install.
        assert!(!m.reasoning_always);
        assert_eq!(m.mmproj_file.as_deref(), Some("mmproj-model-f16.gguf"));
        assert_eq!(m.mmproj_sha256.as_deref(), Some(&*"b".repeat(64)));

        let text_only = sample_resolved(false);
        let m = repo_installed_model("o/r", "w.gguf", &text_only);
        assert!(!m.vision);
        assert_eq!(m.mmproj_file, None);
        assert_eq!(m.mmproj_sha256, None);
    }

    // ── Capability detection: thinking heuristic ─────────────────────────────

    #[test]
    fn detect_thinking_matches_reasoning_self_labels() {
        // A repo or file whose own name advertises reasoning.
        assert!(detect_thinking("acme/Model-Thinking", "model.gguf"));
        assert!(detect_thinking("acme/model", "model-reasoning-Q4_K_M.gguf"));
        assert!(detect_thinking("acme/reasoner-7b", "w.gguf"));
    }

    #[test]
    fn detect_thinking_matches_known_reasoning_families() {
        assert!(detect_thinking("deepseek-ai/DeepSeek-R1-GGUF", "x.gguf"));
        assert!(detect_thinking("org/QwQ-32B-GGUF", "x.gguf"));
        assert!(detect_thinking("ggml-org/gpt-oss-20b-GGUF", "x.gguf"));
        assert!(detect_thinking("mistralai/Magistral-Small-GGUF", "x.gguf"));
    }

    #[test]
    fn detect_thinking_is_case_insensitive() {
        assert!(detect_thinking("ORG/GPT-OSS-20B", "MODEL.GGUF"));
    }

    #[test]
    fn detect_thinking_defaults_false_without_markers() {
        assert!(!detect_thinking(
            "google/gemma-4-12b-it",
            "gemma-4-12b-it-Q4_K_M.gguf"
        ));
        assert!(!detect_thinking("o/r", "w-Q4_K_M.gguf"));
    }

    #[test]
    fn repo_installed_model_flags_thinking_from_name() {
        let m = repo_installed_model(
            "ggml-org/gpt-oss-20b-GGUF",
            "gpt-oss-20b-Q4_K_M.gguf",
            &sample_resolved(false),
        );
        assert!(m.thinking);
    }

    // ── Reasoning-flag resolution helpers ────────────────────────────────────

    #[test]
    fn curated_reasoning_flags_match_every_starter() {
        for s in registry::STARTERS {
            assert_eq!(
                curated_reasoning_flags(s.repo, s.file_name),
                Some((s.thinking, s.reasoning_always)),
                "curated flags must mirror the registry for {}",
                s.repo
            );
        }
    }

    #[test]
    fn curated_reasoning_flags_none_for_pasted_repo() {
        assert_eq!(curated_reasoning_flags("nope/repo", "x.gguf"), None);
    }

    #[test]
    fn reasoning_flags_from_metadata_classify_from_template() {
        // Optional family: thinking on, no badge.
        assert_eq!(
            reasoning_flags_from_metadata(
                Some("{% if enable_thinking %}"),
                Some("qwen3"),
                "any/repo",
                "x.gguf"
            ),
            (true, false)
        );
        // Always family: thinking on, badge.
        assert_eq!(
            reasoning_flags_from_metadata(Some("<think>"), None, "any/repo", "x.gguf"),
            (true, true)
        );
        // Non-reasoning: both off.
        assert_eq!(
            reasoning_flags_from_metadata(Some("plain instruct"), None, "any/repo", "x.gguf"),
            (false, false)
        );
        // A readable template wins over a reasoning-y name.
        assert_eq!(
            reasoning_flags_from_metadata(Some("plain instruct"), None, "org/QwQ-32B", "x.gguf"),
            (false, false)
        );
    }

    #[test]
    fn reasoning_flags_from_metadata_falls_back_to_name_without_template() {
        // No template: the name decides thinking; `reasoning_always` stays off
        // for the runtime backstop. Marker in the repo, then in the file name.
        assert_eq!(
            reasoning_flags_from_metadata(None, None, "org/QwQ-32B", "x.gguf"),
            (true, false)
        );
        assert_eq!(
            reasoning_flags_from_metadata(None, None, "org/plain", "model-reasoning.gguf"),
            (true, false)
        );
        // No template and no marker: both off.
        assert_eq!(
            reasoning_flags_from_metadata(None, None, "org/plain", "model.gguf"),
            (false, false)
        );
        // An empty template is treated as no template and falls back to the name.
        assert_eq!(
            reasoning_flags_from_metadata(Some(""), None, "org/QwQ-32B", "x.gguf"),
            (true, false)
        );
    }

    // ── Model library: delete ────────────────────────────────────────────────

    #[test]
    fn delete_installed_model_inner_removes_orphans_and_flags_builtin_clear() {
        let conn = crate::database::open_in_memory().unwrap();
        let (_dir, store) = make_store();
        let state = DownloadState::default();

        let r = sample_resolved(true);
        let m = repo_installed_model("o/r", "w-Q4_K_M.gguf", &r);
        manifest::insert(&conn, &m).unwrap();
        std::fs::write(store.blob_path(&m.sha256), b"w").unwrap();
        std::fs::write(store.blob_path(m.mmproj_sha256.as_ref().unwrap()), b"m").unwrap();

        // The builtin provider currently points at this model: deletion must
        // flag the clear so the wrapper resets the provider's model field.
        let out = delete_installed_model_inner(&state, &conn, &store, &m.id, &m.id).unwrap();
        assert!(out.clear_builtin);
        assert!(!store.blob_path(&m.sha256).exists());
        assert!(!store.blob_path(m.mmproj_sha256.as_ref().unwrap()).exists());
        assert!(manifest::get(&conn, &m.id).unwrap().is_none());

        // Builtin points elsewhere: no clear.
        let m2 = repo_installed_model("o/r2", "x.gguf", &sample_resolved(false));
        manifest::insert(&conn, &m2).unwrap();
        std::fs::write(store.blob_path(&m2.sha256), b"x").unwrap();
        let out = delete_installed_model_inner(&state, &conn, &store, &m2.id, "other:model.gguf")
            .unwrap();
        assert!(!out.clear_builtin);
    }

    #[test]
    fn delete_installed_model_inner_refuses_while_download_in_flight() {
        let conn = crate::database::open_in_memory().unwrap();
        let (_dir, store) = make_store();
        let state = DownloadState::default();

        let m = repo_installed_model("o/r", "w.gguf", &sample_resolved(false));
        manifest::insert(&conn, &m).unwrap();
        std::fs::write(store.blob_path(&m.sha256), b"w").unwrap();

        // A claimed download slot must refuse the delete and leave the row
        // and blob untouched, even though the in-flight download is a different
        // model: a finishing download could insert or share refcounted blobs.
        let _token = claim_download(&state, "other-model").unwrap();
        let err = delete_installed_model_inner(&state, &conn, &store, &m.id, "").unwrap_err();
        assert_eq!(err, "a download is already in progress");
        assert!(manifest::get(&conn, &m.id).unwrap().is_some());
        assert!(store.blob_path(&m.sha256).exists());

        // Releasing the slot lets the delete proceed.
        release_download(&state, "other-model");
        assert!(delete_installed_model_inner(&state, &conn, &store, &m.id, "").is_ok());
    }

    #[test]
    fn delete_installed_model_inner_propagates_sql_and_io_errors() {
        let state = DownloadState::default();
        // SQL failure: table dropped.
        let conn = crate::database::open_in_memory().unwrap();
        conn.execute_batch("DROP TABLE installed_models;").unwrap();
        let (_dir, store) = make_store();
        assert!(delete_installed_model_inner(&state, &conn, &store, "x:y.gguf", "").is_err());

        // I/O failure: a directory sits where the orphaned blob should be.
        let conn = crate::database::open_in_memory().unwrap();
        let m = repo_installed_model("o/r", "w.gguf", &sample_resolved(false));
        manifest::insert(&conn, &m).unwrap();
        std::fs::create_dir_all(store.blob_path(&m.sha256)).unwrap();
        assert!(delete_installed_model_inner(&state, &conn, &store, &m.id, "").is_err());
    }

    // ── Model library: discard partial ───────────────────────────────────────

    #[test]
    fn discard_partial_validates_hex_and_running_state() {
        let (_dir, store) = make_store();
        let state = DownloadState::default();
        let sha = "a".repeat(64);

        // Invalid digest shapes are rejected before any filesystem use.
        assert!(discard_partial_inner(&state, &store, "short").is_err());
        assert!(discard_partial_inner(&state, &store, &"Z".repeat(64)).is_err());

        // Rejected while any download is claimed (a finishing download may be
        // writing this very partial or about to share its blob).
        let _token = claim_download(&state, "some-model").unwrap();
        let err = discard_partial_inner(&state, &store, &sha).unwrap_err();
        assert!(err.contains("in progress"), "got: {err}");
        release_download(&state, "some-model");

        // Removes an existing partial; a missing partial is fine (idempotent).
        std::fs::write(store.partial_path(&sha), b"bytes").unwrap();
        discard_partial_inner(&state, &store, &sha).unwrap();
        assert!(!store.partial_path(&sha).exists());
        discard_partial_inner(&state, &store, &sha).unwrap();
    }

    #[test]
    fn discard_partial_propagates_unexpected_io_error() {
        let (_dir, store) = make_store();
        let state = DownloadState::default();
        let sha = "b".repeat(64);
        // A directory at the partial path makes remove_file fail with a
        // non-NotFound error which must be propagated.
        std::fs::create_dir_all(store.partial_path(&sha)).unwrap();
        assert!(discard_partial_inner(&state, &store, &sha).is_err());
    }

    // ── Model library: builtin provider model ───────────────────────────────

    #[test]
    fn builtin_provider_model_reads_builtin_entry() {
        let mut cfg = AppConfig::default();
        assert_eq!(builtin_provider_model(&cfg), "");
        for p in &mut cfg.inference.providers {
            if p.id == crate::config::defaults::PROVIDER_ID_BUILTIN {
                p.model = "o/r:w.gguf".to_string();
            }
        }
        assert_eq!(builtin_provider_model(&cfg), "o/r:w.gguf");
        // No builtin entry at all: empty.
        cfg.inference.providers.clear();
        assert_eq!(builtin_provider_model(&cfg), "");
    }

    // ── should_refresh_active_model ──────────────────────────────────────────

    /// Helper: an `AppConfig` whose single provider `id` is active with `model`.
    fn config_with_active_provider(id: &str, model: &str) -> AppConfig {
        use crate::config::schema::Provider;
        let mut cfg = AppConfig::default();
        cfg.inference.active_provider = id.to_string();
        cfg.inference.providers = vec![Provider {
            id: id.to_string(),
            kind: PROVIDER_KIND_BUILTIN.to_string(),
            label: "Test".to_string(),
            base_url: String::new(),
            model: model.to_string(),
            vision: false,
        }];
        cfg
    }

    #[test]
    fn should_refresh_active_model_mirrors_active_provider_write() {
        // Writing the active provider's model refreshes the mirror with the
        // resolved slug (the download-finished path).
        let cfg = config_with_active_provider("builtin", "o/r:w.gguf");
        assert_eq!(
            should_refresh_active_model("builtin", &cfg),
            Some(Some("o/r:w.gguf".to_string()))
        );
    }

    #[test]
    fn should_refresh_active_model_clears_mirror_on_empty_slug() {
        // The delete-model path writes "": the mirror must clear, not keep a
        // stale slug.
        let cfg = config_with_active_provider("builtin", "");
        assert_eq!(should_refresh_active_model("builtin", &cfg), Some(None));
    }

    #[test]
    fn should_refresh_active_model_ignores_non_active_provider() {
        // A write to a provider that is not active never touches the mirror;
        // it tracks the active provider only.
        let cfg = config_with_active_provider("ollama", "gemma3:12b");
        assert_eq!(should_refresh_active_model("builtin", &cfg), None);
    }

    // ── persist_provider_model_locked ────────────────────────────────────────

    /// On-disk providers config used by the serialized-persist tests:
    /// builtin + ollama, with ollama active.
    const LOCKED_PERSIST_CONFIG: &str = r#"
[inference]
active_provider = "ollama"

[[inference.providers]]
id = "builtin"
kind = "builtin"
label = "Built-in (Thuki)"
model = ""

[[inference.providers]]
id = "ollama"
kind = "ollama"
label = "Ollama"
base_url = "http://127.0.0.1:11434"
model = ""
"#;

    #[test]
    fn persist_provider_model_locked_composes_with_guarded_settings_writes() {
        // Three writers through the shared lock-then-read-modify-write
        // pattern: a background model persist, a Settings-UI style provider
        // patch, and a persist on the active provider. No write may be lost,
        // in memory or on disk.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, LOCKED_PERSIST_CONFIG).unwrap();
        let lock = parking_lot::RwLock::new(crate::config::load_from_path(&path).unwrap());

        // Writer 1: background persist (download-finalize path) on the
        // non-active builtin provider: no mirror refresh.
        let mirror =
            persist_provider_model_locked(&path, &lock, PROVIDER_ID_BUILTIN, "org/repo:w.gguf")
                .unwrap();
        assert_eq!(mirror, None);

        // Writer 2: a Settings-UI write through the same guard pattern the
        // settings commands use.
        {
            let mut guard = lock.write();
            let resolved = crate::settings_commands::write_provider_field_to_disk(
                &path,
                "ollama",
                "base_url",
                "http://127.0.0.1:9999",
            )
            .unwrap();
            *guard = resolved;
        }

        // Writer 3: persist on the ACTIVE provider: mirror refreshes.
        let mirror = persist_provider_model_locked(&path, &lock, "ollama", "gemma3:4b").unwrap();
        assert_eq!(mirror, Some(Some("gemma3:4b".to_string())));

        // Every writer's change survives in the shared in-memory config...
        let assert_composed = |cfg: &AppConfig| {
            let provider = |id: &str| cfg.inference.providers.iter().find(|p| p.id == id).unwrap();
            assert_eq!(provider(PROVIDER_ID_BUILTIN).model, "org/repo:w.gguf");
            assert_eq!(provider("ollama").base_url, "http://127.0.0.1:9999");
            assert_eq!(provider("ollama").model, "gemma3:4b");
        };
        assert_composed(&lock.read());
        // ...and in the file a fresh load resolves.
        assert_composed(&crate::config::load_from_path(&path).unwrap());
    }

    #[test]
    fn persist_provider_model_locked_propagates_write_error() {
        // An unknown provider id fails the disk patch; the in-memory config
        // must stay untouched.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, LOCKED_PERSIST_CONFIG).unwrap();
        let lock = parking_lot::RwLock::new(crate::config::load_from_path(&path).unwrap());
        let before = lock.read().clone();

        let err = persist_provider_model_locked(&path, &lock, "no-such-provider", "m").unwrap_err();
        assert!(err.contains("no-such-provider"));
        assert_eq!(*lock.read(), before);
    }

    // ── Model library: system RAM probe ──────────────────────────────────────

    #[test]
    fn system_ram_bytes_returns_positive_on_real_hardware() {
        assert!(system_ram_bytes() > 0);
    }

    #[tokio::test]
    async fn reconcile_keys_capabilities_by_provider() {
        // The same slug under two providers holds two distinct cache entries;
        // a reconcile scoped to one provider only sees that provider's entry.
        let cache = ModelCapabilitiesCache::default();
        cache.0.lock().unwrap().insert(
            ("ollama".to_string(), "shared:slug".to_string()),
            Capabilities {
                vision: true,
                ..Default::default()
            },
        );
        cache.0.lock().unwrap().insert(
            ("builtin".to_string(), "shared:slug".to_string()),
            Capabilities {
                thinking: true,
                ..Default::default()
            },
        );
        let client = reqwest::Client::new();
        let installed = vec!["shared:slug".to_string()];
        let ollama =
            reconcile_capabilities(&client, &cache, "ollama", "http://unused", &installed).await;
        let builtin =
            reconcile_capabilities(&client, &cache, "builtin", "http://unused", &installed).await;
        assert!(ollama["shared:slug"].vision && !ollama["shared:slug"].thinking);
        assert!(builtin["shared:slug"].thinking && !builtin["shared:slug"].vision);
    }
}
