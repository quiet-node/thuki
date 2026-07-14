//! Built-in zero-setup web search.
//!
//! Invisible, model-decided search that runs entirely on the device: a
//! grammar-constrained pre-pass decides per message whether the web is needed,
//! keyless sources are fetched through the SSRF-safe [`crate::net`] transport,
//! and a single writer call answers with numbered citations.
//!
//! The stages are built as independent, injectable units so the orchestrator's
//! decision logic is unit-testable without a live model or network:
//! - [`prefilter`] — the deterministic stage-one search verdict (no model call).
//! - [`prepass`] — the persona-free classifier for the ambiguous middle: the
//!   `no｜cached｜web` decision and query rewrite.
//! - [`clock`] — deterministic place-time resolution for a clock question
//!   naming a place, injected into the system prompt so the model never
//!   does its own timezone arithmetic. Not a search decision: it never
//!   triggers web search.
//! - [`cache`] — the multi-turn source cache backing a `cached` decision.
//! - [`serp_cache`] — process-lifetime, in-memory (never disk) TTL+FIFO cache of
//!   engine SERP lists and extracted page bodies, so a repeat scrape is served
//!   from memory instead of re-hitting a keyless engine (cuts latency and the
//!   engines' volume-triggered rate limits).
//! - [`engine`] — keyless search-engine scraping with rotation.
//! - [`credibility`] — static, compiled-in domain-credibility list (drop /
//!   penalize / boost) consulted by the engine tier's rank fusion.
//! - [`weather`], [`sports`], [`news`], [`encyclopedia`] — intent-routed
//!   keyless verticals tried ahead of the scraped engines.
//! - [`judge`]: the sufficiency check run after a vertical answers. An
//!   insufficient vertical block escalates to the scraped engines instead of
//!   dead-ending on a "the sources do not contain that" refusal.
//! - [`fetch`] — concurrent page fetch + readability extraction.
//! - [`rank`] — chunking + BM25 extractive filter behind a `Scorer` seam.
//! - [`recency`] — bounded published-date extraction and the recency-prior
//!   fusion applied to the engine tier's ranked sources on a freshness-flagged
//!   turn only.
//! - [`assemble`] — group ranked chunks into budgeted numbered source blocks.
//! - [`writer`] — writer prompt assembly with prompt-injection defenses.
//! - [`orchestrator`] — the fixed pipeline tying the stages together.

pub mod assemble;
pub mod cache;
pub mod cite_check;
pub mod clock;
pub mod credibility;
pub mod encyclopedia;
pub mod engine;
pub mod fetch;
pub mod judge;
pub mod news;
pub mod orchestrator;
pub mod prefilter;
pub mod prepass;
pub mod rank;
pub mod recency;
pub mod serp_cache;
pub mod sports;
pub mod weather;
pub mod writer;

/// Honest product User-Agent for keyless API verticals (weather, news, sports).
/// Identifies Thuki with version + homepage so operators can contact us. Not used
/// on SERP scrapers: DuckDuckGo/Mojeek block non-browser UAs on `/html` instantly
/// (browser UA stays deliberate there; see `engine::BROWSER_USER_AGENT`).
pub(crate) const THUKI_USER_AGENT: &str =
    concat!("Thuki/", env!("CARGO_PKG_VERSION"), " (+https://thuki.app)");

/// Attribution required by Open-Meteo's CC BY 4.0 licence. Markdown link so the
/// "Weather data by Open-Meteo.com" hyperlink is present. Single source of
/// truth: the weather vertical embeds it in the writer context and the UI
/// attribution projection ([`crate::commands::source_attribution_for_url`])
/// renders the same link for open-meteo source URLs in the Sources footer.
pub(crate) const OPEN_METEO_ATTRIBUTION: &str =
    "[Weather data by Open-Meteo.com](https://open-meteo.com/) (CC BY 4.0)";

/// Attribution required by Wikipedia's CC BY-SA 4.0 licence, with a hyperlink to
/// the licence text. Single source of truth shared by the encyclopedia
/// vertical's writer context and the UI attribution projection.
pub(crate) const WIKIPEDIA_ATTRIBUTION: &str =
    "Source: Wikipedia ([CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/))";

/// The registration host of a URL in ASCII / Punycode form (`xn--…`), or an
/// empty string when it does not parse. The `url` crate IDNA-encodes domain
/// hosts on parse, so internationalized labels never surface as lookalike
/// Unicode in citation trust labels, credibility matching, or per-domain caps.
pub(crate) fn domain_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// Normalizes a URL to a dedup *key* so obviously-equivalent variants (a
/// trailing slash, or `http` vs `https`) collapse onto the same key without
/// altering any URL actually stored, displayed, or fetched. Two independent
/// engines scraping the same page frequently disagree on exactly these two
/// details (one wraps a redirect, the other serves a bare host-root link),
/// and every exact-string dedup step in the fusion pipeline
/// ([`engine::rrf_fuse`]'s intra-list and cross-list keys,
/// [`engine::dedupe_and_cap`]'s final pass, and [`orchestrator::dedupe_hits`]'s
/// cross-query pass) used the raw URL string as its key, so a same-page
/// variant slipped through every one of them and only got caught by the
/// per-domain cap (which allows more than one, by design, for genuinely
/// distinct pages).
///
/// Deliberately not a full canonicalizer: query strings, `www.` prefixes, and
/// path casing are left untouched, since collapsing those can change page
/// identity and a wrong collapse would silently drop a distinct source. Only
/// the two variant classes named above are folded, matching what a fusion
/// step can safely assume is "the same page" without inspecting content.
pub(crate) fn canonical_url_key(url: &str) -> String {
    let no_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    no_scheme.strip_suffix('/').unwrap_or(no_scheme).to_string()
}

#[cfg(test)]
mod tests {
    use super::canonical_url_key;

    #[test]
    fn domain_of_extracts_host_or_empty() {
        assert_eq!(
            super::domain_of("https://sub.example.com/path"),
            "sub.example.com"
        );
        assert_eq!(super::domain_of("not a url"), "");
    }

    #[test]
    fn domain_of_returns_punycode_for_idn_hosts() {
        // Unicode IDN labels must surface as ASCII xn-- form so a homograph
        // cannot look like a trusted latin domain in citation chrome.
        assert_eq!(
            super::domain_of("https://münchen.example/path"),
            "xn--mnchen-3ya.example"
        );
        assert_eq!(
            super::domain_of("https://xn--mnchen-3ya.example/"),
            "xn--mnchen-3ya.example"
        );
        // Cyrillic lookalike host → punycode, never latin "apple.com".
        let host = super::domain_of("https://аррle.com/x");
        assert!(host.starts_with("xn--"), "got {host}");
        assert_ne!(host, "apple.com");
    }

    #[test]
    fn thuki_user_agent_identifies_product_with_version_and_contact() {
        assert!(super::THUKI_USER_AGENT.starts_with("Thuki/"));
        assert!(super::THUKI_USER_AGENT.contains("(+https://thuki.app)"));
        assert!(super::THUKI_USER_AGENT.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn canonical_url_key_folds_scheme_and_trailing_slash() {
        // https vs http, and a trailing slash vs none: all four collapse to
        // the same key.
        let keys = [
            "https://www.binance.com/en/price/bitcoin",
            "https://www.binance.com/en/price/bitcoin/",
            "http://www.binance.com/en/price/bitcoin",
            "http://www.binance.com/en/price/bitcoin/",
        ]
        .map(canonical_url_key);
        assert!(keys.windows(2).all(|w| w[0] == w[1]));
    }

    #[test]
    fn canonical_url_key_keeps_distinct_paths_distinct() {
        assert_ne!(
            canonical_url_key("https://example.com/a"),
            canonical_url_key("https://example.com/b")
        );
    }

    #[test]
    fn canonical_url_key_root_path_matches_bare_host() {
        assert_eq!(
            canonical_url_key("https://example.com/"),
            canonical_url_key("https://example.com")
        );
    }
}
