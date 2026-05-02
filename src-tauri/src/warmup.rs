use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, Manager};

use crate::config::defaults::VRAM_POLL_INTERVAL_SECS;

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
pub fn warm_up_model(
    warmup: tauri::State<WarmupState>,
    models: tauri::State<crate::models::ActiveModelState>,
    config: tauri::State<parking_lot::RwLock<crate::config::AppConfig>>,
    client: tauri::State<reqwest::Client>,
) {
    let model = models.0.lock().ok().and_then(|g| g.clone());
    if let Some(model) = model {
        let cfg = config.read();
        let endpoint = format!(
            "{}/api/chat",
            cfg.inference.ollama_url.trim_end_matches('/')
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

/// Returns the active model's name if it is currently loaded in Ollama's VRAM,
/// `None` if no model is selected or the selected model is not running.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn get_loaded_model(
    models: tauri::State<'_, crate::models::ActiveModelState>,
    config: tauri::State<'_, parking_lot::RwLock<crate::config::AppConfig>>,
    client: tauri::State<'_, reqwest::Client>,
) -> Result<Option<String>, String> {
    let model = models.0.lock().ok().and_then(|g| g.clone());
    if let Some(model) = model {
        let endpoint = format!(
            "{}/api/ps",
            config.read().inference.ollama_url.trim_end_matches('/')
        );
        get_loaded_model_request(&endpoint, &model, client.inner()).await
    } else {
        Ok(None)
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

/// Unloads the active model from Ollama's VRAM immediately.
///
/// Delegates to `evict_model_request`; returns an error string on failure so
/// the frontend can react (e.g. reset the eject button state). Emits
/// `warmup:model-evicted` on success so the Settings panel updates live.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn evict_model(
    app_handle: tauri::AppHandle,
    warmup: tauri::State<'_, WarmupState>,
    models: tauri::State<'_, crate::models::ActiveModelState>,
    config: tauri::State<'_, parking_lot::RwLock<crate::config::AppConfig>>,
    client: tauri::State<'_, reqwest::Client>,
) -> Result<(), String> {
    let model = models.0.lock().ok().and_then(|g| g.clone());
    if let Some(model) = model {
        let endpoint = format!(
            "{}/api/generate",
            config.read().inference.ollama_url.trim_end_matches('/')
        );
        evict_model_request(&endpoint, &model, client.inner()).await?;
        // Suppress any in-flight warmup callback so a slow warmup that
        // completes after the eviction request does not re-announce the model.
        warmup.mark_evicted();
        let _ = app_handle.emit("warmup:model-evicted", ());
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
                            .ollama_url
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
}
