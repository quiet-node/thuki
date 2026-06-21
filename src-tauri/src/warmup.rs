use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, Manager};

use crate::config::defaults::{
    PROVIDER_KIND_BUILTIN, PROVIDER_KIND_OLLAMA, VRAM_POLL_INTERVAL_SECS,
};

type InFlightSlot = Arc<Mutex<Option<(String, Option<String>, String, u32)>>>;
type OnLoaded = Arc<dyn Fn(String) + Send + Sync + 'static>;

pub struct WarmupState {
    in_flight: InFlightSlot,
    on_loaded: OnLoaded,
    /// Set when the user explicitly evicts the model. Cleared on the next
    /// `fire()` call so an in-flight warmup that completes after eviction does
    /// not re-announce the model as loaded.
    evicted: Arc<AtomicBool>,
}

/// Strips the `:latest` tag suffix that Ollama appends to slugs in `/api/ps`
/// responses when the user stored the model without an explicit tag. Comparison
/// is case-sensitive: Ollama uses lower-case tags exclusively.
pub(crate) fn normalize_slug(s: &str) -> &str {
    s.strip_suffix(":latest").unwrap_or(s)
}

/// The change in VRAM state detected by comparing a previous and current slug.
#[derive(Debug, PartialEq)]
pub(crate) enum VramTransition {
    /// No state change: same model still loaded, or still unloaded.
    None,
    /// A model is now loaded (freshly loaded or switched to a different model).
    Loaded(String),
    /// The model that was previously loaded is no longer in VRAM.
    Evicted,
}

/// Compares the previous and current VRAM slug and returns the transition.
pub(crate) fn detect_vram_transition(
    prev: &Option<String>,
    current: &Option<String>,
) -> VramTransition {
    match (prev, current) {
        (_, Some(slug)) if prev.as_deref() != Some(slug.as_str()) => {
            VramTransition::Loaded(slug.clone())
        }
        (Some(_), None) => VramTransition::Evicted,
        _ => VramTransition::None,
    }
}

/// Converts an inactivity timeout in minutes to an Ollama `keep_alive` string.
/// -1 maps to `"-1"` (Ollama never-unload sentinel); any positive value maps
/// to `"<N>m"`.
pub fn keep_alive_string(minutes: i32) -> String {
    if minutes == -1 {
        "-1".to_string()
    } else {
        format!("{minutes}m")
    }
}

/// Translates the unified `keep_warm_inactivity_minutes` sentinel into the
/// built-in engine runner's own `idle_minutes` convention. Lives here next to
/// [`keep_alive_string`], the symmetric Ollama translator, so both consumers of
/// the unified field (startup seeding in `lib.rs` and the Settings forward in
/// `settings_commands.rs`) translate through one tested boundary.
///
/// Unified field semantics -> runner convention (`0` = idle-unload disabled =
/// forever, `N>0` = unload after N minutes):
/// - `-1` (keep resident forever) -> `0` (disable the runner's idle timer).
/// - `0` (the provider's natural short default) -> `DEFAULT_BUILTIN_IDLE_MINUTES`
///   (~5 min): the built-in engine has no external daemon to defer to, so it
///   applies its own short timer.
/// - `N>0` (explicit minutes) -> `N` as the runner's minute count.
///
/// The loader clamps the field to `[-1, 1440]`, so the catch-all only fires on
/// already-validated values; the `match` is written total regardless.
pub(crate) fn builtin_idle_minutes(keep_warm_inactivity_minutes: i32) -> u32 {
    match keep_warm_inactivity_minutes {
        -1 => 0,
        0 => crate::config::defaults::DEFAULT_BUILTIN_IDLE_MINUTES,
        n if n > 0 => n as u32,
        // Below -1: out-of-contract once the loader clamps; map to the same
        // "forever" disable as -1 so a stray negative never enables a timer.
        _ => 0,
    }
}

/// True when the VRAM poller should query Ollama's `/api/ps` on this tick.
/// The poller observes Ollama's VRAM only: the built-in engine publishes its
/// lifecycle through the engine status watch and an `openai` provider has no
/// local memory to observe, so any non-Ollama active provider skips the HTTP
/// call entirely.
pub(crate) fn vram_poll_active(kind: &str) -> bool {
    kind == PROVIDER_KIND_OLLAMA
}

/// Whether the built-in engine should warm-load on the chat-intent signal:
/// only when a model is actually selected. Mirrors the Ollama arm, which also
/// no-ops without a model. An empty id means no built-in model has been picked
/// yet, so there is nothing to load.
pub(crate) fn builtin_should_warm(model_id: &str) -> bool {
    !model_id.is_empty()
}

/// Builds the prime request body for the built-in engine: a plain
/// `/v1/chat/completions` completion carrying the resolved system prompt and
/// a one-token budget. llama-server's prompt cache (on by default) keeps the
/// system prefix in KV so the first real message skips its prefill.
pub(crate) fn builtin_prime_body(model: &str, system_prompt: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": "ok"}
        ],
        "max_tokens": 1,
        "stream": false
    })
}

/// Fires the built-in engine prime request at the serving port. Best-effort,
/// mirroring `run_warmup`'s error handling: every failure (transport or HTTP)
/// is silently ignored. Deliberately does NOT touch the engine's idle clock:
/// priming is app-summon activity, not user chat; if it touched, idle-unload
/// would never fire for a user who keeps summoning the overlay without
/// chatting.
/// Returns `true` when the prime got an HTTP 200 (the model is now warm and
/// the system-prompt prefix is cached); any transport or non-200 outcome
/// returns `false` so the caller leaves the load un-primed and a later warm
/// can retry.
pub(crate) async fn prime_builtin(
    port: u16,
    model: String,
    system_prompt: String,
    client: reqwest::Client,
) -> bool {
    let body = builtin_prime_body(&model, &system_prompt);
    client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .map(|r| r.status().as_u16())
        .unwrap_or(0)
        == 200
}

/// Port-keyed dedup + cue state for the built-in engine, owned by the app
/// layer so the engine runner stays a pure process actor. `warm_builtin`
/// consults it after `ensure_loaded` resolves the serving port, so at most one
/// prime runs per engine load and the overlay shows the "warming" cue for
/// exactly that window. Keyed on port, not target: a model or context switch
/// forces a new process and a new port, so a port mismatch correctly allows a
/// fresh prime after any restart.
#[derive(Default)]
pub struct BuiltinWarmState {
    inner: std::sync::Mutex<BuiltinWarm>,
}

#[derive(Default)]
struct BuiltinWarm {
    /// Port of a prime currently in flight, if any. Armed by `try_begin`,
    /// cleared by `finish` regardless of outcome so a failed prime can retry.
    in_flight: Option<u16>,
    /// Port whose prime completed successfully. A new process gets a new port,
    /// so a port mismatch allows a fresh prime after a restart.
    primed_port: Option<u16>,
}

impl BuiltinWarmState {
    /// Atomically decides whether to prime the engine on `port`. Returns true
    /// (and arms the in-flight slot) only when no prime is already running for
    /// this port and this port has not already been primed. The two warm
    /// callers (summon + first keystroke) both reach this after `ensure_loaded`
    /// resolves the same reused port, so the loser dedups to a no-op.
    pub fn try_begin(&self, port: u16) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.in_flight == Some(port) || g.primed_port == Some(port) {
            return false;
        }
        g.in_flight = Some(port);
        true
    }

    /// Clears the in-flight slot for `port` and, on success, records the port
    /// as primed so later warm requests for the same load dedup. A `finish`
    /// for a port that no longer owns the slot (engine restarted mid-prime)
    /// leaves the slot untouched.
    pub fn finish(&self, port: u16, success: bool) {
        let mut g = self.inner.lock().unwrap();
        if g.in_flight == Some(port) {
            g.in_flight = None;
        }
        if success {
            g.primed_port = Some(port);
        }
    }

    /// Whether a prime is currently in flight. Seeds the Settings keep-warm
    /// status when the panel mounts during a cold prime (it otherwise learns
    /// the state only from the `warmup:builtin-warming`/`-warmed` events).
    pub fn is_warming(&self) -> bool {
        self.inner.lock().unwrap().in_flight.is_some()
    }
}

/// Built-in arm of `warm_up_model`: starts (or reuses) the engine so the
/// selected model is resident by the time the user submits, then primes the
/// KV cache for the system-prompt prefix. Dedup via [`BuiltinWarmState`]
/// collapses the summon + keystroke warms (and any double-summon) to a single
/// prime per load, so the user's first message never queues behind redundant
/// cold primes. Emits `warmup:builtin-warming` while the prime runs and
/// `warmup:builtin-warmed` when it ends, so the Settings keep-warm status can
/// read "warming…" until the model is actually ready (not just `/health` OK).
/// Best-effort throughout: a superseded load, a dedup skip, or a failed prime
/// is swallowed. Coverage-off: the dedup logic lives in `BuiltinWarmState`
/// and the prime in `prime_builtin`, both tested; this only sequences them.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) async fn warm_builtin(
    app: tauri::AppHandle,
    engine: crate::engine::runner::EngineHandle,
    target: crate::engine::state::Target,
    model_id: String,
    system_prompt: String,
    client: reqwest::Client,
) {
    let Ok(port) = engine.ensure_loaded(target).await else {
        return;
    };
    if !app.state::<BuiltinWarmState>().try_begin(port) {
        return;
    }
    let _ = app.emit("warmup:builtin-warming", ());
    let ok = prime_builtin(port, model_id, system_prompt, client).await;
    app.state::<BuiltinWarmState>().finish(port, ok);
    let _ = app.emit("warmup:builtin-warmed", ());
}

/// Built-in arm of `evict_model`: stops the engine sidecar and resolves once
/// the process exit is confirmed. The `warmup:model-evicted` emit stays in
/// the thin Tauri command because it needs an `AppHandle`.
pub(crate) async fn evict_builtin(engine: &crate::engine::runner::EngineHandle) {
    engine.unload().await;
}

/// Built-in arm of `get_loaded_model`: the display name of the model the engine
/// is *actually* serving, resolved from the live status's `model_path` against
/// `installed` (each entry a `(display_name, weights blob path)` pair), or
/// `None` when the engine is not loaded or the resident blob matches no row.
///
/// This reads true VRAM residency, never the frontend-selected model: switching
/// the active model rewrites config immediately, but the sidecar keeps serving
/// the previous model until a reload, so the configured id would misreport what
/// occupies memory.
pub(crate) fn builtin_loaded_model(
    status: &crate::engine::runner::EngineStatus,
    installed: &[(String, std::path::PathBuf)],
) -> Option<String> {
    if status.state != "loaded" || status.model_path.is_empty() {
        return None;
    }
    let resident = std::path::Path::new(&status.model_path);
    installed
        .iter()
        .find(|(_, path)| path.as_path() == resident)
        .map(|(name, _)| name.clone())
}

impl Default for WarmupState {
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn default() -> Self {
        Self::new()
    }
}

impl WarmupState {
    pub fn new() -> Self {
        Self {
            in_flight: Arc::new(Mutex::new(None)),
            on_loaded: Arc::new(|_| {}),
            evicted: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Constructs a `WarmupState` that calls `cb` with the model name on each
    /// successful warmup. Use this in production; use `new()` in tests.
    pub fn with_on_loaded(cb: Arc<dyn Fn(String) + Send + Sync + 'static>) -> Self {
        Self {
            in_flight: Arc::new(Mutex::new(None)),
            on_loaded: cb,
            evicted: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Marks this state as evicted so any in-flight warmup that completes
    /// after this call will not re-emit `warmup:model-loaded`.
    pub fn mark_evicted(&self) {
        self.evicted.store(true, Ordering::SeqCst);
    }

    /// Fire-and-forget model warm-up. Returns immediately.
    /// No-op if model/endpoint empty or same (model, keep_alive, system_prompt, num_ctx) 4-tuple already in flight.
    /// Any differing field supersedes the in-flight slot and fires a new request.
    /// `keep_alive` is forwarded to Ollama as-is; `None` omits the field so
    /// Ollama uses its server default (typically 5 minutes).
    pub fn fire(
        &self,
        endpoint: String,
        model: String,
        system_prompt: String,
        client: reqwest::Client,
        keep_alive: Option<String>,
        num_ctx: u32,
    ) {
        if model.is_empty() || endpoint.is_empty() {
            return;
        }
        {
            let mut guard = self.in_flight.lock().unwrap();
            if guard.as_ref().map(|(m, k, s, n)| {
                m == &model && k == &keep_alive && s == &system_prompt && *n == num_ctx
            }) == Some(true)
            {
                return;
            }
            *guard = Some((
                model.clone(),
                keep_alive.clone(),
                system_prompt.clone(),
                num_ctx,
            ));
        }
        // A new warmup supersedes any prior eviction.
        self.evicted.store(false, Ordering::SeqCst);
        let in_flight = Arc::clone(&self.in_flight);
        let on_loaded = Arc::clone(&self.on_loaded);
        let evicted = Arc::clone(&self.evicted);
        tauri::async_runtime::spawn(run_warmup(
            endpoint,
            model,
            system_prompt,
            client,
            in_flight,
            keep_alive,
            num_ctx,
            on_loaded,
            evicted,
        ));
    }
}

#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
#[allow(clippy::too_many_arguments)]
pub fn warm_up_model(
    app: tauri::AppHandle,
    warmup: tauri::State<WarmupState>,
    models: tauri::State<crate::models::ActiveModelState>,
    config: tauri::State<parking_lot::RwLock<crate::config::AppConfig>>,
    client: tauri::State<reqwest::Client>,
    engine: tauri::State<crate::engine::runner::EngineHandle>,
    db: tauri::State<crate::history::Database>,
    store: tauri::State<crate::models::storage::ModelStore>,
) {
    let kind = config.read().inference.active_provider_kind().to_string();
    match kind.as_str() {
        PROVIDER_KIND_OLLAMA => {
            let model = models.0.lock().ok().and_then(|g| g.clone());
            if let Some(model) = model {
                let cfg = config.read();
                let endpoint = format!(
                    "{}/api/chat",
                    cfg.inference
                        .active_provider_base_url()
                        .trim_end_matches('/')
                );
                let system_prompt = cfg.prompt.resolved_system.clone();
                let keep_alive = if cfg.inference.keep_warm_inactivity_minutes == 0 {
                    None
                } else {
                    Some(keep_alive_string(
                        cfg.inference.keep_warm_inactivity_minutes,
                    ))
                };
                let num_ctx = cfg.inference.num_ctx;
                drop(cfg);
                warmup.fire(
                    endpoint,
                    model,
                    system_prompt,
                    client.inner().clone(),
                    keep_alive,
                    num_ctx,
                );
            }
        }
        PROVIDER_KIND_BUILTIN => {
            let (model_id, num_ctx, system_prompt) = {
                let cfg = config.read();
                (
                    cfg.inference.active_provider_model().to_string(),
                    cfg.inference.num_ctx,
                    cfg.prompt.resolved_system.clone(),
                )
            };
            if !builtin_should_warm(&model_id) {
                return;
            }
            // Resolve the manifest row to an engine Target inside a scope so the
            // connection guard drops before the spawned load. A poisoned lock is
            // recovered: an unrelated panic does not invalidate the connection.
            let target = {
                let conn = match db.0.lock() {
                    Ok(conn) => conn,
                    Err(poisoned) => poisoned.into_inner(),
                };
                crate::commands::builtin_target(&conn, &store, &model_id, num_ctx)
            };
            // A missing/uninstalled model yields an Err; warmup is best-effort,
            // so just skip rather than surfacing anything.
            if let Ok(target) = target {
                tauri::async_runtime::spawn(warm_builtin(
                    app,
                    engine.inner().clone(),
                    target,
                    model_id,
                    system_prompt,
                    client.inner().clone(),
                ));
            }
        }
        _ => {}
    }
}

/// Core logic for checking whether a specific model is currently loaded in
/// Ollama's VRAM. Queries `/api/ps` and returns `Ok(Some(slug))` if the
/// model appears in the running list, `Ok(None)` if not present or the list
/// is empty, and `Err` on network failure.
pub(crate) async fn get_loaded_model_request(
    endpoint: &str,
    model: &str,
    client: &reqwest::Client,
) -> Result<Option<String>, String> {
    let resp = client
        .get(endpoint)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let found = body
        .get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.get("name").and_then(|n| n.as_str()))
                .any(|name| normalize_slug(name) == normalize_slug(model))
        })
        .unwrap_or(false);

    Ok(if found { Some(model.to_string()) } else { None })
}

/// Returns the engine runner's current lifecycle snapshot, the same payload
/// the `engine:status` event carries. The Settings panel calls this on mount
/// to seed its residency line: the backend emits `engine:status` only on
/// transitions, so without this query an already-loaded engine would read as
/// "stopped" (and Unload now would stay disabled) until the next transition.
/// Thin wrapper over [`crate::engine::runner::EngineHandle::current_status`],
/// which the runner tests cover.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn get_engine_status(
    engine: tauri::State<'_, crate::engine::runner::EngineHandle>,
) -> crate::engine::runner::EngineStatus {
    engine.current_status()
}

/// True while the built-in engine is priming (loaded but the system-prompt
/// prefill has not finished). The Settings keep-warm panel calls this on mount
/// to seed its "warming…" status, since the `warmup:builtin-warming` event it
/// otherwise relies on may have fired before the panel attached its listener.
/// Thin wrapper over [`BuiltinWarmState::is_warming`], which its own tests cover.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn get_builtin_warm_state(warm: tauri::State<'_, BuiltinWarmState>) -> bool {
    warm.is_warming()
}

/// Returns the active model's name if it is currently loaded, `None` if no
/// model is selected or nothing is running. Branches by the active provider's
/// kind: Ollama queries `/api/ps`, the built-in engine reads its own status
/// watch, and `openai` providers always report `None` (there is no local
/// memory to observe).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn get_loaded_model(
    models: tauri::State<'_, crate::models::ActiveModelState>,
    config: tauri::State<'_, parking_lot::RwLock<crate::config::AppConfig>>,
    client: tauri::State<'_, reqwest::Client>,
    engine: tauri::State<'_, crate::engine::runner::EngineHandle>,
    db: tauri::State<'_, crate::history::Database>,
    store: tauri::State<'_, crate::models::storage::ModelStore>,
) -> Result<Option<String>, String> {
    let kind = config.read().inference.active_provider_kind().to_string();
    match kind.as_str() {
        PROVIDER_KIND_BUILTIN => {
            let status = engine.status().borrow().clone();
            // Resolve the engine's resident blob back to its installed name. A
            // poisoned lock is recovered: an unrelated panic must not blind the
            // residency line.
            let installed = {
                let conn = match db.0.lock() {
                    Ok(conn) => conn,
                    Err(poisoned) => poisoned.into_inner(),
                };
                crate::models::manifest::list(&conn)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|m| (m.display_name, store.blob_path(&m.sha256)))
                    .collect::<Vec<_>>()
            };
            Ok(builtin_loaded_model(&status, &installed))
        }
        PROVIDER_KIND_OLLAMA => {
            let model = models.0.lock().ok().and_then(|g| g.clone());
            if let Some(model) = model {
                let endpoint = format!(
                    "{}/api/ps",
                    config
                        .read()
                        .inference
                        .active_provider_base_url()
                        .trim_end_matches('/')
                );
                get_loaded_model_request(&endpoint, &model, client.inner()).await
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

/// Core logic for evicting the active model from Ollama's VRAM. Sends a
/// `/api/generate` request with `keep_alive: "0"` which tells Ollama to evict
/// the model immediately regardless of the configured TTL.
pub(crate) async fn evict_model_request(
    endpoint: &str,
    model: &str,
    client: &reqwest::Client,
) -> Result<(), String> {
    let body = serde_json::json!({
        "model": model,
        "keep_alive": "0",
        "prompt": "",
        "stream": false
    });
    client
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Unloads the active model from local memory immediately. Branches by the
/// active provider's kind: Ollama gets the `/api/generate keep_alive:"0"`
/// request, the built-in engine unloads its sidecar process, and `openai`
/// providers are a no-op (there is no local memory to release).
///
/// The Ollama arm delegates to `evict_model_request`; returns an error string
/// on failure so the frontend can react (e.g. reset the eject button state).
/// Emits `warmup:model-evicted` on success so the Settings panel updates live.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn evict_model(
    app_handle: tauri::AppHandle,
    warmup: tauri::State<'_, WarmupState>,
    models: tauri::State<'_, crate::models::ActiveModelState>,
    config: tauri::State<'_, parking_lot::RwLock<crate::config::AppConfig>>,
    client: tauri::State<'_, reqwest::Client>,
    engine: tauri::State<'_, crate::engine::runner::EngineHandle>,
) -> Result<(), String> {
    let kind = config.read().inference.active_provider_kind().to_string();
    match kind.as_str() {
        PROVIDER_KIND_BUILTIN => {
            // No mark_evicted() here: the WarmupState in-flight slot is only
            // armed by fire(), which is never called for builtin providers.
            // There is no Ollama-era warmup callback to suppress.
            evict_builtin(&engine).await;
            let _ = app_handle.emit("warmup:model-evicted", ());
        }
        PROVIDER_KIND_OLLAMA => {
            let model = models.0.lock().ok().and_then(|g| g.clone());
            if let Some(model) = model {
                let endpoint = format!(
                    "{}/api/generate",
                    config
                        .read()
                        .inference
                        .active_provider_base_url()
                        .trim_end_matches('/')
                );
                evict_model_request(&endpoint, &model, client.inner()).await?;
                // Suppress any in-flight warmup callback so a slow warmup that
                // completes after the eviction request does not re-announce the model.
                warmup.mark_evicted();
                let _ = app_handle.emit("warmup:model-evicted", ());
            }
        }
        _ => {}
    }
    Ok(())
}

/// Spawns a background Tokio task that polls Ollama's `/api/ps` every
/// `VRAM_POLL_INTERVAL_SECS` seconds and emits `warmup:model-loaded` or
/// `warmup:model-evicted` when external VRAM changes are detected. Catches
/// changes Thuki did not initiate: `ollama stop`, TTL expiry, daemon restart.
/// The first tick is skipped to avoid a spurious `Evicted` event at startup
/// before the model has had a chance to warm up.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn spawn_vram_poller(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker =
            tokio::time::interval(std::time::Duration::from_secs(VRAM_POLL_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await; // skip first tick

        let mut prev: Option<String> = None;

        loop {
            ticker.tick().await;

            // The poller is Ollama-specific: skip the tick entirely (no HTTP
            // call) while any other provider kind is active. `prev` is left
            // untouched so a later switch back to Ollama resumes transition
            // detection from the last observed Ollama state.
            let kind = app_handle
                .state::<parking_lot::RwLock<crate::config::AppConfig>>()
                .read()
                .inference
                .active_provider_kind()
                .to_string();
            if !vram_poll_active(&kind) {
                continue;
            }

            let model = app_handle
                .state::<crate::models::ActiveModelState>()
                .0
                .lock()
                .ok()
                .and_then(|g| g.clone());

            let current = match model {
                None => None,
                Some(ref m) => {
                    let endpoint = format!(
                        "{}/api/ps",
                        app_handle
                            .state::<parking_lot::RwLock<crate::config::AppConfig>>()
                            .read()
                            .inference
                            .active_provider_base_url()
                            .trim_end_matches('/')
                    );
                    let client = app_handle.state::<reqwest::Client>().inner().clone();
                    get_loaded_model_request(&endpoint, m, &client)
                        .await
                        .unwrap_or(None)
                }
            };

            match detect_vram_transition(&prev, &current) {
                VramTransition::Loaded(ref slug) => {
                    let _ = app_handle.emit("warmup:model-loaded", slug);
                }
                VramTransition::Evicted => {
                    let _ = app_handle.emit("warmup:model-evicted", ());
                }
                VramTransition::None => {}
            }

            prev = current;
        }
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_warmup(
    endpoint: String,
    model: String,
    system_prompt: String,
    client: reqwest::Client,
    in_flight: InFlightSlot,
    keep_alive: Option<String>,
    num_ctx: u32,
    on_loaded: OnLoaded,
    evicted: Arc<AtomicBool>,
) {
    // Use /api/chat with the resolved system prompt so Ollama primes the KV cache
    // for the prefix the real chat will share. num_predict:1 generates exactly one
    // token: enough to complete the prefill phase (which warms the KV cache and
    // Metal shaders) while releasing the queue in ~200-400ms. num_predict:0 means
    // infinite generation in Ollama's runner, which blocks the queue for seconds.
    //
    // num_ctx MUST match the value sent by real chat requests. The default Ollama
    // context (4 096 tokens) is almost entirely consumed by the system prompt
    // (~4 000 tokens), leaving no room for KV-cache prefix reuse. Using 16 384
    // ensures the system prompt prefix is cached and reused on every subsequent
    // turn. think:false matches the chat template rendered by real requests so
    // Ollama sees the same formatted token sequence and reuses the same runner.
    let messages = serde_json::json!([
        {"role": "system", "content": system_prompt},
        {"role": "user", "content": ""}
    ]);
    let options = serde_json::json!({"num_predict": 1, "num_ctx": num_ctx});

    let body = if let Some(ref ka) = keep_alive {
        serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "think": false,
            "options": options,
            "keep_alive": ka
        })
    } else {
        serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "think": false,
            "options": options
        })
    };

    match client.post(&endpoint).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            if !evicted.load(Ordering::SeqCst) {
                (on_loaded)(model.clone());
            }
        }
        Ok(_) => {}
        Err(_) => {}
    }

    let mut guard = in_flight.lock().unwrap();
    if guard
        .as_ref()
        .map(|(m, k, s, n)| m == &model && k == &keep_alive && s == &system_prompt && *n == num_ctx)
        == Some(true)
    {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::defaults::DEFAULT_NUM_CTX;
    use mockito::Server;
    use std::time::{Duration, Instant};

    const SYS: &str = "You are a helpful assistant.";

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn wait_in_flight_clear(in_flight: &InFlightSlot, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while in_flight.lock().unwrap().is_some() {
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[tokio::test]
    async fn success_resets_in_flight() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/chat", server.url()),
            "llama3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            None,
            DEFAULT_NUM_CTX,
        );

        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        assert!(in_flight.lock().unwrap().is_none());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn http_error_resets_in_flight() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(500)
            .with_body("internal error")
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/chat", server.url()),
            "llama3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            None,
            DEFAULT_NUM_CTX,
        );

        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        assert!(in_flight.lock().unwrap().is_none());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn connection_refused_resets_in_flight() {
        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            "http://127.0.0.1:1/api/chat".to_string(),
            "llama3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            None,
            DEFAULT_NUM_CTX,
        );

        wait_in_flight_clear(&in_flight, Duration::from_secs(10));
        assert!(in_flight.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn same_model_dedup() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body("{}")
            .expect(1)
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/chat", server.url());

        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            SYS.to_string(),
            client.clone(),
            None,
            DEFAULT_NUM_CTX,
        );
        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            SYS.to_string(),
            client.clone(),
            None,
            DEFAULT_NUM_CTX,
        );

        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn different_model_fires_new_request() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body("{}")
            .expect(2)
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/chat", server.url());

        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            SYS.to_string(),
            client.clone(),
            None,
            DEFAULT_NUM_CTX,
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        state.fire(
            endpoint.clone(),
            "phi3".to_string(),
            SYS.to_string(),
            client.clone(),
            None,
            DEFAULT_NUM_CTX,
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn different_model_supersedes_in_flight() {
        // Simulate model A in flight; firing model B should still proceed.
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body("{}")
            .expect(1)
            .create_async()
            .await;

        let state = WarmupState::new();
        // Manually mark model A as in flight.
        *state.in_flight.lock().unwrap() =
            Some(("llama3".to_string(), None, SYS.to_string(), DEFAULT_NUM_CTX));

        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/chat", server.url()),
            "phi3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            None,
            DEFAULT_NUM_CTX,
        );

        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        assert!(in_flight.lock().unwrap().is_none());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn task_clears_only_own_slot() {
        // Simulate: in_flight = Some(("llama3", None, SYS)), task for "phi3" completes.
        // "phi3" task must NOT clear the "llama3" slot.
        let in_flight: InFlightSlot = Arc::new(Mutex::new(Some((
            "llama3".to_string(),
            None,
            SYS.to_string(),
            DEFAULT_NUM_CTX,
        ))));

        // Reuse the no-op callback from WarmupState::new() to share its Fn implementation
        // and avoid an uncovered closure in this connection-refused (no-success) test.
        let state = WarmupState::new();
        let noop = Arc::clone(&state.on_loaded);
        let not_evicted = Arc::clone(&state.evicted);
        run_warmup(
            "http://127.0.0.1:1/api/chat".to_string(),
            "phi3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            Arc::clone(&in_flight),
            None,
            DEFAULT_NUM_CTX,
            noop,
            not_evicted,
        )
        .await;

        assert_eq!(
            in_flight
                .lock()
                .unwrap()
                .as_ref()
                .map(|(m, _, _, _)| m.as_str()),
            Some("llama3"),
            "phi3 task must not clear slot held by llama3"
        );
    }

    #[tokio::test]
    async fn empty_model_no_request() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .expect(0)
            .create_async()
            .await;

        let state = WarmupState::new();
        state.fire(
            format!("{}/api/chat", server.url()),
            String::new(),
            SYS.to_string(),
            reqwest::Client::new(),
            None,
            DEFAULT_NUM_CTX,
        );

        tokio::time::sleep(Duration::from_millis(100)).await;
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn empty_endpoint_no_request() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .expect(0)
            .create_async()
            .await;

        let state = WarmupState::new();
        state.fire(
            String::new(),
            "llama3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            None,
            DEFAULT_NUM_CTX,
        );

        tokio::time::sleep(Duration::from_millis(100)).await;
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn request_body_shape_no_keep_alive() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"model":"llama3","messages":[{"role":"system","content":"You are a helpful assistant."},{"role":"user","content":""}],"stream":false,"options":{"num_predict":1}}"#.to_string(),
            ))
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/chat", server.url()),
            "llama3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            None,
            DEFAULT_NUM_CTX,
        );

        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn request_body_shape_with_keep_alive() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"model":"llama3","messages":[{"role":"system","content":"You are a helpful assistant."},{"role":"user","content":""}],"stream":false,"options":{"num_predict":1},"keep_alive":"30m"}"#.to_string(),
            ))
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/chat", server.url()),
            "llama3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            Some("30m".to_string()),
            DEFAULT_NUM_CTX,
        );

        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn request_body_includes_num_ctx_and_think_false() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .match_body(mockito::Matcher::PartialJsonString(format!(
                r#"{{"think":false,"options":{{"num_predict":1,"num_ctx":{}}}}}"#,
                DEFAULT_NUM_CTX
            )))
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/chat", server.url()),
            "llama3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            None,
            DEFAULT_NUM_CTX,
        );

        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn same_model_different_keep_alive_fires_new_request() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body("{}")
            .expect(2)
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/chat", server.url());

        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            SYS.to_string(),
            client.clone(),
            None,
            DEFAULT_NUM_CTX,
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            SYS.to_string(),
            client.clone(),
            Some("30m".to_string()),
            DEFAULT_NUM_CTX,
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn same_model_different_system_prompt_fires_new_request() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body("{}")
            .expect(2)
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/chat", server.url());

        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            "prompt A".to_string(),
            client.clone(),
            None,
            DEFAULT_NUM_CTX,
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            "prompt B".to_string(),
            client.clone(),
            None,
            DEFAULT_NUM_CTX,
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn same_model_different_num_ctx_fires_new_request() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body("{}")
            .expect(2)
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/chat", server.url());

        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            SYS.to_string(),
            client.clone(),
            None,
            DEFAULT_NUM_CTX,
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            SYS.to_string(),
            client.clone(),
            None,
            DEFAULT_NUM_CTX * 2,
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));

        mock.assert_async().await;
    }

    // ── normalize_slug ───────────────────────────────────────────────────────

    #[test]
    fn normalize_slug_strips_latest() {
        assert_eq!(normalize_slug("llama3:latest"), "llama3");
    }

    #[test]
    fn normalize_slug_leaves_other_tags_intact() {
        assert_eq!(normalize_slug("llama3:8b"), "llama3:8b");
    }

    #[test]
    fn normalize_slug_no_tag_unchanged() {
        assert_eq!(normalize_slug("llama3"), "llama3");
    }

    #[test]
    fn normalize_slug_case_sensitive_latest() {
        assert_eq!(normalize_slug("llama3:LATEST"), "llama3:LATEST");
    }

    #[test]
    fn normalize_slug_ignores_nested_latest() {
        assert_eq!(normalize_slug("llama3:latest:extra"), "llama3:latest:extra");
    }

    // ── detect_vram_transition ───────────────────────────────────────────────

    #[test]
    fn detect_transition_none_to_none() {
        assert_eq!(detect_vram_transition(&None, &None), VramTransition::None);
    }

    #[test]
    fn detect_transition_none_to_loaded() {
        assert_eq!(
            detect_vram_transition(&None, &Some("llama3".to_string())),
            VramTransition::Loaded("llama3".to_string())
        );
    }

    #[test]
    fn detect_transition_loaded_to_evicted() {
        assert_eq!(
            detect_vram_transition(&Some("llama3".to_string()), &None),
            VramTransition::Evicted
        );
    }

    #[test]
    fn detect_transition_same_model_no_change() {
        assert_eq!(
            detect_vram_transition(&Some("llama3".to_string()), &Some("llama3".to_string())),
            VramTransition::None
        );
    }

    #[test]
    fn detect_transition_model_switch() {
        assert_eq!(
            detect_vram_transition(&Some("llama3".to_string()), &Some("phi3".to_string())),
            VramTransition::Loaded("phi3".to_string())
        );
    }

    // ── get_loaded_model_request slug normalization ──────────────────────────

    #[tokio::test]
    async fn get_loaded_model_request_stored_without_latest_matches_ollama_with_latest() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/api/ps")
            .with_status(200)
            .with_body(r#"{"models":[{"name":"llama3:latest"}]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/ps", server.url());
        // Stored as "llama3", Ollama returns "llama3:latest" — should match.
        let result = get_loaded_model_request(&endpoint, "llama3", &client).await;
        assert_eq!(result, Ok(Some("llama3".to_string())));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn get_loaded_model_request_stored_with_latest_matches_ollama_without_latest() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/api/ps")
            .with_status(200)
            .with_body(r#"{"models":[{"name":"llama3"}]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/ps", server.url());
        // Stored as "llama3:latest", Ollama returns "llama3" — should match.
        let result = get_loaded_model_request(&endpoint, "llama3:latest", &client).await;
        assert_eq!(result, Ok(Some("llama3:latest".to_string())));
        mock.assert_async().await;
    }

    #[test]
    fn keep_alive_string_minutes() {
        assert_eq!(keep_alive_string(30), "30m");
        assert_eq!(keep_alive_string(1), "1m");
        assert_eq!(keep_alive_string(1440), "1440m");
    }

    #[test]
    fn keep_alive_string_never() {
        assert_eq!(keep_alive_string(-1), "-1");
    }

    #[test]
    fn builtin_idle_minutes_forever_disables_timer() {
        // -1 (keep resident forever) maps to the runner's "0 = disabled".
        assert_eq!(builtin_idle_minutes(-1), 0);
    }

    #[test]
    fn builtin_idle_minutes_zero_uses_short_default() {
        // 0 (natural short default) maps to the baked-in ~5-minute timer.
        assert_eq!(
            builtin_idle_minutes(0),
            crate::config::defaults::DEFAULT_BUILTIN_IDLE_MINUTES
        );
    }

    #[test]
    fn builtin_idle_minutes_positive_passes_through() {
        assert_eq!(builtin_idle_minutes(30), 30);
        assert_eq!(builtin_idle_minutes(1), 1);
        assert_eq!(builtin_idle_minutes(1440), 1440);
    }

    #[test]
    fn builtin_idle_minutes_below_minus_one_disables_timer() {
        // Out-of-contract once the loader clamps; the total match still maps
        // any stray negative to the "forever" disable rather than a timer.
        assert_eq!(builtin_idle_minutes(-999), 0);
    }

    #[tokio::test]
    async fn evict_model_request_sends_keep_alive_zero_as_string() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"keep_alive":"0","prompt":"","stream":false}"#.to_string(),
            ))
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/generate", server.url());

        evict_model_request(&endpoint, "llama3", &client)
            .await
            .expect("evict should succeed");

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn evict_model_request_returns_error_on_connection_refused() {
        let client = reqwest::Client::new();
        let result =
            evict_model_request("http://127.0.0.1:1/api/generate", "llama3", &client).await;
        assert!(result.is_err(), "connection refused should return Err");
    }

    #[tokio::test]
    async fn get_loaded_model_request_found_returns_some() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/api/ps")
            .with_status(200)
            .with_body(r#"{"models":[{"name":"llama3.2:3b","model":"llama3.2:3b"}]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/ps", server.url());
        let result = get_loaded_model_request(&endpoint, "llama3.2:3b", &client).await;
        assert_eq!(result, Ok(Some("llama3.2:3b".to_string())));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn get_loaded_model_request_not_found_returns_none() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/api/ps")
            .with_status(200)
            .with_body(r#"{"models":[{"name":"phi3:mini","model":"phi3:mini"}]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/ps", server.url());
        let result = get_loaded_model_request(&endpoint, "llama3.2:3b", &client).await;
        assert_eq!(result, Ok(None));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn get_loaded_model_request_empty_models_returns_none() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/api/ps")
            .with_status(200)
            .with_body(r#"{"models":[]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/ps", server.url());
        let result = get_loaded_model_request(&endpoint, "llama3.2:3b", &client).await;
        assert_eq!(result, Ok(None));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn get_loaded_model_request_http_error_returns_none() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/api/ps")
            .with_status(503)
            .with_body("{}")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/ps", server.url());
        let result = get_loaded_model_request(&endpoint, "llama3.2:3b", &client).await;
        assert_eq!(result, Ok(None));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn get_loaded_model_request_connection_refused_returns_err() {
        let client = reqwest::Client::new();
        let result =
            get_loaded_model_request("http://127.0.0.1:1/api/ps", "llama3.2:3b", &client).await;
        assert!(result.is_err(), "connection refused should return Err");
    }

    #[tokio::test]
    async fn get_loaded_model_request_invalid_json_returns_err() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/api/ps")
            .with_status(200)
            .with_body("not valid json")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/ps", server.url());
        let result = get_loaded_model_request(&endpoint, "llama3.2:3b", &client).await;
        assert!(result.is_err(), "invalid JSON body should return Err");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn get_loaded_model_request_multiple_models_finds_correct() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/api/ps")
            .with_status(200)
            .with_body(
                r#"{"models":[{"name":"phi3:mini"},{"name":"llama3.2:3b"},{"name":"gemma:2b"}]}"#,
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/ps", server.url());
        let result = get_loaded_model_request(&endpoint, "llama3.2:3b", &client).await;
        assert_eq!(result, Ok(Some("llama3.2:3b".to_string())));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn on_loaded_callback_fires_on_success() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let fired: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let fired_clone = Arc::clone(&fired);
        let state = WarmupState::with_on_loaded(Arc::new(move |model| {
            fired_clone.lock().unwrap().push(model);
        }));
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/chat", server.url()),
            "llama3.2:3b".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            None,
            DEFAULT_NUM_CTX,
        );

        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        mock.assert_async().await;
        assert_eq!(*fired.lock().unwrap(), vec!["llama3.2:3b".to_string()]);
    }

    #[test]
    fn mark_evicted_sets_flag() {
        let state = WarmupState::new();
        assert!(!state.evicted.load(Ordering::SeqCst), "flag starts false");
        state.mark_evicted();
        assert!(
            state.evicted.load(Ordering::SeqCst),
            "mark_evicted must set flag"
        );
    }

    #[tokio::test]
    async fn eviction_suppresses_on_loaded_callback() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        // Use the noop callback so no uncovered closure body exists.
        // on_loaded_callback_fires_on_success covers the evicted=false branch;
        // this test covers the evicted=true branch of `if !evicted.load(...)`.
        let state = WarmupState::new();
        let on_loaded = Arc::clone(&state.on_loaded);
        let in_flight: InFlightSlot = Arc::new(Mutex::new(Some((
            "llama3.2:3b".to_string(),
            None,
            SYS.to_string(),
            DEFAULT_NUM_CTX,
        ))));
        let evicted = Arc::new(AtomicBool::new(true));

        run_warmup(
            format!("{}/api/chat", server.url()),
            "llama3.2:3b".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            Arc::clone(&in_flight),
            None,
            DEFAULT_NUM_CTX,
            on_loaded,
            evicted,
        )
        .await;

        mock.assert_async().await;
        assert!(
            in_flight.lock().unwrap().is_none(),
            "slot clears even when eviction suppresses the callback"
        );
    }

    // ── Provider-kind branching ──────────────────────────────────────────────

    #[test]
    fn vram_poller_tick_skips_non_ollama() {
        assert!(vram_poll_active("ollama"), "ollama keeps polling /api/ps");
        assert!(!vram_poll_active("builtin"), "builtin must not hit Ollama");
        assert!(!vram_poll_active("openai"), "openai has no VRAM to observe");
        assert!(!vram_poll_active(""), "unresolved kind must not poll");
    }

    /// EngineStatus literal for the prime/loaded-model decision tests.
    fn engine_status(state: &str, port: Option<u16>) -> crate::engine::runner::EngineStatus {
        crate::engine::runner::EngineStatus {
            state: state.to_string(),
            model_path: String::new(),
            port,
            error: None,
        }
    }

    #[test]
    fn builtin_should_warm_requires_a_selected_model() {
        assert!(
            !builtin_should_warm(""),
            "no picked model means nothing to warm-load"
        );
        assert!(
            builtin_should_warm("org/repo:m.gguf"),
            "a selected model warms the engine on the chat-intent signal"
        );
    }

    #[tokio::test]
    async fn builtin_prime_request_hits_v1_with_max_tokens_1() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"model":"org/repo:m.gguf","messages":[{"role":"system","content":"You are a helpful assistant."},{"role":"user","content":"ok"}],"max_tokens":1,"stream":false}"#.to_string(),
            ))
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let port: u16 = server
            .url()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .expect("mockito url ends in a port");
        let ok = prime_builtin(
            port,
            "org/repo:m.gguf".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
        )
        .await;

        assert!(
            ok,
            "a 200 prime reports success so the load is marked primed"
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn builtin_prime_swallows_connection_error() {
        // Port 1 refuses; prime is best-effort and must not panic, exercising
        // the transport-error path of the status capture.
        let ok = prime_builtin(
            1,
            "org/repo:m.gguf".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
        )
        .await;

        assert!(
            !ok,
            "a transport failure reports not-primed so a later warm retries"
        );
    }

    // ── BuiltinWarmState (port-keyed dedup) ──────────────────────────────────

    #[test]
    fn warm_state_first_call_begins_then_dedups_in_flight() {
        let s = BuiltinWarmState::default();
        assert!(s.try_begin(40000), "first call for a port arms the prime");
        assert!(
            !s.try_begin(40000),
            "a second call while the prime is in flight dedups to a no-op"
        );
    }

    #[test]
    fn warm_state_failed_prime_allows_retry() {
        let s = BuiltinWarmState::default();
        assert!(s.try_begin(40000));
        s.finish(40000, false);
        assert!(
            s.try_begin(40000),
            "a failed prime leaves the port un-primed so a later warm retries"
        );
    }

    #[test]
    fn warm_state_successful_prime_dedups_same_port() {
        let s = BuiltinWarmState::default();
        assert!(s.try_begin(40000));
        s.finish(40000, true);
        assert!(
            !s.try_begin(40000),
            "a primed port dedups later warms for the same load"
        );
    }

    #[test]
    fn warm_state_new_port_primes_again_after_success() {
        let s = BuiltinWarmState::default();
        assert!(s.try_begin(40000));
        s.finish(40000, true);
        assert!(
            s.try_begin(40001),
            "a new process/port (restart or model switch) primes fresh"
        );
    }

    #[test]
    fn warm_state_finish_for_unowned_port_leaves_slot_armed() {
        let s = BuiltinWarmState::default();
        assert!(s.try_begin(40000));
        // The engine restarted mid-prime: a finish for a different port must not
        // clear the slot the live prime still owns, but still records its success.
        s.finish(40001, true);
        assert!(
            !s.try_begin(40000),
            "the in-flight slot for 40000 is untouched by finish(40001)"
        );
        assert!(
            !s.try_begin(40001),
            "finish(40001, true) still recorded 40001 as primed"
        );
    }

    #[test]
    fn warm_state_is_warming_tracks_in_flight() {
        let s = BuiltinWarmState::default();
        assert!(!s.is_warming(), "nothing is in flight at rest");
        assert!(s.try_begin(40000));
        assert!(s.is_warming(), "a begun prime reports warming");
        s.finish(40000, true);
        assert!(!s.is_warming(), "a finished prime is no longer warming");
    }

    #[test]
    fn builtin_loaded_model_names_the_resident_blob_not_the_selection() {
        use std::path::PathBuf;
        let resident = PathBuf::from("/blobs/sha_mistral");
        let installed = vec![
            ("Gemma 4 12B".to_string(), PathBuf::from("/blobs/sha_gemma")),
            ("Mistral Nemo 12B".to_string(), resident.clone()),
        ];

        // Loaded: the engine is serving the Mistral blob, so the resident model
        // is named from the live `model_path`, independent of any selection.
        let mut loaded = engine_status("loaded", Some(40123));
        loaded.model_path = resident.display().to_string();
        assert_eq!(
            builtin_loaded_model(&loaded, &installed),
            Some("Mistral Nemo 12B".to_string())
        );

        // Not loaded: nothing is resident even if a path lingers in the status.
        let mut stopped = engine_status("stopped", None);
        stopped.model_path = resident.display().to_string();
        assert_eq!(builtin_loaded_model(&stopped, &installed), None);

        // Loaded but the resident blob matches no installed row: report nothing
        // rather than guessing a name.
        let mut orphan = engine_status("loaded", Some(40123));
        orphan.model_path = "/blobs/sha_unknown".to_string();
        assert_eq!(builtin_loaded_model(&orphan, &installed), None);

        // Loaded with an empty path (defensive): nothing to name.
        assert_eq!(
            builtin_loaded_model(&engine_status("loaded", Some(40123)), &installed),
            None
        );
    }

    // ── evict_builtin against a scripted engine ──────────────────────────────

    /// Minimal scriptable engine process: spawns instantly and answers every
    /// health probe with 200, so `ensure_loaded` resolves without a real
    /// llama-server.
    struct InstantEngineProcess;

    struct InstantChild {
        exit_tx: tokio::sync::watch::Sender<bool>,
        exit_rx: tokio::sync::watch::Receiver<bool>,
    }

    #[async_trait::async_trait]
    impl crate::engine::process::EngineChild for InstantChild {
        async fn wait_exit(&mut self) {
            let _ = self.exit_rx.wait_for(|exited| *exited).await;
        }
        async fn kill(&mut self) {
            let _ = self.exit_tx.send(true);
        }
        fn stderr_tail(&self) -> String {
            String::new()
        }
    }

    #[test]
    fn instant_child_has_no_stderr_tail() {
        let (exit_tx, exit_rx) = tokio::sync::watch::channel(false);
        let child = InstantChild { exit_tx, exit_rx };
        assert_eq!(crate::engine::process::EngineChild::stderr_tail(&child), "");
    }

    #[async_trait::async_trait]
    impl crate::engine::process::EngineProcess for InstantEngineProcess {
        async fn spawn(
            &self,
            _args: &crate::engine::process::SpawnArgs,
        ) -> Result<Box<dyn crate::engine::process::EngineChild>, String> {
            let (exit_tx, exit_rx) = tokio::sync::watch::channel(false);
            Ok(Box::new(InstantChild { exit_tx, exit_rx }))
        }
        fn free_port(&self) -> Result<u16, String> {
            Ok(40123)
        }
        async fn health_probe(&self, _port: u16) -> Result<u16, String> {
            Ok(200)
        }
    }

    #[tokio::test]
    async fn evict_on_builtin_calls_runner_unload() {
        let engine = crate::engine::runner::EngineHandle::spawn(
            Arc::new(InstantEngineProcess),
            0,
            Duration::from_secs(3600),
        );
        engine
            .ensure_loaded(crate::engine::state::Target {
                model_path: std::path::PathBuf::from("/tmp/m.gguf"),
                mmproj_path: None,
                num_ctx: DEFAULT_NUM_CTX,
            })
            .await
            .expect("scripted engine loads");
        assert_eq!(engine.status().borrow().state, "loaded");

        evict_builtin(&engine).await;

        assert_eq!(engine.status().borrow().state, "stopped");
        engine.shutdown().await;
    }
}
