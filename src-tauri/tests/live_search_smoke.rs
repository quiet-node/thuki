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
use thuki_agent_lib::net::reachability::DnsReachability;
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
/// decision with the given route, rewrite, queries, and language.
struct ScriptedPrePass {
    route: SearchRoute,
    standalone: &'static str,
    queries: Vec<&'static str>,
    /// The `lang` the classifier would have named, per `prepass.rs`'s
    /// language-preservation instruction. Defaults to `"en"` for every
    /// existing English-only smoke test; the Vietnamese live smoke sets it
    /// explicitly so `resolve_lang` sees the same signal production would.
    lang: &'static str,
}

#[async_trait]
impl PrePass for ScriptedPrePass {
    async fn decide(
        &self,
        _history: &[ChatMessage],
        _latest_user_message: &str,
        _latest_images: Option<&[String]>,
        _today: &str,
        _cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError> {
        Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: self.route,
            standalone_question: self.standalone.to_string(),
            queries: self.queries.iter().map(|q| q.to_string()).collect(),
            explicit_search: false,
            lang: self.lang.to_string(),
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
            requery_queries: Vec::new(),
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
    live_turn_with_lang(latest_user, route, standalone, queries, "en", "en-US").await
}

/// [`live_turn`], with the classifier's `lang` judgement and the resolved user
/// `locale` threaded through explicitly, for a live smoke in a language other
/// than English (see `live_vietnamese_wiki_answers_via_vietnamese_wikipedia`).
async fn live_turn_with_lang(
    latest_user: &str,
    route: SearchRoute,
    standalone: &'static str,
    queries: Vec<&'static str>,
    lang: &'static str,
    locale: &str,
) -> SearchOutcome {
    let transport = ReqwestTransport::new().expect("transport builds");
    let prepass = ScriptedPrePass {
        route,
        standalone,
        queries,
        lang,
    };
    let judge = AlwaysSufficientJudge;
    let health = EngineHealth::new();
    let recorder = BoundRecorder::noop_for(ConversationId::new("smoke"));
    let cache = TtlSourceCache::new(
        std::time::Duration::from_secs(600),
        thuki_agent_lib::config::defaults::SEARCH_CACHE_MAX_ENTRIES,
    );
    let web_cache = WebCache::new(
        std::time::Duration::from_secs(600),
        std::time::Duration::from_secs(600),
        64,
        128,
    );
    let timings = thuki_agent_lib::websearch::stage_timing::TimingBag::new();
    let deps = SearchDeps {
        prepass: &prepass,
        judge: &judge,
        transport: &transport,
        reachability: &DnsReachability,
        scorer: &Bm25Scorer,
        health: &health,
        recorder: &recorder,
        cache: &cache,
        cache_scope: 1,
        web_cache: &web_cache,
        local_zone: None,
        force_search: false,
        latest_images: None,
        timings: &timings,
    };
    run_search(
        &deps,
        "You are a helpful assistant.",
        &[],
        latest_user,
        16384,
        "2026-07-08",
        locale,
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

#[tokio::test]
#[ignore = "hits the live internet; run explicitly"]
async fn live_vietnamese_wiki_answers_via_vietnamese_wikipedia() {
    // "quang hợp là gì" = "what is photosynthesis" in Vietnamese. The
    // language-parity research verified live that vi.wikipedia's `srsearch`
    // resolves this native-language query correctly (top hit: "Quang hợp",
    // 20265 hits) while the ENGLISH term "photosynthesis" against the SAME
    // vi edition resolves to the wrong article ("Thực vật" / "Plants").
    // Language and locale threading are therefore co-gated for this route:
    // this smoke exercises both at once, through the real `run_search`, with
    // only the classifier stubbed (its live decision quality is validated
    // separately, in `live_language_parity_eval`).
    let outcome = live_turn_with_lang(
        "quang hợp là gì",
        SearchRoute::Wiki,
        "quang hợp là gì",
        vec!["quang hợp"],
        "vi",
        "vi-VN",
    )
    .await;
    if let SearchOutcome::Answer { sources, .. } = &outcome {
        assert!(
            sources[0].url.starts_with("https://vi.wikipedia.org/"),
            "expected the Vietnamese Wikipedia edition, got {}",
            sources[0].url
        );
        // "Quang hợp" (photosynthesis) is the Vietnamese article's own title,
        // so its presence in the fetched text is proof the result is
        // genuinely Vietnamese-language, not an English article that merely
        // happened to resolve.
        assert!(sources[0].text.contains("Quang hợp"));
    }
    expect_answer(outcome, "vietnamese-wiki-photosynthesis");
}

#[tokio::test]
#[ignore = "hits the live internet; run explicitly"]
async fn live_vietnamese_gold_price_answers_via_engines() {
    // "giá vàng hôm nay bao nhiêu" = "what is the price of gold today", one of
    // the mandatory shared-diacritic rows (see `search_decision_eval.jsonl`):
    // it carries no character in U+1EA0-U+1EF9, so only the classifier's
    // `lang` field (not script detection) can name it Vietnamese. Routed
    // `web`/engines rather than `wiki`, so this exercises the DuckDuckGo/
    // Mojeek region + Accept-Language path, distinct from the wiki-edition
    // path the photosynthesis smoke above exercises.
    let outcome = live_turn_with_lang(
        "giá vàng hôm nay bao nhiêu",
        SearchRoute::Web,
        "giá vàng hôm nay bao nhiêu",
        vec!["giá vàng hôm nay"],
        "vi",
        "vi-VN",
    )
    .await;
    expect_answer(outcome, "vietnamese-gold-price");
}
