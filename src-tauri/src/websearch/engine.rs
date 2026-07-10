//! Keyless search-engine client with rotation.
//!
//! General queries that no vertical handles fall through to keyless engine
//! scraping from the user's device. [`web_search`] tries each engine in
//! [`ENGINES`] in order and returns the first engine's results that come back
//! usable: DuckDuckGo's `html` endpoint is primary, and Mojeek is the fallback.
//! A tripped bot challenge (empirically IP-scoped and multi-hour on DuckDuckGo,
//! per the T1 spike) classifies as [`SerpOutcome::Blocked`] and rotates to the
//! next engine rather than yielding nothing, so a single engine's rate-limit is
//! no longer fatal to the whole turn. When every engine is exhausted the caller
//! degrades gracefully to a plain answer.
//!
//! Each engine attempt logs its outcome to stderr under a `[search]` prefix so
//! the decision path is visible in the dev console: which engine ran, whether it
//! was blocked, empty, or returned N hits.
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

/// Browser User-Agent sent verbatim on every engine request so it is
/// indistinguishable from a real browser's; keyless SERP endpoints reject
/// obvious automation. Shared by all engines.
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const DDG_HTML_ENDPOINT: &str = "https://html.duckduckgo.com/html/";
const MOJEEK_ENDPOINT: &str = "https://www.mojeek.com/search";

/// One keyless search engine: a name for logging and cooldown keying, a request
/// builder, a pure SERP parser, and how long to skip it after it blocks. The
/// rotation in [`web_search`] walks these in order.
struct Engine {
    /// Identifier used in the `[search]` stderr log line and as the cooldown key.
    name: &'static str,
    /// Builds the outbound request for `query`. `freshness` biases the request
    /// toward recent results when the turn's standalone question carried a
    /// freshness signal (see [`super::encyclopedia::is_volatile_question`]).
    build: fn(&str, bool) -> HttpRequest,
    /// Parses this engine's SERP HTML into result rows.
    parse: fn(&str) -> Vec<SearchHit>,
    /// How long the engine is skipped after a block (seconds). Matches the
    /// engine's observed block behaviour: hours for DuckDuckGo, soft minutes for
    /// the fallbacks.
    cooldown_s: u64,
}

/// Engines tried in order until one returns usable results. DuckDuckGo is
/// primary (richest results); Mojeek is the fallback because it is
/// scraper-tolerant, keyless, and serves lightweight HTML that survives a
/// DuckDuckGo IP block. Verticals (Wikipedia, weather, news) are a separate
/// intent-routed layer added later; this is the general-web tier only. Other
/// candidates were probed live and rejected: Brave/Startpage/Qwant serve
/// JS-shell pages with no parseable server-side results, Ecosia and Presearch
/// return 403 to non-browser clients, and Bing's organic markup no longer ships
/// in the initial HTML.
const ENGINES: &[Engine] = &[
    Engine {
        name: "duckduckgo",
        build: ddg_html_request,
        parse: parse_ddg_html,
        cooldown_s: crate::config::defaults::ENGINE_COOLDOWN_PRIMARY_S,
    },
    Engine {
        name: "mojeek",
        build: mojeek_request,
        parse: parse_mojeek_html,
        cooldown_s: crate::config::defaults::ENGINE_COOLDOWN_FALLBACK_S,
    },
];

/// Cross-turn engine block memory. When an engine returns a bot challenge or
/// rate-limit response it is marked here and skipped for its cooldown window on
/// subsequent queries, instead of being re-hammered on every query of every
/// turn (which wastes a request, adds latency, and feeds the volume signal that
/// keeps volume-triggered blocks alive).
///
/// Thread-safe: the map is behind a `Mutex`, and the lock is held only for the
/// map lookup/insert, never across I/O. Memory-bounded by construction: keys
/// are the static engine names, so the map can never exceed [`ENGINES`] len.
pub struct EngineHealth {
    blocked_until: std::sync::Mutex<std::collections::HashMap<&'static str, std::time::Instant>>,
}

impl EngineHealth {
    /// Creates an empty registry (no engine cooling).
    pub fn new() -> Self {
        Self {
            blocked_until: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Marks `name` blocked for the next `cooldown_s` seconds.
    fn mark_blocked(&self, name: &'static str, cooldown_s: u64) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(cooldown_s);
        self.blocked_until.lock().unwrap().insert(name, until);
    }

    /// Whether `name` is inside its cooldown window. Expired entries are pruned
    /// on read so the map self-cleans.
    fn is_cooling(&self, name: &'static str) -> bool {
        let mut map = self.blocked_until.lock().unwrap();
        match map.get(name) {
            Some(until) if *until > std::time::Instant::now() => true,
            Some(_) => {
                map.remove(name);
                false
            }
            None => false,
        }
    }
}

impl Default for EngineHealth {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide [`EngineHealth`] shared by every turn, so a block observed
/// on one message is remembered on the next. Coverage-excluded: a static
/// constructor call; the registry's behaviour is tested through instance
/// methods on locally-built registries.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn global_engine_health() -> &'static EngineHealth {
    static GLOBAL: std::sync::LazyLock<EngineHealth> = std::sync::LazyLock::new(EngineHealth::new);
    &GLOBAL
}

/// Runs a keyless web search for `query`, rotating through [`ENGINES`] until one
/// returns usable results. Engines inside their block cooldown (see
/// [`EngineHealth`]) are skipped outright; a bot challenge or rate-limit
/// response marks the engine blocked for its cooldown window and rotates; an
/// empty SERP or transport/SSRF error rotates without marking (an empty page is
/// a bad query, not a ban). When all engines are exhausted the result is an
/// empty list, so the caller degrades gracefully rather than hanging or
/// hallucinating. Every attempt logs its outcome to stderr under `[search]` so
/// the path is visible in the dev console. `freshness` is forwarded to each
/// engine's request builder to bias results toward recent content when the
/// turn's standalone question carried a freshness signal.
///
/// Coverage-excluded: thin async glue over the injectable transport that
/// delegates every decision to the pure, directly-tested helpers (each engine's
/// request builder and parser, [`classify_serp`], [`dedupe_and_cap`],
/// [`EngineHealth`]); its rotation behaviour is still exercised against
/// [`crate::net::transport::FakeHttpTransport`] in the tests below. Excluded
/// only because parallel async coverage attribution is nondeterministic.
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn web_search(
    transport: &dyn HttpTransport,
    query: &str,
    health: &EngineHealth,
    freshness: bool,
) -> Vec<SearchHit> {
    for engine in ENGINES {
        if health.is_cooling(engine.name) {
            eprintln!("[search] engine={} cooling -> skipped", engine.name);
            continue;
        }
        let response = match transport.send(&(engine.build)(query, freshness)).await {
            Ok(response) => response,
            Err(_) => {
                eprintln!(
                    "[search] engine={} transport_error -> rotating",
                    engine.name
                );
                continue;
            }
        };
        let body = String::from_utf8_lossy(&response.body);
        let hits = (engine.parse)(&body);
        match classify_serp(response.status, hits.len(), &body) {
            SerpOutcome::Ok => {
                let capped = dedupe_and_cap(
                    hits,
                    crate::config::defaults::SERP_MAX_RESULTS_PER_QUERY,
                    crate::config::defaults::SERP_MAX_RESULTS_PER_DOMAIN,
                );
                eprintln!("[search] engine={} ok hits={}", engine.name, capped.len());
                return capped;
            }
            SerpOutcome::Blocked => {
                health.mark_blocked(engine.name, engine.cooldown_s);
                eprintln!(
                    "[search] engine={} blocked -> cooldown {}s",
                    engine.name, engine.cooldown_s
                );
            }
            SerpOutcome::Empty => {
                eprintln!("[search] engine={} empty -> rotating", engine.name)
            }
        }
    }
    eprintln!("[search] all engines exhausted, no results");
    Vec::new()
}

/// Builds the DuckDuckGo `html` POST request with browser headers and the
/// form-encoded query. When `freshness` is set (the turn's standalone question
/// carried a freshness signal), the request is biased toward recent results
/// via [`crate::config::defaults::DDG_FRESHNESS_DF_VALUE`], set both as a `df`
/// form field and as a `df` cookie: SearXNG's maintained DuckDuckGo scraper
/// sets the filter both ways because the HTML endpoint honours either.
pub(crate) fn ddg_html_request(query: &str, freshness: bool) -> HttpRequest {
    let mut headers = vec![
        ("User-Agent".to_string(), BROWSER_USER_AGENT.to_string()),
        (
            "Accept".to_string(),
            "text/html,application/xhtml+xml".to_string(),
        ),
        ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
        ("Referer".to_string(), "https://duckduckgo.com/".to_string()),
    ];
    let mut form = vec![
        ("q".to_string(), query.to_string()),
        ("kl".to_string(), "us-en".to_string()),
        ("b".to_string(), String::new()),
    ];
    if freshness {
        let df = crate::config::defaults::DDG_FRESHNESS_DF_VALUE;
        form.push(("df".to_string(), df.to_string()));
        headers.push(("Cookie".to_string(), format!("df={df}")));
    }
    HttpRequest {
        method: HttpMethod::Post,
        url: DDG_HTML_ENDPOINT.to_string(),
        headers,
        form,
    }
}

/// Builds the Mojeek `search` GET request. Mojeek takes the query as a `q` URL
/// parameter and returns lightweight result HTML with direct (non-wrapped)
/// destination URLs. When `freshness` is set, [`crate::config::defaults::MOJEEK_FRESHNESS_OPERATOR`]
/// is appended to the query to bias results toward recent content.
pub(crate) fn mojeek_request(query: &str, freshness: bool) -> HttpRequest {
    // MOJEEK_ENDPOINT is a compile-time-valid absolute URL.
    let mut url = url::Url::parse(MOJEEK_ENDPOINT).expect("static endpoint");
    let q = if freshness {
        format!(
            "{query} {}",
            crate::config::defaults::MOJEEK_FRESHNESS_OPERATOR
        )
    } else {
        query.to_string()
    };
    url.query_pairs_mut().append_pair("q", &q);
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: vec![
            ("User-Agent".to_string(), BROWSER_USER_AGENT.to_string()),
            (
                "Accept".to_string(),
                "text/html,application/xhtml+xml".to_string(),
            ),
            ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
        ],
        form: Vec::new(),
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

/// Parses a Mojeek SERP into result rows. Mojeek lists organic results as
/// `ul.results-standard > li`, each with its title and destination in
/// `h2 a.title` (a direct absolute URL, no redirect wrapper) and its snippet in
/// `p.s` (bold markup flattened to text). Rows without a title anchor or a
/// non-http(s) href are skipped, so the "see more results" sub-links and any
/// malformed rows do not leak through.
pub(crate) fn parse_mojeek_html(body: &str) -> Vec<SearchHit> {
    // Selectors are compile-time constants and cannot fail to parse.
    let row = Selector::parse("ul.results-standard li").expect("static selector");
    let title = Selector::parse("h2 a.title").expect("static selector");
    let snippet = Selector::parse("p.s").expect("static selector");

    let mut hits = Vec::new();
    for result in Html::parse_document(body).select(&row) {
        let Some(anchor) = result.select(&title).next() else {
            continue;
        };
        let title_text = anchor.text().collect::<String>().trim().to_string();
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let url = href.trim().to_string();
        // Mojeek serves absolute destination URLs; anything else is a UI link.
        if title_text.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
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
        let count = per_domain.entry(super::domain_of(&hit.url)).or_insert(0);
        if *count >= max_per_domain {
            continue;
        }
        *count += 1;
        out.push(hit);
    }
    out
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
        assert!(is_ad_result(
            None,
            "https://duckduckgo.com/y.js?ad_domain=x"
        ));
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

    // ── parse_mojeek_html ───────────────────────────────────────────────────

    // Mirrors real Mojeek markup: organic results in `ul.results-standard > li`
    // with the title/URL in `h2 a.title` (direct href) and snippet in `p.s`. The
    // third row has no title anchor (a bare "more" link) and must be skipped.
    const MOJEEK_HTML_FIXTURE: &str = r#"
      <ul class="results-standard">
        <li class="r1">
          <a href="https://rust-lang.org/tools/install/" class="ob"><span class="url">rust-lang.org</span></a>
          <h2><a class="title" href="https://rust-lang.org/tools/install/">Install Rust</a></h2>
          <p class="s">Run rustc --<strong>version</strong> to check.</p>
          <p class="more"><a href="/search?q=more">See more</a></p>
        </li>
        <li class="r2">
          <h2><a class="title" href="https://blog.rust-lang.org/">Rust Blog</a></h2>
          <p class="s">Latest release news.</p>
        </li>
        <li class="r3">
          <p class="more"><a href="/search?q=x">no title here</a></p>
        </li>
      </ul>
    "#;

    #[test]
    fn parse_mojeek_extracts_title_url_snippet() {
        let hits = parse_mojeek_html(MOJEEK_HTML_FIXTURE);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Install Rust");
        assert_eq!(hits[0].url, "https://rust-lang.org/tools/install/");
        assert_eq!(hits[0].snippet, "Run rustc --version to check.");
        assert_eq!(hits[1].url, "https://blog.rust-lang.org/");
    }

    #[test]
    fn parse_mojeek_skips_non_http_missing_href_and_missing_title() {
        // A relative href, a title anchor with no href, and a titleless row are
        // all dropped; only the well-formed absolute-URL row survives.
        let body = r#"
          <ul class="results-standard">
            <li><h2><a class="title" href="/settings">Relative</a></h2></li>
            <li><h2><a class="title">No href</a></h2></li>
            <li><h2><a class="title" href="https://ok.example/">Ok</a></h2></li>
            <li><p class="s">no title</p></li>
          </ul>
        "#;
        let hits = parse_mojeek_html(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://ok.example/");
        assert!(hits[0].snippet.is_empty());
    }

    #[test]
    fn parse_mojeek_empty_on_junk() {
        assert!(parse_mojeek_html("<html><body>nothing</body></html>").is_empty());
    }

    // ── request builders ────────────────────────────────────────────────────

    #[test]
    fn ddg_request_is_post_with_query_form() {
        let req = ddg_html_request("rust bm25", false);
        assert_eq!(req.method, HttpMethod::Post);
        assert_eq!(req.url, DDG_HTML_ENDPOINT);
        assert!(req.form.iter().any(|(k, v)| k == "q" && v == "rust bm25"));
        assert!(req.headers.iter().any(|(k, _)| k == "User-Agent"));
    }

    #[test]
    fn mojeek_request_is_get_with_query_param() {
        let req = mojeek_request("rust version", false);
        assert_eq!(req.method, HttpMethod::Get);
        assert!(req.url.starts_with(MOJEEK_ENDPOINT));
        assert!(req.url.contains("q=rust+version") || req.url.contains("q=rust%20version"));
        assert!(req.form.is_empty());
        assert!(req.headers.iter().any(|(k, _)| k == "User-Agent"));
    }

    // ── freshness operators ──────────────────────────────────────────────────

    #[test]
    fn ddg_request_carries_no_freshness_operator_by_default() {
        let req = ddg_html_request("rust bm25", false);
        assert!(!req.form.iter().any(|(k, _)| k == "df"));
        assert!(!req.headers.iter().any(|(k, _)| k == "Cookie"));
    }

    #[test]
    fn ddg_request_adds_df_form_field_and_cookie_when_fresh() {
        let req = ddg_html_request("rust bm25", true);
        assert!(req.form.iter().any(|(k, v)| k == "df" && v == "w"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Cookie" && v == "df=w"));
        // The query itself is untouched; only the date-filter dimension changes.
        assert!(req.form.iter().any(|(k, v)| k == "q" && v == "rust bm25"));
    }

    #[test]
    fn mojeek_request_carries_no_freshness_operator_by_default() {
        let req = mojeek_request("rust version", false);
        assert!(!req.url.contains("since"));
    }

    #[test]
    fn mojeek_request_appends_since_week_when_fresh() {
        let req = mojeek_request("rust version", true);
        assert!(
            req.url.contains("since%3Aweek") || req.url.contains("since:week"),
            "expected since:week operator in {}",
            req.url
        );
        // The operator is appended to the query, not replacing it.
        assert!(req.url.contains("rust") && req.url.contains("version"));
    }

    // ── web_search rotation over the fake transport ─────────────────────────

    fn ok(url: &str, body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            final_url: url.into(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[tokio::test]
    async fn web_search_forwards_freshness_to_the_engine_request() {
        // freshness=true must reach the DDG request builder: the recorded call
        // carries the df=w form field and the df=w cookie.
        let transport = FakeHttpTransport::new()
            .with_response(DDG_HTML_ENDPOINT, ok(DDG_HTML_ENDPOINT, DDG_HTML_FIXTURE));
        let _ = web_search(&transport, "q", &EngineHealth::new(), true).await;
        let call = transport
            .calls()
            .into_iter()
            .find(|c| c.url == DDG_HTML_ENDPOINT)
            .expect("ddg request recorded");
        assert!(call.form.iter().any(|(k, v)| k == "df" && v == "w"));
        assert!(call
            .headers
            .iter()
            .any(|(k, v)| k == "Cookie" && v == "df=w"));
    }

    #[tokio::test]
    async fn web_search_returns_first_engine_hits_without_rotating() {
        // DuckDuckGo answers Ok, so Mojeek is never queried.
        let transport = FakeHttpTransport::new()
            .with_response(DDG_HTML_ENDPOINT, ok(DDG_HTML_ENDPOINT, DDG_HTML_FIXTURE));
        let hits = web_search(&transport, "q", &EngineHealth::new(), false).await;
        assert_eq!(hits.len(), 2);
        let mojeek_url = mojeek_request("q", false).url;
        assert!(!transport.calls().iter().any(|c| c.url == mojeek_url));
    }

    #[tokio::test]
    async fn web_search_rotates_to_mojeek_when_ddg_blocked() {
        // The live failure mode: DuckDuckGo hard-blocks (HTTP 202 challenge), so
        // the search rotates to Mojeek and returns its results.
        let mojeek_url = mojeek_request("q", false).url;
        let transport = FakeHttpTransport::new()
            .with_response(
                DDG_HTML_ENDPOINT,
                HttpResponse {
                    status: 202,
                    final_url: DDG_HTML_ENDPOINT.into(),
                    body: b"<div class=\"anomaly-modal\">challenge-form</div>".to_vec(),
                },
            )
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let hits = web_search(&transport, "q", &EngineHealth::new(), false).await;
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://rust-lang.org/tools/install/");
    }

    #[tokio::test]
    async fn web_search_empty_when_all_engines_blocked() {
        let mojeek_url = mojeek_request("q", false).url;
        let blocked = |url: &str| HttpResponse {
            status: 429,
            final_url: url.into(),
            body: b"rate limited".to_vec(),
        };
        let transport = FakeHttpTransport::new()
            .with_response(DDG_HTML_ENDPOINT, blocked(DDG_HTML_ENDPOINT))
            .with_response(&mojeek_url, blocked(&mojeek_url));
        assert!(web_search(&transport, "q", &EngineHealth::new(), false)
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn web_search_empty_when_all_engines_transport_error() {
        // No canned responses -> every engine's send errors -> empty.
        let transport = FakeHttpTransport::new();
        assert!(web_search(&transport, "q", &EngineHealth::new(), false)
            .await
            .is_empty());
    }

    // ── EngineHealth cooldown ───────────────────────────────────────────────

    #[test]
    fn health_marks_and_reports_cooling_until_expiry() {
        let health = EngineHealth::new();
        assert!(!health.is_cooling("duckduckgo"));
        health.mark_blocked("duckduckgo", 3600);
        assert!(health.is_cooling("duckduckgo"));
        // A zero-second cooldown is expired on the next read and pruned.
        health.mark_blocked("mojeek", 0);
        assert!(!health.is_cooling("mojeek"));
        assert!(!health.is_cooling("mojeek"));
    }

    #[test]
    fn health_default_is_empty() {
        assert!(!EngineHealth::default().is_cooling("duckduckgo"));
    }

    #[tokio::test]
    async fn web_search_skips_cooling_engine_entirely() {
        // DuckDuckGo is inside its cooldown: no request goes to it at all, and
        // Mojeek serves the query.
        let health = EngineHealth::new();
        health.mark_blocked("duckduckgo", 3600);
        let mojeek_url = mojeek_request("q", false).url;
        let transport = FakeHttpTransport::new()
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let hits = web_search(&transport, "q", &health, false).await;
        assert_eq!(hits.len(), 2);
        assert!(!transport.calls().iter().any(|c| c.url == DDG_HTML_ENDPOINT));
    }

    #[tokio::test]
    async fn web_search_block_marks_cooldown_for_next_query() {
        // First query: DuckDuckGo answers with a challenge -> marked blocked.
        // Second query: DuckDuckGo is skipped without a request (one DDG POST
        // total across both queries).
        let health = EngineHealth::new();
        let mojeek_url = mojeek_request("q", false).url;
        let transport = FakeHttpTransport::new()
            .with_response(
                DDG_HTML_ENDPOINT,
                HttpResponse {
                    status: 202,
                    final_url: DDG_HTML_ENDPOINT.into(),
                    body: b"<div class=\"anomaly-modal\">challenge-form</div>".to_vec(),
                },
            )
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let _ = web_search(&transport, "q", &health, false).await;
        let _ = web_search(&transport, "q", &health, false).await;
        let ddg_posts = transport
            .calls()
            .into_iter()
            .filter(|c| c.url == DDG_HTML_ENDPOINT)
            .count();
        assert_eq!(ddg_posts, 1, "blocked engine must not be re-queried");
        assert!(health.is_cooling("duckduckgo"));
    }
}
