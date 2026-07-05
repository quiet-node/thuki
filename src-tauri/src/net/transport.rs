//! Injectable, SSRF-safe HTTP transport shared by the web-search stack.
//!
//! Callers (the engine client, vertical clients, the page fetcher) depend on
//! the [`HttpTransport`] trait, never on reqwest directly, so their logic is
//! unit-testable against [`FakeHttpTransport`] with no network. The one real
//! backend, [`ReqwestTransport`], is a thin wrapper: it wires the reqwest
//! client with the guarantees below and delegates every decision to the pure,
//! fully-tested helpers in this module and in [`super::ssrf`].
//!
//! Backend guarantees (all load-bearing for SSRF safety):
//! - **No proxy.** The client is built with `.no_proxy()`. A proxy would do
//!   its own DNS and bypass the resolver guard entirely, so this is security,
//!   not hygiene.
//! - **Pinning resolver.** A custom DNS resolver screens every resolved
//!   address ([`super::ssrf::screen_addrs`]) and hands reqwest exactly the
//!   addresses it validated, closing the DNS-rebinding TOCTOU window.
//! - **Guarded redirects.** Each hop is re-validated
//!   ([`super::ssrf::validate_request_url`]) and the hop count is capped, so a
//!   redirect to an internal IP literal (which the resolver never sees) cannot
//!   slip through.
//! - **Bounded body.** The response is streamed and aborted past
//!   [`MAX_HTTP_RESPONSE_BYTES`] decompressed bytes (gzip-bomb safe).

use async_trait::async_trait;
use url::Url;

use crate::config::defaults::MAX_HTTP_RESPONSE_BYTES;

// ─── Value types ─────────────────────────────────────────────────────────────

/// HTTP method supported by the transport. Only the verbs the search stack
/// needs: `GET` for verticals and page fetches, `POST` (form-encoded) for the
/// keyless engine endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

/// A fully-specified outbound request. `form` is sent as an
/// `application/x-www-form-urlencoded` body for [`HttpMethod::Post`] and
/// ignored otherwise.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub form: Vec<(String, String)>,
}

impl HttpRequest {
    /// A header-less `GET` for `url`.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: HttpMethod::Get,
            url: url.into(),
            headers: Vec::new(),
            form: Vec::new(),
        }
    }
}

/// The capped response: status, the final URL after any redirects, and the
/// decompressed body (truncated at the byte cap, which surfaces as an error
/// rather than a silent truncation).
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub final_url: String,
    pub body: Vec<u8>,
}

/// Everything that can go wrong issuing a guarded request.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The URL (or a redirect target) was rejected by the SSRF guard.
    #[error("blocked by SSRF guard: {0}")]
    Ssrf(#[from] super::ssrf::SsrfError),
    /// The URL string could not be parsed.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// The response exceeded the byte cap and was aborted.
    #[error("response exceeded {limit}-byte cap")]
    ResponseTooLarge { limit: usize },
    /// The redirect chain exceeded the hop cap.
    #[error("too many redirects (max {max})")]
    TooManyRedirects { max: usize },
    /// The underlying HTTP client failed (connect, TLS, timeout, read).
    #[error("request failed: {0}")]
    Request(String),
}

// ─── Trait ───────────────────────────────────────────────────────────────────

#[async_trait]
pub trait HttpTransport: Send + Sync {
    /// Issues `req` under the full SSRF/redirect/size policy and returns the
    /// capped response.
    async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError>;
}

// ─── Pure helpers (fully unit-tested) ────────────────────────────────────────

/// Parses a URL string and applies the pre-flight SSRF validation. Called
/// before every request; the reqwest backend calls the same
/// [`super::ssrf::validate_request_url`] again on each redirect hop.
pub(crate) fn parse_and_validate(raw: &str) -> Result<Url, TransportError> {
    let url = Url::parse(raw).map_err(|e| TransportError::InvalidUrl(e.to_string()))?;
    super::ssrf::validate_request_url(&url)?;
    Ok(url)
}

/// Guards the running body size before appending `incoming` bytes to a buffer
/// already holding `current` bytes. Errors (rather than truncating) when the
/// total would exceed `limit`.
pub(crate) fn cap_check(
    current: usize,
    incoming: usize,
    limit: usize,
) -> Result<(), TransportError> {
    if current.saturating_add(incoming) > limit {
        return Err(TransportError::ResponseTooLarge { limit });
    }
    Ok(())
}

/// The decision for one redirect hop: whether to follow `next` given how many
/// URLs have already been visited (`hops`).
pub(crate) enum RedirectVerdict {
    Follow,
    Reject(TransportError),
}

/// Decides a single redirect hop: reject once the hop cap is passed, otherwise
/// re-run the SSRF validation on the target.
pub(crate) fn redirect_decision(next: &Url, hops: usize, max_redirects: usize) -> RedirectVerdict {
    if hops > max_redirects {
        return RedirectVerdict::Reject(TransportError::TooManyRedirects { max: max_redirects });
    }
    match super::ssrf::validate_request_url(next) {
        Ok(()) => RedirectVerdict::Follow,
        Err(e) => RedirectVerdict::Reject(TransportError::Ssrf(e)),
    }
}

// ─── reqwest backend (thin wrapper, excluded from coverage) ──────────────────

/// Production [`HttpTransport`] over a single pooled reqwest client wired with
/// the no-proxy, pinning-resolver, guarded-redirect, and gzip policy described
/// in the module docs. Excluded from the coverage gate: every method is thin
/// OS/network glue delegating to the pure helpers above and to
/// [`PinningResolver`], which are tested directly.
pub struct ReqwestTransport {
    client: reqwest::Client,
}

#[cfg_attr(coverage_nightly, coverage(off))]
impl ReqwestTransport {
    /// Builds the guarded client. Fails only if reqwest cannot construct its
    /// TLS/connection backend.
    pub fn new() -> Result<Self, TransportError> {
        use crate::config::defaults::{
            HTTP_CONNECT_TIMEOUT_S, HTTP_REQUEST_TIMEOUT_S, MAX_HTTP_REDIRECTS,
        };
        use std::sync::Arc;
        use std::time::Duration;

        let client = reqwest::Client::builder()
            .no_proxy()
            .gzip(true)
            .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_S))
            .timeout(Duration::from_secs(HTTP_REQUEST_TIMEOUT_S))
            .dns_resolver(Arc::new(PinningResolver))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let hops = attempt.previous().len();
                match redirect_decision(attempt.url(), hops, MAX_HTTP_REDIRECTS) {
                    RedirectVerdict::Follow => attempt.follow(),
                    RedirectVerdict::Reject(e) => attempt.error(e),
                }
            }))
            .build()
            .map_err(|e| TransportError::Request(e.to_string()))?;
        Ok(Self { client })
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let url = parse_and_validate(&req.url)?;
        let mut builder = match req.method {
            HttpMethod::Get => self.client.get(url),
            HttpMethod::Post => self.client.post(url),
        };
        for (name, value) in &req.headers {
            builder = builder.header(name, value);
        }
        if req.method == HttpMethod::Post {
            builder = builder.form(&req.form);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| TransportError::Request(e.to_string()))?;
        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let body = read_capped(resp, MAX_HTTP_RESPONSE_BYTES).await?;
        Ok(HttpResponse {
            status,
            final_url,
            body,
        })
    }
}

/// Streams a response body, enforcing the decompressed byte cap chunk by chunk.
#[cfg_attr(coverage_nightly, coverage(off))]
async fn read_capped(resp: reqwest::Response, limit: usize) -> Result<Vec<u8>, TransportError> {
    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| TransportError::Request(e.to_string()))?;
        cap_check(buf.len(), chunk.len(), limit)?;
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Custom reqwest DNS resolver that screens every resolved address through the
/// SSRF guard and returns only validated addresses, so reqwest connects to
/// exactly what was validated. Thin wrapper over `tokio::net::lookup_host`
/// (which returns both A and AAAA records) plus the pure
/// [`super::ssrf::screen_addrs`]; excluded from coverage.
struct PinningResolver;

#[cfg_attr(coverage_nightly, coverage(off))]
impl reqwest::dns::Resolve for PinningResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            // Port 0: reqwest overrides it with the URL's port (or scheme
            // default) per its `Resolve` contract.
            let host = name.as_str().to_owned();
            let resolved = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let screened = super::ssrf::screen_addrs(resolved)?;
            let addrs: reqwest::dns::Addrs = Box::new(screened.into_iter());
            Ok(addrs)
        })
    }
}

// ─── In-memory fake (tests only) ─────────────────────────────────────────────

/// Scriptable [`HttpTransport`] for unit tests. Available crate-wide during
/// `cargo test` so downstream modules can drive their logic against canned
/// responses without a network.
#[cfg(test)]
pub(crate) struct FakeHttpTransport {
    responses: std::sync::Mutex<std::collections::HashMap<String, HttpResponse>>,
    calls: std::sync::Mutex<Vec<HttpRequest>>,
}

#[cfg(test)]
impl FakeHttpTransport {
    pub(crate) fn new() -> Self {
        Self {
            responses: std::sync::Mutex::new(std::collections::HashMap::new()),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Scripts a canned response for an exact URL.
    pub(crate) fn with_response(self, url: &str, resp: HttpResponse) -> Self {
        self.responses.lock().unwrap().insert(url.to_string(), resp);
        self
    }

    /// The requests seen so far, in order.
    pub(crate) fn calls(&self) -> Vec<HttpRequest> {
        self.calls.lock().unwrap().clone()
    }
}

#[cfg(test)]
#[async_trait]
impl HttpTransport for FakeHttpTransport {
    async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls.lock().unwrap().push(req.clone());
        self.responses
            .lock()
            .unwrap()
            .get(&req.url)
            .cloned()
            .ok_or_else(|| TransportError::Request(format!("no canned response for {}", req.url)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::ssrf::SsrfError;

    #[test]
    fn parse_and_validate_accepts_global_url() {
        let url = parse_and_validate("https://example.com/path").unwrap();
        assert_eq!(url.host_str(), Some("example.com"));
    }

    #[test]
    fn parse_and_validate_rejects_malformed_url() {
        let e = parse_and_validate("not a url").unwrap_err();
        assert!(matches!(e, TransportError::InvalidUrl(_)));
    }

    #[test]
    fn parse_and_validate_rejects_loopback() {
        let e = parse_and_validate("http://127.0.0.1/").unwrap_err();
        assert!(matches!(
            e,
            TransportError::Ssrf(SsrfError::BlockedAddress(_))
        ));
    }

    #[test]
    fn parse_and_validate_rejects_bad_scheme() {
        let e = parse_and_validate("file:///etc/passwd").unwrap_err();
        assert!(matches!(
            e,
            TransportError::Ssrf(SsrfError::BlockedScheme(_))
        ));
    }

    #[test]
    fn cap_check_allows_under_and_exact() {
        assert!(cap_check(0, 100, 100).is_ok());
        assert!(cap_check(40, 60, 100).is_ok());
    }

    #[test]
    fn cap_check_rejects_over() {
        let e = cap_check(40, 61, 100).unwrap_err();
        assert!(matches!(e, TransportError::ResponseTooLarge { limit: 100 }));
    }

    #[test]
    fn redirect_decision_follows_valid_within_cap() {
        let url = Url::parse("https://example.com/next").unwrap();
        assert!(matches!(
            redirect_decision(&url, 1, 5),
            RedirectVerdict::Follow
        ));
    }

    #[test]
    fn redirect_decision_rejects_past_cap() {
        let url = Url::parse("https://example.com/next").unwrap();
        assert!(matches!(
            redirect_decision(&url, 6, 5),
            RedirectVerdict::Reject(TransportError::TooManyRedirects { max: 5 })
        ));
    }

    #[test]
    fn redirect_decision_rejects_internal_target() {
        let url = Url::parse("http://169.254.169.254/latest/meta-data/").unwrap();
        assert!(matches!(
            redirect_decision(&url, 1, 5),
            RedirectVerdict::Reject(TransportError::Ssrf(SsrfError::BlockedAddress(_)))
        ));
    }

    #[tokio::test]
    async fn fake_returns_canned_response_and_records_call() {
        let resp = HttpResponse {
            status: 200,
            final_url: "https://api.example.com/x".into(),
            body: b"hello".to_vec(),
        };
        let fake = FakeHttpTransport::new().with_response("https://api.example.com/x", resp);
        let got = fake
            .send(&HttpRequest::get("https://api.example.com/x"))
            .await
            .unwrap();
        assert_eq!(got.status, 200);
        assert_eq!(got.body, b"hello");
        assert_eq!(fake.calls().len(), 1);
        assert_eq!(fake.calls()[0].url, "https://api.example.com/x");
    }

    #[tokio::test]
    async fn fake_errors_for_unscripted_url() {
        let fake = FakeHttpTransport::new();
        let e = fake
            .send(&HttpRequest::get("https://nope.example/"))
            .await
            .unwrap_err();
        assert!(matches!(e, TransportError::Request(_)));
    }
}
