//! Concurrent page fetch + readability extraction.
//!
//! After the engine client returns SERP hits, the top few (gated by `num_ctx`)
//! are fetched concurrently through the SSRF-safe [`HttpTransport`] and reduced
//! to readable text via `dom_smoothie` (a Rust port of Mozilla's readability.js).
//! Every hit the stage is asked to cover yields a [`FetchedPage`]: a full fetch
//! that succeeds contributes extracted article text; any failure — transport or
//! SSRF error, non-2xx, unreadable page, or per-URL timeout — degrades to the
//! hit's SERP snippet, so the stage never hangs and never drops a result.
//! Hits beyond the fetch budget contribute their snippet directly.
//!
//! **The per-URL bound is the global deadline.** The page fetches race in
//! parallel and each is capped at [`FETCH_PER_URL_TIMEOUT_S`], so the whole
//! fan-out finishes within roughly that bound. A separate outer timeout over
//! the join would be redundant and, worse, lossy (it would discard pages that
//! already completed), so it is deliberately omitted.
//!
//! Extraction is pure CPU over the fetched HTML, so it is covered directly with
//! fixture pages; only the async fan-out over the transport and `tokio::time`
//! is coverage-excluded, and its behaviour is still exercised against
//! [`crate::net::transport::FakeHttpTransport`].

use crate::config::defaults::{
    FETCH_LARGE_CTX_THRESHOLD, FETCH_MAX_ELEMENTS_TO_PARSE, FETCH_MAX_PAGES_LARGE_CTX,
    FETCH_MAX_PAGES_SMALL_CTX, FETCH_PER_URL_TIMEOUT_S,
};
use crate::net::transport::{HttpRequest, HttpTransport};
use crate::websearch::engine::SearchHit;

/// A page reduced to what the ranking and writer stages consume: the resolved
/// URL, a title, and readable body text. On a fetch/extract failure `text` is
/// the hit's SERP snippet rather than the extracted article.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
    pub url: String,
    pub title: String,
    pub text: String,
}

/// How many result URLs to fully fetch given the context window and how many
/// hits are available. Small windows (`num_ctx` < [`FETCH_LARGE_CTX_THRESHOLD`])
/// can only afford a couple of extracted pages; larger windows afford more.
/// Never exceeds `available`.
pub(crate) fn pages_to_fetch(num_ctx: u32, available: usize) -> usize {
    let cap = if num_ctx < FETCH_LARGE_CTX_THRESHOLD {
        FETCH_MAX_PAGES_SMALL_CTX
    } else {
        FETCH_MAX_PAGES_LARGE_CTX
    };
    available.min(cap)
}

/// Extracts readable article text from raw HTML via `dom_smoothie`, returning
/// the whitespace-normalised text or `None` when the page yields no extractable
/// article (parse error or empty result). Pure CPU over the input — no I/O — so
/// it is unit-tested with fixture pages.
///
/// `dom_smoothie`'s `text_content` (not markdown) is used deliberately: the
/// downstream extractive filter chunks on plain text and the writer consumes
/// numbered text blocks, so markdown structure would have no consumer. Because
/// `text_content` strips tags, base64-image data URIs never reach the output.
pub(crate) fn extract_readable(html: &str, url: &str) -> Option<String> {
    extract_with_limit(html, url, FETCH_MAX_ELEMENTS_TO_PARSE)
}

/// Extraction with an explicit element cap, so tests can drive the DoS bound
/// (an over-limit DOM makes `parse` fail, which degrades to `None` like any
/// other unreadable page).
fn extract_with_limit(html: &str, url: &str, max_elements: usize) -> Option<String> {
    let config = dom_smoothie::Config {
        max_elements_to_parse: max_elements,
        ..Default::default()
    };
    let mut readability = dom_smoothie::Readability::new(html, Some(url), Some(config)).ok()?;
    let article = readability.parse().ok()?;
    let text = normalize_ws(&article.text_content);
    (!text.is_empty()).then_some(text)
}

/// Collapses runs of Unicode whitespace to single spaces and trims, keeping the
/// extracted text compact for token budgeting without altering word content.
fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Builds a [`FetchedPage`] from a hit and the extraction result: the extracted
/// text when present, otherwise the hit's SERP snippet. URL and title always
/// come from the hit (the resolved SERP URL).
pub(crate) fn page_from_parts(hit: &SearchHit, extracted: Option<String>) -> FetchedPage {
    FetchedPage {
        url: hit.url.clone(),
        title: hit.title.clone(),
        text: extracted.unwrap_or_else(|| hit.snippet.clone()),
    }
}

/// Fetches and extracts the top pages for `hits`, returning a [`FetchedPage`]
/// for every hit: the top [`pages_to_fetch`] are fetched concurrently and
/// reduced to readable text (snippet fallback on any failure); the remainder
/// contribute their snippet directly. Never errors, never hangs past the
/// per-URL bound.
///
/// Coverage-excluded: async fan-out over the injectable transport and
/// `tokio::time`. Every decision (how many to fetch, extract-or-snippet,
/// snippet passthrough) lives in the pure helpers tested above and is
/// additionally exercised here against [`FakeHttpTransport`].
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn fetch_pages(
    transport: &dyn HttpTransport,
    hits: &[SearchHit],
    num_ctx: u32,
) -> Vec<FetchedPage> {
    let n = pages_to_fetch(num_ctx, hits.len());
    let (to_fetch, snippet_only) = hits.split_at(n);
    let fetched =
        futures_util::future::join_all(to_fetch.iter().map(|hit| fetch_one(transport, hit))).await;
    fetched
        .into_iter()
        .chain(snippet_only.iter().map(|hit| page_from_parts(hit, None)))
        .collect()
}

/// Fetches and extracts one page, bounded by [`FETCH_PER_URL_TIMEOUT_S`]; any
/// failure (timeout, transport/SSRF error, non-2xx, unreadable) degrades to the
/// hit's SERP snippet.
#[cfg_attr(coverage_nightly, coverage(off))]
async fn fetch_one(transport: &dyn HttpTransport, hit: &SearchHit) -> FetchedPage {
    let request = HttpRequest::get(hit.url.as_str());
    let extracted = match tokio::time::timeout(
        std::time::Duration::from_secs(FETCH_PER_URL_TIMEOUT_S),
        transport.send(&request),
    )
    .await
    {
        Ok(Ok(response)) if (200..300).contains(&response.status) => extract_readable(
            &String::from_utf8_lossy(&response.body),
            &response.final_url,
        ),
        _ => None,
    };
    page_from_parts(hit, extracted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{FakeHttpTransport, HttpResponse};

    /// A fixture page with enough article density that readability extracts it.
    const ARTICLE_HTML: &str = r#"
      <!DOCTYPE html><html><head><title>Ownership</title></head><body>
      <nav>home about contact login signup</nav>
      <article>
        <h1>Rust Ownership Explained</h1>
        <p>Ownership is the most distinctive feature of Rust, and it enables
        memory safety guarantees without a garbage collector, so understanding
        how it works matters a great deal. Each value in Rust has a single
        variable that owns it, and there can only ever be one owner at a time.</p>
        <p>When the owner goes out of scope, the value is dropped and its memory
        is freed automatically. This borrow checking happens entirely at compile
        time, which means there is no runtime cost, and it prevents whole classes
        of bugs such as use after free and double free errors in your programs.</p>
        <p>Borrowing lets you reference a value without taking ownership of it,
        and the compiler proves that every reference stays valid, so you can
        never read memory that has already been freed. The rules are strict, yet
        they catch subtle concurrency and memory bugs before the program runs.</p>
      </article>
      <footer>copyright 2026 all rights reserved</footer>
      </body></html>
    "#;

    fn hit(url: &str, snippet: &str) -> SearchHit {
        SearchHit {
            title: "Title".into(),
            url: url.into(),
            snippet: snippet.into(),
        }
    }

    // ── pages_to_fetch ────────────────────────────────────────────────────────

    #[test]
    fn pages_to_fetch_small_ctx_caps_low() {
        assert_eq!(pages_to_fetch(8192, 10), FETCH_MAX_PAGES_SMALL_CTX);
    }

    #[test]
    fn pages_to_fetch_large_ctx_caps_high() {
        assert_eq!(pages_to_fetch(16384, 10), FETCH_MAX_PAGES_LARGE_CTX);
        assert_eq!(pages_to_fetch(32768, 10), FETCH_MAX_PAGES_LARGE_CTX);
    }

    #[test]
    fn pages_to_fetch_limited_by_availability() {
        assert_eq!(pages_to_fetch(32768, 1), 1);
        assert_eq!(pages_to_fetch(8192, 0), 0);
    }

    // ── extract_readable ──────────────────────────────────────────────────────

    #[test]
    fn extract_readable_pulls_article_text() {
        let text = extract_readable(ARTICLE_HTML, "https://example.com/rust").unwrap();
        assert!(text.contains("Ownership is the most distinctive feature of Rust"));
        assert!(text.len() > 200);
    }

    #[test]
    fn extract_readable_none_on_empty_document() {
        assert!(extract_readable("<html><body></body></html>", "https://x.example/").is_none());
    }

    #[test]
    fn extract_readable_none_when_dom_exceeds_element_cap() {
        // A 1-element cap against the multi-element article makes the parser
        // bail (DoS bound), which degrades to None like any unreadable page.
        assert!(extract_with_limit(ARTICLE_HTML, "https://x.example/", 1).is_none());
    }

    // ── normalize_ws ──────────────────────────────────────────────────────────

    #[test]
    fn normalize_ws_collapses_and_trims() {
        assert_eq!(normalize_ws("  a\n\t b   c  "), "a b c");
    }

    // ── page_from_parts ───────────────────────────────────────────────────────

    #[test]
    fn page_uses_extracted_text_when_present() {
        let page = page_from_parts(&hit("https://a/", "snip"), Some("full body".into()));
        assert_eq!(page.text, "full body");
        assert_eq!(page.url, "https://a/");
    }

    #[test]
    fn page_falls_back_to_snippet_when_absent() {
        let page = page_from_parts(&hit("https://a/", "the snippet"), None);
        assert_eq!(page.text, "the snippet");
    }

    // ── fetch_pages over the fake transport ───────────────────────────────────

    #[tokio::test]
    async fn fetch_pages_extracts_top_and_snippets_rest() {
        let resp = HttpResponse {
            status: 200,
            final_url: "https://a.com/".into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new().with_response("https://a.com/", resp);
        let hits = vec![
            hit("https://a.com/", "snippet a"),
            hit("https://b.com/", "snippet b"),
        ];
        // Small ctx -> fetch 2, but only a.com has a canned page.
        let pages = fetch_pages(&transport, &hits, 8192).await;
        assert_eq!(pages.len(), 2);
        assert!(pages[0]
            .text
            .contains("Ownership is the most distinctive feature"));
        // b.com had no canned response -> transport error -> snippet fallback.
        assert_eq!(pages[1].text, "snippet b");
    }

    #[tokio::test]
    async fn fetch_pages_snippets_beyond_budget() {
        let resp = HttpResponse {
            status: 200,
            final_url: "https://a.com/".into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new().with_response("https://a.com/", resp);
        // 3 hits, small ctx budget of 2 -> the 3rd is snippet-only, never fetched.
        let hits = vec![
            hit("https://a.com/", "snip a"),
            hit("https://a.com/", "snip a2"),
            hit("https://c.com/", "snip c"),
        ];
        let pages = fetch_pages(&transport, &hits, 8192).await;
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[2].text, "snip c");
        assert_eq!(transport.calls().len(), 2);
    }

    #[tokio::test]
    async fn fetch_pages_snippet_on_non_2xx() {
        let resp = HttpResponse {
            status: 500,
            final_url: "https://a.com/".into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new().with_response("https://a.com/", resp);
        let hits = vec![hit("https://a.com/", "the snippet")];
        let pages = fetch_pages(&transport, &hits, 8192).await;
        assert_eq!(pages[0].text, "the snippet");
    }
}
