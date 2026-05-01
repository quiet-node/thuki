use std::sync::{Arc, Mutex};

type InFlightSlot = Arc<Mutex<Option<(String, Option<String>, String)>>>;

pub struct WarmupState {
    in_flight: InFlightSlot,
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
        }
    }

    /// Fire-and-forget model warm-up. Returns immediately.
    /// No-op if model/endpoint empty or same (model, keep_alive, system_prompt) triple already in flight.
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
    ) {
        if model.is_empty() || endpoint.is_empty() {
            return;
        }
        {
            let mut guard = self.in_flight.lock().unwrap();
            if guard
                .as_ref()
                .map(|(m, k, s)| m == &model && k == &keep_alive && s == &system_prompt)
                == Some(true)
            {
                return;
            }
            *guard = Some((model.clone(), keep_alive.clone(), system_prompt.clone()));
        }
        let in_flight = Arc::clone(&self.in_flight);
        tauri::async_runtime::spawn(run_warmup(
            endpoint,
            model,
            system_prompt,
            client,
            in_flight,
            keep_alive,
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
        let keep_alive = if cfg.inference.keep_warm {
            Some(keep_alive_string(
                cfg.inference.keep_warm_inactivity_minutes,
            ))
        } else {
            None
        };
        drop(cfg);
        warmup.fire(
            endpoint,
            model,
            system_prompt,
            client.inner().clone(),
            keep_alive,
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
                .any(|name| name == model)
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
/// the frontend can react (e.g. reset the eject button state).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn evict_model(
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
    }
    Ok(())
}

async fn run_warmup(
    endpoint: String,
    model: String,
    system_prompt: String,
    client: reqwest::Client,
    in_flight: InFlightSlot,
    keep_alive: Option<String>,
) {
    // Use /api/chat with the resolved system prompt so Ollama primes the KV cache
    // for the prefix the real chat will share. num_predict:0 prevents any token
    // generation — the goal is prompt evaluation only.
    let messages = serde_json::json!([
        {"role": "system", "content": system_prompt},
        {"role": "user", "content": ""}
    ]);
    let options = serde_json::json!({"num_predict": 0});

    let body = if let Some(ref ka) = keep_alive {
        serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "options": options,
            "keep_alive": ka
        })
    } else {
        serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "options": options
        })
    };

    match client.post(&endpoint).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => {
            eprintln!("thuki: [warmup] HTTP {} for model={}", resp.status(), model);
        }
        Err(e) => {
            eprintln!("thuki: [warmup] request failed: {e}");
        }
    }

    let mut guard = in_flight.lock().unwrap();
    if guard
        .as_ref()
        .map(|(m, k, s)| m == &model && k == &keep_alive && s == &system_prompt)
        == Some(true)
    {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        );
        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            SYS.to_string(),
            client.clone(),
            None,
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
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        state.fire(
            endpoint.clone(),
            "phi3".to_string(),
            SYS.to_string(),
            client.clone(),
            None,
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
        *state.in_flight.lock().unwrap() = Some(("llama3".to_string(), None, SYS.to_string()));

        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/chat", server.url()),
            "phi3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            None,
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
        ))));

        run_warmup(
            "http://127.0.0.1:1/api/chat".to_string(),
            "phi3".to_string(),
            SYS.to_string(),
            reqwest::Client::new(),
            Arc::clone(&in_flight),
            None,
        )
        .await;

        assert_eq!(
            in_flight
                .lock()
                .unwrap()
                .as_ref()
                .map(|(m, _, _)| m.as_str()),
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
                r#"{"model":"llama3","messages":[{"role":"system","content":"You are a helpful assistant."},{"role":"user","content":""}],"stream":false,"options":{"num_predict":0}}"#.to_string(),
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
                r#"{"model":"llama3","messages":[{"role":"system","content":"You are a helpful assistant."},{"role":"user","content":""}],"stream":false,"options":{"num_predict":0},"keep_alive":"30m"}"#.to_string(),
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
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            SYS.to_string(),
            client.clone(),
            Some("30m".to_string()),
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
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        state.fire(
            endpoint.clone(),
            "llama3".to_string(),
            "prompt B".to_string(),
            client.clone(),
            None,
        );
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));

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
}
