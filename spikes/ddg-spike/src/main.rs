//! T1 spike: durability probe for DuckDuckGo's keyless HTML endpoints.
//!
//! Throwaway, non-production. Sends one query to `html.duckduckgo.com/html/`
//! and/or `lite.duckduckgo.com/lite/` with ddgs-style browser headers,
//! classifies the outcome (ok / rate-limited / CAPTCHA / empty / error),
//! prints a human summary, and appends a JSONL record to a log file for the
//! week-long durability analysis mandated by the search-revamp design doc
//! ("The Assignment").
//!
//! Usage:
//!   ddg-spike [--endpoint html|lite|both] [--network LABEL] [--locale KL]
//!             [--log PATH] <query words...>
//!
//! `--network` is a free-form label for the current network condition
//! (home / vpn / cafe / corporate). `--locale` is a DDG `kl` region code
//! such as `us-en` or `fr-fr`. Defaults: both endpoints, network
//! "unlabeled", locale "us-en", log file `ddg-spike-log.jsonl` in the
//! current directory.

use std::fs::OpenOptions;
use std::io::Write;
use std::process::ExitCode;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use scraper::{Html, Selector};
use serde_json::json;

/// Browser-equivalent headers, mirroring what the `ddgs` project sends.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const ACCEPT: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8";
const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
const REFERER: &str = "https://duckduckgo.com/";

/// Body substrings that identify a bot-detection / CAPTCHA interstitial
/// rather than a results page. Matched case-insensitively.
const CAPTCHA_MARKERS: &[&str] = &[
    "anomaly-modal",
    "challenge-form",
    "cf-challenge",
    "hcaptcha",
    "recaptcha",
];

/// One probed endpoint: name for the log, URL, and CSS selector for result links.
struct Endpoint {
    name: &'static str,
    url: &'static str,
    link_selector: &'static str,
}

const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        name: "html",
        url: "https://html.duckduckgo.com/html/",
        link_selector: "a.result__a",
    },
    Endpoint {
        name: "lite",
        url: "https://lite.duckduckgo.com/lite/",
        link_selector: "a.result-link",
    },
];

/// Parsed CLI arguments.
struct Args {
    endpoint: String,
    network: String,
    locale: String,
    log_path: String,
    query: String,
}

/// Outcome classification for one endpoint probe.
struct Probe {
    status: &'static str,
    http_status: Option<u16>,
    latency_ms: u128,
    results: Vec<(String, String)>,
    error: Option<String>,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to build HTTP client: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut any_failure = false;
    for ep in ENDPOINTS {
        if args.endpoint != "both" && args.endpoint != ep.name {
            continue;
        }
        let probe = run_probe(&client, ep, &args);
        any_failure |= probe.status != "ok";
        print_summary(ep.name, &probe);
        if let Err(e) = append_log(&args, ep.name, &probe) {
            eprintln!("warning: could not write log {}: {e}", args.log_path);
        }
    }

    // Non-zero exit on any failure so shell wrappers can count bad days.
    if any_failure {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Parses flags and the trailing query words. Errors with usage text on
/// missing query or unknown/valueless flags.
fn parse_args() -> Result<Args, String> {
    let usage = "usage: ddg-spike [--endpoint html|lite|both] [--network LABEL] \
                 [--locale KL] [--log PATH] <query words...>";
    let mut endpoint = "both".to_string();
    let mut network = "unlabeled".to_string();
    let mut locale = "us-en".to_string();
    let mut log_path = "ddg-spike-log.jsonl".to_string();
    let mut query_words: Vec<String> = Vec::new();

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        let target = match arg.as_str() {
            "--endpoint" => &mut endpoint,
            "--network" => &mut network,
            "--locale" => &mut locale,
            "--log" => &mut log_path,
            _ if arg.starts_with("--") => return Err(format!("unknown flag {arg}\n{usage}")),
            _ => {
                query_words.push(arg);
                continue;
            }
        };
        *target = argv.next().ok_or_else(|| format!("{arg} needs a value\n{usage}"))?;
    }

    if !["html", "lite", "both"].contains(&endpoint.as_str()) {
        return Err(format!("--endpoint must be html, lite, or both\n{usage}"));
    }
    if query_words.is_empty() {
        return Err(usage.to_string());
    }
    Ok(Args {
        endpoint,
        network,
        locale,
        log_path,
        query: query_words.join(" "),
    })
}

/// Sends the ddgs-style POST to one endpoint and classifies the response.
fn run_probe(client: &reqwest::blocking::Client, ep: &Endpoint, args: &Args) -> Probe {
    let started = Instant::now();
    let response = client
        .post(ep.url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", ACCEPT)
        .header("Accept-Language", ACCEPT_LANGUAGE)
        .header("Referer", REFERER)
        .form(&[("q", args.query.as_str()), ("kl", args.locale.as_str()), ("b", "")])
        .send();

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            return Probe {
                status: "transport_error",
                http_status: None,
                latency_ms: started.elapsed().as_millis(),
                results: Vec::new(),
                error: Some(e.to_string()),
            }
        }
    };

    let http_status = response.status().as_u16();
    let body = response.text().unwrap_or_default();
    let latency_ms = started.elapsed().as_millis();

    let status = classify(http_status, &body, ep);
    let results = if status == "ok" {
        extract_results(&body, ep)
    } else {
        Vec::new()
    };

    Probe {
        status,
        http_status: Some(http_status),
        latency_ms,
        results,
        error: None,
    }
}

/// Maps HTTP status + body content to the log's outcome taxonomy.
fn classify(http_status: u16, body: &str, ep: &Endpoint) -> &'static str {
    let lower = body.to_lowercase();
    if CAPTCHA_MARKERS.iter().any(|m| lower.contains(m)) {
        return "captcha";
    }
    match http_status {
        429 | 418 | 403 => "rate_limited",
        200 => {
            if extract_results(body, ep).is_empty() {
                "empty"
            } else {
                "ok"
            }
        }
        _ => "http_error",
    }
}

/// Pulls (title, resolved URL) pairs out of a results page. DDG wraps hrefs
/// in a `//duckduckgo.com/l/?uddg=<encoded>` redirect; the real URL is the
/// decoded `uddg` query parameter.
fn extract_results(body: &str, ep: &Endpoint) -> Vec<(String, String)> {
    let selector = Selector::parse(ep.link_selector).expect("selector is a compile-time constant");
    Html::parse_document(body)
        .select(&selector)
        .filter_map(|a| {
            let title = a.text().collect::<String>().trim().to_string();
            let href = a.value().attr("href")?;
            let url = resolve_redirect(href);
            if title.is_empty() || url.is_empty() {
                None
            } else {
                Some((title, url))
            }
        })
        .collect()
}

/// Decodes DDG's `uddg` redirect wrapper; passes direct URLs through as-is.
fn resolve_redirect(href: &str) -> String {
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    };
    if let Ok(parsed) = url::Url::parse(&absolute) {
        if parsed.path() == "/l/" {
            if let Some((_, real)) = parsed.query_pairs().find(|(k, _)| k == "uddg") {
                return real.into_owned();
            }
        }
    }
    absolute
}

/// Human-readable one-probe summary on stdout.
fn print_summary(endpoint: &str, probe: &Probe) {
    println!(
        "[{endpoint}] {} http={} {}ms results={}",
        probe.status,
        probe.http_status.map_or("-".to_string(), |s| s.to_string()),
        probe.latency_ms,
        probe.results.len()
    );
    for (title, url) in probe.results.iter().take(3) {
        println!("    {title} -> {url}");
    }
    if let Some(e) = &probe.error {
        println!("    error: {e}");
    }
}

/// Appends one JSONL record for the week-long analysis.
fn append_log(args: &Args, endpoint: &str, probe: &Probe) -> std::io::Result<()> {
    let ts_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let record = json!({
        "ts_unix": ts_unix,
        "endpoint": endpoint,
        "query": args.query,
        "network": args.network,
        "locale": args.locale,
        "status": probe.status,
        "http_status": probe.http_status,
        "latency_ms": probe.latency_ms,
        "result_count": probe.results.len(),
        "top_results": probe.results.iter().take(3)
            .map(|(t, u)| json!({"title": t, "url": u}))
            .collect::<Vec<_>>(),
        "error": probe.error,
    });
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args.log_path)?;
    writeln!(file, "{record}")
}
