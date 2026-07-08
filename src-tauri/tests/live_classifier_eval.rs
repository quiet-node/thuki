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
use thuki_agent_lib::websearch::prepass::{BuiltinPrePass, PrePass, SearchDecision};

use tokio_util::sync::CancellationToken;

/// One labelled corpus row.
#[derive(serde::Deserialize)]
struct EvalRow {
    message: String,
    label: String,
    category: String,
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
    assert!(
        accuracy >= 0.8,
        "two-stage decision accuracy {accuracy:.2} below the 0.80 floor; see misses above"
    );
}
