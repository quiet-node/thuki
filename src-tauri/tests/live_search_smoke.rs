//! Live end-to-end smoke of the web-search retrieval pipeline.
//!
//! These tests hit the REAL internet through the production transport and the
//! production `run_search` orchestrator, with only the model-backed classifier
//! stubbed (its decision quality is validated separately, in the app). They
//! answer the question unit tests cannot: do the verticals, engines, cooldown,
//! fetch, ranking, and assembly actually work against today's live endpoints?
//!
//! All tests are `#[ignore]`d so no CI or coverage gate ever touches the
//! network. Run explicitly, single-threaded and human-paced, with:
//!
//! ```sh
//! cargo test --test live_search_smoke -- --ignored --nocapture --test-threads=1
//! ```

use thuki_agent_lib::commands::ChatMessage;
use thuki_agent_lib::net::transport::ReqwestTransport;
use thuki_agent_lib::trace::{BoundRecorder, ConversationId};
use thuki_agent_lib::websearch::assemble::SourceBlock;
use thuki_agent_lib::websearch::cache::TtlSourceCache;
use thuki_agent_lib::websearch::engine::EngineHealth;
use thuki_agent_lib::websearch::judge::{
    InsufficiencyReason, SufficiencyJudge, SufficiencyVerdict,
};
use thuki_agent_lib::websearch::orchestrator::{run_search, SearchDeps, SearchOutcome};
use thuki_agent_lib::websearch::prepass::{
    InferenceError, PrePass, PrePassDecision, SearchDecision, SearchRoute,
};
use thuki_agent_lib::websearch::rank::Bm25Scorer;
use thuki_agent_lib::websearch::serp_cache::WebCache;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

/// A scripted classifier standing in for the live model: returns a fixed `web`
/// decision with the given route, rewrite, and queries.
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

/// A scripted sufficiency judge standing in for the live model. This harness
/// isolates the retrieval path with a stubbed classifier and hand-picked good
/// queries, so it likewise stubs the judge to always find the vertical answer
/// sufficient: every vertical hit commits, exactly as before the judge existed.
/// The judge's live decision quality, like the classifier's, is validated
/// in-app, not here (there is no resident model in this harness to back it).
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
            reason: InsufficiencyReason::Missing,
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
    let recorder = BoundRecorder::noop_for(ConversationId::new("smoke"));
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
        &|phase| eprintln!("[smoke] phase={phase:?}"),
    )
    .await
}

/// Asserts the outcome is a grounded answer and prints its source summary.
fn expect_answer(outcome: SearchOutcome, label: &str) {
    match outcome {
        SearchOutcome::Answer { messages, sources } => {
            eprintln!("[smoke] {label}: ANSWER with {} source(s)", sources.len());
            for s in &sources {
                eprintln!("[smoke]   [{}] {} ({})", s.index, s.title, s.url);
            }
            let last = messages.last().expect("writer messages non-empty");
            eprintln!(
                "[smoke] writer turn tail: ...{}",
                &last.content[last.content.len().saturating_sub(300)..]
            );
            assert!(!sources.is_empty(), "{label}: no sources");
        }
        SearchOutcome::Unreachable { .. } => {
            panic!("{label}: retrieval unreachable (all sources failed live)")
        }
        SearchOutcome::NoSearch => panic!("{label}: unexpectedly resolved NoSearch"),
        SearchOutcome::Cancelled => panic!("{label}: unexpectedly cancelled"),
    }
}

#[tokio::test]
#[ignore = "hits the live internet; run explicitly"]
async fn live_weather_tokyo_answers_via_open_meteo() {
    let outcome = live_turn(
        "weather in Tokyo",
        SearchRoute::Weather,
        "weather in Tokyo",
        vec!["tokyo weather"],
    )
    .await;
    if let SearchOutcome::Answer { sources, .. } = &outcome {
        assert_eq!(sources[0].url, "https://open-meteo.com/");
        assert!(sources[0].text.contains("Current weather in Tokyo"));
    }
    expect_answer(outcome, "weather-tokyo");
}

#[tokio::test]
#[ignore = "hits the live internet; run explicitly"]
async fn live_f1_winner_answers_via_news_headlines() {
    let outcome = live_turn(
        "who won the most recent F1 race",
        SearchRoute::News,
        "who won the most recent F1 race",
        vec!["f1 race winner"],
    )
    .await;
    if let SearchOutcome::Answer { sources, .. } = &outcome {
        assert_eq!(sources[0].url, "https://news.google.com/");
    }
    expect_answer(outcome, "f1-winner");
}

#[tokio::test]
#[ignore = "hits the live internet; run explicitly"]
async fn live_rust_version_answers_via_engines() {
    let outcome = live_turn(
        "what's the latest stable version of Rust?",
        SearchRoute::Web,
        "latest stable version of rust",
        vec!["latest stable rust version"],
    )
    .await;
    expect_answer(outcome, "rust-version");
}

#[tokio::test]
#[ignore = "hits the live internet; run explicitly"]
async fn live_bitcoin_price_answers_via_engines() {
    let outcome = live_turn(
        "what's the current price of Bitcoin",
        SearchRoute::Web,
        "current price of bitcoin",
        vec!["bitcoin price usd"],
    )
    .await;
    expect_answer(outcome, "bitcoin-price");
}

#[tokio::test]
#[ignore = "hits the live internet; run explicitly"]
async fn live_world_cup_scores_answers_via_espn_sports_vertical() {
    let outcome = live_turn(
        "what's the latest status of the World Cup 2026",
        SearchRoute::Sports,
        "what is the current status of the 2026 World Cup",
        vec!["world cup 2026 scores"],
    )
    .await;
    if let SearchOutcome::Answer { sources, .. } = &outcome {
        assert_eq!(sources[0].url, "https://www.espn.com/");
        assert!(sources[0].title.to_lowercase().contains("world cup"));
    }
    expect_answer(outcome, "world-cup-sports");
}

#[tokio::test]
#[ignore = "hits the live internet; run explicitly"]
async fn live_photosynthesis_answers_via_wikipedia() {
    let outcome = live_turn(
        "what is photosynthesis",
        SearchRoute::Wiki,
        "what is photosynthesis",
        vec!["photosynthesis"],
    )
    .await;
    if let SearchOutcome::Answer { sources, .. } = &outcome {
        assert_eq!(
            sources[0].url,
            "https://en.wikipedia.org/wiki/Photosynthesis"
        );
        assert!(sources[0].text.contains("Photosynthesis is"));
    }
    expect_answer(outcome, "photosynthesis-wiki");
}
