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
//! **[`FETCH_PER_URL_TIMEOUT_S`] is the hard backstop on any one fetch; [`FETCH_SOFT_DEADLINE_MS`]
//! bounds the fan-out as a whole.** The budgeted page fetches race
//! concurrently, but the stage does not wait for every one of them: it
//! proceeds to ranking once [`FETCH_FIRST_K_COMPLETIONS`] have completed or
//! the soft deadline elapses, whichever comes first (see [`fetch_first_k`]).
//! A page still in flight at that point is abandoned, never awaited further,
//! and degrades to its SERP snippet exactly like a genuine per-URL failure;
//! no fetch is ever allowed to run past its own per-URL timeout, since the
//! soft deadline can only end the wait sooner, never later.
//!
//! Extraction is pure CPU over the fetched HTML, so it is covered directly with
//! fixture pages; only the async fan-out over the transport and `tokio::time`
//! is coverage-excluded, and its behaviour is still exercised against
//! [`crate::net::transport::FakeHttpTransport`]. Because that CPU work runs
//! synchronously inside the fan-out, no timeout can preempt it once started;
//! [`estimate_element_count`] instead gates it up front on attacker-controlled
//! HTML, so a hostile page cannot burn parse time before either timeout gets
//! a chance to apply.

use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::FutureExt;

use scraper::{Html, Selector};

use crate::config::defaults::{
    FETCH_FIRST_K_COMPLETIONS, FETCH_LARGE_CTX_THRESHOLD, FETCH_MAX_ELEMENTS_TO_PARSE,
    FETCH_MAX_PAGES_LARGE_CTX, FETCH_MAX_PAGES_SMALL_CTX, FETCH_PER_URL_TIMEOUT_S,
    FETCH_SOFT_DEADLINE_MS, TABLE_EXTRACT_MAX_CELLS_PER_TABLE, TABLE_EXTRACT_MAX_CHARS,
    TABLE_EXTRACT_MAX_TABLES,
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

/// Extracts readable page text from raw HTML: Mozilla-style readability article
/// body plus a bounded harvest of HTML table cells. Returns `None` only when
/// both layers yield nothing usable.
///
/// Horizontal fix for stats/wiki pages where the asked figure lives in a table
/// that readability drops (observed: Economy of Vietnam ~281B prose only).
/// Table extract is pure CPU, size-capped ([`TABLE_EXTRACT_MAX_CHARS`]), and
/// gated by the same element-count DoS bound as the rest of the fetch stage.
///
/// `dom_smoothie`'s `text_content` (not markdown) is used for the article: the
/// downstream extractive filter chunks on plain text and the writer consumes
/// numbered text blocks, so markdown structure would have no consumer. Because
/// `text_content` strips tags, base64-image data URIs never reach the output.
pub(crate) fn extract_readable(html: &str, url: &str) -> Option<String> {
    let article = extract_with_limit(html, url, FETCH_MAX_ELEMENTS_TO_PARSE);
    let tables = extract_table_text(html);
    merge_article_and_tables(article, tables)
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

/// Harvests plain text from HTML `<table>` cells (row-major), bounded by
/// [`TABLE_EXTRACT_MAX_TABLES`], [`TABLE_EXTRACT_MAX_CELLS_PER_TABLE`], and
/// [`TABLE_EXTRACT_MAX_CHARS`]. Returns `None` when the DOM is over the
/// element DoS cap, has no tables, or yields only empty cells.
///
/// Pure CPU over attacker-controlled HTML: never panics; uses `scraper` (already
/// a dep for SERP/date parse). Tables are the horizontal home of level/amount
/// figures that news-style readability often discards.
pub(crate) fn extract_table_text(html: &str) -> Option<String> {
    extract_table_text_with_limit(html, FETCH_MAX_ELEMENTS_TO_PARSE)
}

/// Table extract with an explicit element cap so tests can drive both the
/// pre-parse estimate gate and the post-parse scraper-tree gate.
fn extract_table_text_with_limit(html: &str, max_elements: usize) -> Option<String> {
    // Same pre-gate as readability/date paths: refuse pathological DOMs before
    // building a full scraper tree we would then throw away.
    if estimate_element_count(html) > max_elements {
        return None;
    }
    let doc = Html::parse_document(html);
    // Second bound after parse: scraper's tree can still be huge on odd markup.
    let all = Selector::parse("*").expect("static selector \"*\" always parses");
    if doc.select(&all).count() > max_elements {
        return None;
    }
    let table_sel = Selector::parse("table").expect("static selector \"table\" always parses");
    let cell_sel = Selector::parse("th, td").expect("static selector \"th, td\" always parses");
    let mut out = String::new();
    for (tables_seen, table) in doc.select(&table_sel).enumerate() {
        if tables_seen >= TABLE_EXTRACT_MAX_TABLES {
            break;
        }
        let mut cells = 0usize;
        let mut row_bits: Vec<String> = Vec::new();
        for cell in table.select(&cell_sel) {
            if cells >= TABLE_EXTRACT_MAX_CELLS_PER_TABLE {
                break;
            }
            let cell_text = normalize_ws(&cell.text().collect::<String>());
            if cell_text.is_empty() {
                continue;
            }
            cells += 1;
            row_bits.push(cell_text);
        }
        if row_bits.is_empty() {
            continue;
        }
        let table_line = row_bits.join(" | ");
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&table_line);
        if out.chars().count() >= TABLE_EXTRACT_MAX_CHARS {
            break;
        }
    }
    if out.is_empty() {
        return None;
    }
    // Cap by char, not byte, so multi-byte figures are never split mid-scalar.
    if out.chars().count() > TABLE_EXTRACT_MAX_CHARS {
        let mut cut = None;
        for (count, (byte_idx, _)) in out.char_indices().enumerate() {
            if count == TABLE_EXTRACT_MAX_CHARS {
                cut = Some(byte_idx);
                break;
            }
        }
        if let Some(byte_idx) = cut {
            out.truncate(byte_idx);
        }
    }
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// Merges readability article text with bounded table harvest.
///
/// - Both present: append tables only when they are not already fully covered
///   by a prefix of the article (cheap containment check), so we do not double
///   spend tokens on pages that already inlined the table.
/// - Article only / tables only: that side alone.
/// - Neither: `None` (caller falls back to SERP snippet).
fn merge_article_and_tables(article: Option<String>, tables: Option<String>) -> Option<String> {
    match (article, tables) {
        (Some(a), Some(t)) => {
            // If the article already contains the start of the table harvest,
            // readability likely kept the figures; skip append.
            let probe: String = t.chars().take(48).collect();
            if !probe.is_empty() && a.contains(&probe) {
                Some(a)
            } else {
                Some(format!("{a} {t}"))
            }
        }
        (Some(a), None) => Some(a),
        (None, Some(t)) => Some(t),
        (None, None) => None,
    }
}

/// Collapses runs of Unicode whitespace to single spaces and trims, keeping the
/// extracted text compact for token budgeting without altering word content.
fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cheap upper-bound estimate of the number of elements raw HTML would parse
/// into, counting `<` bytes immediately followed by an ASCII letter (a start
/// tag). Closing tags (`</`), comments (`<!--`), and `<!DOCTYPE>` are excluded
/// by the letter check, so the count tracks actual element count closely
/// enough to gate on the same [`FETCH_MAX_ELEMENTS_TO_PARSE`] bound the real
/// parsers use.
///
/// This is a single linear byte scan with no allocation and no DOM
/// construction, run BEFORE either [`extract_readable`] or
/// [`extract_published_date`] touch the page: both of those hand `html` to a
/// real HTML parser (`dom_smoothie`'s readability, `scraper`'s `html5ever`)
/// that builds a full DOM tree before their own element-count checks run, so
/// neither cap actually prevents the expensive parse on a pathological page,
/// only the more expensive algorithm work after it. `html` is fully
/// attacker-controlled (any page on the web), so this estimate is the actual
/// DoS defense: it is cheap enough to always run, and it can only ever be
/// pessimistic (never undercounts a real tag as zero), so it never lets a
/// hostile page through by accident.
pub(crate) fn estimate_element_count(html: &str) -> usize {
    html.as_bytes()
        .windows(2)
        .filter(|pair| pair[0] == b'<' && pair[1].is_ascii_alphabetic())
        .count()
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
/// snippet passthrough, first-K-or-deadline) lives in the pure helpers tested
/// above and is additionally exercised here against [`FakeHttpTransport`].
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
    let fetched = fetch_first_k(transport, to_fetch, freshness, cache, bypass_cache).await;
    fetched
        .into_iter()
        .chain(
            snippet_only
                .iter()
                .map(|hit| page_from_parts(hit, None, None)),
        )
        .collect()
}

/// Races the fetch of every hit in `to_fetch` and returns as soon as either
/// [`FETCH_FIRST_K_COMPLETIONS`] of them have completed or
/// [`FETCH_SOFT_DEADLINE_MS`] elapses, whichever comes first. Before giving up
/// on the rest, one non-blocking pass claims any OTHER fetch that also
/// finished by that point but had not yet been drained (all `N` racing far
/// ahead of a small `K`, for instance): only fetches still genuinely in
/// flight are abandoned. Dropping the [`FuturesUnordered`] then cancels those
/// remaining futures, and each abandoned hit degrades to its SERP snippet via
/// [`page_from_parts`], the identical fallback a genuine per-URL failure
/// already takes (its information is never dropped, only its extracted-text
/// upgrade). The returned vector preserves `to_fetch`'s order regardless of
/// completion order.
///
/// This bounds the fetch stage's tail latency: without it, one slow host
/// among the budgeted pages holds up the whole turn until its own
/// [`FETCH_PER_URL_TIMEOUT_S`] elapses, even though the rest already
/// answered. That per-URL timeout remains the hard backstop on any single
/// fetch; this soft deadline can only end the overall wait sooner, never
/// later, and never touches an individual fetch's own timeout.
///
/// Coverage-excluded: async fan-out over the injectable transport and
/// `tokio::time`, same rationale as [`fetch_pages`] and [`fetch_one`]; its
/// early-exit, soft-deadline, and straggler-degradation behaviour is
/// exercised against [`FakeHttpTransport`] in the test module below.
#[cfg_attr(coverage_nightly, coverage(off))]
async fn fetch_first_k(
    transport: &dyn HttpTransport,
    to_fetch: &[SearchHit],
    freshness: bool,
    cache: &WebCache,
    bypass_cache: bool,
) -> Vec<FetchedPage> {
    if to_fetch.is_empty() {
        return Vec::new();
    }
    // Never wait on more completions than there are fetches: a small-ctx
    // budget (fewer hits than FETCH_FIRST_K_COMPLETIONS) must still wait for
    // all of them, not hang forever short of an unreachable target.
    let needed = FETCH_FIRST_K_COMPLETIONS.min(to_fetch.len());
    let mut results: Vec<Option<FetchedPage>> = vec![None; to_fetch.len()];
    let mut in_flight: FuturesUnordered<_> = to_fetch
        .iter()
        .enumerate()
        .map(|(index, hit)| async move {
            (
                index,
                fetch_one(transport, hit, freshness, cache, bypass_cache).await,
            )
        })
        .collect();

    let deadline = tokio::time::sleep(Duration::from_millis(FETCH_SOFT_DEADLINE_MS));
    tokio::pin!(deadline);

    let mut completed = 0usize;
    while completed < needed {
        tokio::select! {
            _ = &mut deadline => break,
            next = in_flight.next() => match next {
                Some((index, page)) => {
                    results[index] = Some(page);
                    completed += 1;
                }
                // `in_flight` starts with exactly `to_fetch.len()` futures and
                // only completed ones are ever removed, so it cannot be
                // exhausted (yield `None`) while `completed < needed <=
                // to_fetch.len()`: reaching `None` here would mean every
                // future already completed, which is `completed ==
                // to_fetch.len() >= needed`, contradicting the loop guard.
                // Kept as a safe break rather than an unreachable!() because
                // this is attacker-adjacent async plumbing, not pure logic.
                None => break,
            },
        }
    }
    // Claims any fetch that ALSO finished by now but was not drained above
    // (e.g. every one of a large N raced ahead of a small K): each `next()`
    // here is polled once and only ever returns a value already sitting
    // ready, never waits, so this can only recover completed work, never
    // delay returning past what the loop above already decided.
    while let Some(Some((index, page))) = in_flight.next().now_or_never() {
        results[index] = Some(page);
    }
    // Cancels every still-in-flight fetch: FuturesUnordered drops its
    // remaining futures here, which drops fetch_one's in-progress work
    // (including the underlying transport call) without awaiting it further.
    drop(in_flight);

    to_fetch
        .iter()
        .zip(results)
        .map(|(hit, page)| page.unwrap_or_else(|| page_from_parts(hit, None, None)))
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
///
/// Before either parse runs, [`estimate_element_count`] gates the raw HTML
/// against [`FETCH_MAX_ELEMENTS_TO_PARSE`]: a page over the cap skips BOTH
/// [`extract_readable`] and [`extract_published_date`] entirely and degrades
/// to the snippet fallback below, the same outcome either parser would
/// eventually reach on its own, but without paying for the DOM build first
/// (see [`estimate_element_count`]'s doc for why the parsers' own internal
/// caps do not already prevent that cost).
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
            if estimate_element_count(&html) > FETCH_MAX_ELEMENTS_TO_PARSE {
                (None, None)
            } else {
                let text = extract_readable(&html, &response.final_url);
                let published = freshness
                    .then(|| extract_published_date(&html, time::OffsetDateTime::now_utc()))
                    .flatten();
                (text, published)
            }
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
    use crate::net::transport::{FakeHttpTransport, HttpResponse, TransportError};

    /// Test-only transport that resolves canned URLs instantly and hangs
    /// forever (never resolves) on any other URL, simulating a straggling
    /// host. Used only to drive [`fetch_first_k`]'s early-exit, soft-deadline,
    /// and cancellation behaviour deterministically; kept local to this
    /// module rather than added to [`FakeHttpTransport`] (owned by
    /// `net::transport`, shared by other test suites this task does not
    /// touch).
    struct StragglerTransport {
        fast: std::collections::HashMap<String, HttpResponse>,
    }

    impl StragglerTransport {
        /// An empty transport: every URL hangs forever until registered via
        /// [`Self::with_fast_response`].
        fn new() -> Self {
            Self {
                fast: std::collections::HashMap::new(),
            }
        }

        /// Registers an instantly-resolving canned response for `url`; any
        /// URL not registered hangs forever when fetched.
        fn with_fast_response(mut self, url: &str, resp: HttpResponse) -> Self {
            self.fast.insert(url.to_string(), resp);
            self
        }
    }

    #[async_trait::async_trait]
    impl HttpTransport for StragglerTransport {
        async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
            match self.fast.get(&req.url) {
                Some(resp) => Ok(resp.clone()),
                // Never resolves: the only way `fetch_first_k` moves past
                // this hit is by reaching its first-K count or soft deadline
                // and cancelling this future via drop.
                None => std::future::pending().await,
            }
        }
    }

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

    #[test]
    fn extract_table_text_reads_cells() {
        let html = r#"<!DOCTYPE html><html><body>
          <table><tr><th>Year</th><th>GDP</th></tr>
          <tr><td>2026</td><td>$527 billion</td></tr></table>
          </body></html>"#;
        let text = extract_table_text(html).expect("table cells extract");
        assert!(text.contains("Year"));
        assert!(text.contains("$527 billion"));
        assert!(text.contains("2026"));
    }

    #[test]
    fn extract_readable_keeps_table_figures_when_article_is_thin() {
        // Horizontal pin: sparse prose + a stats table must still yield the
        // level figure for the writer (wiki/economy pages).
        let html = r#"<!DOCTYPE html><html><head><title>Economy</title></head><body>
          <p>It is a developing mixed economy.</p>
          <table><tr><th>Metric</th><th>Value</th></tr>
          <tr><td>Nominal GDP</td><td>$527 billion USD</td></tr></table>
          </body></html>"#;
        let text = extract_readable(html, "https://example.test/economy").unwrap();
        assert!(
            text.contains("$527 billion") || text.contains("527"),
            "table figure must survive extract, got {text:?}"
        );
        assert!(text.contains("Nominal GDP") || text.contains("GDP"));
    }

    #[test]
    fn merge_article_and_tables_skips_duplicate_table_prefix() {
        let article = Some("GDP is $527 billion USD this year.".into());
        // Table harvest starts with text already present in the article.
        let tables = Some("$527 billion USD".into());
        let merged = merge_article_and_tables(article, tables).unwrap();
        // Probe of first 48 chars of tables is already in article → no append.
        assert_eq!(merged, "GDP is $527 billion USD this year.");
    }

    #[test]
    fn merge_article_and_tables_appends_when_novel() {
        let article = Some("Sparse prose about the economy.".into());
        let tables = Some("Nominal GDP | $527 billion USD".into());
        let merged = merge_article_and_tables(article, tables).unwrap();
        assert!(merged.contains("Sparse prose"));
        assert!(merged.contains("$527 billion"));
    }

    #[test]
    fn merge_article_and_tables_sides_alone_or_none() {
        assert_eq!(
            merge_article_and_tables(Some("article only".into()), None).as_deref(),
            Some("article only")
        );
        assert_eq!(
            merge_article_and_tables(None, Some("tables only".into())).as_deref(),
            Some("tables only")
        );
        assert!(merge_article_and_tables(None, None).is_none());
    }

    #[test]
    fn extract_table_text_none_when_dom_over_element_cap() {
        // Pre-gate: estimate_element_count > cap.
        let many = "<div>".repeat(50);
        assert!(extract_table_text_with_limit(&many, 10).is_none());
        // Full production cap still refuses a pathological estimate.
        let huge = "<div>".repeat(FETCH_MAX_ELEMENTS_TO_PARSE + 50);
        assert!(extract_table_text(&huge).is_none());
    }

    #[test]
    fn extract_table_text_none_when_post_parse_tree_over_cap() {
        // Fragment HTML: estimate counts only explicit start tags (table/tr/td
        // = 3), but scraper synthesizes html/head/body so select("*") is larger.
        let html = "<table><tr><td>Cell</td></tr></table>";
        let estimate = estimate_element_count(html);
        assert!(estimate <= 3, "estimate={estimate}");
        // Cap equals estimate: pre-gate passes, post-parse tree exceeds.
        assert!(extract_table_text_with_limit(html, estimate).is_none());
    }

    #[test]
    fn extract_table_text_skips_empty_cells_and_joins_tables() {
        let html = r#"<!DOCTYPE html><html><body>
          <table><tr><td>   </td><td>First</td></tr></table>
          <table><tr><td>Second</td></tr></table>
          </body></html>"#;
        let text = extract_table_text(html).expect("non-empty cells");
        assert!(text.contains("First"));
        assert!(text.contains("Second"));
        // Space between tables when out already non-empty.
        assert!(text.contains("First Second") || text.contains("First") && text.contains("Second"));
    }

    #[test]
    fn extract_table_text_respects_max_tables_and_empty_table() {
        // One empty table (only blank cells) plus one real table.
        let mut html = String::from("<!DOCTYPE html><html><body>");
        html.push_str("<table><tr><td>   </td></tr></table>");
        html.push_str("<table><tr><td>Keep</td></tr></table>");
        // Extra tables beyond the max: first TABLE_EXTRACT_MAX_TABLES only.
        for i in 0..TABLE_EXTRACT_MAX_TABLES + 2 {
            html.push_str(&format!("<table><tr><td>T{i}</td></tr></table>"));
        }
        html.push_str("</body></html>");
        let text = extract_table_text(&html).expect("some cells");
        // Cap: last over-max tables must not all appear.
        let last = format!("T{}", TABLE_EXTRACT_MAX_TABLES + 1);
        assert!(
            !text.contains(&last),
            "table past max must be ignored, got {text:?}"
        );
        assert!(text.contains("Keep") || text.contains("T0"));
    }

    #[test]
    fn extract_table_text_caps_cells_per_table() {
        let mut html = String::from("<!DOCTYPE html><html><body><table><tr>");
        for i in 0..TABLE_EXTRACT_MAX_CELLS_PER_TABLE + 20 {
            html.push_str(&format!("<td>C{i}</td>"));
        }
        html.push_str("</tr></table></body></html>");
        let text = extract_table_text(&html).expect("cells");
        assert!(text.contains("C0"));
        let over = format!("C{}", TABLE_EXTRACT_MAX_CELLS_PER_TABLE + 5);
        assert!(!text.contains(&over));
    }

    #[test]
    fn extract_table_text_caps_total_chars() {
        // One giant cell pushes past TABLE_EXTRACT_MAX_CHARS → truncate path.
        let cell = "Z".repeat(TABLE_EXTRACT_MAX_CHARS + 200);
        let html = format!(
            "<!DOCTYPE html><html><body><table><tr><td>{cell}</td></tr></table></body></html>"
        );
        let text = extract_table_text(&html).expect("capped text");
        assert!(text.chars().count() <= TABLE_EXTRACT_MAX_CHARS);
        assert!(text.chars().all(|c| c == 'Z'));
    }

    // ── estimate_element_count ─────────────────────────────────────────────────

    #[test]
    fn estimate_element_count_counts_start_tags_only() {
        // "<p" is a start tag (counts); "</p" is a close tag ('/' is not a
        // letter, so it does not).
        assert_eq!(estimate_element_count("<p>hi</p>"), 1);
        assert_eq!(estimate_element_count("<div><span>x</span></div>"), 2);
    }

    #[test]
    fn estimate_element_count_ignores_doctype_and_comments() {
        assert_eq!(
            estimate_element_count("<!DOCTYPE html><!-- comment --><p>x</p>"),
            1
        );
    }

    #[test]
    fn estimate_element_count_zero_on_empty_or_tagless_text() {
        assert_eq!(estimate_element_count(""), 0);
        assert_eq!(estimate_element_count("no tags here"), 0);
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

    // ── extraction CPU cap (hostile page) ─────────────────────────────────────

    #[tokio::test]
    async fn fetch_pages_extraction_cap_degrades_hostile_page_to_snippet() {
        // Far more start tags than FETCH_MAX_ELEMENTS_TO_PARSE allows: a
        // hostile/pathological DOM. estimate_element_count gates it before
        // either parse runs, so even a 200 response degrades to the SERP
        // snippet exactly like a genuinely unreadable page would.
        let hostile_html = "<i>x</i>".repeat(FETCH_MAX_ELEMENTS_TO_PARSE + 1);
        let resp = HttpResponse {
            status: 200,
            final_url: "https://hostile.example/".into(),
            body: hostile_html.into_bytes(),
        };
        let transport = FakeHttpTransport::new().with_response("https://hostile.example/", resp);
        let hits = vec![hit("https://hostile.example/", "hostile snippet")];
        let cache = empty_web_cache();
        // freshness=true so a pass would also prove the date parse was
        // skipped, not just the readability extraction.
        let pages = fetch_pages(&transport, &hits, 8192, true, &cache, false).await;
        assert_eq!(pages[0].text, "hostile snippet");
        assert!(pages[0].published.is_none());
        // Gated pages are not real extracted text, so nothing is cached.
        assert!(cache.page_get("https://hostile.example/").is_none());
    }

    // ── first-K-of-N completion / soft deadline ───────────────────────────────

    #[tokio::test]
    async fn fetch_pages_first_k_recovers_all_fetches_that_finish_in_time() {
        // Large ctx -> budget of 5, but all 5 resolve instantly (a fast
        // network): reaching FETCH_FIRST_K_COMPLETIONS (3) must not discard
        // the other 2 as snippets just because they were not among the first
        // 3 drained. The post-loop non-blocking drain must recover them too,
        // since real extracted text already exists for every one of them.
        let resp = |url: &str| HttpResponse {
            status: 200,
            final_url: url.into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new()
            .with_response("https://a.com/", resp("https://a.com/"))
            .with_response("https://b.com/", resp("https://b.com/"))
            .with_response("https://c.com/", resp("https://c.com/"))
            .with_response("https://d.com/", resp("https://d.com/"))
            .with_response("https://e.com/", resp("https://e.com/"));
        let hits = vec![
            hit("https://a.com/", "snip a"),
            hit("https://b.com/", "snip b"),
            hit("https://c.com/", "snip c"),
            hit("https://d.com/", "snip d"),
            hit("https://e.com/", "snip e"),
        ];
        let pages = fetch_pages(&transport, &hits, 32768, false, &empty_web_cache(), false).await;
        assert_eq!(pages.len(), 5);
        for page in &pages {
            assert!(page
                .text
                .contains("Ownership is the most distinctive feature"));
        }
    }

    #[tokio::test]
    async fn fetch_pages_first_k_completes_without_waiting_on_stragglers() {
        // Large ctx -> budget of 5. The first 3 (FETCH_FIRST_K_COMPLETIONS)
        // resolve instantly; the other 2 never resolve at all. Reaching the
        // first-K count must let the stage proceed without ever waiting on
        // the stragglers' own per-URL timeout.
        let resp = |url: &str| HttpResponse {
            status: 200,
            final_url: url.into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = StragglerTransport::new()
            .with_fast_response("https://a.com/", resp("https://a.com/"))
            .with_fast_response("https://b.com/", resp("https://b.com/"))
            .with_fast_response("https://c.com/", resp("https://c.com/"));
        let hits = vec![
            hit("https://a.com/", "snip a"),
            hit("https://b.com/", "snip b"),
            hit("https://c.com/", "snip c"),
            hit("https://d.com/", "snip d"),
            hit("https://e.com/", "snip e"),
        ];
        let pages = fetch_pages(&transport, &hits, 32768, false, &empty_web_cache(), false).await;
        assert_eq!(pages.len(), 5);
        for page in &pages[..3] {
            assert!(page
                .text
                .contains("Ownership is the most distinctive feature"));
        }
        // Order is preserved even though d/e never completed: they degrade
        // to their own SERP snippet, exactly like a genuine per-URL failure.
        assert_eq!(pages[3].text, "snip d");
        assert_eq!(pages[4].text, "snip e");
    }

    #[tokio::test(start_paused = true)]
    async fn fetch_pages_soft_deadline_returns_partial_results() {
        // Large ctx -> budget of 5. Only 2 resolve instantly, short of
        // FETCH_FIRST_K_COMPLETIONS (3); the other 3 never resolve. The soft
        // deadline (shorter than any straggler's own per-URL timeout) must
        // still let the stage proceed with the 2 that did complete.
        let resp = |url: &str| HttpResponse {
            status: 200,
            final_url: url.into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = StragglerTransport::new()
            .with_fast_response("https://a.com/", resp("https://a.com/"))
            .with_fast_response("https://b.com/", resp("https://b.com/"));
        let hits = vec![
            hit("https://a.com/", "snip a"),
            hit("https://b.com/", "snip b"),
            hit("https://c.com/", "snip c"),
            hit("https://d.com/", "snip d"),
            hit("https://e.com/", "snip e"),
        ];
        let pages = fetch_pages(&transport, &hits, 32768, false, &empty_web_cache(), false).await;
        assert_eq!(pages.len(), 5);
        assert!(pages[0]
            .text
            .contains("Ownership is the most distinctive feature"));
        assert!(pages[1]
            .text
            .contains("Ownership is the most distinctive feature"));
        assert_eq!(pages[2].text, "snip c");
        assert_eq!(pages[3].text, "snip d");
        assert_eq!(pages[4].text, "snip e");
    }

    #[tokio::test]
    async fn fetch_pages_first_k_needed_never_exceeds_available_fetches() {
        // Small ctx -> budget of 2, below FETCH_FIRST_K_COMPLETIONS (3): both
        // must still be waited on rather than the target being unreachable.
        let resp = |url: &str| HttpResponse {
            status: 200,
            final_url: url.into(),
            body: ARTICLE_HTML.as_bytes().to_vec(),
        };
        let transport = FakeHttpTransport::new()
            .with_response("https://a.com/", resp("https://a.com/"))
            .with_response("https://b.com/", resp("https://b.com/"));
        let hits = vec![
            hit("https://a.com/", "snip a"),
            hit("https://b.com/", "snip b"),
        ];
        let pages = fetch_pages(&transport, &hits, 8192, false, &empty_web_cache(), false).await;
        assert_eq!(pages.len(), 2);
        for page in &pages {
            assert!(page
                .text
                .contains("Ownership is the most distinctive feature"));
        }
    }
}
