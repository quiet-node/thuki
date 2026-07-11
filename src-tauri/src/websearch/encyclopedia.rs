//! Encyclopedia vertical: Wikipedia, the source for stable definitional and
//! historical facts.
//!
//! General SERP scraping is a poor fit for "what is X" / "who wrote X" style
//! questions when the scraped engines are rate-limited, and Wikipedia's own
//! full-text search plus REST summary API are keyless, stable, and not
//! bot-gated the way SERPs are. The vertical runs only when the classifier
//! routed the turn to `wiki` (a stable definitional/historical fact); the
//! orchestrator gates on that route rather than any substring match here. The
//! flow is search → summary → format: full-text search resolves the question
//! to a canonical article title, the REST summary API returns its lead
//! paragraph, and the result is wrapped into a single [`SourceBlock`] the
//! writer cites.
//!
//! Two deterministic guards defend against the classifier mis-routing a
//! volatile question to `wiki`, because Wikipedia's lead summary describes the
//! stable subject, not its live state (verified live 2026-07-08: the summary
//! for an office such as "President of France" describes the office, not its
//! current holder, and the article for an evolving event is pinned to a past
//! edition). The volatility guard ([`is_volatile_question`]) refuses a question
//! carrying a freshness marker or a present/future year before any request is
//! sent; the year-mismatch guard ([`year_mismatch`]) rejects a resolved article
//! whose title is pinned to a different year than the question asked about.
//! Disambiguation pages are also skipped, since a list of unrelated topics is
//! not an answer. Every guard and every step degrades to `None`, sending the
//! turn down the normal engine path: the vertical can only ever improve a turn,
//! never lose one.
//!
//! Both requests carry a descriptive `User-Agent` identifying Thuki with a
//! contact URL, per
//! [Wikimedia's User-Agent policy](https://meta.wikimedia.org/wiki/User-Agent_policy):
//! verified live 2026-07-08 that a request with no (or a generic) User-Agent
//! is rejected outright with `403`, unlike Open-Meteo or Google News RSS.

use crate::config::defaults::{
    WIKI_VOLATILITY_MARKERS, WIKI_VOLATILITY_MIN_YEAR, WIKI_VOLATILITY_PHRASES,
};
use crate::net::transport::{HttpMethod, HttpRequest, HttpTransport};
use crate::websearch::assemble::SourceBlock;

/// Wikipedia's keyless full-text search and REST summary endpoints.
const SEARCH_ENDPOINT: &str = "https://en.wikipedia.org/w/api.php";
const SUMMARY_ENDPOINT: &str = "https://en.wikipedia.org/api/rest_v1/page/summary";

/// Attribution line required by Wikipedia's CC BY-SA 4.0 licence.
const ATTRIBUTION: &str = "Source: Wikipedia (CC BY-SA 4.0)";

/// Descriptive User-Agent Wikimedia's API etiquette policy requires; a
/// missing or generic one is rejected with `403` (verified live 2026-07-08).
const WIKI_USER_AGENT: &str = "Thuki/1.0 (https://thuki.app; contact@thuki.app)";

/// A resolved Wikipedia summary: its canonical title, lead-paragraph extract,
/// and the article's page URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WikiSummary {
    pub(crate) title: String,
    pub(crate) extract: String,
    pub(crate) page_url: String,
}

/// Whether `question` carries a freshness signal that disqualifies the
/// Wikipedia vertical even when the classifier routed the turn to `wiki`. True
/// when the lowercased question contains any [`WIKI_VOLATILITY_MARKERS`] token,
/// any [`WIKI_VOLATILITY_PHRASES`] whole phrase, or a 4-digit year at or after
/// [`WIKI_VOLATILITY_MIN_YEAR`]. Wikipedia's lead summary answers the stable
/// subject, never its live state, so such a question must fall through to the
/// news / engine tiers.
///
/// This is also the sole freshness signal for the rest of the search pipeline
/// (see `orchestrator::run_web`'s `freshness` variable): the same `true`/`false`
/// gates the DuckDuckGo/Google News date-bias operators and the recency-prior
/// fusion re-ranking, not just the Wikipedia vertical. `WIKI_VOLATILITY_PHRASES`
/// documents the age/biography patterns in detail, including which related
/// phrasings are deliberately excluded and why.
pub(crate) fn is_volatile_question(question: &str) -> bool {
    let lower = question.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.iter().any(|t| WIKI_VOLATILITY_MARKERS.contains(t)) {
        return true;
    }
    if tokens
        .iter()
        .any(|t| four_digit_year(t).is_some_and(|y| y >= WIKI_VOLATILITY_MIN_YEAR))
    {
        return true;
    }
    // Pad the token stream so a phrase matches only on whole-word boundaries.
    let padded = format!(" {} ", tokens.join(" "));
    WIKI_VOLATILITY_PHRASES
        .iter()
        .any(|phrase| padded.contains(&format!(" {phrase} ")))
}

/// Whether `question` names a 4-digit year that conflicts with the resolved
/// article `title`: the question names at least one year, the title names at
/// least one year, and they share none. This rejects a Wikipedia hit pinned to
/// a different edition than the question asked about (e.g. a "2026" question
/// resolving to a "2023" article). When the question names no year, or the
/// title is not year-pinned, or they share a year, there is no mismatch.
pub(crate) fn year_mismatch(question: &str, title: &str) -> bool {
    let q_years = four_digit_years(question);
    if q_years.is_empty() {
        return false;
    }
    let t_years = four_digit_years(title);
    if t_years.is_empty() {
        return false;
    }
    !q_years.iter().any(|y| t_years.contains(y))
}

/// Parses `token` as a bare 4-digit year, or `None` when it is not exactly four
/// ASCII digits. Used by both volatility and year-mismatch guards.
fn four_digit_year(token: &str) -> Option<u32> {
    if token.len() == 4 && token.bytes().all(|b| b.is_ascii_digit()) {
        token.parse::<u32>().ok()
    } else {
        None
    }
}

/// Collects every distinct 4-digit year token in `text`, in first-seen order.
fn four_digit_years(text: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        if let Some(year) = four_digit_year(token) {
            if !out.contains(&year) {
                out.push(year);
            }
        }
    }
    out
}

/// Builds the full-text search GET request resolving `question` to a
/// candidate article title.
pub(crate) fn search_request(question: &str) -> HttpRequest {
    // SEARCH_ENDPOINT is a compile-time-valid absolute URL.
    let mut url = url::Url::parse(SEARCH_ENDPOINT).expect("static endpoint");
    url.query_pairs_mut()
        .append_pair("action", "query")
        .append_pair("list", "search")
        .append_pair("srsearch", question)
        .append_pair("srlimit", "1")
        .append_pair("format", "json");
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: vec![("User-Agent".to_string(), WIKI_USER_AGENT.to_string())],
        form: Vec::new(),
    }
}

/// Parses a search response into the top result's title, or `None` when
/// nothing matched.
pub(crate) fn parse_search_title(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let top = json.get("query")?.get("search")?.get(0)?;
    top.get("title")?.as_str().map(str::to_string)
}

/// Builds the REST summary GET request for `title`.
pub(crate) fn summary_request(title: &str) -> HttpRequest {
    // SUMMARY_ENDPOINT is a compile-time-valid absolute URL.
    let mut url = url::Url::parse(SUMMARY_ENDPOINT).expect("static endpoint");
    url.path_segments_mut()
        .expect("https URL has a path")
        .push(title);
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: vec![("User-Agent".to_string(), WIKI_USER_AGENT.to_string())],
        form: Vec::new(),
    }
}

/// Parses a REST summary response into a [`WikiSummary`], or `None` when the
/// page is a disambiguation page, missing, or otherwise not a plain article
/// with a non-empty extract.
pub(crate) fn parse_summary(body: &str) -> Option<WikiSummary> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    if json.get("type")?.as_str()? != "standard" {
        return None;
    }
    let title = json.get("title")?.as_str()?.to_string();
    let extract = json.get("extract")?.as_str()?.trim().to_string();
    if extract.is_empty() {
        return None;
    }
    let page_url = json
        .get("content_urls")
        .and_then(|c| c.get("desktop"))
        .and_then(|d| d.get("page"))
        .and_then(|p| p.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://en.wikipedia.org/wiki/{title}"));
    Some(WikiSummary {
        title,
        extract,
        page_url,
    })
}

/// Wraps a resolved summary into the single `[1]` source block the writer
/// cites.
pub(crate) fn wiki_source_block(summary: &WikiSummary) -> SourceBlock {
    SourceBlock {
        index: 1,
        url: summary.page_url.clone(),
        title: summary.title.clone(),
        text: format!("{}\n{ATTRIBUTION}.", summary.extract),
    }
}

/// Runs the full encyclopedia vertical for `standalone_question`: full-text
/// search, summary fetch, disambiguation filter, and the year-mismatch guard.
/// The caller (orchestrator) is responsible for gating on the `wiki` route and
/// the [`is_volatile_question`] guard before calling this. Returns `None` on any
/// miss so the caller falls through to the scraped-engine tier.
///
/// Coverage-excluded: thin async glue over the injectable transport
/// delegating every decision to the pure helpers above, which are all tested
/// directly; the glue itself is still exercised against
/// [`crate::net::transport::FakeHttpTransport`] in the tests below.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) async fn fetch_encyclopedia(
    transport: &dyn HttpTransport,
    standalone_question: &str,
) -> Option<SourceBlock> {
    let search_response = match transport.send(&search_request(standalone_question)).await {
        Ok(response) => response,
        Err(e) => {
            eprintln!("[search] vertical=wiki search_transport_error {e}");
            return None;
        }
    };
    if search_response.status != 200 {
        eprintln!(
            "[search] vertical=wiki search_status={} -> engines",
            search_response.status
        );
        return None;
    }
    let Some(title) = parse_search_title(&String::from_utf8_lossy(&search_response.body)) else {
        eprintln!("[search] vertical=wiki no_search_hit -> engines");
        return None;
    };
    let summary_response = match transport.send(&summary_request(&title)).await {
        Ok(response) => response,
        Err(e) => {
            eprintln!("[search] vertical=wiki summary_transport_error {e}");
            return None;
        }
    };
    if summary_response.status != 200 {
        eprintln!(
            "[search] vertical=wiki summary_status={} title={title:?} -> engines",
            summary_response.status
        );
        return None;
    }
    let Some(summary) = parse_summary(&String::from_utf8_lossy(&summary_response.body)) else {
        eprintln!("[search] vertical=wiki summary_unusable title={title:?} -> engines");
        return None;
    };
    // Year-mismatch guard: a resolved article pinned to a different year than
    // the question asked about is the wrong edition (e.g. a 2026 question
    // resolving to a 2023 event article), so fall through to the engines.
    if year_mismatch(standalone_question, &summary.title) {
        eprintln!(
            "[search] vertical=wiki year_mismatch title={:?} -> engines",
            summary.title
        );
        return None;
    }
    eprintln!("[search] vertical=wiki title={}", summary.title);
    Some(wiki_source_block(&summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{FakeHttpTransport, HttpResponse};

    /// Real Wikipedia search response shape (captured live 2026-07-08).
    const SEARCH_FIXTURE: &str = r#"{"batchcomplete":"","query":{"searchinfo":{"totalhits":100},"search":[{"ns":0,"title":"Photosynthesis","pageid":24544}]}}"#;

    /// Real Wikipedia REST summary response shape (captured live 2026-07-08,
    /// trimmed to the fields this module reads).
    const SUMMARY_FIXTURE: &str = r#"{"type":"standard","title":"Photosynthesis","extract":"Photosynthesis is a system of biological processes by which photopigment-bearing organisms convert light energy into chemical energy.","content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Photosynthesis"}}}"#;

    const DISAMBIGUATION_FIXTURE: &str = r#"{"type":"disambiguation","title":"Mercury","extract":"Mercury most commonly refers to: Mercury (planet)...","content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Mercury"}}}"#;

    // ── volatility guard ─────────────────────────────────────────────────────

    #[test]
    fn volatility_guard_passes_stable_questions() {
        // Stable definitional/historical facts carry no freshness signal.
        assert!(!is_volatile_question("what is photosynthesis"));
        assert!(!is_volatile_question("who wrote Hamlet"));
        assert!(!is_volatile_question(
            "when was the Eiffel Tower built in 1889"
        ));
        assert!(!is_volatile_question("the 2018 FIFA World Cup final"));
    }

    #[test]
    fn volatility_guard_refuses_freshness_markers() {
        // Marker words, whole-phrase markers, and present/future years all trip.
        assert!(is_volatile_question("what is the latest iOS version"));
        assert!(is_volatile_question("current president of France"));
        assert!(is_volatile_question("what is the status of the merger"));
        assert!(is_volatile_question("what is trending right now"));
        assert!(is_volatile_question("the best phones this year"));
        assert!(is_volatile_question("what is the World Cup 2026"));
    }

    #[test]
    fn volatility_guard_marker_matches_whole_tokens_only() {
        // "recentralise" contains "recent" as a substring but is not the marker
        // token; whole-token matching must not trip on it.
        assert!(!is_volatile_question("what does recentralise mean"));
    }

    // ── age/biography patterns (live-smoke fix, 2026-07-11) ──────────────────
    //
    // "how old is Tom Cruise" was answered stale (63 instead of the correct 64)
    // because no freshness signal fired, so recency fusion and the engines'
    // date-bias never engaged and every retrieved source was a stale
    // pre-birthday page. These patterns close that gap.

    #[test]
    fn age_question_present_tense_is_volatile() {
        assert!(is_volatile_question("how old is Tom Cruise"));
        assert!(is_volatile_question("what age is the current CEO of Apple"));
    }

    #[test]
    fn possessive_age_phrasing_is_volatile() {
        // Tokenisation splits the apostrophe, so "Cruise's age" reads as the
        // "s age" phrase.
        assert!(is_volatile_question("what is Tom Cruise's age"));
    }

    #[test]
    fn how_long_ago_is_volatile() {
        // A duration from a fixed past date to now changes every year.
        assert!(is_volatile_question("how long ago did the merger happen"));
    }

    #[test]
    fn anniversary_marker_is_volatile() {
        assert!(is_volatile_question("company anniversary announcement"));
    }

    #[test]
    fn eternal_fact_age_question_is_a_known_accepted_over_match() {
        // Documented tradeoff (see WIKI_VOLATILITY_PHRASES rustdoc): no cheap
        // guard distinguishes a person from an era, so this fires too. That is
        // accepted because it only ever adds a mild recency bias, never an
        // incorrect answer.
        assert!(is_volatile_question("how old is the universe"));
    }

    #[test]
    fn past_tense_age_question_is_not_volatile() {
        // Past-tense age at a fixed historical event never changes; flagging it
        // would wrongly disqualify the Wikipedia vertical for a question it
        // answers well (see the `historical_attribute` rows in
        // search_decision_eval.jsonl).
        assert!(!is_volatile_question("how old was Napoleon when he died"));
        assert!(!is_volatile_question(
            "what age was Einstein when he published the theory of relativity"
        ));
    }

    #[test]
    fn birth_date_question_is_not_volatile() {
        // A fixed historical date carries no yearly-changing component.
        assert!(!is_volatile_question("when was Einstein born"));
    }

    #[test]
    fn bare_age_of_phrasing_is_not_volatile() {
        // "age of" is dominated by eternal/historical-era idioms in ordinary
        // English, so it is deliberately not a marker.
        assert!(!is_volatile_question("what is the age of enlightenment"));
    }

    // ── year-mismatch guard ──────────────────────────────────────────────────

    #[test]
    fn year_mismatch_true_when_years_differ() {
        assert!(year_mismatch(
            "what is the status of the World Cup 2026",
            "2023 FIFA Women's World Cup"
        ));
    }

    #[test]
    fn year_mismatch_false_without_conflict() {
        // No year in the question.
        assert!(!year_mismatch("what is photosynthesis", "Photosynthesis"));
        // No year in the title.
        assert!(!year_mismatch("the 2026 world cup", "FIFA World Cup"));
        // Shared year.
        assert!(!year_mismatch("2026 world cup", "2026 FIFA World Cup"));
    }

    #[test]
    fn four_digit_years_extracts_distinct_years_only() {
        assert_eq!(
            four_digit_years("in 2026 and 2026 and 2030"),
            vec![2026, 2030]
        );
        // 12345 is a 5-digit run, not a year; "wc2026" is not a bare token.
        assert_eq!(four_digit_years("12345 wc2026"), Vec::<u32>::new());
    }

    // ── request builders ─────────────────────────────────────────────────────

    #[test]
    fn search_request_carries_question_and_single_result() {
        let req = search_request("what is photosynthesis");
        assert_eq!(req.method, HttpMethod::Get);
        assert!(req.url.starts_with(SEARCH_ENDPOINT));
        assert!(req.url.contains("srsearch=what+is+photosynthesis"));
        assert!(req.url.contains("srlimit=1"));
        // Wikimedia's API policy rejects requests with no descriptive UA.
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "User-Agent" && v == WIKI_USER_AGENT));
    }

    #[test]
    fn summary_request_percent_encodes_title_path_segment() {
        let req = summary_request("President of France");
        assert!(req
            .url
            .starts_with("https://en.wikipedia.org/api/rest_v1/page/summary/President"));
        assert!(req.url.contains("%20") || req.url.contains("of%20France"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "User-Agent" && v == WIKI_USER_AGENT));
    }

    // ── parsers ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_search_title_reads_top_hit() {
        assert_eq!(
            parse_search_title(SEARCH_FIXTURE).as_deref(),
            Some("Photosynthesis")
        );
    }

    #[test]
    fn parse_search_title_none_on_no_hits_or_junk() {
        let empty = r#"{"query":{"search":[]}}"#;
        assert!(parse_search_title(empty).is_none());
        assert!(parse_search_title("not json").is_none());
    }

    #[test]
    fn parse_summary_reads_standard_article() {
        let summary = parse_summary(SUMMARY_FIXTURE).unwrap();
        assert_eq!(summary.title, "Photosynthesis");
        assert!(summary.extract.starts_with("Photosynthesis is"));
        assert_eq!(
            summary.page_url,
            "https://en.wikipedia.org/wiki/Photosynthesis"
        );
    }

    #[test]
    fn parse_summary_none_on_disambiguation() {
        assert!(parse_summary(DISAMBIGUATION_FIXTURE).is_none());
    }

    #[test]
    fn parse_summary_none_on_missing_or_empty_extract() {
        let no_extract = r#"{"type":"standard","title":"X"}"#;
        assert!(parse_summary(no_extract).is_none());
        let blank_extract = r#"{"type":"standard","title":"X","extract":"   "}"#;
        assert!(parse_summary(blank_extract).is_none());
        assert!(parse_summary("junk").is_none());
    }

    #[test]
    fn parse_summary_falls_back_to_constructed_url_without_content_urls() {
        let body = r#"{"type":"standard","title":"Photosynthesis","extract":"Photosynthesis is a process."}"#;
        let summary = parse_summary(body).unwrap();
        assert_eq!(
            summary.page_url,
            "https://en.wikipedia.org/wiki/Photosynthesis"
        );
    }

    // ── source block ─────────────────────────────────────────────────────────

    #[test]
    fn source_block_carries_extract_and_attribution() {
        let summary = WikiSummary {
            title: "Photosynthesis".to_string(),
            extract: "Photosynthesis is a process.".to_string(),
            page_url: "https://en.wikipedia.org/wiki/Photosynthesis".to_string(),
        };
        let block = wiki_source_block(&summary);
        assert_eq!(block.index, 1);
        assert_eq!(block.url, summary.page_url);
        assert_eq!(block.title, "Photosynthesis");
        assert!(block.text.contains("Photosynthesis is a process."));
        assert!(block.text.contains("Wikipedia (CC BY-SA 4.0)"));
    }

    // ── fetch_encyclopedia over the fake transport ───────────────────────────

    #[tokio::test]
    async fn fetch_encyclopedia_resolves_full_chain() {
        let search_url = search_request("what is photosynthesis").url;
        let summary_url = summary_request("Photosynthesis").url;
        let transport = FakeHttpTransport::new()
            .with_response(
                &search_url,
                HttpResponse {
                    status: 200,
                    final_url: search_url.clone(),
                    body: SEARCH_FIXTURE.as_bytes().to_vec(),
                },
            )
            .with_response(
                &summary_url,
                HttpResponse {
                    status: 200,
                    final_url: summary_url.clone(),
                    body: SUMMARY_FIXTURE.as_bytes().to_vec(),
                },
            );
        let block = fetch_encyclopedia(&transport, "what is photosynthesis")
            .await
            .unwrap();
        assert_eq!(block.title, "Photosynthesis");
        assert!(block.text.contains("Photosynthesis is"));
    }

    #[tokio::test]
    async fn fetch_encyclopedia_none_when_search_fails() {
        // Search transport error (no canned response): the vertical falls
        // through. Routing/volatility gating is the orchestrator's job now, so
        // fetch_encyclopedia always issues the search request when called.
        let transport = FakeHttpTransport::new();
        assert!(fetch_encyclopedia(&transport, "what is photosynthesis")
            .await
            .is_none());
        assert!(transport.calls().iter().any(|c| c.url.contains("srsearch")));
    }

    #[tokio::test]
    async fn fetch_encyclopedia_none_on_year_mismatch() {
        // The question names 2026 but the resolved article is a 2023 edition:
        // the year-mismatch guard rejects it so the turn reaches the engines.
        let question = "what is the status of the World Cup 2026";
        let search_url = search_request(question).url;
        let summary_url = summary_request("2023 FIFA Women's World Cup").url;
        let transport = FakeHttpTransport::new()
            .with_response(
                &search_url,
                HttpResponse {
                    status: 200,
                    final_url: search_url.clone(),
                    body: r#"{"query":{"search":[{"title":"2023 FIFA Women's World Cup"}]}}"#
                        .as_bytes()
                        .to_vec(),
                },
            )
            .with_response(
                &summary_url,
                HttpResponse {
                    status: 200,
                    final_url: summary_url.clone(),
                    body: r#"{"type":"standard","title":"2023 FIFA Women's World Cup","extract":"The 2023 tournament was held in Australia and New Zealand.","content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/2023_FIFA_Women%27s_World_Cup"}}}"#
                        .as_bytes()
                        .to_vec(),
                },
            );
        assert!(fetch_encyclopedia(&transport, question).await.is_none());
    }

    #[tokio::test]
    async fn fetch_encyclopedia_none_when_summary_is_disambiguation() {
        let search_url = search_request("what is mercury").url;
        let summary_url = summary_request("Mercury").url;
        let transport = FakeHttpTransport::new()
            .with_response(
                &search_url,
                HttpResponse {
                    status: 200,
                    final_url: search_url.clone(),
                    body: r#"{"query":{"search":[{"title":"Mercury"}]}}"#.as_bytes().to_vec(),
                },
            )
            .with_response(
                &summary_url,
                HttpResponse {
                    status: 200,
                    final_url: summary_url.clone(),
                    body: DISAMBIGUATION_FIXTURE.as_bytes().to_vec(),
                },
            );
        assert!(fetch_encyclopedia(&transport, "what is mercury")
            .await
            .is_none());
    }
}
