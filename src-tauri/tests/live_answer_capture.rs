//! Live answer-capture harness: dev-time tooling for building comparable
//! answer sets over the eval corpus, ahead of pairwise LLM-as-judge scoring
//! (see `docs/search-eval.md`).
//!
//! Modeled closely on `live_search_smoke.rs`: the same `ScriptedPrePass` /
//! `AlwaysSufficientJudge` / `SearchDeps` construction, duplicated here rather
//! than shared, since integration test binaries do not share code across
//! files without a pipeline-level change. This harness runs the REAL
//! `run_search` orchestrator against a small, hardcoded, representative slice
//! of the eval corpus (`src/websearch/search_decision_eval.jsonl`), spanning
//! all four FreshQA-style volatility categories (`never`, `slow`, `fast`,
//! `false-premise`), and appends one JSON line per question to a run file at
//! `target/eval/answers-<unix_ts>.jsonl`.
//!
//! It captures *what the pipeline answers*, not whether it decided to search
//! (the classifier's decision quality is `live_classifier_eval.rs`'s job).
//! `ScriptedPrePass` therefore always forces the `Web`/vertical decision here,
//! exactly as `live_search_smoke.rs` does, so every question is driven
//! through real retrieval regardless of its corpus `label`.
//!
//! `#[ignore]`d so no CI or coverage gate ever touches the network. Run
//! explicitly, single-threaded and human-paced:
//!
//! ```sh
//! cargo test --test live_answer_capture -- --ignored --nocapture --test-threads=1
//! ```

use thuki_agent_lib::commands::ChatMessage;
use thuki_agent_lib::net::transport::ReqwestTransport;
use thuki_agent_lib::trace::{BoundRecorder, ConversationId};
use thuki_agent_lib::websearch::assemble::SourceBlock;
use thuki_agent_lib::websearch::cache::TtlSourceCache;
use thuki_agent_lib::websearch::engine::EngineHealth;
use thuki_agent_lib::websearch::judge::{SufficiencyJudge, SufficiencyVerdict};
use thuki_agent_lib::websearch::orchestrator::{run_search, SearchDeps, SearchOutcome};
use thuki_agent_lib::websearch::prepass::{
    InferenceError, PrePass, PrePassDecision, SearchDecision, SearchRoute,
};
use thuki_agent_lib::websearch::rank::Bm25Scorer;
use thuki_agent_lib::websearch::serp_cache::WebCache;

use async_trait::async_trait;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

/// A scripted classifier standing in for the live model: returns a fixed `web`
/// decision with the given route, rewrite, and queries. Identical to the
/// struct of the same name in `live_search_smoke.rs`; duplicated because
/// integration test binaries cannot import from one another.
struct ScriptedPrePass {
    route: SearchRoute,
    standalone: &'static str,
    queries: Vec<&'static str>,
}

#[async_trait]
impl PrePass for ScriptedPrePass {
    async fn decide(
        &self,
        _history: &[ChatMessage],
        _latest_user_message: &str,
        _today: &str,
        _cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError> {
        Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: self.route,
            standalone_question: self.standalone.to_string(),
            queries: self.queries.iter().map(|q| q.to_string()).collect(),
            explicit_search: false,
        })
    }
}

/// A scripted sufficiency judge standing in for the live model: always finds
/// the retrieved block sufficient, so every vertical hit commits rather than
/// escalating. Identical to `live_search_smoke.rs`'s `AlwaysSufficientJudge`.
struct AlwaysSufficientJudge;

#[async_trait]
impl SufficiencyJudge for AlwaysSufficientJudge {
    async fn judge(
        &self,
        _standalone_question: &str,
        _sources: &[SourceBlock],
        _cancel: &CancellationToken,
    ) -> Result<SufficiencyVerdict, InferenceError> {
        Ok(SufficiencyVerdict {
            sufficient: true,
            missing: String::new(),
        })
    }
}

/// Runs one live turn through the production pipeline and returns the outcome.
async fn live_turn(
    latest_user: &str,
    route: SearchRoute,
    standalone: &'static str,
    queries: Vec<&'static str>,
) -> SearchOutcome {
    let transport = ReqwestTransport::new().expect("transport builds");
    let prepass = ScriptedPrePass {
        route,
        standalone,
        queries,
    };
    let judge = AlwaysSufficientJudge;
    let health = EngineHealth::new();
    let recorder = BoundRecorder::noop_for(ConversationId::new("capture"));
    let cache = TtlSourceCache::new(std::time::Duration::from_secs(600));
    let web_cache = WebCache::new(
        std::time::Duration::from_secs(600),
        std::time::Duration::from_secs(600),
        64,
        128,
    );
    let deps = SearchDeps {
        prepass: &prepass,
        judge: &judge,
        transport: &transport,
        scorer: &Bm25Scorer,
        health: &health,
        recorder: &recorder,
        cache: &cache,
        cache_scope: 1,
        web_cache: &web_cache,
        local_zone: None,
    };
    run_search(
        &deps,
        "You are a helpful assistant.",
        &[],
        latest_user,
        16384,
        "2026-07-08",
        "en-US",
        &CancellationToken::new(),
        &|phase| eprintln!("[capture] phase={phase:?}"),
    )
    .await
}

/// One question driven through the harness: the corpus (or, for the one
/// `false-premise` probe, hand-authored) message, its volatility tag, and the
/// scripted classifier decision that routes it through retrieval.
struct CaptureQuestion {
    question: &'static str,
    volatility: &'static str,
    route: SearchRoute,
    standalone: &'static str,
    queries: Vec<&'static str>,
}

/// A bare url+title reference into a retrieved source, with no notion of
/// which vertical tier it came from ("tier-less" per the run-file contract).
#[derive(serde::Serialize)]
struct SourceRef {
    url: String,
    title: String,
}

/// One appended line of the run file.
#[derive(serde::Serialize)]
struct CaptureRecord {
    question: &'static str,
    volatility: &'static str,
    outcome_kind: &'static str,
    sources: Vec<SourceRef>,
    writer_user_turn: String,
}

/// Printed instead of a writer turn when retrieval produced no answer.
const UNREACHABLE_MARKER: &str = "<unreachable: retrieval produced no citable answer>";
/// Printed instead of a writer turn when the pipeline resolved no-search
/// (not expected here, since `ScriptedPrePass` always forces `Web`/vertical,
/// but handled rather than panicking so the harness never crashes mid-run).
const NOSEARCH_MARKER: &str = "<nosearch: pipeline resolved no-search>";

/// The 8-10 representative questions: mostly drawn verbatim from
/// `src/websearch/search_decision_eval.jsonl` (matching that row's message
/// and volatility tag), spanning all four volatility categories. The corpus
/// carries no `false-premise` row (it was built for search-routing, not
/// FreshQA-style adversarial coverage; see `docs/search-eval.md`), so the one
/// `false-premise` entry below is hand-authored and explicitly not
/// corpus-sourced.
fn capture_questions() -> Vec<CaptureQuestion> {
    vec![
        // never (corpus rows: "what is the capital of France", "who wrote
        // Pride and Prejudice")
        CaptureQuestion {
            question: "what is the capital of France",
            volatility: "never",
            route: SearchRoute::Wiki,
            standalone: "what is the capital of France",
            queries: vec!["capital of France"],
        },
        CaptureQuestion {
            question: "who wrote Pride and Prejudice",
            volatility: "never",
            route: SearchRoute::Wiki,
            standalone: "who wrote Pride and Prejudice",
            queries: vec!["Pride and Prejudice author"],
        },
        // slow (corpus rows: "who is the current CEO of Twitter", "what is
        // the population of France", "who is Tesla's CEO now")
        CaptureQuestion {
            question: "who is the current CEO of Twitter",
            volatility: "slow",
            route: SearchRoute::Web,
            standalone: "who is the current CEO of Twitter",
            queries: vec!["Twitter CEO"],
        },
        CaptureQuestion {
            question: "what is the population of France",
            volatility: "slow",
            route: SearchRoute::Wiki,
            standalone: "what is the population of France",
            queries: vec!["France population"],
        },
        CaptureQuestion {
            question: "who is Tesla's CEO now",
            volatility: "slow",
            route: SearchRoute::Web,
            standalone: "who is Tesla's CEO now",
            queries: vec!["Tesla CEO"],
        },
        // fast (corpus rows: "weather in Tokyo", "what's the current price of
        // Bitcoin", "who won the most recent F1 race", "current standings in
        // the NBA")
        CaptureQuestion {
            question: "weather in Tokyo",
            volatility: "fast",
            route: SearchRoute::Weather,
            standalone: "weather in Tokyo",
            queries: vec!["tokyo weather"],
        },
        CaptureQuestion {
            question: "what's the current price of Bitcoin",
            volatility: "fast",
            route: SearchRoute::Web,
            standalone: "current price of bitcoin",
            queries: vec!["bitcoin price usd"],
        },
        CaptureQuestion {
            question: "who won the most recent F1 race",
            volatility: "fast",
            route: SearchRoute::News,
            standalone: "who won the most recent F1 race",
            queries: vec!["f1 race winner"],
        },
        CaptureQuestion {
            question: "current standings in the NBA",
            volatility: "fast",
            route: SearchRoute::Sports,
            standalone: "current standings in the NBA",
            queries: vec!["nba standings"],
        },
        // false-premise: hand-authored, NOT corpus-sourced (the corpus has no
        // false-premise row today; see docs/search-eval.md). The premise is
        // false: Musk's Twitter acquisition closed in 2022, not 2015.
        CaptureQuestion {
            question: "why did Elon Musk buy Twitter in 2015",
            volatility: "false-premise",
            route: SearchRoute::Web,
            standalone: "why did Elon Musk buy Twitter in 2015",
            queries: vec!["Elon Musk Twitter acquisition date"],
        },
    ]
}

#[tokio::test]
#[ignore = "hits the live internet; run explicitly"]
async fn live_capture_answers_over_volatility_slice() {
    let run_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the epoch")
        .as_secs();
    let out_dir = "target/eval";
    fs::create_dir_all(out_dir).expect("create target/eval");
    let out_path = format!("{out_dir}/answers-{run_ts}.jsonl");
    let mut out_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&out_path)
        .unwrap_or_else(|e| panic!("open {out_path}: {e}"));

    for q in capture_questions() {
        eprintln!(
            "[capture] running: {:?} (volatility={})",
            q.question, q.volatility
        );
        let outcome = live_turn(q.question, q.route, q.standalone, q.queries).await;

        let record = match outcome {
            SearchOutcome::Answer { messages, sources } => {
                let writer_user_turn = messages
                    .last()
                    .map(|m| m.content.clone())
                    .unwrap_or_default();
                let sources = sources
                    .into_iter()
                    .map(|s| SourceRef {
                        url: s.url,
                        title: s.title,
                    })
                    .collect();
                CaptureRecord {
                    question: q.question,
                    volatility: q.volatility,
                    outcome_kind: "answer",
                    sources,
                    writer_user_turn,
                }
            }
            SearchOutcome::Unreachable { .. } => CaptureRecord {
                question: q.question,
                volatility: q.volatility,
                outcome_kind: "unreachable",
                sources: Vec::new(),
                writer_user_turn: UNREACHABLE_MARKER.to_string(),
            },
            SearchOutcome::NoSearch => CaptureRecord {
                question: q.question,
                volatility: q.volatility,
                outcome_kind: "nosearch",
                sources: Vec::new(),
                writer_user_turn: NOSEARCH_MARKER.to_string(),
            },
            SearchOutcome::Cancelled => {
                panic!(
                    "{:?}: unexpectedly cancelled (fresh token every call)",
                    q.question
                )
            }
        };

        let line = serde_json::to_string(&record).expect("serialize capture record");
        eprintln!(
            "[capture]   -> outcome={} sources={}",
            record.outcome_kind,
            record.sources.len()
        );
        writeln!(out_file, "{line}").expect("append capture record");
    }

    eprintln!("[capture] wrote run file: {out_path}");
}
