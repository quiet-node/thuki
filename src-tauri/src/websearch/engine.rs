//! Keyless search-engine client and rotation.
//!
//! General queries that no vertical handles fall through to keyless engine
//! scraping from the user's device: a POST to DuckDuckGo's `html` endpoint with
//! browser-equivalent headers, the SERP HTML parsed into result rows. A tripped
//! bot challenge (empirically IP-scoped and multi-hour on DDG) is classified as
//! [`SerpOutcome::Blocked`] and yields no results, so the caller degrades
//! gracefully. Rotation to other engines is a deferred fast-follow (see
//! [`ddg_search`]).
//!
//! All requests go through the injectable [`HttpTransport`], so the client is
//! tested against fixture SERP HTML with no network. The parsers are pure and
//! total (malformed HTML yields fewer rows, never a panic).

use scraper::{Html, Selector};

use crate::net::transport::{HttpMethod, HttpRequest, HttpTransport};

/// One search-engine result row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// How a raw SERP response classifies before parsing is trusted.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SerpOutcome {
    /// A 200 with at least one parsed row.
    Ok,
    /// A bot challenge or non-200 status; rotate.
    Blocked,
    /// A 200 that parsed to zero rows; rotate.
    Empty,
}

/// Body substrings that mark a bot-detection interstitial rather than results.
const CAPTCHA_MARKERS: &[&str] = &[
    "anomaly-modal",
    "challenge-form",
    "cf-challenge",
    "hcaptcha",
    "recaptcha",
];

/// ddgs-style browser headers. Sent verbatim so the request is indistinguishable
/// from a browser's; DuckDuckGo's keyless endpoints reject obvious automation.
const DDG_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const DDG_HTML_ENDPOINT: &str = "https://html.duckduckgo.com/html/";

/// Runs one keyless DuckDuckGo `html` query over the shared transport and
/// returns the parsed, deduped result rows. Any recoverable failure (bot
/// challenge, non-200, empty page, transport/SSRF error) yields an empty list,
/// so the caller degrades gracefully rather than hanging or hallucinating.
///
/// Coverage-excluded: thin async glue over the injectable transport that
/// delegates every decision to the pure, directly-tested helpers
/// ([`ddg_html_request`], [`classify_serp`], [`parse_ddg_html`],
/// [`dedupe_and_cap`]); its behaviour is still exercised against
/// [`crate::net::transport::FakeHttpTransport`] in the tests below. Excluded
/// only because parallel async coverage attribution is nondeterministic.
///
/// Rotation to alternative engines (Bing, Startpage, Brave) is a deliberate
/// fast-follow: with a single engine there is nothing to rotate to (a tripped
/// DuckDuckGo block is IP-scoped across its endpoints, per the T1 spike), so an
/// engine trait and multi-engine fallback are deferred until those parsers and
/// fixtures exist, per YAGNI.
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn ddg_search(transport: &dyn HttpTransport, query: &str) -> Vec<SearchHit> {
    let response = match transport.send(&ddg_html_request(query)).await {
        Ok(response) => response,
        Err(_) => return Vec::new(),
    };
    let body = String::from_utf8_lossy(&response.body);
    let hits = parse_ddg_html(&body);
    match classify_serp(response.status, hits.len(), &body) {
        SerpOutcome::Ok => dedupe_and_cap(
            hits,
            crate::config::defaults::SERP_MAX_RESULTS_PER_QUERY,
            crate::config::defaults::SERP_MAX_RESULTS_PER_DOMAIN,
        ),
        SerpOutcome::Blocked | SerpOutcome::Empty => Vec::new(),
    }
}

/// Builds the DuckDuckGo `html` POST request with browser headers and the
/// form-encoded query.
pub(crate) fn ddg_html_request(query: &str) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post,
        url: DDG_HTML_ENDPOINT.to_string(),
        headers: vec![
            ("User-Agent".to_string(), DDG_USER_AGENT.to_string()),
            (
                "Accept".to_string(),
                "text/html,application/xhtml+xml".to_string(),
            ),
            ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
            ("Referer".to_string(), "https://duckduckgo.com/".to_string()),
        ],
        form: vec![
            ("q".to_string(), query.to_string()),
            ("kl".to_string(), "us-en".to_string()),
            ("b".to_string(), String::new()),
        ],
    }
}

/// Classifies a raw SERP response. A bot-challenge body or any non-200 status
/// rotates; a 200 with zero parsed rows is empty (also rotates).
pub(crate) fn classify_serp(status: u16, hit_count: usize, body: &str) -> SerpOutcome {
    let lower = body.to_lowercase();
    if CAPTCHA_MARKERS.iter().any(|m| lower.contains(m)) {
        return SerpOutcome::Blocked;
    }
    if status != 200 {
        return SerpOutcome::Blocked;
    }
    if hit_count == 0 {
        SerpOutcome::Empty
    } else {
        SerpOutcome::Ok
    }
}

/// Parses a DuckDuckGo `html` SERP into result rows: title (`a.result__a`),
/// resolved URL (redirect wrapper decoded), and snippet (`a.result__snippet`,
/// bold markup flattened to text). Rows missing a title or URL are skipped.
pub(crate) fn parse_ddg_html(body: &str) -> Vec<SearchHit> {
    // Selectors are compile-time constants and cannot fail to parse.
    let row = Selector::parse("div.result").expect("static selector");
    let title = Selector::parse("a.result__a").expect("static selector");
    let snippet = Selector::parse("a.result__snippet").expect("static selector");

    let mut hits = Vec::new();
    for result in Html::parse_document(body).select(&row) {
        let Some(anchor) = result.select(&title).next() else {
            continue;
        };
        let title_text = anchor.text().collect::<String>().trim().to_string();
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let url = decode_uddg(href);
        if title_text.is_empty() || url.is_empty() {
            continue;
        }
        if is_ad_result(result.value().attr("class"), &url) {
            continue;
        }
        let snippet_text = result
            .select(&snippet)
            .next()
            .map(|s| s.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        hits.push(SearchHit {
            title: title_text,
            url,
            snippet: snippet_text,
        });
    }
    hits
}

/// Whether a result row is a sponsored ad rather than an organic result:
/// DuckDuckGo marks ad rows with a `result--ad` class and points them at a
/// `duckduckgo.com/y.js` ad-redirect URL. Either signal drops the row so ads
/// never reach the fetch or writer stages.
fn is_ad_result(class_attr: Option<&str>, url: &str) -> bool {
    class_attr.is_some_and(|c| c.contains("result--ad")) || url.contains("duckduckgo.com/y.js")
}

/// Resolves a SERP href to an absolute URL, decoding DuckDuckGo's
/// `/l/?uddg=<encoded>` redirect wrapper and promoting protocol-relative and
/// site-relative hrefs to `https`.
pub(crate) fn decode_uddg(href: &str) -> String {
    let absolute = if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if let Some(rest) = href.strip_prefix("//") {
        format!("https://{rest}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href.to_string()
    };
    uddg_target(&absolute).unwrap_or(absolute)
}

/// Extracts the real destination from a DuckDuckGo `/l/?uddg=<encoded>`
/// redirect URL, or `None` when `absolute` is not such a wrapper.
fn uddg_target(absolute: &str) -> Option<String> {
    let parsed = url::Url::parse(absolute).ok()?;
    if parsed.path() != "/l/" {
        return None;
    }
    parsed
        .query_pairs()
        .find(|(key, _)| key == "uddg")
        .map(|(_, real)| real.into_owned())
}

/// Deduplicates by URL and caps total results and per-domain results,
/// preserving first-seen order.
pub(crate) fn dedupe_and_cap(
    hits: Vec<SearchHit>,
    max_total: usize,
    max_per_domain: usize,
) -> Vec<SearchHit> {
    let mut seen_urls = std::collections::HashSet::new();
    let mut per_domain: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for hit in hits {
        if out.len() >= max_total {
            break;
        }
        if !seen_urls.insert(hit.url.clone()) {
            continue;
        }
        let count = per_domain.entry(domain_of(&hit.url)).or_insert(0);
        if *count >= max_per_domain {
            continue;
        }
        *count += 1;
        out.push(hit);
    }
    out
}

/// The registration host of a URL, for the per-domain cap. Empty when the URL
/// does not parse.
fn domain_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{FakeHttpTransport, HttpResponse};

    const DDG_HTML_FIXTURE: &str = r#"
      <div class="result results_links web-result">
        <h2 class="result__title">
          <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&amp;rut=z">Example A</a>
        </h2>
        <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa">Snippet <b>A</b> text</a>
      </div>
      <div class="result web-result">
        <h2 class="result__title">
          <a class="result__a" href="https://example.org/b">Example B</a>
        </h2>
        <a class="result__snippet">Snippet B</a>
      </div>
    "#;

    fn hit(url: &str) -> SearchHit {
        SearchHit {
            title: "t".into(),
            url: url.into(),
            snippet: "s".into(),
        }
    }

    // ── decode_uddg ─────────────────────────────────────────────────────────

    #[test]
    fn decode_uddg_unwraps_redirect() {
        assert_eq!(
            decode_uddg("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&rut=z"),
            "https://example.com/a"
        );
    }

    #[test]
    fn decode_uddg_passes_direct_url_through() {
        assert_eq!(
            decode_uddg("https://example.org/b"),
            "https://example.org/b"
        );
    }

    #[test]
    fn decode_uddg_promotes_protocol_relative_non_wrapper() {
        assert_eq!(decode_uddg("//cdn.example/x"), "https://cdn.example/x");
    }

    #[test]
    fn decode_uddg_unwraps_site_relative_redirect() {
        assert_eq!(
            decode_uddg("/l/?uddg=https%3A%2F%2Fx.example%2Fy"),
            "https://x.example/y"
        );
    }

    #[test]
    fn decode_uddg_returns_unusable_href_as_is() {
        // Not http(s), not protocol- or site-relative: passed through verbatim.
        assert_eq!(decode_uddg("mailto:a@b"), "mailto:a@b");
    }

    // ── parse_ddg_html ──────────────────────────────────────────────────────

    #[test]
    fn parse_html_extracts_title_url_snippet() {
        let hits = parse_ddg_html(DDG_HTML_FIXTURE);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Example A");
        assert_eq!(hits[0].url, "https://example.com/a");
        assert_eq!(hits[0].snippet, "Snippet A text");
        assert_eq!(hits[1].url, "https://example.org/b");
    }

    #[test]
    fn parse_html_empty_on_junk() {
        assert!(parse_ddg_html("<html><body>no results</body></html>").is_empty());
    }

    #[test]
    fn parse_html_skips_ad_rows() {
        // Sponsored rows carry a `result--ad` class and/or a `y.js` ad-redirect
        // URL; neither must reach the writer.
        let body = r#"
          <div class="result result--ad">
            <a class="result__a" href="https://duckduckgo.com/y.js?ad_domain=spam.example">Sponsored</a>
            <a class="result__snippet">buy now</a>
          </div>
          <div class="result">
            <a class="result__a" href="https://real.example/">Real</a>
            <a class="result__snippet">genuine</a>
          </div>
        "#;
        let hits = parse_ddg_html(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://real.example/");
    }

    #[test]
    fn is_ad_result_flags_ad_class_and_yjs_only() {
        assert!(is_ad_result(Some("result result--ad"), "https://real/"));
        assert!(is_ad_result(None, "https://duckduckgo.com/y.js?ad_domain=x"));
        assert!(!is_ad_result(Some("result web-result"), "https://real/"));
        assert!(!is_ad_result(None, "https://real/"));
    }

    #[test]
    fn parse_html_skips_malformed_rows() {
        // Row with no title anchor, a title with no href, and an empty-text
        // title are all skipped; only the well-formed row survives.
        let body = r#"
          <div class="result"><span>no anchor here</span></div>
          <div class="result"><a class="result__a">missing href</a></div>
          <div class="result"><a class="result__a" href="https://x/">   </a></div>
          <div class="result"><a class="result__a" href="https://good/">Good</a></div>
        "#;
        let hits = parse_ddg_html(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://good/");
    }

    // ── classify_serp ───────────────────────────────────────────────────────

    #[test]
    fn classify_ok_when_200_with_hits() {
        assert_eq!(
            classify_serp(200, 3, "<html>results</html>"),
            SerpOutcome::Ok
        );
    }

    #[test]
    fn classify_empty_when_200_no_hits() {
        assert_eq!(classify_serp(200, 0, "<html></html>"), SerpOutcome::Empty);
    }

    #[test]
    fn classify_blocked_on_challenge_body() {
        assert_eq!(
            classify_serp(200, 5, "<div class=\"anomaly-modal\">challenge-form</div>"),
            SerpOutcome::Blocked
        );
    }

    #[test]
    fn classify_blocked_on_non_200() {
        for status in [202u16, 403, 429, 500] {
            assert_eq!(classify_serp(status, 0, ""), SerpOutcome::Blocked);
        }
    }

    // ── dedupe_and_cap ──────────────────────────────────────────────────────

    #[test]
    fn dedupe_removes_duplicate_urls() {
        let hits = vec![
            hit("https://a.com/1"),
            hit("https://a.com/1"),
            hit("https://b.com/2"),
        ];
        let out = dedupe_and_cap(hits, 10, 5);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedupe_caps_per_domain() {
        let hits = vec![
            hit("https://a.com/1"),
            hit("https://a.com/2"),
            hit("https://a.com/3"),
            hit("https://b.com/1"),
        ];
        let out = dedupe_and_cap(hits, 10, 2);
        // a.com capped at 2, b.com 1 = 3 total
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn dedupe_caps_total() {
        let hits = (0..20)
            .map(|i| hit(&format!("https://d{i}.com/")))
            .collect();
        assert_eq!(dedupe_and_cap(hits, 10, 5).len(), 10);
    }

    // ── ddg_html_request ────────────────────────────────────────────────────

    #[test]
    fn ddg_request_is_post_with_query_form() {
        let req = ddg_html_request("rust bm25");
        assert_eq!(req.method, HttpMethod::Post);
        assert_eq!(req.url, DDG_HTML_ENDPOINT);
        assert!(req.form.iter().any(|(k, v)| k == "q" && v == "rust bm25"));
        assert!(req.headers.iter().any(|(k, _)| k == "User-Agent"));
    }

    // ── ddg_search over the fake transport ──────────────────────────────────

    #[tokio::test]
    async fn ddg_search_returns_hits_on_ok_serp() {
        let resp = HttpResponse {
            status: 200,
            final_url: DDG_HTML_ENDPOINT.into(),
            body: DDG_HTML_FIXTURE.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new().with_response(DDG_HTML_ENDPOINT, resp);
        assert_eq!(ddg_search(&transport, "q").await.len(), 2);
    }

    #[tokio::test]
    async fn ddg_search_empty_on_challenge() {
        let resp = HttpResponse {
            status: 202,
            final_url: DDG_HTML_ENDPOINT.into(),
            body: b"<div class=\"anomaly-modal\">challenge-form</div>".to_vec(),
        };
        let transport = FakeHttpTransport::new().with_response(DDG_HTML_ENDPOINT, resp);
        assert!(ddg_search(&transport, "q").await.is_empty());
    }

    #[tokio::test]
    async fn ddg_search_empty_on_transport_error() {
        // No canned response -> the fake errors -> empty.
        let transport = FakeHttpTransport::new();
        assert!(ddg_search(&transport, "q").await.is_empty());
    }

    #[test]
    fn domain_of_extracts_host() {
        assert_eq!(domain_of("https://sub.example.com/path"), "sub.example.com");
        assert_eq!(domain_of("not a url"), "");
    }
}
