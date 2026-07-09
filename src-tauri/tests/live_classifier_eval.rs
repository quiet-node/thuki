//! Live decision-quality eval for the two-stage search trigger.
//!
//! Runs the production stage-one pre-filter plus, for the ambiguous middle, the
//! production [`BuiltinPrePass`] classifier against a REAL llama-server, over
//! the curated should-search / should-not-search corpus committed at
//! `src/websearch/search_decision_eval.jsonl`. This measures the one thing unit
//! tests structurally cannot: the live model's decision quality.
//!
//! `#[ignore]`d so no CI or coverage gate ever needs a live engine. Run against
//! an already-running engine (the app's own sidecar works):
//!
//! ```sh
//! THUKI_EVAL_PORT=<port> cargo test --test live_classifier_eval -- --ignored --nocapture
//! ```

use thuki_agent_lib::websearch::prefilter::{prefilter, PreFilterVerdict};
use thuki_agent_lib::websearch::prepass::{BuiltinPrePass, PrePass, SearchDecision, SearchRoute};

use tokio_util::sync::CancellationToken;

/// One labelled corpus row. `route` is the optional expected retrieval tier,
/// present only on rows where the tier is unambiguous.
#[derive(serde::Deserialize)]
struct EvalRow {
    message: String,
    label: String,
    category: String,
    #[serde(default)]
    route: Option<String>,
}

/// Lowercase label for a classifier route, matching the corpus `route` field.
fn route_label(route: SearchRoute) -> &'static str {
    match route {
        SearchRoute::Weather => "weather",
        SearchRoute::News => "news",
        SearchRoute::Wiki => "wiki",
        SearchRoute::Web => "web",
    }
}

/// The production two-stage decision, collapsed to "would this turn search?".
/// Mirrors `run_search`: ForceNo answers directly, ForceWeb searches, and the
/// ambiguous middle follows the live classifier (`cached` counts as search).
async fn would_search(
    prepass: &BuiltinPrePass,
    message: &str,
    today: &str,
) -> (bool, &'static str) {
    match prefilter(message, today) {
        PreFilterVerdict::ForceNo => (false, "prefilter"),
        PreFilterVerdict::ForceWeb => (true, "prefilter"),
        PreFilterVerdict::Ambiguous => {
            // A failed call mirrors production's ambiguous-turn fallback: no
            // search. It still counts against accuracy, so timeouts show up in
            // the numbers instead of aborting the run.
            match prepass
                .decide(&[], message, today, &CancellationToken::new())
                .await
            {
                Ok(decision) => (
                    !matches!(decision.decision, SearchDecision::No),
                    "classifier",
                ),
                Err(e) => {
                    eprintln!("[eval] classifier error on {message:?}: {e}");
                    (false, "classifier-error")
                }
            }
        }
    }
}

#[tokio::test]
#[ignore = "needs a live llama-server; set THUKI_EVAL_PORT"]
async fn live_two_stage_decision_quality_on_eval_corpus() {
    let port = std::env::var("THUKI_EVAL_PORT")
        .expect("set THUKI_EVAL_PORT to a running llama-server port");
    let prepass = BuiltinPrePass::new(
        reqwest::Client::new(),
        format!("http://127.0.0.1:{port}"),
        "eval".to_string(),
        thuki_agent_lib::config::defaults::PREPASS_TIMEOUT_S,
    );
    let today = "2026-07-08";

    let rows: Vec<EvalRow> = include_str!("../src/websearch/search_decision_eval.jsonl")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("valid corpus row"))
        .collect();

    let mut correct = 0usize;
    let mut classifier_calls = 0usize;
    let mut misses: Vec<String> = Vec::new();
    for row in &rows {
        let want_search = row.label == "search";
        let (got_search, stage) = would_search(&prepass, &row.message, today).await;
        if stage == "classifier" {
            classifier_calls += 1;
        }
        if got_search == want_search {
            correct += 1;
        } else {
            misses.push(format!(
                "  MISS [{}] ({}, via {}): want {} got {} :: {}",
                row.label,
                row.category,
                stage,
                if want_search { "search" } else { "no" },
                if got_search { "search" } else { "no" },
                row.message
            ));
        }
    }

    let total = rows.len();
    let accuracy = correct as f64 / total as f64;
    eprintln!("[eval] corpus={total} correct={correct} accuracy={accuracy:.2} classifier_calls={classifier_calls}");
    for m in &misses {
        eprintln!("{m}");
    }

    // Route accuracy (report-only): for every row carrying an expected route,
    // ask the classifier directly and compare its route hint to the label. This
    // measures the routing quality the deterministic pre-filter cannot, and is
    // never a CI gate (the whole test is `#[ignore]`d).
    let routed: Vec<&EvalRow> = rows.iter().filter(|r| r.route.is_some()).collect();
    let mut route_correct = 0usize;
    let mut route_misses: Vec<String> = Vec::new();
    for row in &routed {
        let want = row.route.as_deref().unwrap();
        let got = match prepass
            .decide(&[], &row.message, today, &CancellationToken::new())
            .await
        {
            Ok(decision) => route_label(decision.route).to_string(),
            Err(e) => {
                eprintln!("[eval] route classifier error on {:?}: {e}", row.message);
                "<error>".to_string()
            }
        };
        if got == want {
            route_correct += 1;
        } else {
            route_misses.push(format!(
                "  ROUTE MISS ({}): want {} got {} :: {}",
                row.category, want, got, row.message
            ));
        }
    }
    let route_total = routed.len();
    let route_accuracy = route_correct as f64 / route_total as f64;
    eprintln!(
        "[eval] route_labelled={route_total} route_correct={route_correct} route_accuracy={route_accuracy:.2}"
    );
    for m in &route_misses {
        eprintln!("{m}");
    }

    assert!(
        accuracy >= 0.8,
        "two-stage decision accuracy {accuracy:.2} below the 0.80 floor; see misses above"
    );
}
