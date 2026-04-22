//! Pre-flight sandbox health probe for the `/search` pipeline.
//!
//! [`probe`] fires concurrent GET requests to the SearXNG and reader
//! health endpoints under a shared 2-second wall-clock budget. Any failure
//! (connection refused, non-2xx, timeout) maps to
//! [`SearchError::SandboxUnavailable`] so the frontend can render the
//! setup-guidance card instead of a generic error bubble.

use std::time::Duration;

use tokio::time::timeout;

use super::types::SearchError;

/// Total wall-clock budget for the concurrent probe across both services.
/// Kept short so a missing sandbox does not stall the user for long.
pub const PROBE_TIMEOUT_SECS: u64 = 2;

/// Probe both sandbox services concurrently. Returns `Ok(())` when both
/// endpoints respond with a 2xx status within [`PROBE_TIMEOUT_SECS`] seconds.
/// Any failure maps to [`SearchError::SandboxUnavailable`].
///
/// `searxng_url` should be the SearXNG base URL (e.g. `http://127.0.0.1:25017`).
/// `reader_url` should be the reader base URL (e.g. `http://127.0.0.1:25018`).
pub async fn probe(
    client: &reqwest::Client,
    searxng_url: &str,
    reader_url: &str,
) -> Result<(), SearchError> {
    let searxng_check = format!("{}/", searxng_url.trim_end_matches('/'));
    let reader_check = format!("{}/healthz", reader_url.trim_end_matches('/'));

    let both = async {
        let (a, b) = tokio::join!(
            client.get(&searxng_check).send(),
            client.get(&reader_check).send(),
        );
        match (a, b) {
            (Ok(ra), Ok(rb)) if ra.status().is_success() && rb.status().is_success() => Ok(()),
            _ => Err(SearchError::SandboxUnavailable),
        }
    };

    timeout(Duration::from_secs(PROBE_TIMEOUT_SECS), both)
        .await
        .unwrap_or(Err(SearchError::SandboxUnavailable))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn probe_returns_ok_when_both_services_healthy() {
        let searxng = MockServer::start().await;
        let reader = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&searxng)
            .await;
        Mock::given(method("GET"))
            .and(path("/healthz"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&reader)
            .await;

        let client = reqwest::Client::new();
        let result = probe(&client, &searxng.uri(), &reader.uri()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn probe_returns_sandbox_unavailable_when_searxng_is_down() {
        let reader = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/healthz"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&reader)
            .await;

        let client = reqwest::Client::new();
        // Port 1 is always refused on localhost.
        let result = probe(&client, "http://127.0.0.1:1", &reader.uri()).await;
        assert_eq!(result, Err(SearchError::SandboxUnavailable));
    }

    #[tokio::test]
    async fn probe_returns_sandbox_unavailable_when_reader_is_down() {
        let searxng = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&searxng)
            .await;

        let client = reqwest::Client::new();
        let result = probe(&client, &searxng.uri(), "http://127.0.0.1:1").await;
        assert_eq!(result, Err(SearchError::SandboxUnavailable));
    }

    #[tokio::test]
    async fn probe_returns_sandbox_unavailable_when_searxng_returns_non_2xx() {
        let searxng = MockServer::start().await;
        let reader = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&searxng)
            .await;
        Mock::given(method("GET"))
            .and(path("/healthz"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&reader)
            .await;

        let client = reqwest::Client::new();
        let result = probe(&client, &searxng.uri(), &reader.uri()).await;
        assert_eq!(result, Err(SearchError::SandboxUnavailable));
    }

    #[tokio::test]
    async fn probe_returns_sandbox_unavailable_when_reader_returns_non_2xx() {
        let searxng = MockServer::start().await;
        let reader = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&searxng)
            .await;
        Mock::given(method("GET"))
            .and(path("/healthz"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&reader)
            .await;

        let client = reqwest::Client::new();
        let result = probe(&client, &searxng.uri(), &reader.uri()).await;
        assert_eq!(result, Err(SearchError::SandboxUnavailable));
    }

    #[tokio::test]
    async fn probe_returns_sandbox_unavailable_on_overall_timeout() {
        let searxng = MockServer::start().await;
        let reader = MockServer::start().await;

        // Delay both responses beyond PROBE_TIMEOUT_SECS.
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200).set_delay(Duration::from_secs(PROBE_TIMEOUT_SECS + 2)),
            )
            .mount(&searxng)
            .await;
        Mock::given(method("GET"))
            .and(path("/healthz"))
            .respond_with(
                ResponseTemplate::new(200).set_delay(Duration::from_secs(PROBE_TIMEOUT_SECS + 2)),
            )
            .mount(&reader)
            .await;

        let client = reqwest::Client::new();
        let result = probe(&client, &searxng.uri(), &reader.uri()).await;
        assert_eq!(result, Err(SearchError::SandboxUnavailable));
    }

    #[test]
    fn probe_timeout_constant_is_two_seconds() {
        assert_eq!(PROBE_TIMEOUT_SECS, 2);
    }
}
