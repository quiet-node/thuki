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

use crate::config::defaults::SPORTS_LEAGUE_MAP;
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

/// Builds the scoreboard GET request for a resolved `(sport, league)` pair.
pub(crate) fn scoreboard_request(sport: &str, league: &str) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Get,
        url: format!("{ESPN_SCOREBOARD_BASE}/{sport}/{league}/scoreboard"),
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

/// Formats one scoreboard event into a single display line, or `None` when the
/// event is missing its name or fewer than two competitors resolve a name (a
/// malformed or unrecognisable row is skipped rather than shown half-blank).
fn format_event(event: &serde_json::Value) -> Option<String> {
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
    let status_type = event.get("status").and_then(|s| s.get("type"));
    let state = status_type
        .and_then(|t| t.get("state"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let short_detail = status_type
        .and_then(|t| t.get("shortDetail"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    Some(format!(
        "{name}: {} ({})",
        lines.join(" vs "),
        status_label(state, short_detail)
    ))
}

/// Parses a scoreboard response body into the league's display name and up to
/// [`MAX_SPORTS_EVENTS`] formatted event lines. Returns `None` only when the
/// body is not JSON or carries no `events` field at all; an `events` array
/// that parses to zero usable lines (e.g. an off-season slate) returns
/// `Some((name, vec![]))`, which the caller treats as a miss.
pub(crate) fn parse_scoreboard(body: &str) -> Option<(String, Vec<String>)> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let league_name = json
        .get("leagues")
        .and_then(|l| l.get(0))
        .and_then(|l| l.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("the league")
        .to_string();
    let events = json.get("events")?.as_array()?;
    let lines = events
        .iter()
        .filter_map(format_event)
        .take(MAX_SPORTS_EVENTS)
        .collect();
    Some((league_name, lines))
}

/// Wraps formatted event lines into the single `[1]` source block the writer
/// cites, naming the league in the title and citing ESPN's homepage (the
/// scoreboard API has no public page of its own).
pub(crate) fn sports_source_block(league_name: &str, lines: &[String]) -> SourceBlock {
    let mut text = format!("Live scores and schedule for {league_name} (via ESPN):");
    for line in lines {
        text.push_str("\n- ");
        text.push_str(line);
    }
    SourceBlock {
        index: 1,
        url: ESPN_PAGE_URL.to_string(),
        title: format!("ESPN scores: {league_name}"),
        text,
    }
}

/// Runs the full sports vertical for `standalone_question`: league detection,
/// scoreboard fetch, parse. Returns `None` on any miss (no league match,
/// transport error, non-200, unparseable body, or zero usable events) so the
/// caller falls through to the news / engine tiers.
///
/// Coverage-excluded: thin async glue over the injectable transport
/// delegating every decision to the pure helpers above, which are all tested
/// directly; the glue itself is still exercised against
/// [`crate::net::transport::FakeHttpTransport`] in the tests below.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) async fn fetch_sports(
    transport: &dyn HttpTransport,
    standalone_question: &str,
) -> Option<SourceBlock> {
    let (sport, league) = detect_league(standalone_question)?;
    let response = match transport.send(&scoreboard_request(sport, league)).await {
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
    let Some((league_name, lines)) = parse_scoreboard(&String::from_utf8_lossy(&response.body))
    else {
        eprintln!("[search] vertical=sports unparseable -> engines");
        return None;
    };
    if lines.is_empty() {
        eprintln!("[search] vertical=sports empty -> engines");
        return None;
    }
    eprintln!(
        "[search] vertical=sports league={league_name} events={}",
        lines.len()
    );
    Some(sports_source_block(&league_name, &lines))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{FakeHttpTransport, HttpResponse};

    /// Real ESPN scoreboard response shape, trimmed to the fields this module
    /// reads (captured live via curl 2026-07-09, `soccer/fifa.world/scoreboard`):
    /// one completed match.
    const WORLD_CUP_FIXTURE: &str = r#"{"leagues": [{"name": "FIFA World Cup"}], "events": [{"name": "Morocco at France", "date": "2026-07-09T20:00Z", "status": {"type": {"state": "post", "completed": true, "shortDetail": "FT"}}, "competitions": [{"competitors": [{"homeAway": "home", "score": "2", "team": {"displayName": "France"}}, {"homeAway": "away", "score": "0", "team": {"displayName": "Morocco"}}]}]}]}"#;

    /// Real ESPN scoreboard response shape, trimmed to the fields this module
    /// reads (captured live via curl 2026-07-09, `football/nfl/scoreboard`):
    /// two not-yet-started fixtures, proving the `pre` (scheduled) status path
    /// carries a date/time.
    const NFL_SCHEDULED_FIXTURE: &str = r#"{"leagues": [{"name": "National Football League"}], "events": [{"name": "New England Patriots at Seattle Seahawks", "date": "2026-09-10T00:20Z", "status": {"type": {"state": "pre", "completed": false, "shortDetail": "9/9 - 8:20 PM EDT"}}, "competitions": [{"competitors": [{"homeAway": "home", "score": "0", "team": {"displayName": "Seattle Seahawks"}}, {"homeAway": "away", "score": "0", "team": {"displayName": "New England Patriots"}}]}]}, {"name": "San Francisco 49ers at Los Angeles Rams", "date": "2026-09-11T00:35Z", "status": {"type": {"state": "pre", "completed": false, "shortDetail": "9/10 - 8:35 PM EDT"}}, "competitions": [{"competitors": [{"homeAway": "home", "score": "0", "team": {"displayName": "Los Angeles Rams"}}, {"homeAway": "away", "score": "0", "team": {"displayName": "San Francisco 49ers"}}]}]}]}"#;

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
    fn scoreboard_request_builds_the_path_from_sport_and_league() {
        let req = scoreboard_request("soccer", "fifa.world");
        assert_eq!(req.method, HttpMethod::Get);
        assert_eq!(
            req.url,
            "https://site.api.espn.com/apis/site/v2/sports/soccer/fifa.world/scoreboard"
        );
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

    // ── parse_scoreboard ──────────────────────────────────────────────────────

    #[test]
    fn parse_scoreboard_reads_real_final_match_fixture() {
        let (league, lines) = parse_scoreboard(WORLD_CUP_FIXTURE).unwrap();
        assert_eq!(league, "FIFA World Cup");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Morocco at France"));
        assert!(lines[0].contains("France 2"));
        assert!(lines[0].contains("Morocco 0"));
        assert!(lines[0].contains("(final)"));
    }

    #[test]
    fn parse_scoreboard_reads_scheduled_events_with_date() {
        let (league, lines) = parse_scoreboard(NFL_SCHEDULED_FIXTURE).unwrap();
        assert_eq!(league, "National Football League");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("scheduled, 9/9 - 8:20 PM EDT"));
        assert!(lines[1].contains("scheduled, 9/10 - 8:35 PM EDT"));
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
        let (_, lines) = parse_scoreboard(&body).unwrap();
        assert_eq!(lines.len(), MAX_SPORTS_EVENTS);
    }

    #[test]
    fn parse_scoreboard_none_on_unparseable_or_missing_events_field() {
        assert!(parse_scoreboard("not json").is_none());
        assert!(parse_scoreboard(r#"{"leagues": [{"name": "X"}]}"#).is_none());
    }

    #[test]
    fn parse_scoreboard_none_when_events_field_is_wrong_type() {
        // "events" present but not an array (e.g. a corrupt/hostile response):
        // the shape guard rejects it rather than panicking.
        assert!(
            parse_scoreboard(r#"{"leagues": [{"name": "X"}], "events": "not an array"}"#).is_none()
        );
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
        let (_, lines) = parse_scoreboard(body).unwrap();
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
            parse_scoreboard(r#"{"leagues": [{"name": "X League"}], "events": []}"#).unwrap();
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
        let (league, lines) = parse_scoreboard(body).unwrap();
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
            "FIFA World Cup",
            &["Morocco at France: France 2 vs Morocco 0 (final)".to_string()],
        );
        assert_eq!(block.index, 1);
        assert_eq!(block.url, "https://www.espn.com/");
        assert_eq!(block.title, "ESPN scores: FIFA World Cup");
        assert!(block
            .text
            .contains("Live scores and schedule for FIFA World Cup"));
        assert!(block.text.contains("Morocco at France"));
    }

    // ── fetch_sports over the fake transport ─────────────────────────────────

    #[tokio::test]
    async fn fetch_sports_resolves_full_chain_on_league_match() {
        let url = scoreboard_request("soccer", "fifa.world").url;
        let transport = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 200,
                final_url: url.clone(),
                body: WORLD_CUP_FIXTURE.as_bytes().to_vec(),
            },
        );
        let block = fetch_sports(&transport, "what's the latest status of the World Cup 2026")
            .await
            .unwrap();
        assert_eq!(block.title, "ESPN scores: FIFA World Cup");
        assert!(block.text.contains("France 2"));
    }

    #[tokio::test]
    async fn fetch_sports_none_when_no_league_matches() {
        let transport = FakeHttpTransport::new();
        assert!(fetch_sports(&transport, "what is photosynthesis")
            .await
            .is_none());
        assert!(transport.calls().is_empty(), "no request sent on a miss");
    }

    #[tokio::test]
    async fn fetch_sports_none_on_transport_error() {
        // No canned response -> transport error -> None.
        let transport = FakeHttpTransport::new();
        assert!(fetch_sports(&transport, "nba scores tonight")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn fetch_sports_none_on_bad_status() {
        let url = scoreboard_request("basketball", "nba").url;
        let transport = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 503,
                final_url: url.clone(),
                body: Vec::new(),
            },
        );
        assert!(fetch_sports(&transport, "nba scores tonight")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn fetch_sports_none_on_empty_events() {
        let url = scoreboard_request("hockey", "nhl").url;
        let transport = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 200,
                final_url: url.clone(),
                body: br#"{"leagues": [{"name": "NHL"}], "events": []}"#.to_vec(),
            },
        );
        assert!(fetch_sports(&transport, "nhl scores tonight")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn fetch_sports_none_on_unparseable_body() {
        let url = scoreboard_request("baseball", "mlb").url;
        let transport = FakeHttpTransport::new().with_response(
            &url,
            HttpResponse {
                status: 200,
                final_url: url.clone(),
                body: b"<html>not json</html>".to_vec(),
            },
        );
        assert!(fetch_sports(&transport, "mlb scores tonight")
            .await
            .is_none());
    }
}
