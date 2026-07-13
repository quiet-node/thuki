//! Sports vertical: ESPN's public scoreboard API.
//!
//! **UNOFFICIAL.** `site.api.espn.com` is ESPN's own frontend backend, not a
//! published/keyed API: there is no SLA, no versioning guarantee, and ESPN can
//! change or remove the endpoint's shape (or the endpoint itself) at any time
//! with no notice. Every failure path in this module — transport error,
//! non-200 status, unparseable body, an empty events array, or a shape the
//! parser does not recognise — degrades to `None`, never an error surfaced to
//! the user: a turn that hits this vertical simply falls through to the news
//! and engine tiers as if the vertical did not exist.
//!
//! Live scores and standings answer poorly from general SERP scraping (the
//! score lives in a widget, not article text) and poorly from the news feed
//! (headlines lag the live state). The scoreboard endpoint is keyless JSON, one
//! request per league, and returns the exact structured data ("who's playing,
//! what's the score, is it over") that both of the other tiers approximate.
//!
//! League detection ([`detect_league`]) is a deterministic keyword map (see
//! [`crate::config::defaults::SPORTS_LEAGUE_MAP`]) rather than a model call: a
//! wrong or missing keyword match just means the vertical does not run for
//! this turn, so a cheap, predictable miss is preferable to spending a model
//! call to decide it.

use crate::config::defaults::{SPORTS_LEAGUE_MAP, SPORTS_SCHEDULE_WINDOW_DAYS};
use crate::net::transport::{HttpMethod, HttpRequest, HttpTransport};
use crate::websearch::assemble::SourceBlock;

/// ESPN's public (unofficial, keyless) scoreboard API base. Path segments are
/// `{sport}/{league}` per [`SPORTS_LEAGUE_MAP`], e.g.
/// `soccer/fifa.world/scoreboard`.
const ESPN_SCOREBOARD_BASE: &str = "https://site.api.espn.com/apis/site/v2/sports";

/// ESPN's homepage, cited as the source URL: the scoreboard API itself has no
/// public-facing page to link to.
const ESPN_PAGE_URL: &str = "https://www.espn.com/";

/// Maximum events listed in one sports source block: enough to cover a full
/// matchday/slate without flooding the writer's source budget.
const MAX_SPORTS_EVENTS: usize = 8;

/// Resolves the lowercased `question` to a `(sport, league)` ESPN path pair via
/// [`SPORTS_LEAGUE_MAP`], or `None` when no keyword matches. A multi-word
/// keyword (containing a space) is matched as a whole phrase; a single-word
/// keyword is matched as a whole token, so "nba" does not fire on an unrelated
/// word that merely contains those letters.
pub(crate) fn detect_league(question: &str) -> Option<(&'static str, &'static str)> {
    let lower = question.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let padded = format!(" {} ", tokens.join(" "));
    for entry in SPORTS_LEAGUE_MAP {
        let (keyword, sport, league) = *entry;
        let hit = if keyword.contains(' ') {
            padded.contains(&format!(" {keyword} "))
        } else {
            tokens.contains(&keyword)
        };
        if hit {
            return Some((sport, league));
        }
    }
    None
}

/// Whether `question` names a league or competition the sports vertical
/// recognises. Thin boolean wrapper over [`detect_league`], mirroring
/// [`super::news::is_news_intent`]'s shape for the orchestrator's gate.
pub(crate) fn is_sports_intent(question: &str) -> bool {
    detect_league(question).is_some()
}

/// Builds the `dates=<start>-<end>` query value (both `YYYYMMDD`) spanning
/// [`SPORTS_SCHEDULE_WINDOW_DAYS`] forward from `today`, an ISO `YYYY-MM-DD`
/// date string, or `None` when `today` does not parse. ESPN's scoreboard
/// defaults to only the current day's slate, so without this window the block
/// cannot answer "when is the next match" once today's fixtures are all live or
/// finished. `today` is program-generated (see the pipeline's `today`
/// parameter), so an unparseable value is a defensive fallback, never expected:
/// the caller then requests the dateless (today-only) URL rather than panicking.
fn scoreboard_dates_param(today: &str) -> Option<String> {
    let mut ymd = today.split('-');
    let year: i32 = ymd.next()?.parse().ok()?;
    let month: u8 = ymd.next()?.parse().ok()?;
    let day: u8 = ymd.next()?.parse().ok()?;
    if ymd.next().is_some() {
        return None;
    }
    let month = time::Month::try_from(month).ok()?;
    let start = time::Date::from_calendar_date(year, month, day).ok()?;
    let end = start.checked_add(time::Duration::days(SPORTS_SCHEDULE_WINDOW_DAYS))?;
    Some(format!(
        "{:04}{:02}{:02}-{:04}{:02}{:02}",
        start.year(),
        u8::from(start.month()),
        start.day(),
        end.year(),
        u8::from(end.month()),
        end.day(),
    ))
}

/// Builds the scoreboard GET request for a resolved `(sport, league)` pair,
/// scoped to a [`SPORTS_SCHEDULE_WINDOW_DAYS`]-day window forward from `today`
/// (see [`scoreboard_dates_param`]) so the response spans the next fixtures, not
/// just today's slate. When `today` does not parse the URL falls back to the
/// dateless form (today only), ESPN's default.
pub(crate) fn scoreboard_request(sport: &str, league: &str, today: &str) -> HttpRequest {
    let base = format!("{ESPN_SCOREBOARD_BASE}/{sport}/{league}/scoreboard");
    let url = match scoreboard_dates_param(today) {
        Some(dates) => format!("{base}?dates={dates}"),
        None => base,
    };
    HttpRequest {
        method: HttpMethod::Get,
        url,
        headers: Vec::new(),
        form: Vec::new(),
    }
}

/// Reads a competitor's display name, or `None` when the shape is missing the
/// field the block needs.
fn competitor_name(competitor: &serde_json::Value) -> Option<String> {
    competitor
        .get("team")?
        .get("displayName")?
        .as_str()
        .map(str::to_string)
}

/// Reads a competitor's score as display text. ESPN normally sends this as a
/// JSON string, but a bare number is accepted too; any other shape (or a
/// missing field, common for a not-yet-started fixture on some leagues)
/// degrades to `"-"` rather than dropping the competitor.
fn competitor_score(competitor: &serde_json::Value) -> String {
    competitor
        .get("score")
        .and_then(|s| {
            s.as_str()
                .map(str::to_string)
                .or_else(|| s.as_i64().map(|n| n.to_string()))
                .or_else(|| s.as_f64().map(|n| n.to_string()))
        })
        .unwrap_or_else(|| "-".to_string())
}

/// Renders an event's status into a short human label. `state` is ESPN's own
/// `pre` (scheduled) / `in` (live) / `post` (final) tri-state; `short_detail`
/// is ESPN's own rendered detail text (a date/time for `pre`, a clock/period
/// for `in`, `"Final"`/`"FT"` for `post`). An unrecognised or missing state
/// falls back to whatever detail text is available, or a generic label.
fn status_label(state: &str, short_detail: &str) -> String {
    match state {
        "pre" if !short_detail.is_empty() => format!("scheduled, {short_detail}"),
        "pre" => "scheduled".to_string(),
        "in" if !short_detail.is_empty() => format!("in progress, {short_detail}"),
        "in" => "in progress".to_string(),
        "post" => "final".to_string(),
        _ if !short_detail.is_empty() => short_detail.to_string(),
        _ => "status unknown".to_string(),
    }
}

/// Extracts the calendar-date portion of an ESPN event's ISO 8601 `date`
/// field (e.g. `"2026-07-09T20:00Z"` -> `"2026-07-09"`), or `None` when the
/// field is missing or not a string. No date-parsing dependency needed: the
/// field is always ISO 8601 with a literal `T` separator, so a byte split
/// suffices; a value with no `T` (a shape drift) degrades to the raw string
/// rather than dropping the date entirely.
///
/// This is the direct fix for the bug this vertical exists to avoid: a
/// `post` (final) event's own status text is just `"Final"`/`"FT"`, with no
/// date at all, so a small model reading an undated "France 2-0 Morocco"
/// score line pattern-matches it onto a superficially similar past
/// tournament instead of trusting it as the live scoreboard it is.
fn event_date(event: &serde_json::Value) -> Option<String> {
    let raw = event.get("date")?.as_str()?;
    let date = raw.split_once('T').map(|(d, _)| d).unwrap_or(raw);
    Some(date.to_string())
}

/// Parses an ESPN ISO-8601 UTC `date` string (`"YYYY-MM-DDTHH:MM[:SS]Z"`) into
/// a UTC [`time::OffsetDateTime`], or `None` on any shape it does not
/// recognise. ESPN emits minute precision with a literal `Z`, which the
/// well-known RFC 3339 parser rejects (it requires seconds), so the fields are
/// split by hand. A value carrying an explicit numeric offset instead of `Z`
/// is treated as unrecognised (returns `None`) rather than risk mis-zoning it:
/// the scoreboard always emits `Z`. Never panics.
fn parse_event_utc(raw: &str) -> Option<time::OffsetDateTime> {
    let (date_part, time_part) = raw.split_once('T')?;
    let mut ymd = date_part.split('-');
    let year: i32 = ymd.next()?.parse().ok()?;
    let month: u8 = ymd.next()?.parse().ok()?;
    let day: u8 = ymd.next()?.parse().ok()?;
    if ymd.next().is_some() {
        return None;
    }
    let time_str = time_part.strip_suffix('Z')?;
    let mut hms = time_str.split(':');
    let hour: u8 = hms.next()?.parse().ok()?;
    let minute: u8 = hms.next()?.parse().ok()?;
    let second: u8 = match hms.next() {
        Some(s) => s.parse().ok()?,
        None => 0,
    };
    if hms.next().is_some() {
        return None;
    }
    let month = time::Month::try_from(month).ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    let clock = time::Time::from_hms(hour, minute, second).ok()?;
    Some(time::PrimitiveDateTime::new(date, clock).assume_utc())
}

/// Formats a scheduled event's kickoff instant in the user's local zone as
/// `"YYYY-MM-DD at HH:MM (Zone)"`, e.g. `"2026-09-09 at 20:20
/// (America/New_York)"`, or `None` when there is no local zone, the `date`
/// field does not parse (see [`parse_event_utc`]), or the zone does not resolve
/// (see [`crate::websearch::clock::resolve_offset`]). Both the date AND the
/// time are localized, so a fixture whose UTC calendar date differs from the
/// user's (a late-evening local kickoff) is dated correctly for them. The
/// offset is looked up for the event's own instant, so it is DST-correct even
/// for a fixture on the far side of a daylight-saving transition. On any miss
/// the caller keeps the UTC-date-only line, this vertical's prior behaviour.
fn event_local_kickoff(event: &serde_json::Value, local_zone: Option<&str>) -> Option<String> {
    let zone = local_zone?;
    let raw = event.get("date")?.as_str()?;
    let utc = parse_event_utc(raw)?;
    let offset = crate::websearch::clock::resolve_offset(zone, utc)?;
    let local = utc.to_offset(offset);
    Some(format!(
        "{:04}-{:02}-{:02} at {:02}:{:02} ({zone})",
        local.year(),
        u8::from(local.month()),
        local.day(),
        local.hour(),
        local.minute()
    ))
}

/// Reads the scoreboard's season year from `leagues[0].season.year`, when
/// ESPN's response includes it, so the block's league label can disambiguate
/// "the 2026 World Cup" from any other year's tournament of the same name.
/// `None` on any missing or wrong-typed link in the chain (a shape drift
/// degrades to no season label, never a panic).
fn season_year(json: &serde_json::Value) -> Option<i64> {
    json.get("leagues")?
        .get(0)?
        .get("season")?
        .get("year")?
        .as_i64()
}

/// Reads an event's ESPN status `state` (`pre` scheduled / `in` live / `post`
/// final), or `""` when the status shape is missing the field. Shared by
/// [`format_event`] (to decide whether to localize a kickoff) and
/// [`parse_scoreboard`] (to compute the in-progress / next-match summaries).
fn event_state(event: &serde_json::Value) -> &str {
    event
        .get("status")
        .and_then(|s| s.get("type"))
        .and_then(|t| t.get("state"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
}

/// Formats one scoreboard event into a single display line, or `None` when the
/// event is missing its name or fewer than two competitors resolve a name (a
/// malformed or unrecognisable row is skipped rather than shown half-blank).
/// The line always carries the event's date when ESPN provides one, whatever
/// its status: a `post` (final) event's own status text has no date at all.
/// `round` is the competition's round/stage label (e.g. "Quarterfinals"),
/// appended to the status parens when present so the writer can tell a
/// quarterfinal from a final and never over-states which round a score decided.
/// `local_zone` is the user's device IANA timezone (when known): a scheduled
/// (`pre`) event's line then carries its kickoff time converted to that zone
/// (see [`event_local_kickoff`]), the "at what time" detail a user drills into.
fn format_event(
    event: &serde_json::Value,
    round: Option<&str>,
    local_zone: Option<&str>,
) -> Option<String> {
    let name = event.get("name")?.as_str()?.to_string();
    let competitors = event
        .get("competitions")?
        .get(0)?
        .get("competitors")?
        .as_array()?;
    let lines: Vec<String> = competitors
        .iter()
        .filter_map(|c| competitor_name(c).map(|team| format!("{team} {}", competitor_score(c))))
        .collect();
    if lines.len() < 2 {
        return None;
    }
    let state = event_state(event);
    let short_detail = event
        .get("status")
        .and_then(|s| s.get("type"))
        .and_then(|t| t.get("shortDetail"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let status = status_label(state, short_detail);
    let detail = match round {
        Some(round) if !round.trim().is_empty() => format!("{status}, {}", round.trim()),
        _ => status,
    };
    let scoreline = lines.join(" vs ");
    // Only scheduled (`pre`) events get a localized kickoff datetime: it is the
    // "at what time" detail a user drills into. A live/final event carries a
    // clock/"Final" status instead, where a start time would mislead. On a miss
    // (non-`pre` state, no zone, or an unparseable date) the line falls back to
    // ESPN's own UTC calendar date, this vertical's prior behaviour.
    let localized_kickoff = if state == "pre" {
        event_local_kickoff(event, local_zone)
    } else {
        None
    };
    let date_suffix = match localized_kickoff.or_else(|| event_date(event)) {
        Some(when) => format!(" on {when}"),
        None => String::new(),
    };
    Some(format!("{name}: {scoreline} ({detail}){date_suffix}"))
}

/// Parses a scoreboard response body into the league's display label (name,
/// plus the season year when present) and the block's formatted lines. Returns
/// `None` only when the body is not JSON or carries no `events` field at all; an
/// `events` array that parses to zero usable lines (e.g. an off-season slate)
/// returns `Some((label, vec![]))`, which the caller treats as a miss.
///
/// The returned line list leads with up to two kinds of computed summary line,
/// then the per-event listing:
/// - `"Currently in progress: <event line>"` for each event whose state is
///   `in` (omitted entirely when none are live).
/// - `"Next scheduled match: <event line>"` for the single earliest-dated event
///   whose state is `pre` (omitted when none are scheduled or none carry a
///   parseable date).
///
/// The summaries are computed from ALL parsed events, before the listing is
/// capped at [`MAX_SPORTS_EVENTS`], so a next fixture beyond the cap is still
/// surfaced. They deliberately duplicate a line that also appears in the
/// listing below: the summary is the code-computed answer ("which match is
/// next" is deterministic from ESPN's own event states, no wall clock needed),
/// the listing is the evidence.
///
/// The round/stage label is read once from the response-level
/// `leagues[0].season.type.name` ("Quarterfinals", "Regular Season", ...) and
/// applied to every event line: one scoreboard response is one league's current
/// slate, so the slate shares a stage. It is threaded to each [`format_event`]
/// so the writer can name the round without inferring it from the score.
/// `local_zone` is threaded to [`format_event`] to localize scheduled kickoff
/// times.
pub(crate) fn parse_scoreboard(
    body: &str,
    local_zone: Option<&str>,
) -> Option<(String, Vec<String>)> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let league_name = json
        .get("leagues")
        .and_then(|l| l.get(0))
        .and_then(|l| l.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("the league")
        .to_string();
    let league_label = match season_year(&json) {
        Some(year) => format!("{league_name} {year}"),
        None => league_name,
    };
    let round = json
        .get("leagues")
        .and_then(|l| l.get(0))
        .and_then(|l| l.get("season"))
        .and_then(|s| s.get("type"))
        .and_then(|t| t.get("name"))
        .and_then(|n| n.as_str());
    let events = json.get("events")?.as_array()?;
    // Format every usable event once, keeping its state and (for `pre` events)
    // its parsed kickoff instant so the summaries can be computed from the full
    // slate before the listing cap is applied.
    let parsed: Vec<(&str, Option<time::OffsetDateTime>, String)> = events
        .iter()
        .filter_map(|event| {
            let line = format_event(event, round, local_zone)?;
            let state = event_state(event);
            let kickoff = event
                .get("date")
                .and_then(|d| d.as_str())
                .and_then(parse_event_utc);
            Some((state, kickoff, line))
        })
        .collect();
    let mut lines: Vec<String> = Vec::new();
    // Every live match, each as its own summary line.
    for (state, _, line) in &parsed {
        if *state == "in" {
            lines.push(format!("Currently in progress: {line}"));
        }
    }
    // The single earliest-dated scheduled match (min by kickoff instant, not
    // array position). Scheduled events whose `date` did not parse are excluded
    // from the selection rather than treated as the earliest.
    if let Some((_, line)) = parsed
        .iter()
        .filter(|(state, _, _)| *state == "pre")
        .filter_map(|(_, kickoff, line)| kickoff.map(|k| (k, line)))
        .min_by_key(|(k, _)| *k)
    {
        lines.push(format!("Next scheduled match: {line}"));
    }
    // The per-event listing (the evidence under the summaries), capped.
    lines.extend(
        parsed
            .iter()
            .take(MAX_SPORTS_EVENTS)
            .map(|(_, _, line)| line.clone()),
    );
    Some((league_label, lines))
}

/// Wraps formatted event lines into the single `[1]` source block the writer
/// cites, naming the league (with its season year, when known) in the title
/// and citing ESPN's homepage (the scoreboard API has no public page of its
/// own). The block is prefixed with an explicit as-of line: the writer's own
/// training data cannot date this response, so it must be told in-band that
/// this scoreboard reflects `today`, not whatever year a similarly-scored
/// past match happens to be memorised as.
pub(crate) fn sports_source_block(
    league_label: &str,
    lines: &[String],
    today: &str,
) -> SourceBlock {
    let mut text = format!(
        "{league_label}, as of {today}.\nLive scores and schedule for {league_label} (via ESPN):"
    );
    for line in lines {
        text.push_str("\n- ");
        text.push_str(line);
    }
    SourceBlock {
        index: 1,
        url: ESPN_PAGE_URL.to_string(),
        title: format!("ESPN scores: {league_label}"),
        text,
    }
}

/// Runs the full sports vertical for `standalone_question`: league detection,
/// scoreboard fetch, parse. Returns `None` on any miss (no league match,
/// transport error, non-200, unparseable body, or zero usable events) so the
/// caller falls through to the news / engine tiers. `today` becomes the
/// source block's as-of line (see [`sports_source_block`]); `local_zone` is the
/// user's device IANA timezone, threaded through to localize scheduled kickoff
/// times (see [`format_event`]).
///
/// Coverage-excluded: thin async glue over the injectable transport
/// delegating every decision to the pure helpers above, which are all tested
/// directly; the glue itself is still exercised against
/// [`crate::net::transport::FakeHttpTransport`] in the tests below.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) async fn fetch_sports(
    transport: &dyn HttpTransport,
    standalone_question: &str,
    today: &str,
    local_zone: Option<&str>,
) -> Option<SourceBlock> {
    // The classifier may route a turn to sports whose competition the league map
    // does not recognise (e.g. a named Grand Prix with no "f1"/"formula 1"
    // token, or a niche league). Log the miss explicitly rather than returning a
    // silent `None`: a route=sports turn that vanished here with no reason line
    // is exactly the forensics gap the trace pass hit. The caller falls through
    // to the news / engine tiers.
    let Some((sport, league)) = detect_league(standalone_question) else {
        eprintln!("[search] vertical=sports no_league_match -> next tier");
        return None;
    };
    let response = match transport
        .send(&scoreboard_request(sport, league, today))
        .await
    {
        Ok(response) => response,
        Err(e) => {
            eprintln!("[search] vertical=sports transport_error {e}");
            return None;
        }
    };
    if response.status != 200 {
        eprintln!(
            "[search] vertical=sports status={} -> engines",
            response.status
        );
        return None;
    }
    let Some((league_label, lines)) =
        parse_scoreboard(&String::from_utf8_lossy(&response.body), local_zone)
    else {
        eprintln!("[search] vertical=sports unparseable -> engines");
        return None;
    };
    if lines.is_empty() {
        eprintln!("[search] vertical=sports empty -> engines");
        return None;
    }
    eprintln!(
        "[search] vertical=sports league={league_label} events={}",
        lines.len()
    );
    Some(sports_source_block(&league_label, &lines, today))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{FakeHttpTransport, HttpResponse};

    /// Real ESPN scoreboard response shape, trimmed to the fields this module
    /// reads (captured live via curl `soccer/fifa.world/scoreboard`, round/stage
    /// shape re-confirmed 2026-07-10): one completed match. Carries
    /// `leagues[0].season.year` (season-year path) AND
    /// `leagues[0].season.type.name` (the round/stage label, "Quarterfinals" in
    /// the live response). This is the exact shape of the bug this module exists
    /// to fix: a `post`/final match whose own status text ["FT"] has no date and
    /// no round, previously letting a small model mistake a live 2026
    /// quarterfinal for the final, or for the famous 2022 semifinal.
    const WORLD_CUP_FIXTURE: &str = r#"{"leagues": [{"name": "FIFA World Cup", "season": {"year": 2026, "type": {"id": "4", "name": "Quarterfinals"}}}], "events": [{"name": "Morocco at France", "date": "2026-07-09T20:00Z", "season": {"year": 2026, "slug": "quarterfinals"}, "status": {"type": {"state": "post", "completed": true, "shortDetail": "FT"}}, "competitions": [{"notes": [], "competitors": [{"homeAway": "home", "score": "2", "team": {"displayName": "France"}}, {"homeAway": "away", "score": "0", "team": {"displayName": "Morocco"}}]}]}]}"#;

    /// Real ESPN scoreboard response shape, trimmed to the fields this module
    /// reads (captured live via curl 2026-07-09, `football/nfl/scoreboard`):
    /// two not-yet-started fixtures, proving the `pre` (scheduled) status path
    /// carries a date/time.
    const NFL_SCHEDULED_FIXTURE: &str = r#"{"leagues": [{"name": "National Football League"}], "events": [{"name": "New England Patriots at Seattle Seahawks", "date": "2026-09-10T00:20Z", "status": {"type": {"state": "pre", "completed": false, "shortDetail": "9/9 - 8:20 PM EDT"}}, "competitions": [{"competitors": [{"homeAway": "home", "score": "0", "team": {"displayName": "Seattle Seahawks"}}, {"homeAway": "away", "score": "0", "team": {"displayName": "New England Patriots"}}]}]}, {"name": "San Francisco 49ers at Los Angeles Rams", "date": "2026-09-11T00:35Z", "status": {"type": {"state": "pre", "completed": false, "shortDetail": "9/10 - 8:35 PM EDT"}}, "competitions": [{"competitors": [{"homeAway": "home", "score": "0", "team": {"displayName": "Los Angeles Rams"}}, {"homeAway": "away", "score": "0", "team": {"displayName": "San Francisco 49ers"}}]}]}]}"#;

    /// Real ESPN scoreboard response shape over a multi-day `?dates=` window,
    /// trimmed to the fields this module reads (captured live 2026-07-10 via
    /// `soccer/fifa.world/scoreboard?dates=20260710-20260717`): one live (`in`)
    /// match and two scheduled (`pre`) matches on different dates. The later
    /// scheduled match (Switzerland at Argentina, 07-12) is placed FIRST in the
    /// array and the earlier one (England at Norway, 07-11) LAST, so the
    /// "next match" summary can only name the right fixture by comparing dates,
    /// not array position.
    const WORLD_CUP_RANGE_FIXTURE: &str = r#"{"leagues": [{"name": "FIFA World Cup", "season": {"year": 2026, "type": {"name": "Quarterfinals"}}}], "events": [
        {"name": "Switzerland at Argentina", "date": "2026-07-12T01:00Z", "status": {"type": {"state": "pre", "shortDetail": "Sat, July 11th at 9:00 PM EDT"}}, "competitions": [{"competitors": [{"homeAway": "home", "score": "0", "team": {"displayName": "Argentina"}}, {"homeAway": "away", "score": "0", "team": {"displayName": "Switzerland"}}]}]},
        {"name": "Belgium at Spain", "date": "2026-07-10T19:00Z", "status": {"type": {"state": "in", "shortDetail": "32'"}}, "competitions": [{"competitors": [{"homeAway": "home", "score": "1", "team": {"displayName": "Spain"}}, {"homeAway": "away", "score": "0", "team": {"displayName": "Belgium"}}]}]},
        {"name": "England at Norway", "date": "2026-07-11T21:00Z", "status": {"type": {"state": "pre", "shortDetail": "Sat, July 11th at 5:00 PM EDT"}}, "competitions": [{"competitors": [{"homeAway": "home", "score": "0", "team": {"displayName": "Norway"}}, {"homeAway": "away", "score": "0", "team": {"displayName": "England"}}]}]}
    ]}"#;

    // ── detect_league ─────────────────────────────────────────────────────────

    #[test]
    fn detect_league_matches_single_word_keywords_case_insensitively() {
        assert_eq!(
            detect_league("what's the score of the Lakers game, NBA update"),
            Some(("basketball", "nba"))
        );
        assert_eq!(
            detect_league("NFL scores this week"),
            Some(("football", "nfl"))
        );
        assert_eq!(detect_league("mlb standings"), Some(("baseball", "mlb")));
        assert_eq!(detect_league("nhl scores"), Some(("hockey", "nhl")));
        assert_eq!(detect_league("who won the f1 race"), Some(("racing", "f1")));
    }

    #[test]
    fn detect_league_matches_multi_word_phrases() {
        assert_eq!(
            detect_league("what's the latest status of the World Cup 2026"),
            Some(("soccer", "fifa.world"))
        );
        assert_eq!(
            detect_league("current standings in the Premier League"),
            Some(("soccer", "eng.1"))
        );
        assert_eq!(
            detect_league("champions league fixtures this week"),
            Some(("soccer", "uefa.champions"))
        );
        assert_eq!(
            detect_league("who is leading formula 1 this season"),
            Some(("racing", "f1"))
        );
    }

    #[test]
    fn detect_league_none_on_no_keyword_match() {
        assert_eq!(detect_league("what is the capital of France"), None);
        assert_eq!(detect_league("weather in Tokyo"), None);
        // Substring collision guard: "nba" must not fire on an unrelated word
        // that merely contains the same letters as a run.
        assert_eq!(detect_league("turbanba is not a word"), None);
    }

    #[test]
    fn is_sports_intent_mirrors_detect_league() {
        assert!(is_sports_intent("nba scores tonight"));
        assert!(!is_sports_intent("what is photosynthesis"));
    }

    // ── request builder ──────────────────────────────────────────────────────

    #[test]
    fn scoreboard_request_builds_the_path_with_a_seven_day_date_window() {
        // A parseable `today` scopes the request to today..today+7 (both
        // YYYYMMDD), so the response spans the next fixtures, not just today's
        // slate: 2026-07-10 -> 20260710-20260717.
        let req = scoreboard_request("soccer", "fifa.world", "2026-07-10");
        assert_eq!(req.method, HttpMethod::Get);
        assert_eq!(
            req.url,
            "https://site.api.espn.com/apis/site/v2/sports/soccer/fifa.world/scoreboard?dates=20260710-20260717"
        );
    }

    #[test]
    fn scoreboard_request_crosses_month_and_year_boundaries() {
        // The window is real date arithmetic, not string math: a late-month
        // `today` rolls into the next month, and a year-end one into the next
        // year.
        assert_eq!(
            scoreboard_request("basketball", "nba", "2026-12-28").url,
            "https://site.api.espn.com/apis/site/v2/sports/basketball/nba/scoreboard?dates=20261228-20270104"
        );
    }

    #[test]
    fn scoreboard_request_falls_back_to_dateless_url_on_unparseable_today() {
        // `today` is program-generated, so a bad value is a defensive fallback:
        // no `dates` param, ESPN's default today-only slate. Every unparseable
        // shape (non-numeric field, too few segments, an extra trailing segment,
        // and an out-of-range field) degrades the same way rather than panicking.
        let dateless = "https://site.api.espn.com/apis/site/v2/sports/soccer/fifa.world/scoreboard";
        for bad in ["not-a-date", "2026-07", "2026-07-10-1", "2026-13-10", ""] {
            let req = scoreboard_request("soccer", "fifa.world", bad);
            assert_eq!(req.url, dateless, "today={bad:?}");
        }
    }

    // ── status_label ──────────────────────────────────────────────────────────

    #[test]
    fn status_label_covers_scheduled_live_final_and_unknown() {
        assert_eq!(
            status_label("pre", "9/9 - 8:20 PM EDT"),
            "scheduled, 9/9 - 8:20 PM EDT"
        );
        assert_eq!(status_label("pre", ""), "scheduled");
        assert_eq!(
            status_label("in", "10:15 - 3rd Qtr"),
            "in progress, 10:15 - 3rd Qtr"
        );
        assert_eq!(status_label("in", ""), "in progress");
        assert_eq!(status_label("post", "FT"), "final");
        assert_eq!(status_label("weird", "Postponed"), "Postponed");
        assert_eq!(status_label("", ""), "status unknown");
    }

    // ── event_date ────────────────────────────────────────────────────────────

    #[test]
    fn event_date_extracts_date_portion_before_t_separator() {
        let event = serde_json::json!({"date": "2026-07-09T20:00Z"});
        assert_eq!(event_date(&event), Some("2026-07-09".to_string()));
    }

    #[test]
    fn event_date_returns_raw_value_when_no_t_separator() {
        // A shape drift (no literal "T"): the raw value is kept rather than
        // dropped, so the line still carries a date, just unformatted.
        let event = serde_json::json!({"date": "2026-07-09"});
        assert_eq!(event_date(&event), Some("2026-07-09".to_string()));
    }

    #[test]
    fn event_date_none_when_missing_or_wrong_type() {
        assert_eq!(event_date(&serde_json::json!({})), None);
        assert_eq!(event_date(&serde_json::json!({"date": 20260709})), None);
    }

    // ── kickoff time (local tz) ───────────────────────────────────────────────

    #[test]
    fn parse_event_utc_reads_minute_and_second_precision() {
        // ESPN's own shape: minute precision, literal `Z`.
        let dt = parse_event_utc("2026-09-10T00:20Z").unwrap();
        assert_eq!(dt.year(), 2026);
        assert_eq!(u8::from(dt.month()), 9);
        assert_eq!(dt.day(), 10);
        assert_eq!(dt.hour(), 0);
        assert_eq!(dt.minute(), 20);
        assert_eq!(dt.offset(), time::UtcOffset::UTC);
        // A second-precision value is also accepted.
        let with_secs = parse_event_utc("2026-09-10T00:20:45Z").unwrap();
        assert_eq!(with_secs.second(), 45);
    }

    #[test]
    fn parse_event_utc_none_on_unrecognised_shapes() {
        // No `T` separator.
        assert!(parse_event_utc("2026-09-10").is_none());
        // A non-`Z` value (explicit numeric offset) is treated as unrecognised
        // rather than risk mis-zoning it.
        assert!(parse_event_utc("2026-09-10T00:20-04:00").is_none());
        // An extra date segment and an extra time segment are both rejected.
        assert!(parse_event_utc("2026-09-10-1T00:20Z").is_none());
        assert!(parse_event_utc("2026-09-10T00:20:00:00Z").is_none());
        // Non-numeric and out-of-range field values.
        assert!(parse_event_utc("2026-xx-10T00:20Z").is_none());
        assert!(parse_event_utc("2026-13-10T00:20Z").is_none());
        assert!(parse_event_utc("2026-09-31T00:20Z").is_none());
        assert!(parse_event_utc("2026-09-10T25:20Z").is_none());
    }

    #[test]
    fn event_local_kickoff_converts_utc_to_local_zone() {
        // 2026-09-10T00:20Z in America/New_York is EDT (UTC-4) in September:
        // 00:20 - 4h = 20:20 the prior evening (2026-09-09). Both the date and
        // the time are localized, so a late-UTC kickoff dates correctly.
        let event = serde_json::json!({"date": "2026-09-10T00:20Z"});
        assert_eq!(
            event_local_kickoff(&event, Some("America/New_York")),
            Some("2026-09-09 at 20:20 (America/New_York)".to_string())
        );
    }

    #[test]
    fn event_local_kickoff_none_on_missing_zone_bad_date_or_unknown_zone() {
        let event = serde_json::json!({"date": "2026-09-10T00:20Z"});
        // No local zone known.
        assert!(event_local_kickoff(&event, None).is_none());
        // Zone present but the date field is missing / wrong-typed.
        assert!(event_local_kickoff(&serde_json::json!({}), Some("America/New_York")).is_none());
        assert!(event_local_kickoff(
            &serde_json::json!({"date": 20260910}),
            Some("America/New_York")
        )
        .is_none());
        // Zone present but the date does not parse (no time component).
        let dateless = serde_json::json!({"date": "2026-09-10"});
        assert!(event_local_kickoff(&dateless, Some("America/New_York")).is_none());
        // Zone present but does not resolve.
        assert!(event_local_kickoff(&event, Some("Mars/Olympus_Mons")).is_none());
    }

    #[test]
    fn format_event_appends_local_kickoff_only_for_scheduled_events() {
        // A scheduled (`pre`) event's LISTING line carries its localized kickoff
        // datetime. The NFL fixture is all `pre`, so the returned list leads with
        // a "Next scheduled match:" summary; the first listing entry (index 1) is
        // the first fixture, whose kickoff is what we assert here.
        let (_, scheduled) =
            parse_scoreboard(NFL_SCHEDULED_FIXTURE, Some("America/New_York")).unwrap();
        assert!(scheduled[0].starts_with("Next scheduled match:"));
        assert!(scheduled[1].contains("on 2026-09-09 at 20:20 (America/New_York)"));
        // A final (`post`) event never gets a kickoff datetime even with a zone:
        // it keeps ESPN's own UTC calendar date and no localized time (the zone
        // label appears only in a kickoff suffix, so its absence proves the skip;
        // the event name "Morocco at France" contains an unrelated " at ").
        let (_, finished) = parse_scoreboard(WORLD_CUP_FIXTURE, Some("America/New_York")).unwrap();
        assert!(finished[0].contains("on 2026-07-09"));
        assert!(!finished[0].contains("(America/New_York)"));
    }

    #[test]
    fn event_local_kickoff_is_dst_correct_at_the_event_instant() {
        // America/Los_Angeles 2026 spring-forward is 2026-03-08T10:00:00Z. An
        // event at 09:00Z is still PST (UTC-8, local 01:00); one at 11:00Z is
        // PDT (UTC-7, local 04:00). The offset is looked up for the event's own
        // instant, so a fixture either side of the transition localizes right.
        let before = serde_json::json!({"date": "2026-03-08T09:00Z"});
        let after = serde_json::json!({"date": "2026-03-08T11:00Z"});
        assert_eq!(
            event_local_kickoff(&before, Some("America/Los_Angeles")),
            Some("2026-03-08 at 01:00 (America/Los_Angeles)".to_string())
        );
        assert_eq!(
            event_local_kickoff(&after, Some("America/Los_Angeles")),
            Some("2026-03-08 at 04:00 (America/Los_Angeles)".to_string())
        );
    }

    // ── season_year ───────────────────────────────────────────────────────────

    #[test]
    fn season_year_reads_leagues_zero_season_year() {
        let json = serde_json::json!({"leagues": [{"name": "X", "season": {"year": 2026}}]});
        assert_eq!(season_year(&json), Some(2026));
    }

    #[test]
    fn season_year_none_on_any_missing_or_wrong_typed_link() {
        assert_eq!(season_year(&serde_json::json!({})), None, "no leagues key");
        assert_eq!(
            season_year(&serde_json::json!({"leagues": []})),
            None,
            "empty leagues array"
        );
        assert_eq!(
            season_year(&serde_json::json!({"leagues": [{"name": "X"}]})),
            None,
            "no season key"
        );
        assert_eq!(
            season_year(&serde_json::json!({"leagues": [{"season": {}}]})),
            None,
            "no year key"
        );
        assert_eq!(
            season_year(&serde_json::json!({"leagues": [{"season": {"year": "2026"}}]})),
            None,
            "year is not an integer"
        );
    }

    // ── parse_scoreboard ──────────────────────────────────────────────────────

    #[test]
    fn parse_scoreboard_reads_real_final_match_fixture() {
        // The league label carries the season year (present in this fixture),
        // and the event line carries its own date even though it is a `post`
        // (final) event whose status text alone ("FT" -> "final") has none:
        // this is the concrete fix for the bug where an undated final score
        // reads as a past tournament rather than the live one it is. The line
        // also names the round ("Quarterfinals", from the response-level
        // season.type.name), so the writer can never call a quarterfinal the
        // final.
        let (league, lines) = parse_scoreboard(WORLD_CUP_FIXTURE, None).unwrap();
        assert_eq!(league, "FIFA World Cup 2026");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Morocco at France"));
        assert!(lines[0].contains("France 2"));
        assert!(lines[0].contains("Morocco 0"));
        // Status and round are grouped in the parens: "(final, Quarterfinals)".
        assert!(lines[0].contains("(final, Quarterfinals)"));
        assert!(lines[0].contains("on 2026-07-09"));
    }

    #[test]
    fn parse_scoreboard_omits_round_when_season_type_absent_or_blank() {
        // No season.type at all -> no round appended (the NFL fixture path). The
        // fixture is all `pre`, so index 0 is the "Next scheduled match:"
        // summary; the round-omission property is asserted on the first LISTING
        // line (index 1).
        let (_, nfl) = parse_scoreboard(NFL_SCHEDULED_FIXTURE, None).unwrap();
        assert!(nfl[1].contains("(scheduled, 9/9 - 8:20 PM EDT)"));
        assert!(!nfl[1].contains("(scheduled, 9/9 - 8:20 PM EDT,"));
        // A present-but-blank season.type.name is treated as absent, not
        // appended as a dangling ", ".
        let blank = r#"{"leagues": [{"name": "X", "season": {"type": {"name": "   "}}}], "events": [{"name": "A at B", "status": {"type": {"state": "post", "shortDetail": "Final"}}, "competitions": [{"competitors": [{"team": {"displayName": "A"}, "score": "1"}, {"team": {"displayName": "B"}, "score": "2"}]}]}]}"#;
        let (_, lines) = parse_scoreboard(blank, None).unwrap();
        assert!(lines[0].contains("(final)"));
        assert!(!lines[0].contains("final,"));
    }

    #[test]
    fn parse_scoreboard_reads_scheduled_events_with_date() {
        // No `season` field in this fixture: the league label falls back to
        // the plain name, proving the season-year path is optional. Both `pre`
        // fixtures are listed with their date; the list also leads with the
        // "Next scheduled match:" summary (index 0), so the two listing entries
        // are at indices 1 and 2.
        let (league, lines) = parse_scoreboard(NFL_SCHEDULED_FIXTURE, None).unwrap();
        assert_eq!(league, "National Football League");
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("Next scheduled match:"));
        assert!(lines[1].contains("scheduled, 9/9 - 8:20 PM EDT"));
        assert!(lines[1].contains("on 2026-09-10"));
        assert!(lines[2].contains("scheduled, 9/10 - 8:35 PM EDT"));
        assert!(lines[2].contains("on 2026-09-11"));
    }

    #[test]
    fn parse_scoreboard_leads_with_in_progress_and_next_match_summaries() {
        // The range fixture carries one live match and two scheduled matches on
        // different dates, with the LATER scheduled match placed first in the
        // array. The returned list leads with the computed summaries: the live
        // match, then the EARLIEST scheduled match (England at Norway, 07-11),
        // NOT the array-first scheduled match (Switzerland at Argentina, 07-12).
        // This is the deterministic answer to "which match is next", computed
        // from ESPN's own event states with no wall clock.
        let (league, lines) = parse_scoreboard(WORLD_CUP_RANGE_FIXTURE, None).unwrap();
        assert_eq!(league, "FIFA World Cup 2026");
        // Two summaries (in-progress, next) + three listing lines.
        assert_eq!(lines.len(), 5);
        assert!(lines[0].starts_with("Currently in progress: "));
        assert!(lines[0].contains("Belgium at Spain"));
        assert!(lines[0].contains("(in progress, 32', Quarterfinals)"));
        assert!(lines[1].starts_with("Next scheduled match: "));
        // The next match is the earliest-dated `pre` event, not the array-first
        // one: England at Norway (07-11), never Switzerland at Argentina (07-12).
        assert!(lines[1].contains("England at Norway"));
        assert!(!lines[1].contains("Switzerland at Argentina"));
        assert!(lines[1].contains("on 2026-07-11"));
        // The listing (evidence) still carries all three events below the
        // summaries, in the response's original order.
        assert!(lines[2].contains("Switzerland at Argentina"));
        assert!(lines[3].contains("Belgium at Spain"));
        assert!(lines[4].contains("England at Norway"));
    }

    #[test]
    fn parse_scoreboard_omits_next_summary_when_no_scheduled_events() {
        // An all-`post` slate (the final-match fixture) has no scheduled match,
        // so no "Next scheduled match:" line is emitted at all.
        let (_, lines) = parse_scoreboard(WORLD_CUP_FIXTURE, None).unwrap();
        assert!(!lines.iter().any(|l| l.starts_with("Next scheduled match:")));
        assert!(!lines
            .iter()
            .any(|l| l.starts_with("Currently in progress:")));
    }

    #[test]
    fn parse_scoreboard_omits_in_progress_summary_when_none_live() {
        // The NFL fixture is all `pre`: a "Next scheduled match:" summary is
        // emitted but no "Currently in progress:" line, since nothing is live.
        let (_, lines) = parse_scoreboard(NFL_SCHEDULED_FIXTURE, None).unwrap();
        assert!(lines.iter().any(|l| l.starts_with("Next scheduled match:")));
        assert!(!lines
            .iter()
            .any(|l| l.starts_with("Currently in progress:")));
    }

    #[test]
    fn parse_scoreboard_computes_next_summary_before_the_listing_cap() {
        // The next-match summary is computed from ALL parsed events, before the
        // listing is capped at MAX_SPORTS_EVENTS. Here the only scheduled match
        // sits past the cap (after MAX_SPORTS_EVENTS + 2 finished games): it is
        // dropped from the listing, yet the summary still names it.
        let mut events: Vec<serde_json::Value> = (0..MAX_SPORTS_EVENTS + 2)
            .map(|i| {
                serde_json::json!({
                    "name": format!("Team A{i} at Team B{i}"),
                    "status": {"type": {"state": "post", "shortDetail": "Final"}},
                    "competitions": [{"competitors": [
                        {"score": "1", "team": {"displayName": format!("Team B{i}")}},
                        {"score": "0", "team": {"displayName": format!("Team A{i}")}}
                    ]}]
                })
            })
            .collect();
        events.push(serde_json::json!({
            "name": "Future Cup Final",
            "date": "2026-08-01T18:00Z",
            "status": {"type": {"state": "pre", "shortDetail": "8/1 - 2:00 PM EDT"}},
            "competitions": [{"competitors": [
                {"score": "0", "team": {"displayName": "Home Side"}},
                {"score": "0", "team": {"displayName": "Away Side"}}
            ]}]
        }));
        let body = serde_json::json!({"leagues": [{"name": "Cup"}], "events": events}).to_string();
        let (_, lines) = parse_scoreboard(&body, None).unwrap();
        // One summary line + MAX_SPORTS_EVENTS listing lines.
        assert_eq!(lines.len(), MAX_SPORTS_EVENTS + 1);
        assert!(lines[0].starts_with("Next scheduled match: "));
        assert!(lines[0].contains("Future Cup Final"));
        // The scheduled match was capped OUT of the listing (only summary has it).
        assert!(
            !lines[1..].iter().any(|l| l.contains("Future Cup Final")),
            "the next match sits past the listing cap; only the summary surfaces it"
        );
    }

    #[test]
    fn parse_scoreboard_caps_at_max_events() {
        let events: Vec<serde_json::Value> = (0..MAX_SPORTS_EVENTS + 4)
            .map(|i| {
                serde_json::json!({
                    "name": format!("Team A{i} at Team B{i}"),
                    "status": {"type": {"state": "post", "completed": true, "shortDetail": "Final"}},
                    "competitions": [{"competitors": [
                        {"homeAway": "home", "score": "1", "team": {"displayName": format!("Team B{i}")}},
                        {"homeAway": "away", "score": "0", "team": {"displayName": format!("Team A{i}")}}
                    ]}]
                })
            })
            .collect();
        let body =
            serde_json::json!({"leagues": [{"name": "Test League"}], "events": events}).to_string();
        let (_, lines) = parse_scoreboard(&body, None).unwrap();
        assert_eq!(lines.len(), MAX_SPORTS_EVENTS);
    }

    #[test]
    fn parse_scoreboard_none_on_unparseable_or_missing_events_field() {
        assert!(parse_scoreboard("not json", None).is_none());
        assert!(parse_scoreboard(r#"{"leagues": [{"name": "X"}]}"#, None).is_none());
    }

    #[test]
    fn parse_scoreboard_none_when_events_field_is_wrong_type() {
        // "events" present but not an array (e.g. a corrupt/hostile response):
        // the shape guard rejects it rather than panicking.
        assert!(parse_scoreboard(
            r#"{"leagues": [{"name": "X"}], "events": "not an array"}"#,
            None
        )
        .is_none());
    }

    #[test]
    fn parse_scoreboard_exercises_every_defensive_branch() {
        // Every shape-drift case format_event and competitor_name/score guard
        // against, each isolated to its own event so no single malformed field
        // masks another: a non-string name, a missing "competitions" key, an
        // empty competitions array, a competitions[0] with no "competitors"
        // key, a competitor with no "team" key, a competitor whose "team" has
        // no "displayName", and a bare-float score (the as_f64 fallback).
        let body = r#"{"events": [
            {"name": 123, "competitions": [{"competitors": []}]},
            {"name": "No Competitions Key"},
            {"name": "Empty Competitions Array", "competitions": []},
            {"name": "No Competitors Key", "competitions": [{}]},
            {"name": "Missing Team Competitor", "status": {"type": {"state": "post", "shortDetail": "Final"}}, "competitions": [{"competitors": [
                {"score": "1"},
                {"team": {"displayName": "Solo Team"}, "score": "2"}
            ]}]},
            {"name": "Team Missing DisplayName", "status": {"type": {"state": "post", "shortDetail": "Final"}}, "competitions": [{"competitors": [
                {"team": {}, "score": "1"},
                {"team": {"displayName": "Solo Team Two"}, "score": "2"}
            ]}]},
            {"name": "Float Score Game", "status": {"type": {"state": "post", "shortDetail": "Final"}}, "competitions": [{"competitors": [
                {"team": {"displayName": "Home X"}, "score": 3.5},
                {"team": {"displayName": "Away X"}, "score": 1}
            ]}]}
        ]}"#;
        let (_, lines) = parse_scoreboard(body, None).unwrap();
        // Only "Float Score Game" has two competitors that both resolve a
        // name; every other event drops out (a wrong-type/missing field or a
        // competitor that fails to resolve leaves fewer than two names).
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Float Score Game"));
        assert!(lines[0].contains("Home X 3.5"));
        assert!(lines[0].contains("Away X 1"));
    }

    #[test]
    fn parse_scoreboard_empty_events_array_yields_some_with_no_lines() {
        // An off-season slate: valid shape, zero games. The caller (fetch_sports)
        // treats this as a miss, but the pure parser itself returns Some.
        let (league, lines) =
            parse_scoreboard(r#"{"leagues": [{"name": "X League"}], "events": []}"#, None).unwrap();
        assert_eq!(league, "X League");
        assert!(lines.is_empty());
    }

    #[test]
    fn parse_scoreboard_skips_malformed_events_and_falls_back_league_name() {
        // No name, only one competitor, and a non-array competitors field are
        // all skipped; missing leagues/name falls back to a generic label.
        let body = r#"{"events": [
            {"status": {}, "competitions": [{"competitors": []}]},
            {"name": "Solo", "competitions": [{"competitors": [{"team": {"displayName": "Only One"}, "score": "1"}]}]},
            {"name": "Bad Shape", "competitions": [{"competitors": "not an array"}]},
            {"name": "Good Game", "status": {"type": {"state": "post", "shortDetail": "Final"}}, "competitions": [{"competitors": [
                {"team": {"displayName": "Home Team"}, "score": 3},
                {"team": {"displayName": "Away Team"}}
            ]}]}
        ]}"#;
        let (league, lines) = parse_scoreboard(body, None).unwrap();
        assert_eq!(league, "the league");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Good Game"));
        // Numeric score coerced to text; missing score degrades to "-".
        assert!(lines[0].contains("Home Team 3"));
        assert!(lines[0].contains("Away Team -"));
    }

    // ── sports_source_block ──────────────────────────────────────────────────

    #[test]
    fn source_block_names_league_and_cites_espn_homepage() {
        let block = sports_source_block(
            "FIFA World Cup 2026",
            &["Morocco at France: France 2 vs Morocco 0 (final) on 2026-07-09".to_string()],
            "2026-07-09",
        );
        assert_eq!(block.index, 1);
        assert_eq!(block.url, "https://www.espn.com/");
        assert_eq!(block.title, "ESPN scores: FIFA World Cup 2026");
        assert!(block
            .text
            .contains("Live scores and schedule for FIFA World Cup 2026"));
        assert!(block.text.contains("Morocco at France"));
    }

    #[test]
    fn source_block_is_prefixed_with_an_as_of_line() {
        // The as-of line is the fix for the writer misdating a scoreboard
        // it has no other way to date: it must lead the block, naming both
        // the competition and today's date.
        let block = sports_source_block("FIFA World Cup 2026", &["line".to_string()], "2026-07-09");
        assert!(block
            .text
            .starts_with("FIFA World Cup 2026, as of 2026-07-09."));
    }

    // ── fetch_sports over the fake transport ─────────────────────────────────

    #[tokio::test]
    async fn fetch_sports_resolves_full_chain_on_league_match() {
        let url = scoreboard_request("soccer", "fifa.world", "2026-07-09").url;
        let transport = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 200,
                final_url: url.clone(),
                body: WORLD_CUP_FIXTURE.as_bytes().to_vec(),
            },
        );
        let block = fetch_sports(
            &transport,
            "what's the latest status of the World Cup 2026",
            "2026-07-09",
            None,
        )
        .await
        .unwrap();
        assert_eq!(block.title, "ESPN scores: FIFA World Cup 2026");
        assert!(block.text.contains("France 2"));
        assert!(block.text.contains("as of 2026-07-09"));
    }

    #[tokio::test]
    async fn fetch_sports_none_when_no_league_matches() {
        let transport = FakeHttpTransport::new();
        assert!(
            fetch_sports(&transport, "what is photosynthesis", "2026-07-09", None)
                .await
                .is_none()
        );
        assert!(transport.calls().is_empty(), "no request sent on a miss");
    }

    #[tokio::test]
    async fn fetch_sports_none_on_transport_error() {
        // No canned response -> transport error -> None.
        let transport = FakeHttpTransport::new();
        assert!(
            fetch_sports(&transport, "nba scores tonight", "2026-07-09", None)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn fetch_sports_none_on_bad_status() {
        let url = scoreboard_request("basketball", "nba", "2026-07-09").url;
        let transport = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 503,
                final_url: url.clone(),
                body: Vec::new(),
            },
        );
        assert!(
            fetch_sports(&transport, "nba scores tonight", "2026-07-09", None)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn fetch_sports_none_on_empty_events() {
        let url = scoreboard_request("hockey", "nhl", "2026-07-09").url;
        let transport = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 200,
                final_url: url.clone(),
                body: br#"{"leagues": [{"name": "NHL"}], "events": []}"#.to_vec(),
            },
        );
        assert!(
            fetch_sports(&transport, "nhl scores tonight", "2026-07-09", None)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn fetch_sports_none_on_unparseable_body() {
        let url = scoreboard_request("baseball", "mlb", "2026-07-09").url;
        let transport = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 200,
                final_url: url.clone(),
                body: b"<html>not json</html>".to_vec(),
            },
        );
        assert!(
            fetch_sports(&transport, "mlb scores tonight", "2026-07-09", None)
                .await
                .is_none()
        );
    }
}
