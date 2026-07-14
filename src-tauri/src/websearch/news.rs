//! News vertical: Google News RSS, the intent-routed source for current events.
//!
//! Current-events questions ("who won the race", "any news about X") are served
//! poorly by general SERP scraping when the scraped engines are rate-limited,
//! and Google News' RSS search feed is a decade-stable, keyless XML endpoint
//! that is not bot-gated the way SERPs are. When a turn's question is
//! recognisably news intent, the feed is queried first and its headlines are
//! assembled into a single dated source block the writer cites directly.
//!
//! Headlines only, deliberately: the feed's article links are opaque
//! `news.google.com/rss/articles/...` URLs that no longer redirect for non-JS
//! clients (verified live 2026-07-08: they return a ~600 KB JS interstitial),
//! so fetching them would feed the readability stage junk. Fresh headlines with
//! publisher and date are information-dense enough to answer the who-won /
//! what-happened class of question on their own. A feed miss falls through to
//! the scraped engines with nothing lost.
//!
//! The parser is hand-rolled over the feed's stable `<item>` shape rather than
//! pulling in an XML dependency: it is pure, total (malformed XML yields fewer
//! rows, never a panic), and bounded by [`MAX_NEWS_ITEMS`].

use crate::net::transport::{HttpMethod, HttpRequest, HttpTransport};
use crate::websearch::assemble::SourceBlock;
use crate::websearch::lang::news_locale;
use crate::websearch::THUKI_USER_AGENT;

/// Words that signal a current-events question. Matched on whole tokens of the
/// lowercased standalone question.
const NEWS_WORDS: &[&str] = &[
    "news",
    "headline",
    "headlines",
    "breaking",
    "won",
    "wins",
    "winner",
    "election",
    "elections",
    "match",
    "game",
    "race",
    "championship",
    "tournament",
    "announced",
    "announcement",
    "happened",
];

/// Google News RSS search endpoint. The `rss/search` form is used deliberately:
/// topic paths redirect to opaque hash URLs, the search form has been stable
/// for a decade.
const NEWS_ENDPOINT: &str = "https://news.google.com/rss/search";

/// Maximum headlines taken from one feed: enough breadth to answer a
/// current-events question without flooding the writer's source budget.
const MAX_NEWS_ITEMS: usize = 6;

/// One parsed feed row: a headline (with its ` - Publisher` suffix) and its
/// publication date string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewsItem {
    pub(crate) title: String,
    pub(crate) date: String,
}

/// Whether `question` is a current-events question the news feed should serve.
pub(crate) fn is_news_intent(question: &str) -> bool {
    question
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .any(|t| NEWS_WORDS.contains(&t))
}

/// Builds the Google News RSS search GET request for `query`. When `freshness`
/// is set (the turn's standalone question carried a freshness signal),
/// [`crate::config::defaults::NEWS_FRESHNESS_OPERATOR`] is appended to the
/// query: the feed's default ordering skews stale, and the operator narrows it
/// to recent coverage.
///
/// The feed's locale triple (`hl`, `gl`, `ceid`) follows the turn's resolved
/// `lang` (see [`crate::websearch::lang::resolve_lang`]) and is derived from a
/// single allowlist row ([`news_locale`]), so the three values cannot disagree.
/// That matters more here than anywhere else: an inconsistent triple does not
/// error and does not return an empty feed, it silently serves the ENGLISH feed,
/// which would surface English headlines labelled as the user's language. An
/// unallowlisted language sends no locale parameters at all.
pub(crate) fn news_request(query: &str, freshness: bool, lang: &str) -> HttpRequest {
    // NEWS_ENDPOINT is a compile-time-valid absolute URL.
    let mut url = url::Url::parse(NEWS_ENDPOINT).expect("static endpoint");
    let q = if freshness {
        format!(
            "{query} {}",
            crate::config::defaults::NEWS_FRESHNESS_OPERATOR
        )
    } else {
        query.to_string()
    };
    url.query_pairs_mut().append_pair("q", &q);
    if let Some(locale) = news_locale(lang) {
        url.query_pairs_mut()
            .append_pair("hl", &locale.hl())
            .append_pair("gl", locale.gl())
            .append_pair("ceid", &locale.ceid());
    }
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: vec![("User-Agent".to_string(), THUKI_USER_AGENT.to_string())],
        form: Vec::new(),
    }
}

/// Parses a Google News RSS body into headline rows: one per `<item>`, title
/// from `<title>` (CDATA unwrapped, entities unescaped) and date from
/// `<pubDate>`. Titleless rows are skipped; output is capped at
/// [`MAX_NEWS_ITEMS`].
pub(crate) fn parse_news_rss(body: &str) -> Vec<NewsItem> {
    let mut items = Vec::new();
    for chunk in body.split("<item>").skip(1) {
        if items.len() >= MAX_NEWS_ITEMS {
            break;
        }
        let chunk = chunk.split("</item>").next().unwrap_or(chunk);
        let Some(raw_title) = tag_text(chunk, "title") else {
            continue;
        };
        let title = unescape_xml(&unwrap_cdata(&raw_title));
        if title.trim().is_empty() {
            continue;
        }
        let date = tag_text(chunk, "pubDate").unwrap_or_default();
        items.push(NewsItem {
            title: title.trim().to_string(),
            date: date.trim().to_string(),
        });
    }
    items
}

/// Assembles headline rows into the single `[1]` source block the writer
/// cites: one dated headline per line.
pub(crate) fn news_source_block(items: &[NewsItem], query: &str) -> SourceBlock {
    let mut text = format!("Recent news headlines for \"{query}\":");
    for item in items {
        text.push_str("\n- ");
        text.push_str(&item.title);
        if !item.date.is_empty() {
            text.push_str(&format!(" ({})", item.date));
        }
    }
    SourceBlock {
        index: 1,
        url: "https://news.google.com/".to_string(),
        title: format!("Google News headlines: {query}"),
        text,
    }
}

/// Extracts the inner text of the first `<tag>...</tag>` in `chunk`, or `None`.
fn tag_text(chunk: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = chunk.find(&open)? + open.len();
    let end = chunk[start..].find(&close)? + start;
    Some(chunk[start..end].to_string())
}

/// Unwraps a `<![CDATA[...]]>` wrapper when present.
fn unwrap_cdata(text: &str) -> String {
    text.trim()
        .strip_prefix("<![CDATA[")
        .and_then(|t| t.strip_suffix("]]>"))
        .unwrap_or(text.trim())
        .to_string()
}

/// Unescapes the XML entities the feed uses in titles.
fn unescape_xml(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Queries the news feed for `query` and returns the assembled headline source
/// block, or `None` on any failure (transport error, non-200, empty feed) so
/// the caller falls through to the engine tier.
///
/// Coverage-excluded: thin async glue over the injectable transport delegating
/// every decision to the pure helpers above; still exercised against
/// [`crate::net::transport::FakeHttpTransport`] in the tests below.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) async fn fetch_news(
    transport: &dyn HttpTransport,
    query: &str,
    freshness: bool,
    lang: &str,
) -> Option<SourceBlock> {
    let response = match transport.send(&news_request(query, freshness, lang)).await {
        Ok(response) => response,
        Err(_) => {
            eprintln!("[search] vertical=news transport_error -> engines");
            return None;
        }
    };
    if response.status != 200 {
        eprintln!(
            "[search] vertical=news status={} -> engines",
            response.status
        );
        return None;
    }
    let items = parse_news_rss(&String::from_utf8_lossy(&response.body));
    if items.is_empty() {
        eprintln!("[search] vertical=news empty -> engines");
        return None;
    }
    eprintln!("[search] vertical=news headlines={}", items.len());
    Some(news_source_block(&items, query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{FakeHttpTransport, HttpResponse};

    /// Mirrors the live feed shape (captured 2026-07-08): plain titles with a
    /// ` - Publisher` suffix, pubDate rows, and CDATA/entity variants.
    const RSS_FIXTURE: &str = r#"<?xml version="1.0"?><rss><channel>
      <title>"F1" - Google News</title>
      <item><title>Leclerc wins dramatic British GP - Formula 1</title>
        <link>https://news.google.com/rss/articles/CBMiAAA</link>
        <pubDate>Wed, 08 Jul 2026 01:11:35 GMT</pubDate></item>
      <item><title><![CDATA[Verstappen &amp; Norris clash - BBC Sport]]></title>
        <link>https://news.google.com/rss/articles/CBMiBBB</link></item>
      <item><link>https://news.google.com/notitle</link></item>
      <item><title>   </title><link>https://news.google.com/blank</link></item>
    </channel></rss>"#;

    // ── intent gate ──────────────────────────────────────────────────────────

    #[test]
    fn news_intent_matches_current_events_questions() {
        assert!(is_news_intent("who won the most recent F1 race"));
        assert!(is_news_intent("any news about the merger"));
        assert!(is_news_intent("latest headlines"));
        assert!(!is_news_intent("what is the capital of France"));
        assert!(!is_news_intent("weather in Tokyo"));
    }

    // ── request builder ──────────────────────────────────────────────────────

    #[test]
    fn news_request_is_get_on_rss_search() {
        let req = news_request("f1 race winner", false, "en");
        assert_eq!(req.method, HttpMethod::Get);
        assert!(req.url.starts_with(NEWS_ENDPOINT));
        assert!(req.url.contains("q=f1+race+winner"));
        assert!(req.url.contains("ceid=US%3Aen"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "User-Agent" && v == THUKI_USER_AGENT));
    }

    #[test]
    fn news_request_carries_the_locale_triple_of_the_request_language() {
        // The language is the TURN's, passed in. All three parameters come from
        // one row, so they cannot disagree: a disagreeing triple would silently
        // serve the ENGLISH feed under a Vietnamese label.
        let req = news_request("tin tức mới nhất", false, "vi");
        assert!(req.url.contains("hl=vi-VN"), "{}", req.url);
        assert!(req.url.contains("gl=VN"), "{}", req.url);
        assert!(req.url.contains("ceid=VN%3Avi"), "{}", req.url);
    }

    #[test]
    fn an_unallowlisted_language_sends_no_locale_parameters() {
        // No row, so no triple at all, which serves the English feed honestly
        // rather than assembling an unverified triple that would serve the
        // English feed while claiming to be the user's language.
        for hostile in ["evil", "../../vi", "xx", ""] {
            let req = news_request("q", false, hostile);
            assert!(!req.url.contains("ceid="), "{}", req.url);
            assert!(!req.url.contains("hl="), "{}", req.url);
        }
    }

    #[test]
    fn news_request_on_an_english_query_keeps_the_us_feed() {
        // The English row is exactly what this request always sent.
        let locale = news_locale("en").expect("english has a news row");
        assert_eq!(locale.hl(), "en-US");
        assert_eq!(locale.gl(), "US");
        assert_eq!(locale.ceid(), "US:en");
    }

    // ── freshness operator ──────────────────────────────────────────────────

    #[test]
    fn news_request_carries_no_freshness_operator_by_default() {
        let req = news_request("f1 race winner", false, "en");
        assert!(!req.url.contains("when"));
    }

    #[test]
    fn news_request_appends_when_7d_when_fresh() {
        let req = news_request("f1 race winner", true, "en");
        assert!(
            req.url.contains("when%3A7d") || req.url.contains("when:7d"),
            "expected when:7d operator in {}",
            req.url
        );
        assert!(req.url.contains("f1") && req.url.contains("winner"));
    }

    // ── parser / block assembly ──────────────────────────────────────────────

    #[test]
    fn parse_rss_extracts_titles_and_dates() {
        let items = parse_news_rss(RSS_FIXTURE);
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].title,
            "Leclerc wins dramatic British GP - Formula 1"
        );
        assert_eq!(items[0].date, "Wed, 08 Jul 2026 01:11:35 GMT");
        // CDATA unwrapped and entity unescaped; missing pubDate degrades empty.
        assert_eq!(items[1].title, "Verstappen & Norris clash - BBC Sport");
        assert_eq!(items[1].date, "");
    }

    #[test]
    fn parse_rss_empty_on_junk_and_caps_items() {
        assert!(parse_news_rss("not xml at all").is_empty());
        let many: String = (0..MAX_NEWS_ITEMS + 4)
            .map(|i| format!("<item><title>T{i} - P</title></item>"))
            .collect();
        assert_eq!(parse_news_rss(&many).len(), MAX_NEWS_ITEMS);
    }

    #[test]
    fn cdata_and_entities_handled() {
        assert_eq!(unwrap_cdata("<![CDATA[x]]>"), "x");
        assert_eq!(unwrap_cdata("plain"), "plain");
        assert_eq!(unescape_xml("a &amp; b &#39;c&#39;"), "a & b 'c'");
    }

    #[test]
    fn source_block_lists_dated_headlines() {
        let items = parse_news_rss(RSS_FIXTURE);
        let block = news_source_block(&items, "f1 race");
        assert_eq!(block.index, 1);
        assert_eq!(block.url, "https://news.google.com/");
        assert!(block.title.contains("f1 race"));
        assert!(block
            .text
            .contains("- Leclerc wins dramatic British GP - Formula 1 (Wed, 08 Jul 2026"));
        assert!(block
            .text
            .contains("- Verstappen & Norris clash - BBC Sport"));
    }

    // ── fetch_news over the fake transport ───────────────────────────────────

    #[tokio::test]
    async fn fetch_news_returns_block_on_ok_feed() {
        let url = news_request("f1", false, "en").url;
        let transport = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 200,
                final_url: url.clone(),
                body: RSS_FIXTURE.as_bytes().to_vec(),
            },
        );
        let block = fetch_news(&transport, "f1", false, "en").await.unwrap();
        assert!(block.text.contains("Leclerc"));
    }

    #[tokio::test]
    async fn fetch_news_none_on_error_bad_status_or_empty() {
        // No canned response -> transport error -> None.
        assert!(fetch_news(&FakeHttpTransport::new(), "f1", false, "en")
            .await
            .is_none());
        let url = news_request("f1", false, "en").url;
        let bad = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 503,
                final_url: url.clone(),
                body: Vec::new(),
            },
        );
        assert!(fetch_news(&bad, "f1", false, "en").await.is_none());
        let empty = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 200,
                final_url: url.clone(),
                body: b"<rss><channel></channel></rss>".to_vec(),
            },
        );
        assert!(fetch_news(&empty, "f1", false, "en").await.is_none());
    }

    #[tokio::test]
    async fn fetch_news_forwards_freshness_to_the_request() {
        // freshness=true must reach news_request: the recorded call's URL
        // carries the when:7d operator.
        let url = news_request("f1", true, "en").url;
        let transport = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 200,
                final_url: url.clone(),
                body: RSS_FIXTURE.as_bytes().to_vec(),
            },
        );
        let block = fetch_news(&transport, "f1", true, "en").await.unwrap();
        assert!(block.text.contains("Leclerc"));
        assert!(transport.calls().iter().any(|c| c.url == url));
    }
}
