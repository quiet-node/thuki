//! Bounded published-date extraction and freshness-gated recency-prior fusion.
//!
//! Relevance-only ranking systematically fails to surface the newest item on a
//! time-sensitive query even when a genuinely fresher source is in the
//! candidate set (see arXiv:2509.19376); a small recency prior fused with the
//! existing relevance score fixes this without hurting relevance on the rest.
//! This module owns the two pieces: extracting a page's published or modified
//! date from its raw HTML ([`extract_published_date`]), and fusing that date
//! with an already-ranked chunk set's relevance score
//! ([`recency_reorder`]).
//!
//! The fusion runs ONLY when the turn's standalone question already carries a
//! freshness signal (see [`crate::websearch::encyclopedia::is_volatile_question`],
//! reused by [`crate::websearch::orchestrator::run_engine_tier`] as the gate):
//! a non-fresh turn's ranking is completely untouched and pays no extra cost.
//! It reorders sources, never adds or removes one, and it never runs ahead of
//! the credibility filtering the engine tier already applied (dropped domains
//! never reach the fetch stage, so they are never in this pass's input either).
//!
//! Date extraction is defensive by construction: page HTML is
//! attacker-controlled, so parsing never panics, and a pathological page is
//! bounded by the SAME DoS cap ([`FETCH_MAX_ELEMENTS_TO_PARSE`]) already
//! applied to readability extraction (see
//! [`crate::websearch::fetch::extract_readable`]), reused here rather than
//! adding a second one. `scraper` (already a dependency, used by
//! [`crate::websearch::engine`]'s SERP parsing) is reused for the HTML query,
//! so no new HTML-parsing dependency is added either.

use std::collections::HashMap;

use scraper::{Html, Selector};
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::config::defaults::{
    FETCH_MAX_ELEMENTS_TO_PARSE, RECENCY_ALPHA, RECENCY_FUTURE_TOLERANCE_HOURS,
    RECENCY_HALF_LIFE_DAYS, RECENCY_NEUTRAL_SCORE,
};
use crate::websearch::fetch::FetchedPage;
use crate::websearch::rank::ScoredChunk;

/// CSS selector for a JSON-LD block, the highest-priority date source.
const JSON_LD_SELECTOR: &str = r#"script[type="application/ld+json"]"#;
/// CSS selector for the OpenGraph/Article "published" meta tag.
const META_PUBLISHED_SELECTOR: &str = r#"meta[property="article:published_time"]"#;
/// CSS selector for the OpenGraph "updated" meta tag, the fallback within the
/// meta tier when no published-time meta tag is present.
const META_UPDATED_SELECTOR: &str = r#"meta[property="og:updated_time"]"#;
/// CSS selector for the lowest-priority date source: the first `<time>`
/// element carrying a machine-readable `datetime` attribute.
const TIME_SELECTOR: &str = "time[datetime]";

/// Parses `html` into a queryable DOM, or `None` when its element count
/// exceeds [`FETCH_MAX_ELEMENTS_TO_PARSE`]. `scraper`'s underlying parser
/// (`html5ever`) is total: it never fails on malformed markup, it just yields
/// fewer/different elements, so the only rejection here is the DoS bound.
/// Mirrors the same defense-in-depth cap `dom_smoothie`'s readability
/// extraction already applies to this exact response body, so a pathological
/// page cannot burn CPU twice over.
fn parse_bounded(html: &str) -> Option<Html> {
    let doc = Html::parse_document(html);
    let all = Selector::parse("*").expect("static selector \"*\" always parses");
    if doc.select(&all).count() > FETCH_MAX_ELEMENTS_TO_PARSE {
        return None;
    }
    Some(doc)
}

/// Reads `datePublished`, falling back to `dateModified`, off one parsed
/// JSON-LD value that is itself a plain object (not an array or `@graph`
/// wrapper; see [`json_ld_date`] for those shapes). `None` when neither field
/// is a string. Never panics: `serde_json::Value` field access is total.
fn date_from_json_ld_object(value: &Value) -> Option<String> {
    value
        .get("datePublished")
        .and_then(Value::as_str)
        .or_else(|| value.get("dateModified").and_then(Value::as_str))
        .map(str::to_string)
}

/// Reads a JSON-LD date off one parsed value, handling the two real-world
/// shapes beyond a plain object: a top-level array of objects, or an
/// `@graph` array (schema.org's multi-entity wrapper). Only one level of
/// array unwrapping is attempted (no recursion into nested arrays/`@graph`),
/// which covers every shape observed in the wild while keeping the walk
/// depth-bounded regardless of how deeply an adversarial payload nests.
fn date_from_json_ld_value(value: &Value) -> Option<String> {
    if let Some(date) = date_from_json_ld_object(value) {
        return Some(date);
    }
    let array = value
        .as_array()
        .or_else(|| value.get("@graph").and_then(Value::as_array))?;
    array.iter().find_map(date_from_json_ld_object)
}

/// Scans every JSON-LD `<script>` block in document order and returns the
/// first date found (see [`date_from_json_ld_value`]). A block whose text is
/// not valid JSON is skipped rather than aborting the scan, since one bad
/// block on a page must not hide a good one later in the document.
fn json_ld_date(doc: &Html) -> Option<String> {
    let selector = Selector::parse(JSON_LD_SELECTOR).expect("static selector always parses");
    doc.select(&selector).find_map(|script| {
        let text: String = script.text().collect();
        serde_json::from_str::<Value>(text.trim())
            .ok()
            .and_then(|value| date_from_json_ld_value(&value))
    })
}

/// Reads the `content` attribute of the first element `selector` matches, or
/// `None` when nothing matches or the matched element carries no `content`
/// attribute.
fn meta_content(doc: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).expect("static selector always parses");
    doc.select(&selector)
        .next()?
        .value()
        .attr("content")
        .map(str::to_string)
}

/// Reads the published-time meta tag, falling back to the updated-time one
/// when the page carries no published-time tag.
fn meta_date(doc: &Html) -> Option<String> {
    meta_content(doc, META_PUBLISHED_SELECTOR).or_else(|| meta_content(doc, META_UPDATED_SELECTOR))
}

/// Reads the `datetime` attribute of the first `<time datetime="...">`
/// element in document order, the lowest-priority date source.
fn time_tag_date(doc: &Html) -> Option<String> {
    let selector = Selector::parse(TIME_SELECTOR).expect("static selector always parses");
    doc.select(&selector)
        .next()?
        .value()
        .attr("datetime")
        .map(str::to_string)
}

/// Extracts a page's published or modified date from its raw HTML, or `None`
/// when no recognised source carries a valid one. Tries, in priority order:
/// JSON-LD `datePublished`/`dateModified` (see [`json_ld_date`]), then the
/// `article:published_time`/`og:updated_time` meta tags (see [`meta_date`]),
/// then the first `<time datetime>` element (see [`time_tag_date`]). Each
/// candidate string is parsed and validated by [`parse_flexible_date`]; a
/// candidate that fails to parse (or parses to an implausible future date)
/// falls through exactly like a missing candidate, it does not short-circuit
/// the whole extraction.
///
/// `now` is the instant future-dated candidates are validated against (see
/// [`parse_flexible_date`]); callers pass the real current time, tests pass a
/// fixed one for determinism.
pub(crate) fn extract_published_date(html: &str, now: OffsetDateTime) -> Option<OffsetDateTime> {
    let doc = parse_bounded(html)?;
    [json_ld_date(&doc), meta_date(&doc), time_tag_date(&doc)]
        .into_iter()
        .find_map(|candidate| candidate.and_then(|raw| parse_flexible_date(&raw, now)))
}

/// Parses a bare `YYYY-MM-DD` date (no time-of-day component), assumed to
/// mean UTC midnight of that day. Rejects any extra `-`-separated segment,
/// any non-numeric field, and any calendar date the field values cannot
/// build (e.g. day 32). Never panics: every step is a checked parse or
/// constructor.
fn parse_date_only(s: &str) -> Option<OffsetDateTime> {
    let mut parts = s.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let month = time::Month::try_from(month).ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    Some(time::PrimitiveDateTime::new(date, time::Time::MIDNIGHT).assume_utc())
}

/// Parses `raw` as a date, strictly: RFC 3339 / ISO 8601 first (`time`'s
/// well-known RFC 3339 format, which requires a full date, time-of-day, and
/// offset), falling back to the bare `YYYY-MM-DD` date-only form (see
/// [`parse_date_only`]). Any other shape returns `None`; parsing never
/// panics on hostile input, `time::OffsetDateTime::parse` and every fallback
/// step are `Result`/`Option`-returning.
///
/// A result more than [`RECENCY_FUTURE_TOLERANCE_HOURS`] ahead of `now` is
/// also rejected (returns `None`): page metadata claiming to be from the
/// future is either a clock-skew artefact or a malformed/hostile value, and
/// treating it as undated is safer than trusting it as evidence of freshness.
pub(crate) fn parse_flexible_date(raw: &str, now: OffsetDateTime) -> Option<OffsetDateTime> {
    let trimmed = raw.trim();
    let parsed = OffsetDateTime::parse(trimmed, &Rfc3339)
        .ok()
        .or_else(|| parse_date_only(trimmed))?;
    let tolerance = time::Duration::hours(RECENCY_FUTURE_TOLERANCE_HOURS);
    (parsed <= now + tolerance).then_some(parsed)
}

/// Exponential recency decay for `published`: `exp(-ln(2) * age_days /
/// RECENCY_HALF_LIFE_DAYS)`, halving once per half-life, so a source twice as
/// old scores a quarter as fresh, four times as old an eighth, and so on,
/// asymptotically toward (never reaching) `0.0`.
///
/// `published = None` (no extractable date) scores exactly
/// [`RECENCY_NEUTRAL_SCORE`] (`0.5`), deliberately the SAME value a dated
/// source gets at exactly one half-life (`exp(-ln(2) * 1) = 0.5`): an undated
/// source reads as "moderately fresh" rather than being punished (`0.0`,
/// which would read as maximally stale) or rewarded (`1.0`, maximally fresh)
/// for a fetch-stage extraction gap the page's real age has nothing to do
/// with.
pub(crate) fn recency_score(published: Option<OffsetDateTime>, now: OffsetDateTime) -> f64 {
    let Some(published) = published else {
        return RECENCY_NEUTRAL_SCORE;
    };
    // Clamped at 0 so a `published` instant somehow after `now` (should not
    // happen: `parse_flexible_date` already rejects anything beyond the
    // clock-skew tolerance) reads as "as fresh as possible" rather than
    // pushing the exponent's argument negative and the score above 1.0.
    let age_days = ((now - published).as_seconds_f64() / 86_400.0).max(0.0);
    (-std::f64::consts::LN_2 * age_days / RECENCY_HALF_LIFE_DAYS).exp()
}

/// Re-orders `chunks`'s SOURCES (grouped by URL) by the recency-prior fusion
/// score: `RECENCY_ALPHA * recency + (1 - RECENCY_ALPHA) * relevance_norm`.
/// `relevance_norm` is each URL's strongest chunk score (see
/// [`ScoredChunk::score`]) normalised to `[0, 1]` against the strongest
/// source in this candidate set (`0.0` for every source in the degenerate
/// case where every score is `0.0` or the set is empty); `recency` comes
/// from [`recency_score`] against `pages`' extracted published dates.
///
/// Only ORDER changes. The exact set of chunks is unchanged (so the exact set
/// of sources [`crate::websearch::assemble::assemble_context`] later groups
/// them into is unchanged too), a URL's own chunks keep their existing
/// relative order, and URLs tied on final score keep their existing relative
/// order (a stable sort). This is why the pass cannot resurrect a
/// credibility-dropped domain or promote a source above the
/// credibility-boost contract: `chunks` only ever contains URLs that already
/// survived [`crate::websearch::engine::rrf_fuse_classified`] (which
/// hard-drops before fusion) and BM25 relevance thresholding (see
/// [`crate::websearch::rank::select_chunks`]); this pass reorders that
/// already-filtered set and nothing else.
///
/// Called only when the turn's freshness signal is set (see
/// `orchestrator::run_engine_tier`); a non-fresh turn never calls this and
/// pays no cost.
pub(crate) fn recency_reorder(
    chunks: &[ScoredChunk],
    pages: &[FetchedPage],
    now: OffsetDateTime,
) -> Vec<ScoredChunk> {
    if chunks.is_empty() {
        return Vec::new();
    }
    let published: HashMap<&str, Option<OffsetDateTime>> = pages
        .iter()
        .map(|page| (page.url.as_str(), page.published))
        .collect();

    // First-seen URL order (the same best-first grouping order
    // `assemble_context` would use on this exact `chunks` slice today) plus
    // each URL's best (max) chunk score, the source-level relevance signal.
    let mut order: Vec<&str> = Vec::new();
    let mut best_score: HashMap<&str, f64> = HashMap::new();
    for chunk in chunks {
        let url = chunk.url.as_str();
        match best_score.get_mut(url) {
            Some(existing) => {
                if chunk.score > *existing {
                    *existing = chunk.score;
                }
            }
            None => {
                order.push(url);
                best_score.insert(url, chunk.score);
            }
        }
    }

    let max_relevance = order
        .iter()
        .map(|url| best_score.get(*url).copied().unwrap_or(0.0))
        .fold(f64::NEG_INFINITY, f64::max);

    let mut final_score: HashMap<&str, f64> = HashMap::new();
    for url in &order {
        let raw_relevance = best_score.get(*url).copied().unwrap_or(0.0);
        let relevance_norm = if max_relevance > 0.0 {
            (raw_relevance / max_relevance).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let recency = recency_score(published.get(*url).copied().flatten(), now);
        final_score.insert(
            *url,
            RECENCY_ALPHA * recency + (1.0 - RECENCY_ALPHA) * relevance_norm,
        );
    }

    let mut ranked_urls = order;
    ranked_urls.sort_by(|a, b| {
        let sa = final_score.get(*a).copied().unwrap_or(0.0);
        let sb = final_score.get(*b).copied().unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut out = Vec::with_capacity(chunks.len());
    for url in ranked_urls {
        out.extend(chunks.iter().filter(|c| c.url == url).cloned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed instant so date-math tests are deterministic.
    fn now() -> OffsetDateTime {
        time::macros::datetime!(2026-07-10 12:00:00 UTC)
    }

    fn chunk(url: &str, score: f64) -> ScoredChunk {
        ScoredChunk {
            url: url.into(),
            title: "T".into(),
            text: "x".into(),
            score,
        }
    }

    fn page(url: &str, published: Option<OffsetDateTime>) -> FetchedPage {
        FetchedPage {
            url: url.into(),
            title: "T".into(),
            text: "x".into(),
            published,
        }
    }

    // ── parse_flexible_date ──────────────────────────────────────────────────

    #[test]
    fn parses_rfc3339_with_z() {
        let parsed = parse_flexible_date("2026-07-08T10:00:00Z", now()).unwrap();
        assert_eq!(parsed, time::macros::datetime!(2026-07-08 10:00:00 UTC));
    }

    #[test]
    fn parses_rfc3339_with_numeric_offset() {
        // 10:00+02:00 is 08:00 UTC.
        let parsed = parse_flexible_date("2026-07-08T10:00:00+02:00", now()).unwrap();
        assert_eq!(parsed, time::macros::datetime!(2026-07-08 08:00:00 UTC));
    }

    #[test]
    fn parses_bare_date_only_form_as_utc_midnight() {
        let parsed = parse_flexible_date("2026-07-08", now()).unwrap();
        assert_eq!(parsed, time::macros::datetime!(2026-07-08 00:00:00 UTC));
    }

    #[test]
    fn date_only_rejects_extra_segment() {
        assert!(parse_flexible_date("2026-07-08-05", now()).is_none());
    }

    #[test]
    fn date_only_rejects_out_of_range_calendar_date() {
        assert!(parse_flexible_date("2026-02-30", now()).is_none());
    }

    #[test]
    fn date_only_rejects_out_of_range_month() {
        assert!(parse_flexible_date("2026-13-01", now()).is_none());
    }

    #[test]
    fn date_only_rejects_missing_month_segment() {
        assert!(parse_flexible_date("2026", now()).is_none());
    }

    #[test]
    fn date_only_rejects_missing_day_segment() {
        assert!(parse_flexible_date("2026-07", now()).is_none());
    }

    #[test]
    fn date_only_rejects_non_numeric_month() {
        assert!(parse_flexible_date("2026-ab-01", now()).is_none());
    }

    #[test]
    fn date_only_rejects_non_numeric_day() {
        assert!(parse_flexible_date("2026-07-ab", now()).is_none());
    }

    #[test]
    fn malformed_and_hostile_strings_never_panic_and_are_undated() {
        for raw in [
            "",
            "not a date",
            "99999999999999999999-01-01",
            "'; DROP TABLE sources; --",
            "🎉🎉🎉",
            "2026-13-40T99:99:99Z",
            &"9".repeat(10_000),
        ] {
            assert!(parse_flexible_date(raw, now()).is_none());
        }
    }

    #[test]
    fn future_date_within_tolerance_is_accepted() {
        // now() is 12:00 UTC; 6 hours ahead is inside the 24h clock-skew guard.
        assert!(parse_flexible_date("2026-07-10T18:00:00Z", now()).is_some());
    }

    #[test]
    fn future_date_beyond_tolerance_is_undated() {
        // 48 hours ahead of now() is well past the 24h clock-skew guard.
        assert!(parse_flexible_date("2026-07-12T12:00:00Z", now()).is_none());
    }

    // ── extract_published_date: each source, priority order ─────────────────

    #[test]
    fn extracts_json_ld_date_published() {
        let html = r#"<html><head>
          <script type="application/ld+json">{"datePublished":"2026-07-08T00:00:00Z"}</script>
        </head><body></body></html>"#;
        assert_eq!(
            extract_published_date(html, now()),
            Some(time::macros::datetime!(2026-07-08 00:00:00 UTC))
        );
    }

    #[test]
    fn json_ld_falls_back_to_date_modified() {
        let html = r#"<html><head>
          <script type="application/ld+json">{"dateModified":"2026-07-08T00:00:00Z"}</script>
        </head></html>"#;
        assert_eq!(
            extract_published_date(html, now()),
            Some(time::macros::datetime!(2026-07-08 00:00:00 UTC))
        );
    }

    #[test]
    fn json_ld_reads_top_level_array_shape() {
        let html = r#"<html><head>
          <script type="application/ld+json">[{"headline":"x"},{"datePublished":"2026-07-08T00:00:00Z"}]</script>
        </head></html>"#;
        assert_eq!(
            extract_published_date(html, now()),
            Some(time::macros::datetime!(2026-07-08 00:00:00 UTC))
        );
    }

    #[test]
    fn json_ld_reads_at_graph_shape() {
        let html = r#"<html><head>
          <script type="application/ld+json">{"@graph":[{"headline":"x"},{"datePublished":"2026-07-08T00:00:00Z"}]}</script>
        </head></html>"#;
        assert_eq!(
            extract_published_date(html, now()),
            Some(time::macros::datetime!(2026-07-08 00:00:00 UTC))
        );
    }

    #[test]
    fn malformed_json_ld_block_is_skipped_not_fatal() {
        // Invalid JSON on the ld+json block must not abort the whole
        // extraction; the meta tag below is still reachable.
        let html = r#"<html><head>
          <script type="application/ld+json">{ not valid json </script>
          <meta property="article:published_time" content="2026-07-08T00:00:00Z">
        </head></html>"#;
        assert_eq!(
            extract_published_date(html, now()),
            Some(time::macros::datetime!(2026-07-08 00:00:00 UTC))
        );
    }

    #[test]
    fn extracts_meta_published_time_when_no_json_ld() {
        let html = r#"<html><head><meta property="article:published_time" content="2026-07-08"></head></html>"#;
        assert_eq!(
            extract_published_date(html, now()),
            Some(time::macros::datetime!(2026-07-08 00:00:00 UTC))
        );
    }

    #[test]
    fn meta_falls_back_to_og_updated_time() {
        let html =
            r#"<html><head><meta property="og:updated_time" content="2026-07-08"></head></html>"#;
        assert_eq!(
            extract_published_date(html, now()),
            Some(time::macros::datetime!(2026-07-08 00:00:00 UTC))
        );
    }

    #[test]
    fn extracts_first_time_tag_when_no_json_ld_or_meta() {
        let html = r#"<html><body><time datetime="2026-07-08">Jul 8</time></body></html>"#;
        assert_eq!(
            extract_published_date(html, now()),
            Some(time::macros::datetime!(2026-07-08 00:00:00 UTC))
        );
    }

    #[test]
    fn json_ld_outranks_meta_and_time_tag() {
        let html = r#"<html><head>
          <script type="application/ld+json">{"datePublished":"2026-07-01T00:00:00Z"}</script>
          <meta property="article:published_time" content="2026-07-05">
        </head><body><time datetime="2026-07-09">Jul 9</time></body></html>"#;
        assert_eq!(
            extract_published_date(html, now()),
            Some(time::macros::datetime!(2026-07-01 00:00:00 UTC))
        );
    }

    #[test]
    fn meta_outranks_time_tag_when_no_json_ld() {
        let html = r#"<html><head>
          <meta property="article:published_time" content="2026-07-05">
        </head><body><time datetime="2026-07-09">Jul 9</time></body></html>"#;
        assert_eq!(
            extract_published_date(html, now()),
            Some(time::macros::datetime!(2026-07-05 00:00:00 UTC))
        );
    }

    #[test]
    fn no_recognised_date_source_is_undated() {
        let html = "<html><head><title>No dates here</title></head><body><p>text</p></body></html>";
        assert!(extract_published_date(html, now()).is_none());
    }

    #[test]
    fn json_ld_object_with_no_date_field_falls_through_to_undated() {
        // Valid JSON, valid JSON-LD object, but no datePublished, no
        // dateModified, and no @graph array to look inside: the whole
        // extraction must still end up undated rather than panicking or
        // matching something it should not.
        let html = r#"<html><head>
          <script type="application/ld+json">{"headline":"just a headline"}</script>
        </head></html>"#;
        assert!(extract_published_date(html, now()).is_none());
    }

    #[test]
    fn element_cap_exceeded_yields_undated() {
        // A synthetic DOM far past FETCH_MAX_ELEMENTS_TO_PARSE, carrying a
        // real date, must still come back undated: the bound is enforced
        // before any selector runs.
        let mut html = String::from("<html><body>");
        for _ in 0..(FETCH_MAX_ELEMENTS_TO_PARSE + 500) {
            html.push_str("<div></div>");
        }
        html.push_str(r#"<time datetime="2026-07-08">Jul 8</time></body></html>"#);
        assert!(extract_published_date(&html, now()).is_none());
    }

    // ── recency_score ─────────────────────────────────────────────────────────

    #[test]
    fn undated_source_scores_neutral() {
        assert_eq!(recency_score(None, now()), RECENCY_NEUTRAL_SCORE);
    }

    #[test]
    fn published_now_scores_near_one() {
        assert!((recency_score(Some(now()), now()) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn published_one_half_life_ago_matches_the_neutral_score() {
        let published = now() - time::Duration::days(RECENCY_HALF_LIFE_DAYS as i64);
        let score = recency_score(Some(published), now());
        assert!((score - RECENCY_NEUTRAL_SCORE).abs() < 1e-9);
    }

    #[test]
    fn published_two_half_lives_ago_scores_a_quarter() {
        let published = now() - time::Duration::days(2 * RECENCY_HALF_LIFE_DAYS as i64);
        let score = recency_score(Some(published), now());
        assert!((score - 0.25).abs() < 1e-9);
    }

    #[test]
    fn published_after_now_is_clamped_not_above_one() {
        // Defensive: recency_score itself does not assume the future-date
        // guard already ran, so it must never produce a score above 1.0.
        let published = now() + time::Duration::hours(1);
        assert!((recency_score(Some(published), now()) - 1.0).abs() < 1e-9);
    }

    // ── recency_reorder ─────────────────────────────────────────────────────

    #[test]
    fn empty_input_is_empty() {
        assert!(recency_reorder(&[], &[], now()).is_empty());
    }

    #[test]
    fn newer_beats_older_at_equal_relevance() {
        let chunks = vec![chunk("https://old/", 5.0), chunk("https://new/", 5.0)];
        let pages = vec![
            page("https://old/", Some(now() - time::Duration::days(90))),
            page("https://new/", Some(now() - time::Duration::days(1))),
        ];
        let out = recency_reorder(&chunks, &pages, now());
        assert_eq!(out[0].url, "https://new/");
        assert_eq!(out[1].url, "https://old/");
    }

    #[test]
    fn strong_relevance_still_beats_marginally_newer_junk() {
        // "junk" is barely relevant (score just above zero) but very fresh;
        // "strong" is far more relevant but old. At RECENCY_ALPHA = 0.3 the
        // relevance gap must still decide the order.
        let chunks = vec![chunk("https://strong/", 100.0), chunk("https://junk/", 1.0)];
        let pages = vec![
            page("https://strong/", Some(now() - time::Duration::days(60))),
            page("https://junk/", Some(now())),
        ];
        let out = recency_reorder(&chunks, &pages, now());
        assert_eq!(out[0].url, "https://strong/");
        assert_eq!(out[1].url, "https://junk/");
    }

    #[test]
    fn undated_source_is_never_dropped_and_competes_on_relevance() {
        let chunks = vec![chunk("https://dated/", 3.0), chunk("https://undated/", 3.0)];
        let pages = vec![page(
            "https://dated/",
            Some(now() - time::Duration::days(1)),
        )];
        // "https://undated/" has no matching page entry at all: recency_reorder
        // must still carry it through rather than treat the miss as a drop.
        let out = recency_reorder(&chunks, &pages, now());
        let urls: Vec<&str> = out.iter().map(|c| c.url.as_str()).collect();
        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"https://dated/"));
        assert!(urls.contains(&"https://undated/"));
    }

    #[test]
    fn future_dated_source_behaves_as_undated_never_dropped() {
        // A page carrying a date rejected by parse_flexible_date's future
        // guard is represented as `published: None` by the time it reaches
        // recency_reorder (extraction already filtered it): still present,
        // still competing on relevance, never dropped.
        let chunks = vec![chunk("https://a/", 4.0), chunk("https://b/", 4.0)];
        let pages = vec![page("https://a/", None), page("https://b/", None)];
        let out = recency_reorder(&chunks, &pages, now());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn never_introduces_a_url_not_in_the_input_set() {
        // Stands in for "never resurrects a credibility-dropped domain": the
        // function can only ever reorder the URLs it was given.
        let chunks = vec![
            chunk("https://a/", 2.0),
            chunk("https://b/", 5.0),
            chunk("https://c/", 1.0),
        ];
        let pages = vec![
            page("https://a/", Some(now())),
            page("https://b/", Some(now() - time::Duration::days(30))),
            page("https://c/", None),
        ];
        let mut input_urls: Vec<&str> = chunks.iter().map(|c| c.url.as_str()).collect();
        input_urls.sort_unstable();
        let out = recency_reorder(&chunks, &pages, now());
        let mut output_urls: Vec<&str> = out.iter().map(|c| c.url.as_str()).collect();
        output_urls.sort_unstable();
        assert_eq!(input_urls, output_urls);
    }

    #[test]
    fn ties_preserve_original_relative_order() {
        // Identical score, identical (absent) date: final_score ties exactly,
        // so the stable sort must keep the original a-before-b order.
        let chunks = vec![chunk("https://a/", 2.0), chunk("https://b/", 2.0)];
        let out = recency_reorder(&chunks, &[], now());
        assert_eq!(out[0].url, "https://a/");
        assert_eq!(out[1].url, "https://b/");
    }

    #[test]
    fn a_urls_own_chunks_keep_their_relative_order() {
        let chunks = vec![
            chunk("https://a/", 9.0),
            chunk("https://a/", 3.0),
            chunk("https://b/", 1.0),
        ];
        let pages = vec![
            page("https://a/", Some(now() - time::Duration::days(90))),
            page("https://b/", Some(now())),
        ];
        let out = recency_reorder(&chunks, &pages, now());
        // "b" (fresh, low relevance) still loses to "a" (old, high relevance)
        // at RECENCY_ALPHA = 0.3, and "a"'s own two chunks keep 9.0 before 3.0.
        assert_eq!(out[0].url, "https://a/");
        assert_eq!(out[0].score, 9.0);
        assert_eq!(out[1].url, "https://a/");
        assert_eq!(out[1].score, 3.0);
        assert_eq!(out[2].url, "https://b/");
    }

    #[test]
    fn tracks_max_score_across_multiple_chunks_for_the_same_url() {
        // "a"'s chunks arrive lowest-score-first (2.0, then 8.0); its true
        // relevance is the MAX across its own chunks (8.0), not the first one
        // seen. Both sources are undated (equal neutral recency), so relevance
        // alone decides order: a bug that tracked only the first-seen score
        // would rank "b" (5.0) ahead of "a" (2.0) instead of behind it (8.0).
        let chunks = vec![
            chunk("https://a/", 2.0),
            chunk("https://b/", 5.0),
            chunk("https://a/", 8.0),
        ];
        let out = recency_reorder(&chunks, &[], now());
        assert_eq!(out[0].url, "https://a/");
    }

    #[test]
    fn zero_score_candidate_set_falls_back_to_pure_recency() {
        // Degenerate input select_chunks would never itself produce (it drops
        // zero-score chunks), constructed directly to exercise the
        // max_relevance <= 0.0 guard: relevance_norm is 0.0 for every URL, so
        // order is decided purely by recency.
        let chunks = vec![chunk("https://old/", 0.0), chunk("https://new/", 0.0)];
        let pages = vec![
            page("https://old/", Some(now() - time::Duration::days(90))),
            page("https://new/", Some(now() - time::Duration::days(1))),
        ];
        let out = recency_reorder(&chunks, &pages, now());
        assert_eq!(out[0].url, "https://new/");
        assert_eq!(out[1].url, "https://old/");
    }
}
