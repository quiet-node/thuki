use std::sync::{Arc, Mutex};

pub struct WarmupState {
    in_flight: Arc<Mutex<Option<String>>>,
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
    /// No-op if model/endpoint empty or same model already in flight.
    /// A different model supersedes the in-flight slot and fires a new request.
    /// `keep_alive` is forwarded to Ollama as-is; `None` omits the field so
    /// Ollama uses its server default (typically 5 minutes).
    pub fn fire(
        &self,
        endpoint: String,
        model: String,
        client: reqwest::Client,
        keep_alive: Option<String>,
    ) {
        if model.is_empty() || endpoint.is_empty() {
            return;
        }
        {
            let mut guard = self.in_flight.lock().unwrap();
            if guard.as_deref() == Some(model.as_str()) {
                return;
            }
            *guard = Some(model.clone());
        }
        let in_flight = Arc::clone(&self.in_flight);
        tauri::async_runtime::spawn(run_warmup(endpoint, model, client, in_flight, keep_alive));
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
            "{}/api/generate",
            cfg.inference.ollama_url.trim_end_matches('/')
        );
        let keep_alive = if cfg.inference.keep_warm {
            Some(keep_alive_string(
                cfg.inference.keep_warm_inactivity_minutes,
            ))
        } else {
            None
        };
        drop(cfg);
        warmup.fire(endpoint, model, client.inner().clone(), keep_alive);
    }
}

/// Unloads the active model from Ollama's VRAM immediately.
///
/// Sends a `/api/generate` request with `keep_alive: 0` which tells Ollama
/// to evict the model right now regardless of the configured TTL. Fire-and-forget;
/// failures are logged and swallowed since the button is best-effort.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn evict_model(
    models: tauri::State<crate::models::ActiveModelState>,
    config: tauri::State<parking_lot::RwLock<crate::config::AppConfig>>,
    client: tauri::State<reqwest::Client>,
) {
    let model = models.0.lock().ok().and_then(|g| g.clone());
    if let Some(model) = model {
        let endpoint = format!(
            "{}/api/generate",
            config.read().inference.ollama_url.trim_end_matches('/')
        );
        let client = client.inner().clone();
        tauri::async_runtime::spawn(async move {
            let body = serde_json::json!({
                "model": model,
                "keep_alive": 0,
                "prompt": "",
                "stream": false
            });
            if let Err(e) = client.post(&endpoint).json(&body).send().await {
                eprintln!("thuki: [evict] request failed: {e}");
            }
        });
    }
}

async fn run_warmup(
    endpoint: String,
    model: String,
    client: reqwest::Client,
    in_flight: Arc<Mutex<Option<String>>>,
    keep_alive: Option<String>,
) {
    let body = if let Some(ref ka) = keep_alive {
        serde_json::json!({
            "model": model,
            "prompt": "",
            "stream": false,
            "keep_alive": ka
        })
    } else {
        serde_json::json!({
            "model": model,
            "prompt": "",
            "stream": false
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
    if guard.as_deref() == Some(model.as_str()) {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;
    use std::time::{Duration, Instant};

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn wait_in_flight_clear(in_flight: &Arc<Mutex<Option<String>>>, timeout: Duration) {
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
            .mock("POST", "/api/generate")
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/generate", server.url()),
            "llama3".to_string(),
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
            .mock("POST", "/api/generate")
            .with_status(500)
            .with_body("internal error")
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/generate", server.url()),
            "llama3".to_string(),
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
            "http://127.0.0.1:1/api/generate".to_string(),
            "llama3".to_string(),
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
            .mock("POST", "/api/generate")
            .with_status(200)
            .with_body("{}")
            .expect(1)
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/generate", server.url());

        state.fire(endpoint.clone(), "llama3".to_string(), client.clone(), None);
        state.fire(endpoint.clone(), "llama3".to_string(), client.clone(), None);

        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn different_model_fires_new_request() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .with_status(200)
            .with_body("{}")
            .expect(2)
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/generate", server.url());

        state.fire(endpoint.clone(), "llama3".to_string(), client.clone(), None);
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        state.fire(endpoint.clone(), "phi3".to_string(), client.clone(), None);
        wait_in_flight_clear(&in_flight, Duration::from_secs(5));

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn different_model_supersedes_in_flight() {
        // Simulate model A in flight; firing model B should still proceed.
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .with_status(200)
            .with_body("{}")
            .expect(1)
            .create_async()
            .await;

        let state = WarmupState::new();
        // Manually mark model A as in flight.
        *state.in_flight.lock().unwrap() = Some("llama3".to_string());

        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/generate", server.url()),
            "phi3".to_string(),
            reqwest::Client::new(),
            None,
        );

        wait_in_flight_clear(&in_flight, Duration::from_secs(5));
        assert!(in_flight.lock().unwrap().is_none());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn task_clears_only_own_slot() {
        // Simulate: in_flight = Some("llama3"), task for "phi3" completes.
        // "phi3" task must NOT clear the "llama3" slot.
        let in_flight: Arc<Mutex<Option<String>>> =
            Arc::new(Mutex::new(Some("llama3".to_string())));

        run_warmup(
            "http://127.0.0.1:1/api/generate".to_string(),
            "phi3".to_string(),
            reqwest::Client::new(),
            Arc::clone(&in_flight),
            None,
        )
        .await;

        assert_eq!(
            in_flight.lock().unwrap().as_deref(),
            Some("llama3"),
            "phi3 task must not clear slot held by llama3"
        );
    }

    #[tokio::test]
    async fn empty_model_no_request() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .expect(0)
            .create_async()
            .await;

        let state = WarmupState::new();
        state.fire(
            format!("{}/api/generate", server.url()),
            String::new(),
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
            .mock("POST", "/api/generate")
            .expect(0)
            .create_async()
            .await;

        let state = WarmupState::new();
        state.fire(
            String::new(),
            "llama3".to_string(),
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
            .mock("POST", "/api/generate")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"model":"llama3","prompt":"","stream":false}"#.to_string(),
            ))
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/generate", server.url()),
            "llama3".to_string(),
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
            .mock("POST", "/api/generate")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"model":"llama3","prompt":"","stream":false,"keep_alive":"30m"}"#.to_string(),
            ))
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            format!("{}/api/generate", server.url()),
            "llama3".to_string(),
            reqwest::Client::new(),
            Some("30m".to_string()),
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
}
