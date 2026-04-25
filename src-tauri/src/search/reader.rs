//! HTTP client for the Trafilatura-based reader sidecar.
//!
//! The reader URL and timeouts come from [`crate::config::AppConfig`] via
//! [`crate::search::config::SearchRuntimeConfig`]; the client itself owns no
//! defaults, so there is exactly one source of truth for the sandbox base
//! URL (`config::defaults::DEFAULT_READER_URL`).
//!
//! The agentic `/search` pipeline calls [`ReaderClient::fetch_batch_cancellable`]
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
use futures_util::stream::{FuturesUnordered, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::config::defaults::DEFAULT_READER_RETRY_DELAY_MS;
use crate::search::chunker::Page;
use crate::search::errors::{is_transient_connect_error, retry_once};

/// Errors callers must handle.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ReaderError {
    /// All URLs failed with transient connect errors. Pipeline should emit
    /// `ReaderUnavailable` and fall back to snippets.
    #[error("reader service unavailable")]
    ServiceUnavailable,
    /// The whole batch did not finish within `batch_timeout_s` (the value
    /// supplied via [`SearchRuntimeConfig`](crate::search::config::SearchRuntimeConfig)).
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
    batch_timeout_s: u64,
}

impl ReaderClient {
    /// Build a client pointed at `base` with explicit timeouts.
    ///
    /// `per_url_timeout_s` caps each individual HTTP round-trip.
    /// `batch_timeout_s` caps the entire parallel fetch batch.
    /// Production code passes the values resolved from
    /// [`SearchRuntimeConfig`](crate::search::config::SearchRuntimeConfig);
    /// tests pass shorter values to exercise the timeout paths quickly.
    pub fn new_with_base(
        base: impl Into<String>,
        per_url_timeout_s: u64,
        batch_timeout_s: u64,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(per_url_timeout_s))
            .build()
            .expect("reader http client");
        Self {
            client,
            base: base.into(),
            batch_timeout_s,
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
            match tokio::time::timeout(Duration::from_secs(self.batch_timeout_s), batch).await {
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

    /// Fetch pages for every URL in `urls`, racing against `cancel`.
    /// Calls `on_url_fetched(url)` after each successful fetch (Page or Empty)
    /// as it completes, providing live progress to callers during the batch.
    ///
    /// Uses `FuturesUnordered` so each completion fires `on_url_fetched`
    /// immediately rather than waiting for the whole batch to finish.
    ///
    /// The callback is taken as `&dyn Fn` (dynamic dispatch) to produce a single
    /// monomorphization, ensuring all code paths are counted once by coverage tools.
    pub async fn fetch_batch_with_progress(
        &self,
        urls: &[String],
        cancel: &CancellationToken,
        on_url_fetched: &(dyn Fn(String) + Send + Sync),
    ) -> Result<ReaderBatchResult, ReaderError> {
        if urls.is_empty() {
            return Ok(ReaderBatchResult::default());
        }

        let total = urls.len();
        let semaphore = Arc::new(Semaphore::new(total.min(5)));
        let futures: FuturesUnordered<_> = urls
            .iter()
            .map(|u| {
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
            })
            .collect();

        let mut futures = futures;
        let mut result = ReaderBatchResult::default();
        let mut any_succeeded = false;
        let mut service_unavailable_count = 0usize;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(self.batch_timeout_s);
        loop {
            match tokio::time::timeout_at(deadline, futures.next()).await {
                Err(_elapsed) => return Err(ReaderError::BatchTimeout),
                Ok(None) => break,
                Ok(Some(outcome)) => match outcome {
                    FetchOutcome::Cancelled => return Err(ReaderError::Cancelled),
                    FetchOutcome::Page(p) => {
                        any_succeeded = true;
                        on_url_fetched(p.url.clone());
                        result.pages.push(p);
                    }
                    FetchOutcome::Empty(url) => {
                        any_succeeded = true;
                        on_url_fetched(url.clone());
                        result.empty_urls.push(url);
                    }
                    FetchOutcome::Failed(url) => {
                        result.failed_urls.push(url);
                    }
                    FetchOutcome::ServiceUnavailable(url) => {
                        service_unavailable_count += 1;
                        result.failed_urls.push(url);
                    }
                },
            }
        }

        if !any_succeeded && service_unavailable_count == total {
            return Err(ReaderError::ServiceUnavailable);
        }

        Ok(result)
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
    let res = retry_once(
        Duration::from_millis(DEFAULT_READER_RETRY_DELAY_MS),
        do_call,
    )
    .await;

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
    use crate::config::defaults::DEFAULT_READER_PER_URL_TIMEOUT_S;
    use crate::search::config::TEST_READER_BATCH_TIMEOUT_S;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn client_for(server: &MockServer) -> ReaderClient {
        ReaderClient::new_with_base(
            server.uri(),
            DEFAULT_READER_PER_URL_TIMEOUT_S,
            TEST_READER_BATCH_TIMEOUT_S,
        )
    }

    /// Callback used in tests that must never fire. The body is excluded from
    /// coverage because it is intentionally dead code in those scenarios.
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn never_called(_: String) {
        panic!("callback must not fire in this test");
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
        let client = ReaderClient::new_with_base(
            "http://127.0.0.1:1".to_string(),
            DEFAULT_READER_PER_URL_TIMEOUT_S,
            TEST_READER_BATCH_TIMEOUT_S,
        );
        let res = client.fetch_batch(&["https://a.com/1".to_string()]).await;
        assert_eq!(res, Err(ReaderError::ServiceUnavailable));
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
        let client = ReaderClient::new_with_base(
            "http://127.0.0.1:1".to_string(),
            DEFAULT_READER_PER_URL_TIMEOUT_S,
            TEST_READER_BATCH_TIMEOUT_S,
        );
        let pages = client.fetch_batch(&[]).await.unwrap();
        assert!(pages.pages.is_empty());
        assert!(pages.empty_urls.is_empty());
        assert!(pages.failed_urls.is_empty());
    }

    #[tokio::test]
    async fn fetch_batch_records_non_json_200_as_failed() {
        // Line 216: r.json::<ExtractResponse>() fails (Err arm), maps to
        // FetchOutcome::Failed.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(b"not json at all".to_vec(), "application/json"),
            )
            .mount(&server)
            .await;

        let client = client_for(&server).await;
        let result = client
            .fetch_batch(&["https://a.com/bad-json".to_string()])
            .await
            .unwrap();
        assert!(result.pages.is_empty());
        assert_eq!(
            result.failed_urls,
            vec!["https://a.com/bad-json".to_string()]
        );
    }

    #[tokio::test]
    async fn fetch_batch_batch_timeout_returns_error() {
        let server = MockServer::start().await;
        // Response delays longer than TEST_READER_BATCH_TIMEOUT_S (1 s).
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(5))
                    .set_body_json(serde_json::json!({
                        "url": "https://a.com/slow", "title": "t",
                        "markdown": "ok", "status": "ok"
                    })),
            )
            .mount(&server)
            .await;

        let client = client_for(&server).await;
        let res = client
            .fetch_batch(&["https://a.com/slow".to_string()])
            .await;
        assert_eq!(res, Err(ReaderError::BatchTimeout));
    }

    #[tokio::test]
    async fn fetch_batch_records_builder_error_as_failed() {
        // Line 202: reqwest errors that are neither is_connect() nor match the
        // transient-string classifier land in FetchOutcome::Failed.
        // "http://:1" produces "builder error: empty host" which satisfies
        // both conditions.
        let client = ReaderClient::new_with_base(
            "http://:1".to_string(),
            DEFAULT_READER_PER_URL_TIMEOUT_S,
            TEST_READER_BATCH_TIMEOUT_S,
        );
        let result = client
            .fetch_batch(&["https://a.com/any".to_string()])
            .await
            .unwrap();
        assert!(result.pages.is_empty());
        assert_eq!(result.failed_urls, vec!["https://a.com/any".to_string()]);
    }

    // ── fetch_batch_with_progress tests ────────────────────────────────────────

    #[tokio::test]
    async fn progress_empty_url_list_returns_empty_no_callbacks() {
        let client = ReaderClient::new_with_base(
            "http://127.0.0.1:1".to_string(),
            DEFAULT_READER_PER_URL_TIMEOUT_S,
            TEST_READER_BATCH_TIMEOUT_S,
        );
        let result = client
            .fetch_batch_with_progress(&[], &CancellationToken::new(), &never_called)
            .await
            .unwrap();
        assert!(result.pages.is_empty());
        assert!(result.empty_urls.is_empty());
        assert!(result.failed_urls.is_empty());
    }

    #[tokio::test]
    async fn progress_calls_callback_for_page_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://a.com/1", "title": "t", "markdown": "hello", "status": "ok"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server).await;
        let called = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let called2 = called.clone();
        let cb = move |url: String| {
            called2.lock().unwrap().push(url);
        };
        let result = client
            .fetch_batch_with_progress(
                &["https://a.com/1".to_string()],
                &CancellationToken::new(),
                &cb,
            )
            .await
            .unwrap();
        assert_eq!(result.pages.len(), 1);
        assert_eq!(*called.lock().unwrap(), vec!["https://a.com/1".to_string()]);
    }

    #[tokio::test]
    async fn progress_calls_callback_for_empty_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://a.com/2", "title": "t", "markdown": "", "status": "empty"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server).await;
        let called = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let called2 = called.clone();
        let cb = move |url: String| {
            called2.lock().unwrap().push(url);
        };
        let result = client
            .fetch_batch_with_progress(
                &["https://a.com/2".to_string()],
                &CancellationToken::new(),
                &cb,
            )
            .await
            .unwrap();
        assert_eq!(result.empty_urls, vec!["https://a.com/2".to_string()]);
        assert_eq!(*called.lock().unwrap(), vec!["https://a.com/2".to_string()]);
    }

    #[tokio::test]
    async fn progress_no_callback_for_failed_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = client_for(&server).await;
        let result = client
            .fetch_batch_with_progress(
                &["https://a.com/x".to_string()],
                &CancellationToken::new(),
                &never_called,
            )
            .await
            .unwrap();
        assert_eq!(result.failed_urls.len(), 1);
        assert!(result.pages.is_empty());
    }

    #[tokio::test]
    async fn progress_reports_service_unavailable_when_all_fail_with_connect_error() {
        let client = ReaderClient::new_with_base(
            "http://127.0.0.1:1".to_string(),
            DEFAULT_READER_PER_URL_TIMEOUT_S,
            TEST_READER_BATCH_TIMEOUT_S,
        );
        let res = client
            .fetch_batch_with_progress(
                &["https://a.com/1".to_string()],
                &CancellationToken::new(),
                &never_called,
            )
            .await;
        assert_eq!(res, Err(ReaderError::ServiceUnavailable));
    }

    #[tokio::test]
    async fn progress_cancellation_returns_cancelled_error() {
        let server = MockServer::start().await;
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
            .fetch_batch_with_progress(&["https://a.com/slow".to_string()], &cancel, &never_called)
            .await;
        assert_eq!(res, Err(ReaderError::Cancelled));
    }

    #[tokio::test]
    async fn progress_callback_fires_once_per_successful_url_in_mixed_batch() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://ok.com/1", "title": "t", "markdown": "ok", "status": "ok"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server).await;
        let called = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let called2 = called.clone();
        let cb = move |url: String| {
            called2.lock().unwrap().push(url);
        };
        let result = client
            .fetch_batch_with_progress(
                &["https://ok.com/1".to_string()],
                &CancellationToken::new(),
                &cb,
            )
            .await
            .unwrap();
        assert_eq!(result.pages.len(), 1);
        assert_eq!(called.lock().unwrap().len(), 1);
    }
}
