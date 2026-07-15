//! Settings panel Tauri command surface.
//!
//! Implements the IPC contract used by the Settings window:
//!
//! - [`get_config`] — reads the current `AppConfig` snapshot from managed state.
//! - [`set_config_field`] — security-validated per-field write that round-trips
//!   through the loader so clamp / cross-field invariants always apply.
//! - [`reset_config`] — replaces one section (or the whole file) with the
//!   compiled defaults.
//! - [`reload_config_from_disk`] — re-reads the file (called on Settings
//!   window focus, replaces the file watcher subsystem the eng review collapsed).
//! - [`get_corrupt_marker`] — returns and consumes the recovery marker the
//!   loader wrote when a corrupt config file was renamed.
//! - [`reveal_config_in_finder`] — opens Finder with the config file selected.
//!
//! ## Security model
//!
//! `set_config_field` is the only frontend-callable surface that mutates user
//! configuration on disk. It:
//!
//! 1. Validates `(section, key)` against `defaults::ALLOWED_FIELDS`. Any pair
//!    not in the allowlist is rejected with a typed `UnknownField` error.
//!    This prevents the GUI from writing fields that do not exist or are
//!    intentionally not user-tunable (e.g. activation timing, vision limits).
//! 2. Coerces the inbound `serde_json::Value` to the TOML type already present
//!    in the on-disk file. Type drift (string for an integer field, etc.) is
//!    rejected with a typed `TypeMismatch` error rather than silently coerced.
//! 3. Round-trips through `loader::load_from_path` so the loader's clamp /
//!    empty-fallback / cross-field invariant rules apply identically to GUI
//!    edits and hand-edits. The loader is the single source of truth for what
//!    constitutes a valid `AppConfig`; the GUI cannot bypass it.
//!
//! Concurrency: every disk-mutating config path in the app serializes on the
//! `parking_lot::RwLock<AppConfig>` write guard, taken BEFORE the on-disk
//! read-modify-write and held until the in-memory snapshot is replaced. The
//! disk I/O is synchronous `std::fs`, so no `.await` ever runs under the
//! guard. This applies to every mutating command in this module and to
//! `crate::models::persist_provider_model_locked`, the one config writer
//! outside it; any new writer must follow the same pattern or a concurrent
//! writer's stale re-read can revert its change. Concurrent invokes execute
//! in order; last-write-wins on the same field is the intended semantic
//! (matches user expectation when rapidly tabbing between fields).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use parking_lot::RwLock;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tauri::{AppHandle, Emitter, Manager, State};
use toml_edit::{value as toml_value, Array, DocumentMut, Item, Table, Value as TomlValue};

use crate::config::{
    self,
    defaults::{ALLOWED_FIELDS, ALLOWED_SECTIONS},
    AppConfig, ConfigError, CorruptMarker, CONFIG_FILE_NAME,
};

/// Frontend event emitted to every webview after the in-memory `AppConfig`
/// has been replaced. Subscribers (the main overlay's `ConfigProvider` and
/// the Settings window) refetch via `get_config` so React state matches the
/// authoritative `RwLock<AppConfig>` snapshot. Without this broadcast, only
/// backend-side consumers (e.g. `ask_model` reading `State<RwLock<AppConfig>>` per
/// invocation) see config edits; frontend-driven values like window dims
/// stay frozen at the mount-time snapshot.
pub const CONFIG_UPDATED_EVENT: &str = "thuki://config-updated";

/// Emits `CONFIG_UPDATED_EVENT` to every webview. Errors are intentionally
/// swallowed: an emit failure must not break a successful disk write.
#[cfg_attr(coverage_nightly, coverage(off))]
fn emit_config_updated(app: &AppHandle) {
    let _ = app.emit(CONFIG_UPDATED_EVENT, ());
}

/// Resolves the absolute path to the user config file.
///
/// Centralizes the `app.path().app_config_dir() + CONFIG_FILE_NAME` join used
/// across the settings commands. On a successful lookup the returned path
/// matches the path the loader uses, so writes round-trip cleanly.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn config_path(app: &AppHandle) -> Result<PathBuf, ConfigError> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|source| ConfigError::IoError {
            path: PathBuf::from("<app_config_dir>"),
            source: std::io::Error::other(source.to_string()),
        })?;
    Ok(dir.join(CONFIG_FILE_NAME))
}

/// Returns whether a `(section, key)` pair is permitted by the allowlist.
fn is_allowed_field(section: &str, key: &str) -> bool {
    ALLOWED_FIELDS
        .iter()
        .any(|(s, k)| *s == section && *k == key)
}

/// Returns whether a section name is permitted by the section allowlist.
fn is_allowed_section(section: &str) -> bool {
    ALLOWED_SECTIONS.contains(&section)
}

/// True when `url` is an absolute http(s) URL. Same rule as the loader's
/// private `is_http_url`: provider base URLs the backend will POST to must
/// be rejected at write time rather than silently dropped at the next load.
pub(crate) fn is_http_url(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("http://") || url.starts_with("https://")
}

/// Returns true when the post-write `AppConfig` flips `[debug] trace_enabled`
/// relative to the pre-write snapshot. Pulled out so the predicate is
/// covered by tests instead of riding inside the coverage-off Tauri command
/// bodies that own the hot-swap.
pub(crate) fn trace_enabled_changed(prior_enabled: bool, resolved: &AppConfig) -> bool {
    resolved.debug.trace_enabled != prior_enabled
}

/// Returns the post-write `[debug] trace_retention_days` value when a config
/// write changed it relative to the pre-write snapshot, `None` when it is
/// unchanged. Pulled out so the change detection is unit-tested instead of
/// riding inside the coverage-off command bodies that fire the retention prune.
pub(crate) fn trace_retention_days_changed(prior_days: i64, resolved: &AppConfig) -> Option<i64> {
    let new_days = resolved.debug.trace_retention_days;
    (new_days != prior_days).then_some(new_days)
}

/// Fires a retention prune when a config write changed `[debug]
/// trace_retention_days`, applying the new window immediately so shortening
/// retention reclaims disk without waiting for the next launch. Thin dispatch
/// (excluded from coverage): the change predicate and the prune walk are each
/// tested on their own. Best-effort; a walk error is swallowed so a config
/// write never fails because of a hostile traces directory.
#[cfg_attr(coverage_nightly, coverage(off))]
fn prune_traces_on_retention_change(app: &AppHandle, prior_days: i64, resolved: &AppConfig) {
    if let Some(new_days) = trace_retention_days_changed(prior_days, resolved) {
        let _ = prune_traces_for_retention(&traces_root(app), new_days, SystemTime::now());
    }
}

/// Returns the engine runner's new `idle_minutes` value when the post-write
/// `AppConfig` changed `[inference] keep_warm_inactivity_minutes` relative to
/// the pre-write snapshot, `None` when it is unchanged. The unified field is
/// translated through [`crate::warmup::builtin_idle_minutes`] so the runner's
/// own `0 = forever` convention stays an implementation detail. Pulled out so
/// the predicate is covered by tests instead of riding inside the coverage-off
/// Tauri command bodies that forward the new value to the running engine actor.
pub(crate) fn keep_warm_idle_minutes_changed(
    prior_minutes: i32,
    resolved: &AppConfig,
) -> Option<u32> {
    let new_minutes = resolved.inference.keep_warm_inactivity_minutes;
    (new_minutes != prior_minutes).then(|| crate::warmup::builtin_idle_minutes(new_minutes))
}

/// Forwards a changed `[inference] keep_warm_inactivity_minutes` value to the
/// engine runner actor (translated to the runner's `idle_minutes`) so the new
/// residency policy applies without a restart. The runner is managed
/// regardless of the active provider, so forwarding is harmless when Ollama is
/// active (the Ollama path enforces residency through `keep_alive`, not the
/// runner). Spawned because the config commands are synchronous while the
/// actor's mailbox is async. Thin dispatch; the predicate and the actor's
/// `SetIdleMinutes` handling are both tested on their own.
#[cfg_attr(coverage_nightly, coverage(off))]
fn forward_keep_warm_idle_minutes(app: &AppHandle, prior_minutes: i32, resolved: &AppConfig) {
    if let Some(minutes) = keep_warm_idle_minutes_changed(prior_minutes, resolved) {
        let engine = app
            .state::<crate::engine::runner::EngineHandle>()
            .inner()
            .clone();
        tauri::async_runtime::spawn(async move { engine.set_idle_minutes(minutes).await });
    }
}

/// True when a config write moved the ACTIVE provider away from the built-in
/// engine (builtin -> ollama/openai). Switching between non-builtin kinds or
/// onto builtin never matches. Pulled out so the predicate is covered by
/// tests instead of riding inside the coverage-off Tauri command bodies that
/// fire the engine unload.
pub(crate) fn builtin_deactivated(prior_kind: &str, resolved: &AppConfig) -> bool {
    prior_kind == crate::config::defaults::PROVIDER_KIND_BUILTIN
        && resolved.inference.active_provider_kind()
            != crate::config::defaults::PROVIDER_KIND_BUILTIN
}

/// True when a config write moved the ACTIVE provider away from Ollama
/// (ollama -> builtin/openai). The mirror of [`builtin_deactivated`]: switching
/// between non-ollama kinds or onto ollama never matches. Pulled out so the
/// predicate is covered by tests instead of riding inside the coverage-off
/// command bodies that fire the Ollama eviction.
pub(crate) fn ollama_deactivated(prior_kind: &str, resolved: &AppConfig) -> bool {
    prior_kind == crate::config::defaults::PROVIDER_KIND_OLLAMA
        && resolved.inference.active_provider_kind()
            != crate::config::defaults::PROVIDER_KIND_OLLAMA
}

/// Fires a best-effort engine unload when a config write switched the active
/// provider away from the built-in engine. Without it, a multi-GB
/// llama-server stays resident until quit: the eviction UI branches by the
/// NEW provider kind (the builtin arm becomes unreachable) and the default
/// idle policy of 0 never unloads. Spawned so the switch neither blocks on
/// nor can fail because of the engine actor; an in-flight builtin request is
/// deliberately interrupted, matching an explicit user eviction.
#[cfg_attr(coverage_nightly, coverage(off))]
fn unload_engine_if_builtin_deactivated(app: &AppHandle, prior_kind: &str, resolved: &AppConfig) {
    if builtin_deactivated(prior_kind, resolved) {
        let engine = app
            .state::<crate::engine::runner::EngineHandle>()
            .inner()
            .clone();
        tauri::async_runtime::spawn(async move { engine.unload().await });
    }
}

/// Fires a best-effort Ollama eviction when a config write switched the active
/// provider away from Ollama (ollama -> builtin/openai). The mirror of
/// [`unload_engine_if_builtin_deactivated`]: without it the model Thuki loaded
/// into Ollama's VRAM lingers for its `keep_alive` TTL after the user has moved
/// on, holding memory for a provider that is no longer active. Only the model
/// Thuki was chatting with (the Ollama provider's configured `model`) is
/// evicted; models other apps loaded are left alone. Spawned so the switch
/// never blocks on, nor can fail because of, Ollama being unreachable.
#[cfg_attr(coverage_nightly, coverage(off))]
fn evict_ollama_if_deactivated(app: &AppHandle, prior_kind: &str, resolved: &AppConfig) {
    if !ollama_deactivated(prior_kind, resolved) {
        return;
    }
    // The provider switch moves only the active_provider pointer; the Ollama
    // provider entry still carries the model + endpoint Thuki was using.
    let Some(ollama) = resolved
        .inference
        .providers
        .iter()
        .find(|p| p.kind == crate::config::defaults::PROVIDER_KIND_OLLAMA)
    else {
        return;
    };
    let model = ollama.model.clone();
    if model.is_empty() {
        return;
    }
    let endpoint = format!("{}/api/generate", ollama.base_url.trim_end_matches('/'));
    let client = app.state::<reqwest::Client>().inner().clone();
    // Suppress any in-flight warmup that would re-announce the model as loaded
    // after we evict it, matching the explicit Unload-now path.
    app.state::<crate::warmup::WarmupState>().mark_evicted();
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = crate::warmup::evict_model_request(&endpoint, &model, &client).await;
        let _ = app_handle.emit("warmup:model-evicted", ());
    });
}

// ─── Tauri command surface ──────────────────────────────────────────────────

/// Returns the current resolved `AppConfig` snapshot.
///
/// The Settings window invokes this on mount to seed form state without
/// depending on event delivery (Tauri silently drops emits to closed
/// windows; mount-time fetch + focus-event reload guarantees the open
/// window always reflects the on-disk truth).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn get_config(state: State<'_, RwLock<AppConfig>>) -> AppConfig {
    state.read().clone()
}

/// Writes one field of the config file, returning the resolved `AppConfig`
/// after the loader has clamped / corrected the new value.
///
/// See module docs for the full security and concurrency contract.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn set_config_field(
    section: String,
    key: String,
    value: JsonValue,
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
    trace_recorder: State<'_, std::sync::Arc<crate::trace::LiveTraceRecorder>>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let (prior_trace_enabled, prior_keep_warm_minutes, prior_trace_retention_days) = {
        let guard = state.read();
        (
            guard.debug.trace_enabled,
            guard.inference.keep_warm_inactivity_minutes,
            guard.debug.trace_retention_days,
        )
    };
    let resolved = {
        let mut guard = state.write();
        let resolved = write_field_to_disk(&path, &section, &key, value)?;
        *guard = resolved.clone();
        resolved
    };
    // Hot-swap the live trace recorder on `[debug] trace_enabled` flips
    // so the user does not need to restart Thuki for the toggle to
    // take effect. Off → On installs a fresh `RegistryRecorder` rooted
    // at `app_data_dir()/traces/`; On → Off installs a `NoopRecorder`,
    // which lets in-flight streaming tasks finish writing through their
    // cached `Arc<FileRecorder>` clones (via `Arc` semantics) while new
    // events fall through to noop.
    if trace_enabled_changed(prior_trace_enabled, &resolved) {
        let new_inner = crate::build_trace_inner(&app, resolved.debug.trace_enabled);
        trace_recorder.replace(new_inner);
    }
    // Forward an `[inference] keep_warm_inactivity_minutes` change to the
    // engine runner so the new residency policy applies without restarting
    // Thuki.
    forward_keep_warm_idle_minutes(&app, prior_keep_warm_minutes, &resolved);
    // Enforce a changed `[debug] trace_retention_days` immediately so a
    // shortened window reclaims disk without waiting for the next launch.
    prune_traces_on_retention_change(&app, prior_trace_retention_days, &resolved);
    emit_config_updated(&app);
    Ok(resolved)
}

/// Sets the Ollama provider's `base_url` and returns the resolved `AppConfig`.
///
/// The Ollama URL is not a flat `set_config_field` key: it lives on the
/// `[[inference.providers]]` Ollama entry, so it has its own command. Mirrors
/// `set_config_field`'s lock + persist + broadcast contract.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn set_ollama_url(
    base_url: String,
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let resolved = {
        let mut guard = state.write();
        let resolved = write_provider_field_to_disk(
            &path,
            crate::config::defaults::PROVIDER_ID_OLLAMA,
            "base_url",
            base_url.trim(),
        )?;
        *guard = resolved.clone();
        resolved
    };
    emit_config_updated(&app);
    Ok(resolved)
}

/// Patches one `(section, key)` to disk and returns the resolved `AppConfig`
/// the loader produces from the new file. Pulled out of the Tauri wrapper so
/// the allowlist guard, document patch, atomic write, and post-write reload
/// are all exercised by the test suite without needing an `AppHandle`.
pub(crate) fn write_field_to_disk(
    path: &Path,
    section: &str,
    key: &str,
    value: JsonValue,
) -> Result<AppConfig, ConfigError> {
    if !is_allowed_section(section) {
        return Err(ConfigError::UnknownSection {
            section: section.to_string(),
        });
    }
    if !is_allowed_field(section, key) {
        return Err(ConfigError::UnknownField {
            section: section.to_string(),
            key: key.to_string(),
        });
    }

    let mut doc = read_document(path)?;
    // An allowed section can be absent from an older on-disk file that was
    // seeded before the section was added to the schema: the loader fills it
    // from defaults in memory but never rewrites the file. Materialize an empty
    // table so the field can be patched in. `section` is already validated
    // against `ALLOWED_SECTIONS` above, so this can only create a real schema
    // section, never an arbitrary one. Without this, writing any field in a
    // not-yet-persisted section fails with `UnknownSection`.
    if doc.get(section).and_then(Item::as_table).is_none() {
        doc.insert(section, Item::Table(Table::new()));
    }
    patch_document(&mut doc, section, key, value)?;
    // When the user saves the system prompt, mark it as explicitly customized
    // so the upgrade-migration path in the loader (empty + !customized →
    // restore default) does not overwrite a deliberate clear on next boot.
    if section == "prompt" && key == "system" {
        if let Some(table) = doc.get_mut("prompt").and_then(Item::as_table_mut) {
            table.insert("system_customized", toml_value(true));
        }
    }

    config::atomic_write_bytes(path, doc.to_string().as_bytes()).map_err(|source| {
        ConfigError::IoError {
            path: path.to_path_buf(),
            source,
        }
    })?;

    config::load_from_path(path)
}

/// Switches the active inference provider and returns the resolved `AppConfig`.
///
/// Validates that `provider_id` names an entry in the on-disk
/// `[[inference.providers]]` list, persists `[inference] active_provider`,
/// refreshes the managed config, and re-mirrors the in-memory
/// [`crate::models::ActiveModelState`] onto the new active provider's model
/// (Some when non-empty, None otherwise) so chat routes correctly without a
/// restart. Mirrors `set_ollama_url`'s lock + persist + broadcast contract.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn set_active_provider(
    provider_id: String,
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
    active_model: State<'_, crate::models::ActiveModelState>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let prior_kind = state.read().inference.active_provider_kind().to_string();
    let resolved = {
        let mut guard = state.write();
        let resolved = write_active_provider_to_disk(&path, &provider_id)?;
        *guard = resolved.clone();
        resolved
    };
    if let Some(mirror) = crate::models::should_refresh_active_model(&provider_id, &resolved) {
        if let Ok(mut guard) = active_model.0.lock() {
            *guard = mirror;
        }
    }
    // Switching away from a local provider releases its memory immediately so
    // the now-inactive provider holds no RAM/VRAM: the built-in engine's
    // sidecar is killed, and the Ollama model is evicted from VRAM. Exactly one
    // fires (the prior kind is builtin, ollama, or openai); openai is remote and
    // needs neither.
    unload_engine_if_builtin_deactivated(&app, &prior_kind, &resolved);
    evict_ollama_if_deactivated(&app, &prior_kind, &resolved);
    emit_config_updated(&app);
    Ok(resolved)
}

/// Patches one field (`model`, `base_url`, `label`, or `vision`) on the
/// provider whose id is `provider_id` and returns the resolved `AppConfig`.
///
/// Generalizes `set_ollama_url` to every editable provider field. A `model`
/// write on the active provider also re-mirrors the in-memory
/// [`crate::models::ActiveModelState`] so chat routes to the new selection
/// without a restart. Mirrors `set_ollama_url`'s lock + persist + broadcast
/// contract.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn update_provider_field(
    provider_id: String,
    field: String,
    value: String,
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
    active_model: State<'_, crate::models::ActiveModelState>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let resolved = {
        let mut guard = state.write();
        let resolved = write_provider_field_to_disk(&path, &provider_id, &field, &value)?;
        *guard = resolved.clone();
        resolved
    };
    if field == "model" {
        if let Some(mirror) = crate::models::should_refresh_active_model(&provider_id, &resolved) {
            if let Ok(mut guard) = active_model.0.lock() {
                *guard = mirror;
            }
        }
    }
    emit_config_updated(&app);
    Ok(resolved)
}

/// Adds the single OpenAI-compatible provider (fixed id `"openai"`) and
/// returns the resolved `AppConfig`. Empty label falls back to the compiled
/// default. Mirrors `set_ollama_url`'s lock + persist + broadcast contract.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn add_openai_provider(
    label: String,
    base_url: String,
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let resolved = {
        let mut guard = state.write();
        let resolved = add_openai_provider_to_disk(&path, &label, &base_url)?;
        *guard = resolved.clone();
        resolved
    };
    emit_config_updated(&app);
    Ok(resolved)
}

/// Removes the OpenAI-compatible provider and returns the resolved
/// `AppConfig`. When it was active, the active pointer falls back to the
/// built-in provider in the same atomic edit. Best-effort cleanup: each
/// removed provider id's Keychain API key is deleted (a Keychain failure
/// never undoes the config removal), and the in-memory active-model mirror
/// is refreshed onto whatever provider is active after the removal.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn remove_openai_provider(
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
    active_model: State<'_, crate::models::ActiveModelState>,
    secrets: State<'_, crate::keychain::Secrets>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let (resolved, removed_ids) = {
        let mut guard = state.write();
        let (resolved, removed_ids) = remove_openai_provider_from_disk(&path)?;
        *guard = resolved.clone();
        (resolved, removed_ids)
    };
    cleanup_provider_secrets(secrets.0.as_ref(), &removed_ids);
    let active_id = resolved.inference.active_provider.clone();
    if let Some(mirror) = crate::models::should_refresh_active_model(&active_id, &resolved) {
        if let Ok(mut guard) = active_model.0.lock() {
            *guard = mirror;
        }
    }
    emit_config_updated(&app);
    Ok(resolved)
}

/// Persists `[inference] active_provider = provider_id` after validating that
/// the id names an entry in the on-disk `[[inference.providers]]` list,
/// preserving the rest of the file via `toml_edit`, then reloads + resolves.
/// Sibling of [`write_provider_field_to_disk`]; pulled out of the Tauri
/// wrapper so the validation, atomic write, and post-write reload are
/// exercised without an `AppHandle`.
pub(crate) fn write_active_provider_to_disk(
    path: &Path,
    provider_id: &str,
) -> Result<AppConfig, ConfigError> {
    let mut doc = read_document(path)?;
    let providers = doc
        .get("inference")
        .and_then(|i| i.get("providers"))
        .and_then(|p| p.as_array_of_tables());
    let Some(providers) = providers else {
        return Err(ConfigError::UnknownSection {
            section: "inference.providers".to_string(),
        });
    };
    let known = providers
        .iter()
        .any(|t| t.get("id").and_then(|v| v.as_str()) == Some(provider_id));
    if !known {
        return Err(ConfigError::UnknownField {
            section: "inference.providers".to_string(),
            key: provider_id.to_string(),
        });
    }
    if let Some(table) = doc.get_mut("inference").and_then(Item::as_table_mut) {
        table.insert("active_provider", toml_value(provider_id));
    }
    config::atomic_write_bytes(path, doc.to_string().as_bytes()).map_err(|source| {
        ConfigError::IoError {
            path: path.to_path_buf(),
            source,
        }
    })?;
    config::load_from_path(path)
}

/// Patches a single field (`model`, `base_url`, `label`, or `vision`) on the
/// `[[inference.providers]]` entry whose `id` matches `provider_id`, preserving
/// the rest of the file via `toml_edit`, then reloads + resolves. Backs the
/// `set_active_model` (model), `set_ollama_url` (base_url), and
/// `update_provider_field` write paths. Pulled out of the Tauri wrappers so
/// the field allowlist, per-field validation, table lookup, atomic write, and
/// post-write reload are exercised without an `AppHandle`.
pub(crate) fn write_provider_field_to_disk(
    path: &Path,
    provider_id: &str,
    field: &str,
    value: &str,
) -> Result<AppConfig, ConfigError> {
    if !matches!(field, "model" | "base_url" | "label" | "vision") {
        return Err(ConfigError::UnknownField {
            section: "inference.providers".to_string(),
            key: field.to_string(),
        });
    }
    let mut doc = read_document(path)?;
    let providers = doc
        .get_mut("inference")
        .and_then(|i| i.get_mut("providers"))
        .and_then(|p| p.as_array_of_tables_mut());
    let Some(providers) = providers else {
        return Err(ConfigError::UnknownSection {
            section: "inference.providers".to_string(),
        });
    };
    let mut patched = false;
    for table in providers.iter_mut() {
        if table.get("id").and_then(|v| v.as_str()) == Some(provider_id) {
            let kind = table
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let item = validate_provider_value(&kind, field, value)?;
            table.insert(field, item);
            patched = true;
            break;
        }
    }
    if !patched {
        return Err(ConfigError::UnknownField {
            section: "inference.providers".to_string(),
            key: provider_id.to_string(),
        });
    }
    config::atomic_write_bytes(path, doc.to_string().as_bytes()).map_err(|source| {
        ConfigError::IoError {
            path: path.to_path_buf(),
            source,
        }
    })?;
    config::load_from_path(path)
}

/// Validates and coerces one provider field value into a TOML item.
///
/// Per-field rules:
/// - `model`: free-form string, trimmed.
/// - `label`: trimmed; a trimmed-empty value on an `openai`-kind provider
///   heals to the compiled default label, mirroring the add path so the card
///   heading never renders blank.
/// - `base_url`: rejected for the built-in provider (it has no URL); must be
///   an absolute http(s) URL for the network kinds.
/// - `vision`: the strings `"true"` / `"false"`, stored as a TOML boolean so
///   the schema's typed `bool` round-trips.
///
/// Validation errors come back as `TypeMismatch` whose message the Settings
/// UI surfaces verbatim in the inline error pill.
pub(crate) fn validate_provider_value(
    kind: &str,
    field: &str,
    value: &str,
) -> Result<Item, ConfigError> {
    let mismatch = |message: &str| ConfigError::TypeMismatch {
        section: "inference.providers".to_string(),
        key: field.to_string(),
        message: message.to_string(),
    };
    match field {
        "model" => Ok(toml_value(value.trim())),
        "label" => {
            let trimmed = value.trim();
            if trimmed.is_empty() && kind == crate::config::defaults::PROVIDER_KIND_OPENAI {
                // Mirrors `add_openai_provider_to_disk`: an empty label heals
                // to the compiled default instead of persisting a blank
                // heading.
                return Ok(toml_value(crate::config::defaults::DEFAULT_OPENAI_LABEL));
            }
            Ok(toml_value(trimmed))
        }
        "base_url" => {
            if kind == crate::config::defaults::PROVIDER_KIND_BUILTIN {
                return Err(mismatch("The built-in provider has no base URL."));
            }
            if !is_http_url(value) {
                return Err(mismatch("Base URL must start with http:// or https://."));
            }
            Ok(toml_value(value.trim()))
        }
        "vision" => match value {
            "true" => Ok(toml_value(true)),
            "false" => Ok(toml_value(false)),
            _ => Err(mismatch("vision must be \"true\" or \"false\".")),
        },
        other => Err(ConfigError::UnknownField {
            section: "inference.providers".to_string(),
            key: other.to_string(),
        }),
    }
}

/// Appends the single OpenAI-compatible provider record to the on-disk
/// `[[inference.providers]]` array, then reloads + resolves. At most one
/// `openai`-kind record may exist (fixed id `"openai"`, mirroring the single
/// Ollama URL); a second add is rejected. An empty `label` falls back to
/// [`crate::config::defaults::DEFAULT_OPENAI_LABEL`]. Pulled out of the Tauri
/// wrapper so the validation, duplicate guard, atomic write, and post-write
/// reload are exercised without an `AppHandle`.
pub(crate) fn add_openai_provider_to_disk(
    path: &Path,
    label: &str,
    base_url: &str,
) -> Result<AppConfig, ConfigError> {
    use crate::config::defaults::{DEFAULT_OPENAI_LABEL, PROVIDER_ID_OPENAI, PROVIDER_KIND_OPENAI};

    if !is_http_url(base_url) {
        return Err(ConfigError::TypeMismatch {
            section: "inference.providers".to_string(),
            key: "base_url".to_string(),
            message: "Base URL must start with http:// or https://.".to_string(),
        });
    }
    let mut doc = read_document(path)?;
    let providers = doc
        .get_mut("inference")
        .and_then(|i| i.get_mut("providers"))
        .and_then(|p| p.as_array_of_tables_mut());
    let Some(providers) = providers else {
        return Err(ConfigError::UnknownSection {
            section: "inference.providers".to_string(),
        });
    };
    let already_exists = providers
        .iter()
        .any(|t| t.get("kind").and_then(|v| v.as_str()) == Some(PROVIDER_KIND_OPENAI));
    if already_exists {
        return Err(ConfigError::TypeMismatch {
            section: "inference.providers".to_string(),
            key: PROVIDER_ID_OPENAI.to_string(),
            message: "An OpenAI-compatible provider already exists.".to_string(),
        });
    }
    let label = label.trim();
    let label = if label.is_empty() {
        DEFAULT_OPENAI_LABEL
    } else {
        label
    };
    // The typed constructor is the single source of truth for the record's
    // shape (kind, empty model, vision off); this just transcribes it to TOML.
    let provider =
        crate::config::schema::openai_provider(PROVIDER_ID_OPENAI, label, base_url.trim());
    let mut table = Table::new();
    table.insert("id", toml_value(provider.id.as_str()));
    table.insert("kind", toml_value(provider.kind.as_str()));
    table.insert("label", toml_value(provider.label.as_str()));
    table.insert("base_url", toml_value(provider.base_url.as_str()));
    table.insert("model", toml_value(provider.model.as_str()));
    table.insert("vision", toml_value(provider.vision));
    providers.push(table);

    config::atomic_write_bytes(path, doc.to_string().as_bytes()).map_err(|source| {
        ConfigError::IoError {
            path: path.to_path_buf(),
            source,
        }
    })?;
    config::load_from_path(path)
}

/// Best-effort Keychain cleanup after a provider removal: deletes the API-key
/// secret stored under each removed provider id. Hand-edited files can carry
/// an arbitrary id on an `openai`-kind row (the loader preserves it, and the
/// frontend stores the key under `provider.id`), so cleanup must follow the
/// ids actually removed rather than the fixed default id. Failures are
/// ignored: a Keychain error never undoes the config removal. Rows missing
/// an `id` collapse to an empty string in `removed_ids` and are skipped.
pub(crate) fn cleanup_provider_secrets(
    store: &dyn crate::keychain::SecretStore,
    removed_ids: &[String],
) {
    for id in removed_ids {
        if id.is_empty() {
            continue;
        }
        let _ = store.delete(id);
    }
}

/// Removes every `openai`-kind entry from the on-disk
/// `[[inference.providers]]` array, returning the resolved `AppConfig` and
/// the ids of the removed entries (for Keychain cleanup). When a removed
/// provider was active, `active_provider` falls back to the built-in
/// provider in the same atomic edit. Errors when no OpenAI-compatible
/// provider exists. Pulled out of the Tauri wrapper so the removal,
/// fallback, atomic write, and post-write reload are exercised without an
/// `AppHandle`.
pub(crate) fn remove_openai_provider_from_disk(
    path: &Path,
) -> Result<(AppConfig, Vec<String>), ConfigError> {
    use crate::config::defaults::{PROVIDER_ID_BUILTIN, PROVIDER_ID_OPENAI, PROVIDER_KIND_OPENAI};

    let mut doc = read_document(path)?;
    let providers = doc
        .get_mut("inference")
        .and_then(|i| i.get_mut("providers"))
        .and_then(|p| p.as_array_of_tables_mut());
    let Some(providers) = providers else {
        return Err(ConfigError::UnknownSection {
            section: "inference.providers".to_string(),
        });
    };
    let removed_ids: Vec<String> = providers
        .iter()
        .filter(|t| t.get("kind").and_then(|v| v.as_str()) == Some(PROVIDER_KIND_OPENAI))
        .map(|t| {
            t.get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    if removed_ids.is_empty() {
        return Err(ConfigError::UnknownField {
            section: "inference.providers".to_string(),
            key: PROVIDER_ID_OPENAI.to_string(),
        });
    }
    providers.retain(|t| t.get("kind").and_then(|v| v.as_str()) != Some(PROVIDER_KIND_OPENAI));

    let active_removed = doc
        .get("inference")
        .and_then(|i| i.get("active_provider"))
        .and_then(|v| v.as_str())
        .is_some_and(|active| removed_ids.iter().any(|id| id == active));
    if active_removed {
        if let Some(table) = doc.get_mut("inference").and_then(Item::as_table_mut) {
            table.insert("active_provider", toml_value(PROVIDER_ID_BUILTIN));
        }
    }

    config::atomic_write_bytes(path, doc.to_string().as_bytes()).map_err(|source| {
        ConfigError::IoError {
            path: path.to_path_buf(),
            source,
        }
    })?;
    Ok((config::load_from_path(path)?, removed_ids))
}

/// Resets one section (or the whole file when `section` is `None`) to the
/// compiled defaults, returning the resulting `AppConfig`.
///
/// Section reset is implemented by replacing only the named section's table in
/// the on-disk document with the table from `AppConfig::default()`. Other
/// sections, top-level comments, and key ordering inside untouched sections
/// are preserved.
///
/// Whole-file reset rewrites the file with `atomic_write(&AppConfig::default)`,
/// which produces byte-for-byte identical output to a fresh first-run seed.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn reset_config(
    section: Option<String>,
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
    trace_recorder: State<'_, std::sync::Arc<crate::trace::LiveTraceRecorder>>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let (prior_trace_enabled, prior_keep_warm_minutes, prior_trace_retention_days) = {
        let guard = state.read();
        (
            guard.debug.trace_enabled,
            guard.inference.keep_warm_inactivity_minutes,
            guard.debug.trace_retention_days,
        )
    };
    let resolved = {
        let mut guard = state.write();
        let resolved = reset_section_on_disk(&path, section.as_deref())?;
        *guard = resolved.clone();
        resolved
    };
    // Hot-swap the live trace recorder if `reset_config` flipped the
    // `[debug] trace_enabled` value (resetting the whole file or just
    // the `[debug]` section both restore the compiled default of
    // `false`, so an On → Off transition is the realistic case).
    if trace_enabled_changed(prior_trace_enabled, &resolved) {
        let new_inner = crate::build_trace_inner(&app, resolved.debug.trace_enabled);
        trace_recorder.replace(new_inner);
    }
    // A whole-file or `[inference]` reset restores the default residency
    // policy; forward it so the engine runner picks it up immediately.
    forward_keep_warm_idle_minutes(&app, prior_keep_warm_minutes, &resolved);
    // A whole-file or `[debug]` reset restores the default retention window;
    // prune to it so a shortened window (e.g. from a prior keep-forever) takes
    // effect immediately.
    prune_traces_on_retention_change(&app, prior_trace_retention_days, &resolved);
    emit_config_updated(&app);
    Ok(resolved)
}

/// Replaces one section (or the entire file when `section` is `None`) with
/// the compiled defaults and returns the resolved `AppConfig`. Pulled out of
/// the Tauri wrapper so the allowlist guard, table-replacement, atomic
/// write, and post-write reload are exercised by the test suite without
/// needing an `AppHandle`.
pub(crate) fn reset_section_on_disk(
    path: &Path,
    section: Option<&str>,
) -> Result<AppConfig, ConfigError> {
    if let Some(section_name) = section {
        if !is_allowed_section(section_name) {
            return Err(ConfigError::UnknownSection {
                section: section_name.to_string(),
            });
        }
        let mut doc = read_document(path)?;
        let defaults = AppConfig::default();
        let defaults_str =
            toml::to_string_pretty(&defaults).expect("AppConfig is always serializable to TOML");
        let defaults_doc: DocumentMut = defaults_str
            .parse()
            .expect("defaults serialize to a parseable TOML document");
        // is_allowed_section above guarantees `section_name` is one of the
        // top-level keys produced by `AppConfig::default()` serialization, so
        // the lookup is infallible by construction.
        let new_section = defaults_doc
            .get(section_name)
            .cloned()
            .expect("ALLOWED_SECTIONS implies AppConfig::default has this section");
        doc.insert(section_name, new_section);
        config::atomic_write_bytes(path, doc.to_string().as_bytes()).map_err(|source| {
            ConfigError::IoError {
                path: path.to_path_buf(),
                source,
            }
        })?;
    } else {
        config::atomic_write(path, &AppConfig::default()).map_err(|source| {
            ConfigError::IoError {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }

    config::load_from_path(path)
}

/// Re-reads the config file from disk and replaces the in-memory `AppConfig`.
///
/// Bound to the Settings window's `tauri://focus` event and to the explicit
/// "↻ Refresh from disk" button in the About tab. Replaces the file-watcher
/// subsystem the eng review collapsed (see design doc Outside Voice
/// Resolution).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn reload_config_from_disk(
    app: AppHandle,
    state: State<'_, RwLock<AppConfig>>,
    trace_recorder: State<'_, std::sync::Arc<crate::trace::LiveTraceRecorder>>,
) -> Result<AppConfig, ConfigError> {
    let path = config_path(&app)?;
    let (prior_trace_enabled, prior_keep_warm_minutes, prior_trace_retention_days, prior_kind) = {
        let guard = state.read();
        (
            guard.debug.trace_enabled,
            guard.inference.keep_warm_inactivity_minutes,
            guard.debug.trace_retention_days,
            guard.inference.active_provider_kind().to_string(),
        )
    };
    let resolved = {
        let mut guard = state.write();
        let resolved = config::load_from_path(&path)?;
        *guard = resolved.clone();
        resolved
    };
    // Hot-swap the live trace recorder if a manual edit to config.toml
    // flipped `[debug] trace_enabled` and the user clicked "Refresh
    // from disk" to pick it up.
    if trace_enabled_changed(prior_trace_enabled, &resolved) {
        let new_inner = crate::build_trace_inner(&app, resolved.debug.trace_enabled);
        trace_recorder.replace(new_inner);
    }
    // Manual edits to `[inference] keep_warm_inactivity_minutes` reach the
    // engine runner through the same refresh path.
    forward_keep_warm_idle_minutes(&app, prior_keep_warm_minutes, &resolved);
    // A hand-edited `[debug] trace_retention_days` picked up on refresh is
    // enforced right away, the only non-restart path for a manual edit.
    prune_traces_on_retention_change(&app, prior_trace_retention_days, &resolved);
    // A hand-edited `active_provider` that moved away from a local provider
    // releases its memory (builtin sidecar killed, Ollama model evicted),
    // mirroring the Settings radio path.
    unload_engine_if_builtin_deactivated(&app, &prior_kind, &resolved);
    evict_ollama_if_deactivated(&app, &prior_kind, &resolved);
    emit_config_updated(&app);
    Ok(resolved)
}

/// Returns and consumes the corrupt-recovery marker, if one exists.
///
/// The Settings window invokes this on mount; if a marker is returned, it
/// renders a dismissible recovery banner. The marker is deleted from disk on
/// read so the banner appears at most once per corrupt event.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn get_corrupt_marker(app: AppHandle) -> Result<Option<CorruptMarker>, ConfigError> {
    let path = config_path(&app)?;
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(config::consume_corrupt_marker(&dir))
}

/// Opens Finder with the user's `config.toml` selected.
///
/// Thin FFI wrapper (excluded from coverage) over `open -R`, which is the
/// macOS-native "reveal in Finder" affordance.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn reveal_config_in_finder(app: AppHandle) -> Result<(), String> {
    let path = config_path(&app).map_err(|e| e.to_string())?;
    std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ─── Trace folder actions ───────────────────────────────────────────────────

/// Resolves the on-disk root under which trace recordings are written.
///
/// Single source of truth for the traces directory, shared by
/// [`crate::build_trace_inner`] (which installs the recorder) and the
/// [`open_traces_in_finder`] / [`free_traces`] commands, so the path they
/// operate on can never drift apart. Resolves to `app_data_dir()/traces`,
/// falling back to a temp-dir path when the platform data directory is
/// unavailable (matching the recorder's own fallback).
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn traces_root(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .map(|d| d.join("traces"))
        .unwrap_or_else(|_| std::env::temp_dir().join("thuki").join("traces"))
}

/// Deletes every recorded trace under `root`, leaving an empty root in place.
///
/// Pure and testable: the [`free_traces`] command derives `root` server-side
/// and delegates here so no frontend-supplied path is ever a deletion target.
///
/// Behavior:
/// - Missing `root` is a no-op returning `Ok(())` (nothing to clear).
/// - Otherwise the whole tree is removed (including per-domain subdirectories
///   such as `chat/<id>.jsonl`) and an empty `root` is recreated.
/// - Any other I/O error is propagated, never panicked on.
///
/// Open file handles are safe on macOS: unlinking a `.jsonl` file that the
/// live recorder currently holds open does not disturb the recorder, which
/// keeps appending to the now-unlinked inode; the next conversation opens a
/// fresh file under the recreated root.
pub(crate) fn clear_traces_dir(root: &Path) -> std::io::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(root)?;
    std::fs::create_dir_all(root)
}

/// Opens the traces folder in Finder.
///
/// Thin FFI wrapper (excluded from coverage) that ensures the directory
/// exists first, so the action always succeeds even before any trace has
/// been recorded, then opens it with the macOS-native `open`.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn open_traces_in_finder(app: AppHandle) -> Result<(), String> {
    let root = traces_root(&app);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    std::process::Command::new("open")
        .arg(&root)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Deletes all recorded traces from disk.
///
/// Thin command wrapper (excluded from coverage): it derives the deletion
/// root server-side via [`traces_root`] (the frontend never supplies a path,
/// so there is no traversal or client-controlled deletion target) and hands
/// the real work to [`clear_traces_dir`], where it is unit-tested.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn free_traces(app: AppHandle) -> Result<(), String> {
    clear_traces_dir(&traces_root(&app)).map_err(|e| e.to_string())
}

/// On-disk trace footprint returned to the Settings window.
///
/// `count` is the number of trace files; `bytes` is their combined size.
/// The frontend renders these as a muted subtext (e.g. "12 traces · 4.2 MB
/// on disk") beneath the trace actions.
#[derive(Debug, Serialize, PartialEq)]
pub struct TracesStats {
    pub count: u64,
    pub bytes: u64,
}

/// Counts the trace files under `root` and sums their byte size.
///
/// Pure and testable: walks the traces tree iteratively (the `root` plus its
/// per-domain subdirectories such as `chat/`), counting regular files and
/// summing `metadata().len()`. Reads metadata only, never file contents, so
/// memory stays bounded to two integers regardless of trace volume.
///
/// Symlinks are not followed: `DirEntry::file_type` and `DirEntry::metadata`
/// report on the link itself, so a symlinked entry is neither a dir to descend
/// nor a file to count, and is skipped. A missing `root` yields `(0, 0)`.
/// I/O errors mid-walk are propagated rather than panicked on. Snapshot
/// semantics are fine: no lock is taken even if a recorder writes concurrently.
pub(crate) fn traces_stats_for(root: &Path) -> std::io::Result<(u64, u64)> {
    if !root.exists() {
        return Ok((0, 0));
    }
    let mut count: u64 = 0;
    let mut bytes: u64 = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                bytes += entry.metadata()?.len();
                count += 1;
            }
        }
    }
    Ok((count, bytes))
}

/// Deletes every regular trace file under `root` whose modification time is
/// older than `now - max_age`, returning the count removed.
///
/// Walks `root` and its per-domain subdirectories (`chat/` etc.) iteratively,
/// reading only metadata so memory stays bounded to a few integers regardless
/// of trace volume. `now` is a parameter (not `SystemTime::now()`) so the
/// pruning cutoff is deterministic in tests. The directory structure is left
/// intact; only files are removed.
///
/// Safety / correctness:
/// - The deletion target is composed entirely from the app-owned `root`; no
///   frontend-supplied path ever reaches here (callers resolve `root` via
///   [`traces_root`]).
/// - Symlinks are never followed: `DirEntry::file_type` reports on the link
///   itself, so a symlinked entry is neither descended (dir) nor deleted
///   (file), it is skipped. This keeps a symlink from redirecting a delete
///   outside the traces tree.
/// - A file whose mtime is in the future relative to `now` (or when `now`
///   predates it) is kept: `SystemTime::duration_since` returns `Err` and the
///   entry is skipped rather than deleted on a clock ambiguity.
/// - A missing `root` yields `Ok(0)`; I/O errors mid-walk propagate via `?`.
///
/// The live recorder's currently-open `.jsonl` file always has a recent mtime,
/// so any sane retention window naturally excludes it. Even if it were caught,
/// unlinking an open trace file is safe on macOS (see [`clear_traces_dir`]):
/// the recorder keeps appending to the now-unlinked inode and the next
/// conversation opens a fresh file under the intact tree.
pub(crate) fn prune_traces_older_than(
    root: &Path,
    max_age: Duration,
    now: SystemTime,
) -> std::io::Result<u64> {
    if !root.exists() {
        return Ok(0);
    }
    let mut pruned: u64 = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                let mtime = entry.metadata()?.modified()?;
                // Future-dated file (or `now` in the past): keep it. `Ok(age)`
                // only when `now >= mtime`, so an older-than-window file is the
                // sole delete path.
                if let Ok(age) = now.duration_since(mtime) {
                    if age > max_age {
                        std::fs::remove_file(entry.path())?;
                        pruned += 1;
                    }
                }
            }
        }
    }
    Ok(pruned)
}

/// Maps a config-clamped `trace_retention_days` value to a prune of the traces
/// tree, returning the number of files removed.
///
/// The sentinel `-1` (keep forever) and any non-positive value short-circuit
/// with no walk; a positive day count is converted to a `max_age` window and
/// handed to [`prune_traces_older_than`]. `now` is injected so the mapping is
/// deterministic in tests. The loader clamps `retention_days` to `-1` or
/// `1..=3650` before this ever runs, so the positive branch always sees a sane
/// day count; the multiply is saturating purely as never-panic defense.
pub(crate) fn prune_traces_for_retention(
    root: &Path,
    retention_days: i64,
    now: SystemTime,
) -> std::io::Result<u64> {
    if retention_days < 1 {
        // -1 is the keep-forever sentinel; any other non-positive value cannot
        // reach here post-clamp but is treated the same (no pruning) so a
        // negative day count can never become a huge `max_age`.
        return Ok(0);
    }
    let max_age = Duration::from_secs((retention_days as u64).saturating_mul(86_400));
    prune_traces_older_than(root, max_age, now)
}

/// Runs one retention prune at startup, deleting trace files older than the
/// configured `[debug] trace_retention_days` window.
///
/// Thin wrapper (excluded from coverage): reads the clamped retention value
/// from managed config, resolves the traces root server-side via
/// [`traces_root`], and delegates to [`prune_traces_for_retention`] with the
/// wall clock. Best-effort and metadata-only: a walk error is swallowed so a
/// hostile filesystem never blocks launch, and the bounded-memory walk keeps
/// startup off the hot path.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn prune_traces_at_startup(app: &AppHandle) {
    let retention_days = app
        .state::<RwLock<AppConfig>>()
        .read()
        .debug
        .trace_retention_days;
    let _ = prune_traces_for_retention(&traces_root(app), retention_days, SystemTime::now());
}

/// Reports the current on-disk trace footprint to the Settings window.
///
/// Thin command wrapper (excluded from coverage): resolves the root
/// server-side via [`traces_root`] and delegates counting to
/// [`traces_stats_for`], where the logic is unit-tested.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn traces_stats(app: AppHandle) -> Result<TracesStats, String> {
    let (count, bytes) = traces_stats_for(&traces_root(&app)).map_err(|e| e.to_string())?;
    Ok(TracesStats { count, bytes })
}

// ─── Document I/O + JSON→TOML coercion (testable internals) ─────────────────

/// Information returned to the frontend in the rare case where `set_config_field`
/// is called with a value the loader silently corrected (e.g. a numeric outside
/// its `BOUNDS_*` range, reset to the compiled default).
///
/// The corrected value is already in the returned `AppConfig`; this struct
/// exists for test harnesses that want to assert on the correction path.
#[derive(Debug, Serialize, PartialEq)]
pub struct PatchOutcome {
    pub section: String,
    pub key: String,
}

/// Reads and parses the TOML document. Maps any I/O or parse error to the
/// appropriate `ConfigError` variant so the IPC boundary surfaces a typed
/// failure.
pub(crate) fn read_document(path: &Path) -> Result<DocumentMut, ConfigError> {
    let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::IoError {
        path: path.to_path_buf(),
        source,
    })?;
    contents
        .parse::<DocumentMut>()
        .map_err(|e| ConfigError::Parse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
}

/// Locates `[section][key]` inside `doc` and overwrites it with `value`,
/// preserving the existing TOML type. Rejects type drift with `TypeMismatch`.
///
/// If the key is absent from the section (e.g. the user hand-edited it out),
/// a new item is inserted with the type inferred from the JSON value rather
/// than returning an error. Inference rules for absent keys:
///
/// | JSON type             | Inserted TOML type |
/// | :-------------------- | :----------------- |
/// | Bool                  | Boolean            |
/// | Integer number        | Integer            |
/// | Float number          | Float              |
/// | String                | String             |
/// | Array of strings      | Array              |
/// | Object / null / other | TypeMismatch error |
///
/// Type-coercion rules for existing items (existing item type -> accepted JSON):
///
/// | Existing TOML type | Accepted JSON                          |
/// | :----------------- | :------------------------------------- |
/// | Integer            | Number with no fractional part         |
/// | Float              | Number (integer also accepted)         |
/// | String             | String                                 |
/// | Boolean            | Bool                                   |
/// | Array              | Array of strings                       |
///
/// Other primitive combinations (object, null, mixed-type arrays) are
/// rejected.
pub(crate) fn patch_document(
    doc: &mut DocumentMut,
    section: &str,
    key: &str,
    value: JsonValue,
) -> Result<(), ConfigError> {
    let table = doc
        .get_mut(section)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| ConfigError::UnknownSection {
            section: section.to_string(),
        })?;

    // The schema-derived template is the authoritative type source: it
    // captures the TOML type the loader expects regardless of what the
    // on-disk file currently holds. Preferring it over `existing` heals
    // legacy files whose type drifted (e.g. an f64-typed field persisted
    // as TOML Integer after a first save from a JS whole-number payload
    // through `json_value_to_toml_item`). Falling back to the existing
    // item, and finally to JSON inference, only matters for keys outside
    // `AppConfig` — the allowlist normally gates this away first, so
    // those branches are kept as defense-in-depth.
    let coerced = if let Some(template) = schema_template_item(section, key) {
        coerce_json_to_toml(&template, value, section, key)?
    } else if let Some(existing) = table.get(key) {
        coerce_json_to_toml(existing, value, section, key)?
    } else {
        json_value_to_toml_item(value, section, key)?
    };
    table.insert(key, coerced);
    Ok(())
}

/// Returns the `Item` that `AppConfig::default()` produces for `(section, key)`
/// after a TOML round-trip. The serialized defaults document is the closest
/// thing we have to a schema reflection: every tunable field in `AppConfig`
/// appears in it with the TOML type the loader expects. Used by
/// `patch_document` to keep the on-disk type stable when the field is
/// missing from the user's file.
///
/// Returns `None` only when the lookup falls outside `ALLOWED_FIELDS`
/// (impossible in practice — callers gate on that allowlist first — but the
/// `Option` keeps this function honest at the type boundary).
fn schema_template_item(section: &str, key: &str) -> Option<Item> {
    let defaults_str = toml::to_string_pretty(&AppConfig::default())
        .expect("AppConfig is always serializable to TOML");
    let defaults_doc: DocumentMut = defaults_str
        .parse()
        .expect("defaults serialize to a parseable TOML document");
    defaults_doc
        .get(section)
        .and_then(Item::as_table)
        .and_then(|t| t.get(key))
        .cloned()
}

/// Converts a JSON value to a TOML item by inferring the type from the JSON,
/// used when the target key is absent from the on-disk document.
pub(crate) fn json_value_to_toml_item(
    value: JsonValue,
    section: &str,
    key: &str,
) -> Result<Item, ConfigError> {
    let type_mismatch = |msg: &str| ConfigError::TypeMismatch {
        section: section.to_string(),
        key: key.to_string(),
        message: msg.to_string(),
    };

    Ok(match &value {
        JsonValue::Bool(b) => toml_value(*b),
        JsonValue::Number(n) => {
            // Else branch: u64 above i64::MAX only; unreachable via ALLOWED_FIELDS
            // (all tunables are u32/u64 within i64::MAX). Loader clamps regardless.
            if let Some(i) = n.as_i64() {
                toml_value(i)
            } else {
                toml_value(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        JsonValue::String(s) => toml_value(s.as_str()),
        JsonValue::Array(arr) => {
            let mut toml_arr = Array::new();
            for item in arr {
                let s = item.as_str().ok_or_else(|| ConfigError::TypeMismatch {
                    section: section.to_string(),
                    key: key.to_string(),
                    message: "array elements must be strings".into(),
                })?;
                toml_arr.push(s);
            }
            toml_value(toml_arr)
        }
        _ => {
            return Err(type_mismatch(&format!(
                "cannot infer TOML type from {}",
                json_type_name(&value)
            )));
        }
    })
}

/// Coerces `value` to a `toml_edit::Item` whose primitive type matches the
/// type of `existing`. The function inspects the existing item's discriminator
/// rather than the schema, so it stays in lock-step with whatever the loader
/// most-recently wrote (which, after seeding, includes every tunable field).
pub(crate) fn coerce_json_to_toml(
    existing: &Item,
    value: JsonValue,
    section: &str,
    key: &str,
) -> Result<Item, ConfigError> {
    let mismatch = |expected: &str| ConfigError::TypeMismatch {
        section: section.to_string(),
        key: key.to_string(),
        message: format!("expected {expected}, got {}", json_type_name(&value)),
    };

    let existing_value = existing
        .as_value()
        .ok_or_else(|| ConfigError::TypeMismatch {
            section: section.to_string(),
            key: key.to_string(),
            message: "existing field is not a primitive".into(),
        })?;

    Ok(match existing_value {
        TomlValue::Integer(_) => {
            let n = value.as_i64().or_else(|| {
                value.as_f64().and_then(|f| {
                    if f.fract() == 0.0 && f.is_finite() {
                        Some(f as i64)
                    } else {
                        None
                    }
                })
            });
            let n = n.ok_or_else(|| mismatch("integer number"))?;
            toml_value(n)
        }
        TomlValue::Float(_) => {
            // `serde_json::Value::as_f64` already widens integer payloads to
            // f64 (it inspects the inner Number, returning Some for both
            // i64/u64 variants), so the legacy `or_else(as_i64)` fallback
            // here was unreachable and dead. Drop it.
            let f = value.as_f64().ok_or_else(|| mismatch("number"))?;
            toml_value(f)
        }
        TomlValue::String(_) => {
            let s = value.as_str().ok_or_else(|| mismatch("string"))?;
            toml_value(s)
        }
        TomlValue::Boolean(_) => {
            let b = value.as_bool().ok_or_else(|| mismatch("boolean"))?;
            toml_value(b)
        }
        TomlValue::Array(_) => {
            let json_arr = value
                .as_array()
                .ok_or_else(|| mismatch("array of strings"))?;
            let mut arr = Array::new();
            for item in json_arr {
                let s = item.as_str().ok_or_else(|| ConfigError::TypeMismatch {
                    section: section.to_string(),
                    key: key.to_string(),
                    message: "array elements must be strings".into(),
                })?;
                arr.push(s);
            }
            toml_value(arr)
        }
        TomlValue::Datetime(_) | TomlValue::InlineTable(_) => {
            return Err(ConfigError::TypeMismatch {
                section: section.to_string(),
                key: key.to_string(),
                message: "field type not supported by GUI writes".into(),
            })
        }
    })
}

/// Returns a stable, human-readable name for a JSON value's primitive type.
/// Used in error messages so the frontend can surface "expected integer, got
/// string" without inspecting the raw `Value` itself.
fn json_type_name(v: &JsonValue) -> &'static str {
    match v {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "float"
            }
        }
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests;
