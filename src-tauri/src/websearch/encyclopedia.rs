//! Encyclopedia vertical: Wikipedia, the source for stable definitional and
//! historical facts.
//!
//! General SERP scraping is a poor fit for "what is X" / "who wrote X" style
//! questions when the scraped engines are rate-limited, and Wikipedia's own
//! full-text search plus REST summary API are keyless, stable, and not
//! bot-gated the way SERPs are. When a turn's standalone question is
//! recognisably a stable factual/definitional question, the flow is
//! search → summary → format: full-text search resolves the question to a
//! canonical article title, the REST summary API returns its lead paragraph,
//! and the result is wrapped into a single [`SourceBlock`] the writer cites.
//!
//! Deliberately excludes bare "who is" questions: verified live 2026-07-08,
//! Wikipedia's lead summary for an office ("President of France") describes
//! the office, not its current holder, so "who is the (current) president of
//! X" would surface a non-answer dressed as a citation. Only verbs that
//! signal a stable, non-volatile fact ("who wrote", "when was", "define")
//! trigger this vertical; volatile-officeholder questions fall through to the
//! scraped-engine/news tiers instead. Also skips disambiguation pages, since a
//! list of unrelated topics is not an answer. Every step degrades to `None`,
//! sending the turn down the normal engine path: the vertical can only ever
//! improve a turn, never lose one.
//!
//! Both requests carry a descriptive `User-Agent` identifying Thuki with a
//! contact URL, per
//! [Wikimedia's User-Agent policy](https://meta.wikimedia.org/wiki/User-Agent_policy):
//! verified live 2026-07-08 that a request with no (or a generic) User-Agent
//! is rejected outright with `403`, unlike Open-Meteo or Google News RSS.

use crate::net::transport::{HttpMethod, HttpRequest, HttpTransport};
use crate::websearch::assemble::SourceBlock;

/// Phrases that signal a stable, definitional/historical question this
/// vertical can answer confidently. Matched as substrings of the lowercased
/// standalone question; each is distinctive enough as a phrase that a false
/// positive substring match is not a realistic concern.
const ENCYCLOPEDIA_PHRASES: &[&str] = &[
    "what is",
    "what are",
    "what was",
    "what were",
    "define",
    "definition of",
    "meaning of",
    "who wrote",
    "who invented",
    "who discovered",
    "who founded",
    "who created",
    "when was",
    "when did",
    "capital of",
    "population of",
    "explain",
];

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

/// Whether `question` is a stable factual/definitional question this vertical
/// should try to answer.
pub(crate) fn is_encyclopedia_intent(question: &str) -> bool {
    let lower = question.to_lowercase();
    ENCYCLOPEDIA_PHRASES.iter().any(|p| lower.contains(p))
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

/// Runs the full encyclopedia vertical for `standalone_question`: intent
/// check, full-text search, summary fetch, disambiguation filter. Returns
/// `None` on any miss so the caller falls through to the scraped-engine tier.
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
    if !is_encyclopedia_intent(standalone_question) {
        return None;
    }
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

    // ── intent gate ──────────────────────────────────────────────────────────

    #[test]
    fn encyclopedia_intent_matches_stable_factual_questions() {
        assert!(is_encyclopedia_intent("what is photosynthesis"));
        assert!(is_encyclopedia_intent("who wrote Hamlet"));
        assert!(is_encyclopedia_intent("define entropy"));
        assert!(is_encyclopedia_intent("when was the Eiffel Tower built"));
        assert!(is_encyclopedia_intent("capital of Japan"));
    }

    #[test]
    fn encyclopedia_intent_excludes_volatile_and_unrelated_questions() {
        // Bare "who is" is deliberately not a trigger: volatile officeholder
        // questions must not be answered from a static lead summary.
        assert!(!is_encyclopedia_intent("who is the president of France"));
        assert!(!is_encyclopedia_intent("weather in Tokyo"));
        assert!(!is_encyclopedia_intent("who won the F1 race"));
        assert!(!is_encyclopedia_intent("tell me a joke"));
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
    async fn fetch_encyclopedia_none_when_not_encyclopedic_or_search_fails() {
        let transport = FakeHttpTransport::new();
        // Not an encyclopedic question: no request is even sent.
        assert!(fetch_encyclopedia(&transport, "who won the F1 race")
            .await
            .is_none());
        assert!(transport.calls().is_empty());
        // Encyclopedic question but search transport error: falls through.
        assert!(fetch_encyclopedia(&transport, "what is photosynthesis")
            .await
            .is_none());
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
