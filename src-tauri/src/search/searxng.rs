//! SearXNG client for the `/search` pipeline.
//!
//! Talks to the locally hosted SearXNG sandbox (see `sandbox/search-box/`).
//! The endpoint is compiled in and never derived from user input: there is no
//! user-controlled URL, host, or port anywhere in this module, which
//! structurally eliminates SSRF as an attack vector.
//!
//! Snippets returned from SearXNG are HTML-entity-decoded and length-capped
//! before being exposed to the caller; the caller composes plain-text prompts,
//! so no XML escaping is applied.

use std::time::Duration;

use super::types::{SearchError, SearxResponse, SearxResult};

/// Base URL of the SearXNG sandbox (scheme + host + port, no path). Used by
/// `search_all` / `search_all_with_base` to construct per-query endpoints.
/// Hardcoded to the localhost-only sandbox binding.
#[allow(dead_code)]
pub const SEARXNG_BASE_URL: &str = "http://127.0.0.1:25017";

/// Fully-qualified SearXNG search endpoint. Hardcoded to the localhost-only
/// sandbox binding; the caller cannot override the host.
pub const SEARXNG_ENDPOINT: &str = "http://127.0.0.1:25017/search";

/// Hard timeout for the SearXNG HTTP request. Picked to accommodate the
/// engine's longest outgoing request timeout (15 s in sandbox config) plus a
/// small margin for local overhead.
pub const SEARXNG_TIMEOUT: Duration = Duration::from_secs(20);

/// Maximum number of results forwarded to the synthesis stage. Trimming here
/// bounds prompt size and keeps the synthesis window well within the model's
/// effective attention length.
pub const MAX_RESULTS: usize = 10;

/// Maximum character length retained per snippet/title. Uses character count
/// (not bytes) so multi-byte text is not mid-codepoint-truncated.
pub const MAX_SNIPPET_CHARS: usize = 500;

/// Maximum query length forwarded to SearXNG. Caps the input surface that
/// reaches the external engine; the LLM optimiser already produces short
/// keyword-dense queries, so this is a defence-in-depth bound.
pub const MAX_QUERY_CHARS: usize = 500;

/// Executes a SearXNG search against the provided `endpoint`. Decodes HTML
/// entities on titles/snippets and truncates long fields to a fixed character
/// budget. Returns at most [`MAX_RESULTS`] entries.
///
/// Production callers pass [`SEARXNG_ENDPOINT`]; the endpoint is surfaced as
/// a parameter strictly for testability (mockito-backed unit tests) and must
/// never be wired to a user-controlled value.
///
/// Errors:
/// - [`SearchError::EmptyQuery`] when the query is whitespace-only.
/// - [`SearchError::SearxUnavailable`] on transport failure (connection
///   refused, DNS failure, timeout, body decode error).
/// - [`SearchError::SearxHttp`] when the response status is not 2xx.
/// - [`SearchError::NoResults`] when SearXNG returns an empty result set.
pub async fn search(
    client: &reqwest::Client,
    endpoint: &str,
    query: &str,
) -> Result<Vec<SearxResult>, SearchError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(SearchError::EmptyQuery);
    }
    let bounded = truncate_chars(trimmed, MAX_QUERY_CHARS);

    let url = format!("{}?q={}&format=json", endpoint, url_encode(&bounded));
    let response = client
        .get(&url)
        .timeout(SEARXNG_TIMEOUT)
        .send()
        .await
        .map_err(|_| SearchError::SearxUnavailable)?;

    if !response.status().is_success() {
        return Err(SearchError::SearxHttp(response.status().as_u16()));
    }

    let body: SearxResponse = response
        .json()
        .await
        .map_err(|_| SearchError::SearxUnavailable)?;

    let results: Vec<SearxResult> = body
        .results
        .into_iter()
        .filter(|r| !r.url.trim().is_empty())
        .take(MAX_RESULTS)
        .map(|r| SearxResult {
            title: truncate_chars(&decode_entities(&r.title), MAX_SNIPPET_CHARS),
            url: r.url,
            content: truncate_chars(&decode_entities(&r.content), MAX_SNIPPET_CHARS),
        })
        .collect();

    if results.is_empty() {
        return Err(SearchError::NoResults);
    }
    Ok(results)
}

/// Run multiple SearXNG queries in parallel and return the URL-deduplicated
/// union of results. Individual query failures (HTTP errors, empty results,
/// invalid queries) do not abort the batch; they are silently treated as
/// no-op contributions.
///
/// This matches the gap-round tolerance rule: the pipeline can accept
/// partial coverage in a refinement round without erroring out.
///
/// Complexity: O(N) HTTP round-trips (parallelized). Dedup is O(R) over the
/// total result count, bounded by the SearXNG per-query result cap.
#[allow(dead_code)]
pub async fn search_all(queries: &[String]) -> Result<Vec<SearxResult>, SearchError> {
    search_all_with_base(SEARXNG_BASE_URL, queries).await
}

/// Run multiple SearXNG queries in parallel against a fully-qualified endpoint
/// URL. Unlike [`search_all_with_base`], this accepts the complete endpoint
/// (e.g. `http://127.0.0.1:25017/search`) rather than just the base. Used by
/// the agentic gap loop, which already holds the endpoint URL.
///
/// Production callers should use [`search_all`] which uses the compiled-in
/// constant. This variant is public for use within the `search` module and for
/// tests that already have an endpoint URL.
///
/// Complexity: O(N) HTTP round-trips (parallelized). Dedup is O(R) over the
/// total result count, bounded by the SearXNG per-query result cap.
#[allow(dead_code)]
pub async fn search_all_with_endpoint(
    endpoint: &str,
    queries: &[String],
) -> Result<Vec<SearxResult>, SearchError> {
    if queries.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::new();
    let futures = queries.iter().map(|q| search(&client, endpoint, q));
    let results = futures_util::future::join_all(futures).await;

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut merged: Vec<SearxResult> = Vec::new();
    for r in results {
        match r {
            Ok(items) => {
                for item in items {
                    if seen.insert(item.url.clone()) {
                        merged.push(item);
                    }
                }
            }
            // Flaky or empty query in the batch does not poison the rest.
            Err(_) => continue,
        }
    }
    Ok(merged)
}

/// Test-friendly variant of `search_all`. Accepts an arbitrary base URL so
/// tests can point to a mock server. Production code must use `search_all`,
/// which passes the hardcoded `SEARXNG_BASE_URL` constant and is therefore
/// not subject to SSRF from user input.
///
/// Complexity: O(N) HTTP round-trips (parallelized). Dedup is O(R) over the
/// total result count, bounded by the SearXNG per-query result cap.
#[allow(dead_code)]
pub async fn search_all_with_base(
    base: &str,
    queries: &[String],
) -> Result<Vec<SearxResult>, SearchError> {
    if queries.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::new();
    let endpoint = format!("{}/search", base.trim_end_matches('/'));

    let futures = queries.iter().map(|q| search(&client, &endpoint, q));
    let results = futures_util::future::join_all(futures).await;

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut merged: Vec<SearxResult> = Vec::new();
    for r in results {
        match r {
            Ok(items) => {
                for item in items {
                    if seen.insert(item.url.clone()) {
                        merged.push(item);
                    }
                }
            }
            // Flaky or empty query in the batch does not poison the rest.
            Err(_) => continue,
        }
    }
    Ok(merged)
}

/// Decodes HTML entities (`&amp;`, `&lt;`, `&nbsp;`, numeric entities, etc.)
/// into their literal characters. Live web snippets frequently embed entities;
/// passing them through unchanged to the synthesis model causes the model to
/// treat the content as corrupted.
fn decode_entities(s: &str) -> String {
    html_escape::decode_html_entities(s).into_owned()
}

/// Percent-encodes a query string for safe use as an HTTP query parameter.
/// Applies RFC 3986 "query" rules: unreserved characters pass through, every
/// other byte is encoded as `%HH`. Used instead of the `reqwest` `.query()`
/// builder because the latter requires a feature flag (`serde_urlencoded`)
/// not enabled by the workspace's dependency set.
fn url_encode(s: &str) -> String {
    const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.~";
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        if UNRESERVED.contains(byte) {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Truncates `s` to at most `max` Unicode scalar values, preserving codepoint
/// boundaries. Returns the input unchanged when shorter than the budget.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_returns_unchanged_when_short() {
        assert_eq!(truncate_chars("hello", 10), "hello");
    }

    #[test]
    fn truncate_chars_truncates_on_codepoint_boundary() {
        let input = "αβγδε";
        assert_eq!(input.chars().count(), 5);
        let out = truncate_chars(input, 3);
        assert_eq!(out, "αβγ");
    }

    #[test]
    fn truncate_chars_handles_empty_input() {
        assert_eq!(truncate_chars("", 5), "");
    }

    #[test]
    fn truncate_chars_exact_boundary() {
        assert_eq!(truncate_chars("abcde", 5), "abcde");
    }

    #[test]
    fn decode_entities_handles_named_entities() {
        assert_eq!(decode_entities("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(decode_entities("&lt;b&gt;"), "<b>");
        assert_eq!(decode_entities("&quot;q&quot;"), "\"q\"");
    }

    #[test]
    fn decode_entities_handles_numeric_entities() {
        assert_eq!(decode_entities("&#160;"), "\u{00A0}");
        assert_eq!(decode_entities("&#x2014;"), "—");
    }

    #[test]
    fn decode_entities_passthrough_plain_text() {
        assert_eq!(decode_entities("plain text"), "plain text");
    }

    #[tokio::test]
    async fn search_rejects_empty_query() {
        let client = reqwest::Client::new();
        let err = search(&client, "http://ignored", "   ").await.unwrap_err();
        assert_eq!(err, SearchError::EmptyQuery);
    }

    #[tokio::test]
    async fn search_returns_results_on_happy_path() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/search", server.url());
        let body = serde_json::json!({
            "results": [
                { "title": "Rust &amp; Async", "url": "https://a", "content": "About &lt;rust&gt;" },
                { "title": "Tokio", "url": "https://b", "content": "Runtime" }
            ]
        })
        .to_string();
        let mock = server
            .mock("GET", "/search")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("q".into(), "rust async".into()),
                mockito::Matcher::UrlEncoded("format".into(), "json".into()),
            ]))
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let results = search(&client, &endpoint, "rust async").await.unwrap();

        mock.assert_async().await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust & Async");
        assert_eq!(results[0].content, "About <rust>");
        assert_eq!(results[1].url, "https://b");
    }

    #[tokio::test]
    async fn search_maps_http_error_to_searx_http() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/search", server.url());
        let mock = server
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_status(503)
            .with_body("unavailable")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let err = search(&client, &endpoint, "hi").await.unwrap_err();

        mock.assert_async().await;
        assert_eq!(err, SearchError::SearxHttp(503));
    }

    #[tokio::test]
    async fn search_maps_transport_failure_to_unavailable() {
        let client = reqwest::Client::new();
        let err = search(&client, "http://127.0.0.1:1/search", "hi")
            .await
            .unwrap_err();
        assert_eq!(err, SearchError::SearxUnavailable);
    }

    #[tokio::test]
    async fn search_maps_bad_json_to_unavailable() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/search", server.url());
        let mock = server
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body("not json")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let err = search(&client, &endpoint, "hi").await.unwrap_err();

        mock.assert_async().await;
        assert_eq!(err, SearchError::SearxUnavailable);
    }

    #[tokio::test]
    async fn search_maps_empty_results_to_no_results() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/search", server.url());
        let mock = server
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(r#"{"results":[]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let err = search(&client, &endpoint, "hi").await.unwrap_err();

        mock.assert_async().await;
        assert_eq!(err, SearchError::NoResults);
    }

    #[tokio::test]
    async fn search_filters_results_with_blank_url() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/search", server.url());
        let body = serde_json::json!({
            "results": [
                { "title": "no url", "url": "", "content": "x" },
                { "title": "ok", "url": "https://ok", "content": "y" }
            ]
        })
        .to_string();
        let mock = server
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let results = search(&client, &endpoint, "hi").await.unwrap();

        mock.assert_async().await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://ok");
    }

    #[tokio::test]
    async fn search_caps_results_to_max() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/search", server.url());
        let many: Vec<serde_json::Value> = (0..MAX_RESULTS + 5)
            .map(|i| serde_json::json!({ "title": format!("t{i}"), "url": format!("https://{i}"), "content": "c" }))
            .collect();
        let body = serde_json::json!({ "results": many }).to_string();
        let mock = server
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let results = search(&client, &endpoint, "hi").await.unwrap();

        mock.assert_async().await;
        assert_eq!(results.len(), MAX_RESULTS);
    }

    #[tokio::test]
    async fn search_truncates_long_snippets() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/search", server.url());
        let long = "x".repeat(MAX_SNIPPET_CHARS + 50);
        let body = serde_json::json!({
            "results": [
                { "title": long.clone(), "url": "https://a", "content": long.clone() }
            ]
        })
        .to_string();
        let mock = server
            .mock("GET", "/search")
            .match_query(mockito::Matcher::Any)
            .with_body(body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let results = search(&client, &endpoint, "hi").await.unwrap();

        mock.assert_async().await;
        assert_eq!(results[0].title.chars().count(), MAX_SNIPPET_CHARS);
        assert_eq!(results[0].content.chars().count(), MAX_SNIPPET_CHARS);
    }

    #[tokio::test]
    async fn search_truncates_long_query_before_sending() {
        let mut server = mockito::Server::new_async().await;
        let endpoint = format!("{}/search", server.url());
        let long_query = "a".repeat(MAX_QUERY_CHARS + 50);
        let expected = "a".repeat(MAX_QUERY_CHARS);
        let mock = server
            .mock("GET", "/search")
            .match_query(mockito::Matcher::UrlEncoded("q".into(), expected))
            .with_body(r#"{"results":[{"title":"t","url":"https://x","content":"c"}]}"#)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let _ = search(&client, &endpoint, &long_query).await.unwrap();
        mock.assert_async().await;
    }

    #[test]
    fn endpoint_is_localhost_sandbox() {
        assert!(SEARXNG_ENDPOINT.starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn base_url_is_localhost_sandbox() {
        assert!(SEARXNG_BASE_URL.starts_with("http://127.0.0.1:"));
    }
}

#[cfg(test)]
mod parallel_tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fixture(q: &str, url: &str) -> serde_json::Value {
        serde_json::json!({
            "query": q,
            "results": [{"url": url, "title": "t", "content": "c"}]
        })
    }

    #[tokio::test]
    async fn search_all_merges_unique_urls_across_queries() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("a", "https://x.com/1")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "b"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("b", "https://x.com/2")))
            .mount(&server)
            .await;

        let out = search_all_with_base(&server.uri(), &["a".to_string(), "b".to_string()])
            .await
            .unwrap();
        let urls: Vec<&str> = out.iter().map(|r| r.url.as_str()).collect();
        assert!(urls.contains(&"https://x.com/1"));
        assert!(urls.contains(&"https://x.com/2"));
        assert_eq!(urls.len(), 2);
    }

    #[tokio::test]
    async fn search_all_skips_queries_that_return_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("a", "https://x.com/1")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "b"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"query": "b", "results": []})),
            )
            .mount(&server)
            .await;

        let out = search_all_with_base(&server.uri(), &["a".to_string(), "b".to_string()])
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
    }

    #[tokio::test]
    async fn search_all_deduplicates_by_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("x", "https://x.com/1")))
            .mount(&server)
            .await;

        let out = search_all_with_base(&server.uri(), &["a".to_string(), "b".to_string()])
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
    }

    #[tokio::test]
    async fn search_all_tolerates_one_query_failing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "ok"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("ok", "https://x.com/1")),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "bad"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let out = search_all_with_base(&server.uri(), &["ok".to_string(), "bad".to_string()])
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://x.com/1");
    }

    #[tokio::test]
    async fn search_all_empty_input_returns_empty() {
        let out = search_all_with_base("http://127.0.0.1:1", &[])
            .await
            .unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn search_all_with_empty_slice_returns_empty_without_network() {
        // Covers lines 116-118: search_all delegates to search_all_with_base;
        // the empty-slice guard in search_all_with_base fires before any HTTP
        // call, so no network is needed.
        let out = search_all(&[]).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn search_all_with_endpoint_empty_slice_returns_empty_without_network() {
        // Covers line 137: search_all_with_endpoint short-circuits on empty
        // query slice before touching the network.
        let out = search_all_with_endpoint("http://127.0.0.1:1/search", &[])
            .await
            .unwrap();
        assert!(out.is_empty());
    }
}
