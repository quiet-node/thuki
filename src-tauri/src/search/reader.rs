//! HTTP client for the Trafilatura-based reader sidecar running at
//! `config::READER_BASE_URL`.
//!
//! The agentic /search pipeline calls [`ReaderClient::fetch_batch_cancellable`]
//! to fan out full-page fetches over the top-K SearXNG results when snippets
//! are not sufficient to answer. The client is conservative by design:
//!
//! - concurrency is bounded by a local semaphore (never more in flight than
//!   the batch it was handed);
//! - each URL honors a per-URL reqwest timeout plus cancellation;
//! - the whole batch honors a per-batch timeout;
//! - a single retry on transient connect errors (see `errors` module) so a
//!   container blip does not silently drop results;
//! - when the reader sidecar is unreachable entirely, we return
//!   `ServiceUnavailable` so the pipeline can emit a warning and fall back.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::search::chunker::Page;
use crate::search::config::{
    READER_BASE_URL, READER_BATCH_TIMEOUT_S, READER_PER_URL_TIMEOUT_S, READER_RETRY_DELAY_MS,
};
use crate::search::errors::{is_transient_connect_error, retry_once};

/// Errors callers must handle.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ReaderError {
    /// All URLs failed with transient connect errors. Pipeline should emit
    /// `ReaderUnavailable` and fall back to snippets.
    #[error("reader service unavailable")]
    ServiceUnavailable,
    /// The whole batch did not finish within `READER_BATCH_TIMEOUT_S`.
    #[error("reader batch timed out")]
    BatchTimeout,
    /// The cancellation token fired before the batch completed.
    #[error("cancelled")]
    Cancelled,
}

/// Aggregate outcome of a batch fetch. Sum of sizes equals the input URL count.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReaderBatchResult {
    /// Pages with non-empty markdown.
    pub pages: Vec<Page>,
    /// URLs where the reader succeeded but extracted nothing useful.
    pub empty_urls: Vec<String>,
    /// URLs where the fetch failed (HTTP non-2xx or reqwest error).
    pub failed_urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExtractResponse {
    url: String,
    title: String,
    markdown: String,
    status: String,
}

/// Thin wrapper over a `reqwest::Client` pointed at the reader sidecar.
#[derive(Clone)]
pub struct ReaderClient {
    client: Client,
    base: String,
}

impl ReaderClient {
    /// Build a client pointed at `config::READER_BASE_URL`.
    pub fn new() -> Self {
        Self::new_with_base(READER_BASE_URL.to_string())
    }

    /// Build a client pointed at `base`. Used by tests with a mock server.
    pub fn new_with_base(base: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(READER_PER_URL_TIMEOUT_S))
            .build()
            .expect("reader http client");
        Self {
            client,
            base: base.into(),
        }
    }

    /// Fetch pages for every URL in `urls`.
    ///
    /// Convenience wrapper used when no cancellation is needed (tests, etc.).
    /// Production code should always call the `_cancellable` variant.
    pub async fn fetch_batch(&self, urls: &[String]) -> Result<ReaderBatchResult, ReaderError> {
        self.fetch_batch_cancellable(urls, &CancellationToken::new())
            .await
    }

    /// Fetch pages for every URL in `urls`, racing against `cancel`.
    ///
    /// Complexity: O(N) HTTP round-trips (parallelized), bounded by the
    /// 5-slot semaphore and the batch timeout.
    pub async fn fetch_batch_cancellable(
        &self,
        urls: &[String],
        cancel: &CancellationToken,
    ) -> Result<ReaderBatchResult, ReaderError> {
        if urls.is_empty() {
            return Ok(ReaderBatchResult::default());
        }

        let semaphore = Arc::new(Semaphore::new(urls.len().min(5)));
        let fetches = urls.iter().map(|u| {
            let sem = semaphore.clone();
            let cancel = cancel.clone();
            let client = self.client.clone();
            let base = self.base.clone();
            let url = u.clone();
            async move {
                let _permit = sem.acquire_owned().await.ok();
                tokio::select! {
                    _ = cancel.cancelled() => FetchOutcome::Cancelled,
                    res = fetch_one(&client, &base, &url) => res,
                }
            }
        });

        let batch = async { join_all(fetches).await };
        let outcomes =
            match tokio::time::timeout(Duration::from_secs(READER_BATCH_TIMEOUT_S), batch).await {
                Ok(v) => v,
                Err(_) => return Err(ReaderError::BatchTimeout),
            };

        let mut result = ReaderBatchResult::default();
        let mut any_succeeded = false;
        let mut service_unavailable_count = 0usize;

        for outcome in outcomes {
            match outcome {
                FetchOutcome::Cancelled => return Err(ReaderError::Cancelled),
                FetchOutcome::Page(p) => {
                    any_succeeded = true;
                    result.pages.push(p);
                }
                FetchOutcome::Empty(url) => {
                    any_succeeded = true;
                    result.empty_urls.push(url);
                }
                FetchOutcome::Failed(url) => result.failed_urls.push(url),
                FetchOutcome::ServiceUnavailable(url) => {
                    service_unavailable_count += 1;
                    result.failed_urls.push(url);
                }
            }
        }

        if !any_succeeded && service_unavailable_count == urls.len() {
            return Err(ReaderError::ServiceUnavailable);
        }

        Ok(result)
    }
}

impl Default for ReaderClient {
    fn default() -> Self {
        Self::new()
    }
}

enum FetchOutcome {
    Page(Page),
    Empty(String),
    Failed(String),
    ServiceUnavailable(String),
    Cancelled,
}

async fn fetch_one(client: &Client, base: &str, url: &str) -> FetchOutcome {
    let endpoint = format!("{}/extract", base.trim_end_matches('/'));
    let do_call = || async {
        client
            .post(&endpoint)
            .json(&serde_json::json!({ "url": url }))
            .send()
            .await
    };
    let res = retry_once(Duration::from_millis(READER_RETRY_DELAY_MS), do_call).await;

    match res {
        Err(e) => {
            // reqwest wraps the OS-level message in source chains, so
            // `to_string()` alone misses "Connection refused". Use
            // `is_connect()` first (catches TCP-level failures), then fall
            // back to the string classifier for timeout/DNS variants where
            // `is_connect()` is false.
            if e.is_connect() || is_transient_connect_error(&e.to_string()) {
                FetchOutcome::ServiceUnavailable(url.to_string())
            } else {
                FetchOutcome::Failed(url.to_string())
            }
        }
        Ok(r) => {
            if !r.status().is_success() {
                return FetchOutcome::Failed(url.to_string());
            }
            match r.json::<ExtractResponse>().await {
                Ok(body) if body.status == "ok" => FetchOutcome::Page(Page {
                    url: body.url,
                    title: body.title,
                    markdown: body.markdown,
                }),
                Ok(_empty) => FetchOutcome::Empty(url.to_string()),
                Err(_) => FetchOutcome::Failed(url.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn client_for(server: &MockServer) -> ReaderClient {
        ReaderClient::new_with_base(server.uri())
    }

    #[tokio::test]
    async fn fetch_batch_returns_pages_for_success_responses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://a.com/1", "title": "t", "markdown": "hello", "status": "ok"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server).await;
        let pages = client
            .fetch_batch(&["https://a.com/1".to_string()])
            .await
            .unwrap();
        assert_eq!(pages.pages.len(), 1);
        assert_eq!(pages.pages[0].markdown, "hello");
        assert!(pages.empty_urls.is_empty());
    }

    #[tokio::test]
    async fn fetch_batch_records_empty_status_urls_as_empty() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://a.com/2", "title": "t", "markdown": "", "status": "empty"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server).await;
        let pages = client
            .fetch_batch(&["https://a.com/2".to_string()])
            .await
            .unwrap();
        assert!(pages.pages.is_empty());
        assert_eq!(pages.empty_urls, vec!["https://a.com/2".to_string()]);
    }

    #[tokio::test]
    async fn fetch_batch_classifies_non2xx_as_dropped() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(502))
            .mount(&server)
            .await;

        let client = client_for(&server).await;
        let pages = client
            .fetch_batch(&["https://a.com/x".to_string()])
            .await
            .unwrap();
        assert!(pages.pages.is_empty());
        assert_eq!(pages.failed_urls.len(), 1);
    }

    #[tokio::test]
    async fn fetch_batch_reports_unreachable_service() {
        // server not started; port 1 is unprivileged nothingness.
        let client = ReaderClient::new_with_base("http://127.0.0.1:1".to_string());
        let res = client.fetch_batch(&["https://a.com/1".to_string()]).await;
        assert!(matches!(res, Err(ReaderError::ServiceUnavailable)));
    }

    #[tokio::test]
    async fn fetch_batch_cancellation_returns_cancelled_error() {
        let server = MockServer::start().await;
        // Delay response long enough to be cancelled before it returns.
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(10))
                    .set_body_json(serde_json::json!({
                        "url": "https://a.com/slow", "title": "t", "markdown": "", "status": "ok"
                    })),
            )
            .mount(&server)
            .await;

        let client = client_for(&server).await;
        let cancel = CancellationToken::new();
        let cancel_handle = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_handle.cancel();
        });
        let res = client
            .fetch_batch_cancellable(&["https://a.com/slow".to_string()], &cancel)
            .await;
        assert_eq!(res, Err(ReaderError::Cancelled));
    }

    #[tokio::test]
    async fn empty_url_list_returns_empty_result() {
        let client = ReaderClient::new_with_base("http://127.0.0.1:1".to_string());
        let pages = client.fetch_batch(&[]).await.unwrap();
        assert!(pages.pages.is_empty());
        assert!(pages.empty_urls.is_empty());
        assert!(pages.failed_urls.is_empty());
    }
}
