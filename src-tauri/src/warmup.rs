use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct WarmupState {
    in_flight: Arc<AtomicBool>,
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
            in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Fire-and-forget model warm-up. Returns immediately.
    /// No-op if model is empty, endpoint is empty, or a request is already in-flight.
    pub fn fire(&self, endpoint: String, model: String, client: reqwest::Client) {
        if model.is_empty() || endpoint.is_empty() {
            return;
        }
        if self
            .in_flight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let in_flight = Arc::clone(&self.in_flight);
        tauri::async_runtime::spawn(run_warmup(endpoint, model, client, in_flight));
    }
}

async fn run_warmup(
    endpoint: String,
    model: String,
    client: reqwest::Client,
    in_flight: Arc<AtomicBool>,
) {
    let body = serde_json::json!({
        "model": model,
        "prompt": "",
        "stream": false
    });

    match client.post(&endpoint).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            // Model loaded into VRAM; discard response body.
        }
        Ok(resp) => {
            eprintln!("thuki: [warmup] HTTP {} for model={}", resp.status(), model);
        }
        Err(e) => {
            eprintln!("thuki: [warmup] request failed: {e}");
        }
    }

    in_flight.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;
    use std::time::{Duration, Instant};

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn wait_in_flight_false(in_flight: &Arc<AtomicBool>, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while in_flight.load(Ordering::SeqCst) {
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
        );

        wait_in_flight_false(&in_flight, Duration::from_secs(5));
        assert!(!in_flight.load(Ordering::SeqCst));
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
        );

        wait_in_flight_false(&in_flight, Duration::from_secs(5));
        assert!(!in_flight.load(Ordering::SeqCst));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn connection_refused_resets_in_flight() {
        // Point at a port that has nothing listening.
        let state = WarmupState::new();
        let in_flight = Arc::clone(&state.in_flight);
        state.fire(
            "http://127.0.0.1:1/api/generate".to_string(),
            "llama3".to_string(),
            reqwest::Client::new(),
        );

        wait_in_flight_false(&in_flight, Duration::from_secs(10));
        assert!(!in_flight.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dedup_fires_only_one_request() {
        let mut server = Server::new_async().await;
        // Expect exactly one request.
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

        state.fire(endpoint.clone(), "llama3".to_string(), client.clone());
        state.fire(endpoint.clone(), "llama3".to_string(), client.clone());

        wait_in_flight_false(&in_flight, Duration::from_secs(5));
        mock.assert_async().await;
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
        state.fire(String::new(), "llama3".to_string(), reqwest::Client::new());

        tokio::time::sleep(Duration::from_millis(100)).await;
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn request_body_shape() {
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
        );

        wait_in_flight_false(&in_flight, Duration::from_secs(5));
        mock.assert_async().await;
    }
}
