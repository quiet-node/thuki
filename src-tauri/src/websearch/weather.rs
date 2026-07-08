//! Weather vertical: Open-Meteo, the first intent-routed keyless API source.
//!
//! Weather questions are the worst fit for SERP scraping (the answer lives in
//! widgets, not article text) and the best fit for an official API: Open-Meteo
//! is keyless, ToS-clean for a free non-commercial app (with the required
//! CC BY 4.0 attribution shipped in the source block), returns structured JSON,
//! and cannot bot-block the way scraped engines do. When a turn's standalone
//! question is recognisably a weather question with an extractable location,
//! this vertical answers it directly and the scraped-engine tier is skipped
//! entirely.
//!
//! The flow is geocode → forecast → format: the location text is resolved to
//! coordinates via Open-Meteo's geocoding API, the forecast fetched for those
//! coordinates, and the result formatted into a single readable [`SourceBlock`]
//! the writer cites as `[1]`. Every step degrades by returning `None`, which
//! sends the turn down the normal engine path instead: the vertical can only
//! ever improve a turn, never lose one.

use crate::net::transport::{HttpMethod, HttpRequest, HttpTransport};
use crate::websearch::assemble::SourceBlock;

/// Words that signal a weather question. Matched on whole tokens of the
/// lowercased standalone question.
const WEATHER_WORDS: &[&str] = &[
    "weather",
    "forecast",
    "temperature",
    "rain",
    "raining",
    "snow",
    "snowing",
    "sunny",
    "cloudy",
    "windy",
    "humidity",
    "humid",
];

/// Tokens stripped from the question when isolating the location text: the
/// weather words themselves plus interrogative/time/filler words that commonly
/// surround them ("what's the weather like in Tokyo tomorrow").
const NON_LOCATION_WORDS: &[&str] = &[
    "weather",
    "forecast",
    "temperature",
    "rain",
    "raining",
    "snow",
    "snowing",
    "sunny",
    "cloudy",
    "windy",
    "humidity",
    "humid",
    "what",
    "whats",
    "is",
    "the",
    "like",
    "in",
    "at",
    "for",
    "of",
    "a",
    "an",
    "it",
    "will",
    "be",
    "going",
    "to",
    "today",
    "tonight",
    "tomorrow",
    "now",
    "right",
    "currently",
    "current",
    "this",
    "week",
    "weekend",
    "how",
    "hows",
    "hot",
    "cold",
    "warm",
    "out",
    "outside",
    "and",
    "there",
    "here",
    // Contraction fragments left over by the alphanumeric tokenizer splitting
    // "what's" / "it'll" / "isn't" on the apostrophe.
    "s",
    "t",
    "m",
    "d",
    "ll",
    "re",
    "ve",
];

/// Endpoints for Open-Meteo's keyless APIs.
const GEOCODE_ENDPOINT: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";

/// The attribution line required by Open-Meteo's CC BY 4.0 licence, appended to
/// every weather source block.
const ATTRIBUTION: &str = "Weather data by Open-Meteo.com (CC BY 4.0)";

/// A geocoded place: the display name and the coordinates the forecast needs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GeoPlace {
    pub(crate) name: String,
    pub(crate) country: String,
    pub(crate) latitude: f64,
    pub(crate) longitude: f64,
}

/// Extracts the location text from a weather question, or `None` when the
/// question is not about weather or carries no location to geocode. The
/// remaining words after stripping weather/filler tokens are taken verbatim
/// (original casing) as the geocoding query; a question like "will it rain
/// today" leaves nothing and correctly falls through to the engine tier.
pub(crate) fn weather_location(question: &str) -> Option<String> {
    let lower = question.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if !tokens.iter().any(|t| WEATHER_WORDS.contains(t)) {
        return None;
    }
    // Walk the ORIGINAL text so the location keeps its casing ("New York"),
    // keeping only tokens that are not weather/filler words.
    let location: Vec<&str> = question
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .filter(|t| !NON_LOCATION_WORDS.contains(&t.to_lowercase().as_str()))
        .collect();
    if location.is_empty() {
        return None;
    }
    Some(location.join(" "))
}

/// Builds the geocoding GET request resolving `location` to coordinates.
pub(crate) fn geocode_request(location: &str) -> HttpRequest {
    // GEOCODE_ENDPOINT is a compile-time-valid absolute URL.
    let mut url = url::Url::parse(GEOCODE_ENDPOINT).expect("static endpoint");
    url.query_pairs_mut()
        .append_pair("name", location)
        .append_pair("count", "1")
        .append_pair("language", "en")
        .append_pair("format", "json");
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: Vec::new(),
        form: Vec::new(),
    }
}

/// Parses a geocoding response into the top place, or `None` when the location
/// did not resolve (Open-Meteo omits `results` entirely on no match).
pub(crate) fn parse_geocode(body: &str) -> Option<GeoPlace> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let top = json.get("results")?.get(0)?;
    Some(GeoPlace {
        name: top.get("name")?.as_str()?.to_string(),
        country: top
            .get("country")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string(),
        latitude: top.get("latitude")?.as_f64()?,
        longitude: top.get("longitude")?.as_f64()?,
    })
}

/// Builds the forecast GET request for `place`: current conditions plus a
/// 3-day daily outlook, in the place's local timezone.
pub(crate) fn forecast_request(place: &GeoPlace) -> HttpRequest {
    // FORECAST_ENDPOINT is a compile-time-valid absolute URL.
    let mut url = url::Url::parse(FORECAST_ENDPOINT).expect("static endpoint");
    url.query_pairs_mut()
        .append_pair("latitude", &place.latitude.to_string())
        .append_pair("longitude", &place.longitude.to_string())
        .append_pair(
            "current",
            "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m",
        )
        .append_pair(
            "daily",
            "temperature_2m_max,temperature_2m_min,precipitation_probability_max,weather_code",
        )
        .append_pair("timezone", "auto")
        .append_pair("forecast_days", "3");
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: Vec::new(),
        form: Vec::new(),
    }
}

/// Maps a WMO weather code to a short human-readable condition.
pub(crate) fn weather_code_text(code: i64) -> &'static str {
    match code {
        0 => "clear sky",
        1 => "mainly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "fog",
        51..=57 => "drizzle",
        61..=67 => "rain",
        71..=77 => "snow",
        80..=82 => "rain showers",
        85 | 86 => "snow showers",
        95..=99 => "thunderstorm",
        _ => "mixed conditions",
    }
}

/// Formats a forecast response into the human-readable weather report the
/// writer cites, or `None` when the body is not the expected shape. The report
/// carries current conditions, a 3-day outlook, and the CC BY attribution.
pub(crate) fn format_forecast(body: &str, place: &GeoPlace) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let current = json.get("current")?;
    let temp = current.get("temperature_2m")?.as_f64()?;
    let feels = current.get("apparent_temperature")?.as_f64()?;
    let humidity = current.get("relative_humidity_2m")?.as_f64()?;
    let wind = current.get("wind_speed_10m")?.as_f64()?;
    let condition = weather_code_text(current.get("weather_code")?.as_i64()?);

    let mut out = format!(
        "Current weather in {}, {}: {condition}, {temp:.0}°C (feels like {feels:.0}°C), \
         humidity {humidity:.0}%, wind {wind:.0} km/h.",
        place.name, place.country,
    );

    // The 3-day outlook is additive: a malformed daily section degrades to a
    // current-conditions-only report rather than dropping the whole vertical.
    if let Some(daily) = json.get("daily") {
        let days = daily.get("time").and_then(|t| t.as_array());
        let maxes = daily.get("temperature_2m_max").and_then(|t| t.as_array());
        let mins = daily.get("temperature_2m_min").and_then(|t| t.as_array());
        let rains = daily
            .get("precipitation_probability_max")
            .and_then(|t| t.as_array());
        let codes = daily.get("weather_code").and_then(|t| t.as_array());
        if let (Some(days), Some(maxes), Some(mins), Some(rains), Some(codes)) =
            (days, maxes, mins, rains, codes)
        {
            for (i, day_value) in days.iter().take(3).enumerate() {
                if let (Some(day), Some(max), Some(min), Some(rain), Some(code)) = (
                    day_value.as_str(),
                    maxes.get(i).and_then(|v| v.as_f64()),
                    mins.get(i).and_then(|v| v.as_f64()),
                    rains.get(i).and_then(|v| v.as_f64()),
                    codes.get(i).and_then(|v| v.as_i64()),
                ) {
                    out.push_str(&format!(
                        "\n{day}: {}, {min:.0}-{max:.0}°C, {rain:.0}% chance of precipitation.",
                        weather_code_text(code),
                    ));
                }
            }
        }
    }
    out.push_str(&format!("\n{ATTRIBUTION}."));
    Some(out)
}

/// Wraps a formatted weather report into the single `[1]` source block the
/// writer cites.
pub(crate) fn weather_source_block(report: String, place: &GeoPlace) -> SourceBlock {
    SourceBlock {
        index: 1,
        url: "https://open-meteo.com/".to_string(),
        title: format!("Weather for {}, {}", place.name, place.country),
        text: report,
    }
}

/// Runs the full weather vertical for `standalone_question`: intent + location
/// check, geocode, forecast, format. Returns `None` on any miss so the caller
/// falls through to the scraped-engine tier.
///
/// Coverage-excluded: thin async glue over the injectable transport delegating
/// every decision to the pure helpers above, which are all tested directly;
/// the glue itself is still exercised against
/// [`crate::net::transport::FakeHttpTransport`] in the tests below.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) async fn fetch_weather(
    transport: &dyn HttpTransport,
    standalone_question: &str,
) -> Option<SourceBlock> {
    let location = weather_location(standalone_question)?;
    let geo_response = transport.send(&geocode_request(&location)).await.ok()?;
    let place = parse_geocode(&String::from_utf8_lossy(&geo_response.body))?;
    let forecast_response = transport.send(&forecast_request(&place)).await.ok()?;
    let report = format_forecast(&String::from_utf8_lossy(&forecast_response.body), &place)?;
    eprintln!("[search] vertical=weather place={}", place.name);
    Some(weather_source_block(report, &place))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{FakeHttpTransport, HttpResponse};

    /// Real Open-Meteo geocoding response shape (captured live 2026-07-08).
    const GEOCODE_FIXTURE: &str = r#"{"results":[{"id":1850147,"name":"Tokyo","latitude":35.6895,"longitude":139.69171,"country_code":"JP","timezone":"Asia/Tokyo","country":"Japan","admin1":"Tokyo"}],"generationtime_ms":2.16}"#;

    /// Real Open-Meteo forecast response shape (captured live 2026-07-08).
    const FORECAST_FIXTURE: &str = r#"{"latitude":35.7,"longitude":139.6875,"timezone":"Asia/Tokyo","current_units":{"temperature_2m":"°C"},"current":{"time":"2026-07-08T10:00","temperature_2m":25.5,"relative_humidity_2m":61,"apparent_temperature":27.9,"weather_code":1,"wind_speed_10m":2.6},"daily":{"time":["2026-07-08","2026-07-09","2026-07-10"],"temperature_2m_max":[28.3,29.6,30.3],"temperature_2m_min":[20.5,21.4,21.6],"precipitation_probability_max":[0,10,80],"weather_code":[3,1,61]}}"#;

    fn tokyo() -> GeoPlace {
        GeoPlace {
            name: "Tokyo".into(),
            country: "Japan".into(),
            latitude: 35.6895,
            longitude: 139.69171,
        }
    }

    // ── weather_location ─────────────────────────────────────────────────────

    #[test]
    fn location_extracted_from_weather_questions() {
        assert_eq!(
            weather_location("weather in Tokyo").as_deref(),
            Some("Tokyo")
        );
        assert_eq!(
            weather_location("what's the weather like in New York tomorrow").as_deref(),
            Some("New York")
        );
        assert_eq!(
            weather_location("is it going to rain in London this weekend").as_deref(),
            Some("London")
        );
        assert_eq!(
            weather_location("temperature in Ho Chi Minh City right now").as_deref(),
            Some("Ho Chi Minh City")
        );
    }

    #[test]
    fn non_weather_questions_yield_none() {
        assert_eq!(weather_location("latest rust version"), None);
        assert_eq!(weather_location("who won the F1 race"), None);
    }

    #[test]
    fn weather_question_without_location_yields_none() {
        // No location to geocode: falls through to the engine tier.
        assert_eq!(weather_location("will it rain today"), None);
        assert_eq!(weather_location("what's the weather like"), None);
    }

    // ── request builders ─────────────────────────────────────────────────────

    #[test]
    fn geocode_request_carries_location_and_single_result() {
        let req = geocode_request("New York");
        assert_eq!(req.method, HttpMethod::Get);
        assert!(req.url.starts_with(GEOCODE_ENDPOINT));
        assert!(req.url.contains("name=New+York"));
        assert!(req.url.contains("count=1"));
    }

    #[test]
    fn forecast_request_carries_coordinates_and_fields() {
        let req = forecast_request(&tokyo());
        assert!(req.url.starts_with(FORECAST_ENDPOINT));
        assert!(req.url.contains("latitude=35.6895"));
        assert!(req.url.contains("temperature_2m"));
        assert!(req.url.contains("forecast_days=3"));
    }

    // ── parsers / formatting ─────────────────────────────────────────────────

    #[test]
    fn parse_geocode_reads_top_place() {
        let place = parse_geocode(GEOCODE_FIXTURE).unwrap();
        assert_eq!(place, tokyo());
    }

    #[test]
    fn parse_geocode_none_on_no_results_or_junk() {
        // Open-Meteo omits `results` entirely on no match.
        assert!(parse_geocode(r#"{"generationtime_ms":0.86}"#).is_none());
        assert!(parse_geocode("not json").is_none());
    }

    #[test]
    fn format_forecast_reports_current_and_outlook() {
        let report = format_forecast(FORECAST_FIXTURE, &tokyo()).unwrap();
        assert!(report.contains("Current weather in Tokyo, Japan"));
        assert!(report.contains("mainly clear, 26°C (feels like 28°C)"));
        assert!(report.contains("humidity 61%"));
        assert!(report.contains("2026-07-10: rain, 22-30°C, 80% chance of precipitation."));
        assert!(report.contains("Open-Meteo.com (CC BY 4.0)"));
    }

    #[test]
    fn format_forecast_degrades_to_current_only_without_daily() {
        let body = r#"{"current":{"temperature_2m":10.0,"relative_humidity_2m":50,"apparent_temperature":8.0,"weather_code":0,"wind_speed_10m":5.0}}"#;
        let report = format_forecast(body, &tokyo()).unwrap();
        assert!(report.contains("clear sky, 10°C"));
        assert!(!report.contains("2026-"));
        assert!(report.contains("CC BY 4.0"));
    }

    #[test]
    fn format_forecast_skips_malformed_daily_arrays() {
        // `daily` present but missing its arrays: outlook is skipped, current
        // conditions still reported.
        let body = r#"{"current":{"temperature_2m":10.0,"relative_humidity_2m":50,"apparent_temperature":8.0,"weather_code":0,"wind_speed_10m":5.0},"daily":{"time":["2026-07-08"]}}"#;
        let report = format_forecast(body, &tokyo()).unwrap();
        assert!(report.contains("clear sky, 10°C"));
        assert!(!report.contains("2026-07-08:"));
    }

    #[test]
    fn format_forecast_none_on_missing_current() {
        assert!(format_forecast(r#"{"daily":{}}"#, &tokyo()).is_none());
        assert!(format_forecast("junk", &tokyo()).is_none());
    }

    #[test]
    fn weather_codes_map_to_conditions() {
        assert_eq!(weather_code_text(0), "clear sky");
        assert_eq!(weather_code_text(1), "mainly clear");
        assert_eq!(weather_code_text(2), "partly cloudy");
        assert_eq!(weather_code_text(3), "overcast");
        assert_eq!(weather_code_text(48), "fog");
        assert_eq!(weather_code_text(55), "drizzle");
        assert_eq!(weather_code_text(65), "rain");
        assert_eq!(weather_code_text(73), "snow");
        assert_eq!(weather_code_text(81), "rain showers");
        assert_eq!(weather_code_text(86), "snow showers");
        assert_eq!(weather_code_text(96), "thunderstorm");
        assert_eq!(weather_code_text(42), "mixed conditions");
    }

    // ── fetch_weather over the fake transport ────────────────────────────────

    #[tokio::test]
    async fn fetch_weather_resolves_full_chain() {
        let geo_url = geocode_request("Tokyo").url;
        let fc_url = forecast_request(&tokyo()).url;
        let transport = FakeHttpTransport::new()
            .with_response(
                &geo_url,
                HttpResponse {
                    status: 200,
                    final_url: geo_url.clone(),
                    body: GEOCODE_FIXTURE.as_bytes().to_vec(),
                },
            )
            .with_response(
                &fc_url,
                HttpResponse {
                    status: 200,
                    final_url: fc_url.clone(),
                    body: FORECAST_FIXTURE.as_bytes().to_vec(),
                },
            );
        let block = fetch_weather(&transport, "weather in Tokyo").await.unwrap();
        assert_eq!(block.index, 1);
        assert_eq!(block.title, "Weather for Tokyo, Japan");
        assert!(block.text.contains("Current weather in Tokyo"));
    }

    #[tokio::test]
    async fn fetch_weather_none_when_not_weather_or_geocode_fails() {
        let transport = FakeHttpTransport::new();
        // Not a weather question: no request is even sent.
        assert!(fetch_weather(&transport, "latest rust version")
            .await
            .is_none());
        assert!(transport.calls().is_empty());
        // Weather question but geocode transport error: falls through.
        assert!(fetch_weather(&transport, "weather in Tokyo")
            .await
            .is_none());
    }
}
