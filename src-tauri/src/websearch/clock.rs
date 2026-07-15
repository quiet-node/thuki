//! Deterministic place-time resolution for clock questions naming a place.
//!
//! The bug this fixes: a clock question like "what's today's date? And what
//! time in SF now?" correctly skips web search (see
//! [`crate::websearch::prefilter::is_clock_question`]), but the persona
//! system prompt only ever carried the DEVICE'S local time
//! (`commands::local_datetime_context`); the model was left to convert that
//! into a remote place's time itself, and small local models are unreliable
//! at arithmetic. The fix moves the arithmetic into code: when a clock
//! question names a place (see
//! [`crate::websearch::prefilter::clock_question_place`]), the place is
//! geocoded to its IANA timezone via the same Open-Meteo client
//! [`crate::websearch::weather`] uses for the weather vertical, and the
//! current wall-clock time in that zone is computed here, DST-aware, by
//! reading the system tz database (`/usr/share/zoneinfo`, shipped by every
//! macOS install; see the `tz` crate) rather than asking the model to guess
//! an offset. The resolved line is injected into the per-turn system prompt
//! (`commands::system_prompt_with_datetime`), so the model only ever reads
//! the answer back, never computes it.
//!
//! Every step degrades to `None` on any miss (no place in the question, the
//! place does not geocode, a transport error, an unresolvable timezone
//! name): the caller then injects nothing extra, so the model falls back to
//! today's behaviour (its own local-time context plus a caveat instruction).
//! This vertical never triggers a web search: it is a pure geocode-plus-
//! computation, entirely independent of the search decision.

use time::OffsetDateTime;

use crate::net::transport::HttpTransport;
use crate::websearch::weather::{geocode_request, parse_geocode, GeoPlace};

/// Resolves `place`'s current wall-clock time via geocode + system tzdata, or
/// `None` on any miss. `now_utc` is the instant to resolve.
///
/// `lang` is the language of the user's own message (see
/// [`crate::websearch::lang::resolve_lang`]), which localises the place name the
/// geocoder returns and therefore the name in the injected time line. This path
/// runs before and independently of the search decision, so there is no
/// classifier judgement to draw on: the caller resolves it from the raw message
/// and the locale alone.
///
/// Coverage-excluded: thin async glue over the injectable transport,
/// delegating every decision to [`parse_geocode`] and
/// [`format_place_time_line`] (both fully tested elsewhere); the glue itself
/// is exercised against [`crate::net::transport::FakeHttpTransport`] in the
/// tests below.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) async fn resolve_place_time(
    transport: &dyn HttpTransport,
    place: &str,
    now_utc: OffsetDateTime,
    lang: &str,
) -> Option<String> {
    // Bare send, not `net::transport::send_with_retry`: this clock feature sits
    // outside the search decision entirely (see the module docs above) and is not
    // part of the search vertical stack the retry policy covers, even though it
    // issues the identical Open-Meteo geocoding request `websearch::weather` does.
    let response = transport.send(&geocode_request(place, lang)).await.ok()?;
    let geo = parse_geocode(&String::from_utf8_lossy(&response.body))?;
    format_place_time_line(&geo, now_utc)
}

/// Formats the resolved place-time context line injected into the system
/// prompt, e.g. `"Current time in San Francisco (America/Los_Angeles):
/// 06:42, Friday, 2026-07-10."`. `None` when `geo`'s timezone does not
/// resolve (see [`resolve_offset`]).
pub(crate) fn format_place_time_line(geo: &GeoPlace, now_utc: OffsetDateTime) -> Option<String> {
    let offset = resolve_offset(&geo.timezone, now_utc)?;
    let local = now_utc.to_offset(offset);
    Some(format!(
        "Current time in {} ({}): {:02}:{:02}, {}, {:04}-{:02}-{:02}.",
        geo.name,
        geo.timezone,
        local.hour(),
        local.minute(),
        local.weekday(),
        local.year(),
        u8::from(local.month()),
        local.day(),
    ))
}

/// Looks up `iana_tz`'s UTC offset at `now_utc`, DST-aware, by reading the
/// system tz database (`tz::TimeZone::from_posix_tz` resolves a bare name
/// relative to `/usr/share/zoneinfo`). `None` on any failure: an implausible
/// or unknown zone name (see [`is_plausible_iana_zone`]), a corrupt system
/// tzdata entry, or an instant outside the zone's representable range.
///
/// Shared with the sports vertical, which converts each scheduled event's UTC
/// kickoff into the user's local zone at the event instant (see
/// [`crate::websearch::sports`]); the DST-correct offset is looked up for that
/// timestamp, not "now".
pub(crate) fn resolve_offset(iana_tz: &str, now_utc: OffsetDateTime) -> Option<time::UtcOffset> {
    if !is_plausible_iana_zone(iana_tz) {
        return None;
    }
    let tz = tz::TimeZone::from_posix_tz(iana_tz).ok()?;
    let local_time_type = tz.find_local_time_type(now_utc.unix_timestamp()).ok()?;
    time::UtcOffset::from_whole_seconds(local_time_type.ut_offset()).ok()
}

/// Whether `name` looks like a plausible bare IANA zone name (e.g.
/// "America/Los_Angeles"). Defense-in-depth: `tz::TimeZone::from_posix_tz`
/// treats a leading `/` or `:` as a filesystem path (absolute, or relative
/// to the system timezone directory), so an untrusted `timezone` field from
/// the geocoding response is validated to look like a real IANA name before
/// being handed to it, rather than trusting a third-party API response not
/// to smuggle a path. Real IANA names are ASCII letters, digits, `_`, `+`,
/// `-`, and `/` between path segments, never start with `/` or `:`, and
/// never contain `..`.
fn is_plausible_iana_zone(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.starts_with(':')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{FakeHttpTransport, HttpResponse};

    fn fixed_utc() -> OffsetDateTime {
        // 2026-07-10 13:42:00 UTC, a Friday; -7h (PDT, summer) => 06:42 local.
        time::macros::datetime!(2026-07-10 13:42:00 UTC)
    }

    fn san_francisco() -> GeoPlace {
        GeoPlace {
            name: "San Francisco".into(),
            country: "United States".into(),
            latitude: 37.77493,
            longitude: -122.41942,
            timezone: "America/Los_Angeles".into(),
        }
    }

    // ── is_plausible_iana_zone ────────────────────────────────────────────────

    #[test]
    fn plausible_zone_names_are_accepted() {
        assert!(is_plausible_iana_zone("America/Los_Angeles"));
        assert!(is_plausible_iana_zone("Asia/Tokyo"));
        assert!(is_plausible_iana_zone("UTC"));
        assert!(is_plausible_iana_zone("Etc/GMT+5"));
    }

    #[test]
    fn implausible_zone_names_are_rejected() {
        // Defense-in-depth: a leading `/` or `:` would be read as a
        // filesystem path by `tz::TimeZone::from_posix_tz`.
        assert!(!is_plausible_iana_zone("/etc/passwd"));
        assert!(!is_plausible_iana_zone(":UTC"));
        assert!(!is_plausible_iana_zone("America/../../etc/passwd"));
        assert!(!is_plausible_iana_zone(""));
        assert!(!is_plausible_iana_zone("America/Los Angeles"));
        assert!(!is_plausible_iana_zone("America/Los_Angeles;rm -rf"));
    }

    // ── resolve_offset / format_place_time_line ─────────────────────────────

    #[test]
    fn format_place_time_line_converts_known_zone() {
        let line = format_place_time_line(&san_francisco(), fixed_utc()).unwrap();
        assert_eq!(
            line,
            "Current time in San Francisco (America/Los_Angeles): 06:42, Friday, 2026-07-10."
        );
    }

    #[test]
    fn format_place_time_line_none_on_empty_timezone() {
        let mut place = san_francisco();
        place.timezone = String::new();
        assert!(format_place_time_line(&place, fixed_utc()).is_none());
    }

    #[test]
    fn format_place_time_line_none_on_unknown_zone() {
        let mut place = san_francisco();
        place.timezone = "Mars/Olympus_Mons".into();
        assert!(format_place_time_line(&place, fixed_utc()).is_none());
    }

    #[test]
    fn dst_spring_forward_shifts_the_offset() {
        // America/Los_Angeles 2026 spring-forward transitions at
        // 2026-03-08T10:00:00Z (02:00 local PST -> 03:00 local PDT).
        // 09:00 UTC is still standard time (PST, UTC-8, local 01:00); 11:00
        // UTC is after the jump (PDT, UTC-7, local 04:00). Verified against
        // ICU (Intl.DateTimeFormat with timeZone America/Los_Angeles).
        let before = time::macros::datetime!(2026-03-08 09:00:00 UTC);
        let after = time::macros::datetime!(2026-03-08 11:00:00 UTC);
        let before_line = format_place_time_line(&san_francisco(), before).unwrap();
        let after_line = format_place_time_line(&san_francisco(), after).unwrap();
        assert!(before_line.contains("01:00"), "{before_line}");
        assert!(after_line.contains("04:00"), "{after_line}");
    }

    #[test]
    fn dst_fall_back_shifts_the_offset() {
        // America/Los_Angeles 2026 fall-back transitions at
        // 2026-11-01T09:00:00Z (02:00 local PDT -> 01:00 local PST).
        // 08:59 UTC is still daylight time (PDT, UTC-7, local 01:59); 10:00
        // UTC is after the fall-back (PST, UTC-8, local 02:00).
        let before = time::macros::datetime!(2026-11-01 08:59:00 UTC);
        let after = time::macros::datetime!(2026-11-01 10:00:00 UTC);
        let before_line = format_place_time_line(&san_francisco(), before).unwrap();
        let after_line = format_place_time_line(&san_francisco(), after).unwrap();
        assert!(before_line.contains("01:59"), "{before_line}");
        assert!(after_line.contains("02:00"), "{after_line}");
    }

    // ── resolve_place_time (async glue over FakeHttpTransport) ──────────────

    #[tokio::test]
    async fn resolve_place_time_resolves_full_chain() {
        let geo_url = geocode_request("San Francisco", "en").url;
        let body = br#"{"results":[{"name":"San Francisco","country":"United States","latitude":37.77493,"longitude":-122.41942,"timezone":"America/Los_Angeles"}]}"#;
        let transport = FakeHttpTransport::new().with_response(
            &geo_url,
            HttpResponse {
                status: 200,
                final_url: geo_url.clone(),
                body: body.to_vec(),
            },
        );
        let line = resolve_place_time(&transport, "San Francisco", fixed_utc(), "en")
            .await
            .unwrap();
        assert!(
            line.contains("San Francisco (America/Los_Angeles)"),
            "{line}"
        );
        assert!(line.contains("06:42"), "{line}");
    }

    #[tokio::test]
    async fn resolve_place_time_none_on_geocode_miss() {
        // "SF" does not geocode against the real Open-Meteo API (verified
        // live); the fake transport has no canned response either, so the
        // transport error surfaces the same fallback path.
        let transport = FakeHttpTransport::new();
        assert!(resolve_place_time(&transport, "SF", fixed_utc(), "en")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn resolve_place_time_none_on_empty_geocode_results() {
        let geo_url = geocode_request("Nowhereville", "en").url;
        let transport = FakeHttpTransport::new().with_response(
            &geo_url,
            HttpResponse {
                status: 200,
                final_url: geo_url.clone(),
                body: br#"{"generationtime_ms":0.1}"#.to_vec(),
            },
        );
        assert!(
            resolve_place_time(&transport, "Nowhereville", fixed_utc(), "en")
                .await
                .is_none()
        );
    }
}
