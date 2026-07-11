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
use crate::websearch::recency::extract_published_date;
use crate::websearch::serp_cache::WebCache;

/// A page reduced to what the ranking and writer stages consume: the resolved
/// URL, a title, and readable body text. On a fetch/extract failure `text` is
/// the hit's SERP snippet rather than the extracted article.
///
/// `published` is the best-effort published/modified date extracted from the
/// raw HTML on a successful fetch (see [`crate::websearch::recency`]), used
/// only by the freshness-gated recency fusion. It is `None` for a snippet
/// fallback, a failed fetch, AND a page cache hit: the page cache stores only
/// extracted text, not the raw HTML the date lives in, so a warm cache entry
/// degrades to undated (never zero, never dropped, see
/// [`crate::config::defaults::RECENCY_NEUTRAL_SCORE`]) rather than paying for
/// a second fetch just to recover a date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
    pub url: String,
    pub title: String,
    pub text: String,
    pub published: Option<time::OffsetDateTime>,
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
/// come from the hit (the resolved SERP URL). `published` carries through
/// whatever date, if any, [`extract_published_date`] recovered from the raw
/// HTML; callers with no raw HTML (snippet-only, cache hit) pass `None`.
pub(crate) fn page_from_parts(
    hit: &SearchHit,
    extracted: Option<String>,
    published: Option<time::OffsetDateTime>,
) -> FetchedPage {
    FetchedPage {
        url: hit.url.clone(),
        title: hit.title.clone(),
        text: extracted.unwrap_or_else(|| hit.snippet.clone()),
        published,
    }
}

/// Fetches and extracts the top pages for `hits`, returning a [`FetchedPage`]
/// for every hit: the top [`pages_to_fetch`] are fetched concurrently and
/// reduced to readable text (snippet fallback on any failure); the remainder
/// contribute their snippet directly. Never errors, never hangs past the
/// per-URL bound.
///
/// `cache` is the in-memory page cache: each fetched URL is looked up before the
/// network fetch (a hit skips the fetch entirely), UNLESS `bypass_cache` is set,
/// and a successful extraction is written back so a later turn reuses it
/// REGARDLESS of `bypass_cache` (a fresh fetch always refreshes the entry).
/// Only the fetched slice consults the cache; snippet-only hits beyond the
/// fetch budget were never going to hit the network and stay snippet-only.
///
/// `bypass_cache` carries the same read-bypass, write-through contract as
/// [`crate::websearch::engine::web_search`]'s `bypass_cache`: an explicit user
/// re-search must not be re-served a page pulled from cache within its TTL,
/// but the fresh fetch still refreshes the entry so the next, non-explicit
/// turn benefits from the up-to-date page instead of re-fetching it too.
///
/// `freshness` gates published-date extraction (see [`fetch_one`]): a
/// non-fresh turn's [`FetchedPage::published`] is always `None`, so it pays
/// no extra parse cost for a date the recency fusion will never read (that
/// pass only runs when `freshness` is set; see
/// `orchestrator::run_engine_tier`).
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
    freshness: bool,
    cache: &WebCache,
    bypass_cache: bool,
) -> Vec<FetchedPage> {
    let n = pages_to_fetch(num_ctx, hits.len());
    let (to_fetch, snippet_only) = hits.split_at(n);
    let fetched = futures_util::future::join_all(
        to_fetch
            .iter()
            .map(|hit| fetch_one(transport, hit, freshness, cache, bypass_cache)),
    )
    .await;
    fetched
        .into_iter()
        .chain(
            snippet_only
                .iter()
                .map(|hit| page_from_parts(hit, None, None)),
        )
        .collect()
}

/// Fetches and extracts one page, bounded by [`FETCH_PER_URL_TIMEOUT_S`]; any
/// failure (timeout, transport/SSRF error, non-2xx, unreadable) degrades to the
/// hit's SERP snippet.
///
/// Consults `cache` first, UNLESS `bypass_cache` is set: a cached extracted body
/// for this URL skips the network fetch entirely when reading is allowed. On a
/// fresh, successful extraction the body is written back to the cache
/// regardless of `bypass_cache` (a bypassing fetch still refreshes a stale
/// entry); a snippet fallback (no extractable text) and any failed fetch are
/// never cached, so only real article text is ever reused. The cache is read
/// before and written after the `await`, never across it, so its lock is never
/// held over I/O.
///
/// On a successful fetch, [`extract_published_date`] also runs against the raw
/// HTML (the same response body `extract_readable` consumes) so the recency
/// fusion has a date to work with, but ONLY when `freshness` is set: on a
/// non-fresh turn the recency pass never runs (see
/// `orchestrator::run_engine_tier`) and would never read the date, so paying
/// for a second HTML parse on every fetched page would be pure waste. A cache
/// hit skips extraction regardless of `freshness` (see [`FetchedPage::published`]'s
/// doc).
#[cfg_attr(coverage_nightly, coverage(off))]
async fn fetch_one(
    transport: &dyn HttpTransport,
    hit: &SearchHit,
    freshness: bool,
    cache: &WebCache,
    bypass_cache: bool,
) -> FetchedPage {
    if !bypass_cache {
        if let Some(text) = cache.page_get(hit.url.as_str()) {
            eprintln!("[search] page cache hit url={}", hit.url);
            return page_from_parts(hit, Some(text), None);
        }
    }
    let request = HttpRequest::get(hit.url.as_str());
    let (extracted, published) = match tokio::time::timeout(
        std::time::Duration::from_secs(FETCH_PER_URL_TIMEOUT_S),
        transport.send(&request),
    )
    .await
    {
        Ok(Ok(response)) if (200..300).contains(&response.status) => {
            let html = String::from_utf8_lossy(&response.body);
            let text = extract_readable(&html, &response.final_url);
            let published = freshness
                .then(|| extract_published_date(&html, time::OffsetDateTime::now_utc()))
                .flatten();
            (text, published)
        }
        _ => (None, None),
    };
    if let Some(text) = &extracted {
        cache.page_put(hit.url.as_str(), text.clone());
    }
    page_from_parts(hit, extracted, published)
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

    /// [`ARTICLE_HTML`] plus a JSON-LD `datePublished`, so the freshness-gated
    /// extraction path in [`fetch_one`] has a real date to find.
    const DATED_ARTICLE_HTML: &str = r#"
      <!DOCTYPE html><html><head><title>Ownership</title>
      <script type="application/ld+json">{"datePublished":"2026-07-08T00:00:00Z"}</script>
      </head><body>
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
      </article>
      </body></html>
    "#;

    fn hit(url: &str, snippet: &str) -> SearchHit {
        SearchHit {
            title: "Title".into(),
            url: url.into(),
            snippet: snippet.into(),
        }
    }

    /// A fresh, empty page cache for the fetch tests that do not exercise
    /// caching, so every fetch behaves exactly as it did before caching.
    fn empty_web_cache() -> WebCache {
        WebCache::new(
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(600),
            64,
            128,
        )
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
        let page = page_from_parts(&hit("https://a/", "snip"), Some("full body".into()), None);
        assert_eq!(page.text, "full body");
        assert_eq!(page.url, "https://a/");
        assert!(page.published.is_none());
    }

    #[test]
    fn page_falls_back_to_snippet_when_absent() {
        let page = page_from_parts(&hit("https://a/", "the snippet"), None, None);
        assert_eq!(page.text, "the snippet");
    }

    #[test]
    fn page_carries_through_a_published_date() {
        let now = time::OffsetDateTime::now_utc();
        let page = page_from_parts(&hit("https://a/", "snip"), Some("body".into()), Some(now));
        assert_eq!(page.published, Some(now));
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
        let pages = fetch_pages(&transport, &hits, 8192, false, &empty_web_cache(), false).await;
        assert_eq!(pages.len(), 2);
        assert!(pages[0]
            .text
            .contains("Ownership is the most distinctive feature"));
        // b.com had no canned response -> transport error -> snippet fallback.
        assert_eq!(pages[1].text, "snippet b");
    }

    // ── freshness-gated date extraction ───────────────────────────────────────

    #[tokio::test]
    async fn fetch_pages_extracts_published_date_when_freshness_is_set() {
        let resp = HttpResponse {
            status: 200,
            final_url: "https://a.com/".into(),
            body: DATED_ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new().with_response("https://a.com/", resp);
        let hits = vec![hit("https://a.com/", "snippet a")];
        let pages = fetch_pages(&transport, &hits, 8192, true, &empty_web_cache(), false).await;
        assert_eq!(
            pages[0].published,
            Some(time::macros::datetime!(2026-07-08 00:00:00 UTC))
        );
    }

    #[tokio::test]
    async fn fetch_pages_skips_date_extraction_when_freshness_is_unset() {
        // Same dated fixture as above, but freshness=false: a non-fresh turn
        // never reads a source's date (the recency pass never runs for it),
        // so extraction must not even be attempted.
        let resp = HttpResponse {
            status: 200,
            final_url: "https://a.com/".into(),
            body: DATED_ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new().with_response("https://a.com/", resp);
        let hits = vec![hit("https://a.com/", "snippet a")];
        let pages = fetch_pages(&transport, &hits, 8192, false, &empty_web_cache(), false).await;
        assert!(pages[0].published.is_none());
    }

    #[tokio::test]
    async fn fetch_pages_snippets_beyond_budget() {
        let resp = |url: &str| HttpResponse {
            status: 200,
            final_url: url.into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new()
            .with_response("https://a.com/", resp("https://a.com/"))
            .with_response("https://b.com/", resp("https://b.com/"));
        // 3 hits (distinct URLs, as they always are post-dedupe), small ctx
        // budget of 2 -> the top two are fetched, the 3rd is snippet-only and
        // never reaches the network.
        let hits = vec![
            hit("https://a.com/", "snip a"),
            hit("https://b.com/", "snip b"),
            hit("https://c.com/", "snip c"),
        ];
        let pages = fetch_pages(&transport, &hits, 8192, false, &empty_web_cache(), false).await;
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
        let pages = fetch_pages(&transport, &hits, 8192, false, &empty_web_cache(), false).await;
        assert_eq!(pages[0].text, "the snippet");
    }

    // ── page cache integration ────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_pages_caches_extracted_page_and_reuses_it() {
        // First fetch extracts and caches the page; the second fetch is served
        // from the cache with no new network request.
        let resp = HttpResponse {
            status: 200,
            final_url: "https://a.com/".into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new().with_response("https://a.com/", resp);
        let hits = vec![hit("https://a.com/", "snippet a")];
        let cache = empty_web_cache();
        let pages1 = fetch_pages(&transport, &hits, 8192, false, &cache, false).await;
        assert!(pages1[0]
            .text
            .contains("Ownership is the most distinctive feature"));
        assert_eq!(transport.calls().len(), 1);
        // Second call hits the cache: same text, still exactly one network call.
        let pages2 = fetch_pages(&transport, &hits, 8192, false, &cache, false).await;
        assert_eq!(pages2[0].text, pages1[0].text);
        assert_eq!(transport.calls().len(), 1);
    }

    #[tokio::test]
    async fn fetch_pages_does_not_cache_snippet_fallback() {
        // No canned response -> transport error -> snippet fallback. A snippet
        // fallback is not real extracted text, so nothing is written to the cache.
        let transport = FakeHttpTransport::new();
        let hits = vec![hit("https://b.com/", "snippet b")];
        let cache = empty_web_cache();
        let pages = fetch_pages(&transport, &hits, 8192, false, &cache, false).await;
        assert_eq!(pages[0].text, "snippet b");
        assert!(cache.page_get("https://b.com/").is_none());
    }

    // ── cache bypass (explicit re-search) ───────────────────────────────────

    #[tokio::test]
    async fn fetch_pages_bypass_cache_refetches_despite_warm_page_cache() {
        // A warm page cache entry would normally be served with no network call
        // (see `fetch_pages_caches_extracted_page_and_reuses_it`). With
        // `bypass_cache=true` the read is skipped and the page is fetched again.
        let resp = HttpResponse {
            status: 200,
            final_url: "https://a.com/".into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new().with_response("https://a.com/", resp);
        let hits = vec![hit("https://a.com/", "snippet a")];
        let cache = empty_web_cache();
        cache.page_put("https://a.com/", "stale cached body".into());
        let pages = fetch_pages(&transport, &hits, 8192, false, &cache, true).await;
        // The fresh fetch's extracted text won, not the stale cached body.
        assert!(pages[0]
            .text
            .contains("Ownership is the most distinctive feature"));
        assert_ne!(pages[0].text, "stale cached body");
        assert_eq!(transport.calls().len(), 1);
    }

    #[tokio::test]
    async fn fetch_pages_bypass_cache_refreshes_the_stale_entry() {
        // The fresh page from a bypassing fetch must overwrite the cache, so the
        // very next NON-bypassing fetch is served the refreshed body rather
        // than the stale one the user just distrusted, with no further request.
        let resp = HttpResponse {
            status: 200,
            final_url: "https://a.com/".into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new().with_response("https://a.com/", resp);
        let hits = vec![hit("https://a.com/", "snippet a")];
        let cache = empty_web_cache();
        cache.page_put("https://a.com/", "stale cached body".into());
        let bypassed = fetch_pages(&transport, &hits, 8192, false, &cache, true).await;
        assert!(bypassed[0]
            .text
            .contains("Ownership is the most distinctive feature"));
        assert_eq!(transport.calls().len(), 1);
        // A second, non-bypassing fetch reads the cache: it must see the
        // REPLACED entry, not the original stale body, and issue no request.
        let served = fetch_pages(&transport, &hits, 8192, false, &cache, false).await;
        assert_eq!(served[0].text, bypassed[0].text);
        assert_eq!(transport.calls().len(), 1);
    }

    #[tokio::test]
    async fn fetch_pages_bypass_cache_false_keeps_page_cache_hit_behavior() {
        // bypass_cache=false must behave exactly like the pre-existing
        // cache-hit path: a warm entry is served with zero network calls.
        let transport = FakeHttpTransport::new();
        let hits = vec![hit("https://a.com/", "snippet a")];
        let cache = empty_web_cache();
        cache.page_put("https://a.com/", "already cached body".into());
        let pages = fetch_pages(&transport, &hits, 8192, false, &cache, false).await;
        assert_eq!(pages[0].text, "already cached body");
        assert!(transport.calls().is_empty());
    }
}
