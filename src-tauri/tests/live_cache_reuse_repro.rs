//! Reproduction for the cache-reuse "derivable-age rejected" smoke failure.
//!
//! Smoke trace: turn 1 "How old is Elon Musk now?" answered "55 years old,
//! born June 28 1971"; turn 2 (identical question) took the `cached` reuse
//! path, the sufficiency judge ran (~2.2 s of real inference), returned
//! `missing = "Elon Musk's current age"`, and the turn escalated instead of
//! reusing. The derivation carve-out was present in the running binary yet the
//! judge still rejected.
//!
//! Two hypotheses:
//!   (a) the DATA path drops the birth-date text before the judge sees it
//!       (`bound_pages` truncation, `select_chunks`, or `assemble_context`).
//!   (b) the judge MODEL ignores the derivation clause in practice.
//!
//! This repro settles (a) deterministically, with NO model: it stores fixture
//! pages exactly as production does (`CachedSearch { pages, route: Web }`, so
//! the real `bound_pages` byte caps run at store), then drives the REAL
//! `run_search` reuse path for the follow-up and captures, via a judge that
//! records every source handed to it, the exact assembled text the judge sees.
//! If that text contains "June 28, 1971" / "born", the data path is sound and
//! the failure is (b); the live trace already shows the real judge rejecting
//! that same good data.
//!
//! `#[ignore]`d like every other live/repro test here, so no CI or coverage
//! gate runs it. Run explicitly:
//!
//! ```sh
//! cargo test --test live_cache_reuse_repro -- --ignored --nocapture --test-threads=1
//! ```

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use thuki_agent_lib::commands::ChatMessage;
use thuki_agent_lib::net::reachability::DnsReachability;
use thuki_agent_lib::net::transport::ReqwestTransport;
use thuki_agent_lib::trace::{BoundRecorder, ConversationId};
use thuki_agent_lib::websearch::assemble::{assemble_context, SourceBlock};
use thuki_agent_lib::websearch::cache::{CachedSearch, SourceCache, TtlSourceCache};
use thuki_agent_lib::websearch::engine::EngineHealth;
use thuki_agent_lib::websearch::fetch::FetchedPage;
use thuki_agent_lib::websearch::judge::{
    InsufficiencyReason, SufficiencyJudge, SufficiencyVerdict,
};
use thuki_agent_lib::websearch::orchestrator::{
    run_search, SearchDeps, SearchOutcome, Synthesizer,
};
use thuki_agent_lib::websearch::prepass::{
    InferenceError, PrePass, PrePassDecision, SearchDecision, SearchRoute,
};
use thuki_agent_lib::websearch::rank::{select_chunks, Bm25Scorer};
use thuki_agent_lib::websearch::serp_cache::WebCache;

/// The classifier stand-in: a fixed `cached` / `web`-route decision for the
/// follow-up, exactly what the live trace showed the model return on turn 2.
struct CachedPrePass;

#[async_trait]
impl PrePass for CachedPrePass {
    async fn decide(
        &self,
        _history: &[ChatMessage],
        _latest_user_message: &str,
        _latest_images: Option<&[String]>,
        _today: &str,
        _cancel: &CancellationToken,
    ) -> Result<PrePassDecision, InferenceError> {
        Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "how old is Elon Musk now".to_string(),
            queries: vec!["elon musk age".to_string()],
            explicit_search: false,
            lang: "en".to_string(),
        })
    }
}

/// Records every source the reuse gate hands the judge, so the test can inspect
/// the EXACT assembled text the judge saw. Returns `sufficient` so the turn
/// completes as a reuse (no live escalation) once the capture is taken.
struct CapturingJudge {
    seen: Arc<Mutex<Vec<SourceBlock>>>,
}

#[async_trait]
impl SufficiencyJudge for CapturingJudge {
    async fn judge(
        &self,
        _standalone_question: &str,
        sources: &[SourceBlock],
        _cancel: &CancellationToken,
    ) -> Result<SufficiencyVerdict, InferenceError> {
        *self.seen.lock().unwrap() = sources.to_vec();
        Ok(SufficiencyVerdict {
            sufficient: true,
            missing: String::new(),
            reason: InsufficiencyReason::Missing,
            requery_queries: Vec::new(),
        })
    }
}

/// Stands in for the writer, which smoke 2 proved derives an age from a birth
/// date. Records the prompt it receives (its system message embeds the
/// assembled sources) and returns a derived answer, so the reuse path resolves
/// to `AnswerReused` without a resident model.
struct DerivingSynthesizer {
    seen_prompt: Arc<Mutex<String>>,
}

#[async_trait]
impl Synthesizer for DerivingSynthesizer {
    async fn synthesize(
        &self,
        messages: &[ChatMessage],
        _cancel: &CancellationToken,
    ) -> Result<String, InferenceError> {
        let joined = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        *self.seen_prompt.lock().unwrap() = joined.clone();
        // Non-circular: derive ONLY when the birth date actually reached the
        // writer prompt. When it did not (the data path dropped it), decline
        // with the sentinel exactly as the real cache-tier writer would, so a
        // truncated reuse escalates rather than falsely "passing".
        if joined.contains("June 28, 1971") || joined.contains("1971") {
            Ok("Elon Musk is 55 years old, born June 28, 1971 [1].".to_string())
        } else {
            // The cache-tier writer's decline sentinel (see
            // `writer::INSUFFICIENT_EVIDENCE_SENTINEL`, `pub(crate)`), as a literal
            // so this out-of-crate repro can model the real decline.
            Ok("INSUFFICIENT_EVIDENCE".to_string())
        }
    }
}

/// One fixture page mirroring a turn-1 fetched bio/profile page.
fn page(url: &str, title: &str, text: &str) -> FetchedPage {
    FetchedPage {
        url: url.into(),
        title: title.into(),
        text: text.into(),
        published: None,
    }
}

/// A realistic, large (>24 KiB) Wikipedia-style Elon Musk extract. The birth
/// date sits in the lead exactly as it does on the real article; the rest is
/// many `Musk`-heavy paragraphs (SpaceX, Tesla, wealth, views) that do NOT
/// restate the birth date, so the birth-date chunk must survive `select_chunks`
/// top-3 (`RANK_MAX_CHUNKS_PER_PAGE`) among a dozen competing `Musk` chunks and
/// `assemble_context`'s token budget to reach the gate. This is the input on
/// which `bound_pages` / top-K / budget are NOT no-ops.
fn elon_wikipedia_text() -> String {
    let lead = "Elon Reeve Musk (born June 28, 1971) is a businessman and investor known \
        for his key roles in the space company SpaceX and the automotive company Tesla. \
        Musk is the wealthiest person in the world according to several estimates. He is \
        the founder, chairman, CEO, and chief technology officer of SpaceX; the angel \
        investor, CEO, product architect, and former chairman of Tesla; and the owner of \
        the social platform X. Musk was born in Pretoria to Maye and Errol Musk, and \
        briefly attended the University of Pretoria before immigrating to Canada. ";
    // Musk-heavy filler paragraphs, none restating the birth date, so many
    // chunks score on "Musk" and compete with the lead for the top-3 slots.
    let filler = "Musk co-founded the online city guide Zip2 with his brother Kimbal Musk, \
        and the company was acquired by Compaq in a deal that made Musk a millionaire. Musk \
        then co-founded the online bank X.com, which merged with Confinity to form the \
        company later known as PayPal. Musk was ousted as PayPal chief executive but \
        remained the largest shareholder when the company was sold to eBay. With the \
        proceeds Musk founded SpaceX, where Musk served as chief executive and lead \
        designer, pursuing reusable rockets. Musk led SpaceX through the Falcon 1, Falcon 9, \
        and Falcon Heavy programs, and Musk directed the development of the Dragon and \
        Starship vehicles. Musk also became an early investor in Tesla, and Musk soon took \
        over as chairman and product architect before becoming chief executive. Under Musk, \
        Tesla shipped the Roadster, Model S, Model 3, Model X, and Model Y. Musk founded \
        The Boring Company to build tunnels, and Musk co-founded Neuralink and OpenAI. Musk \
        has drawn attention for his statements on social media, and Musk acquired Twitter, \
        which Musk rebranded as X. Commentators have described Musk as one of the most \
        influential and controversial business figures of the era, and Musk has repeatedly \
        set ambitious targets for the companies Musk runs. ";
    let mut text = String::with_capacity(30 * 1024);
    text.push_str(lead);
    // Repeat the Musk-heavy filler until the page is comfortably over the 24 KiB
    // per-page cap, so `bound_pages` truncation and the per-page top-K both bite.
    while text.len() < 30 * 1024 {
        text.push_str(filler);
    }
    text
}

/// A large Forbes-style profile whose net-worth prose dominates and whose only
/// birth-date mention sits deep in the text (past the 24 KiB cap), modelling a
/// page where store-time truncation could cut the date entirely.
fn elon_forbes_text() -> String {
    let head = "Elon Musk is the founder, CEO of SpaceX and CEO of Tesla, and according to \
        this profile Musk is among the wealthiest people in the world. As of 2026 the net \
        worth of Musk is estimated at roughly $240 billion. ";
    let body = "The profile tracks the fortune of Musk across Tesla equity, SpaceX stake, \
        and other holdings of Musk. Analysts note that the wealth of Musk swings with the \
        Tesla share price, and Musk has pledged large amounts of Tesla stock. The profile \
        also discusses the compensation package of Musk, the philanthropy of Musk, and the \
        many companies Musk controls. Musk remains a polarising figure whose ventures Musk \
        continues to expand. ";
    let mut text = String::with_capacity(30 * 1024);
    text.push_str(head);
    while text.len() < 26 * 1024 {
        text.push_str(body);
    }
    // Birth date buried well past the 24 KiB per-page cap: whether it survives
    // is exactly the store-time-truncation question.
    text.push_str("Biographical note: Elon Musk was born on June 28, 1971. ");
    text
}

/// The turn-1 fetched pages, realistically sized so the byte caps, per-page
/// top-K, and assembly budget are all live rather than no-ops.
fn elon_pages() -> Vec<FetchedPage> {
    vec![
        page(
            "https://en.wikipedia.org/wiki/Elon_Musk",
            "Elon Musk - Wikipedia",
            &elon_wikipedia_text(),
        ),
        page(
            "https://www.forbes.com/profile/elon-musk/",
            "Elon Musk profile - Forbes",
            &elon_forbes_text(),
        ),
    ]
}

/// Prints whether the birth-date text survived each stage, then asserts the
/// data path delivers it to the judge. Diagnoses hypothesis (a).
#[tokio::test]
#[ignore = "cache-reuse repro; run explicitly with --ignored --nocapture"]
async fn cache_reuse_repro_delivers_birthdate_to_the_reuse_gate() {
    let cache = TtlSourceCache::new(
        std::time::Duration::from_secs(600),
        thuki_agent_lib::config::defaults::SEARCH_CACHE_MAX_ENTRIES,
    );
    // Store EXACTLY as production's `run_engine_tier` does: the real byte caps
    // run inside `store` (`bound_pages`).
    cache.store(
        1,
        CachedSearch {
            pages: elon_pages(),
            route: SearchRoute::Web,
        },
    );

    // Stage 1: bounded stored pages. Did the birth-date text survive the byte
    // caps at store time?
    let stored = cache.entries(1);
    eprintln!("[repro] stored entries: {}", stored.len());
    for (ei, entry) in stored.iter().enumerate() {
        for (pi, p) in entry.pages.iter().enumerate() {
            eprintln!(
                "[repro] entry {ei} page {pi}: {} bytes | has\"1971\"={} has\"born\"={} url={}",
                p.text.len(),
                p.text.contains("1971"),
                p.text.contains("born"),
                p.url,
            );
        }
    }

    // Stage 2 (direct): run the SAME `select_chunks` the reuse gate runs over
    // the bounded stored pages against the follow-up question. This isolates the
    // per-page top-3 (`RANK_MAX_CHUNKS_PER_PAGE`) selection from the rest of the
    // pipeline, so we can see whether the birth-date chunk survives it.
    let bounded_pages: Vec<_> = stored.iter().flat_map(|e| e.pages.clone()).collect();
    let chunks = select_chunks(&bounded_pages, "how old is Elon Musk now", &Bm25Scorer);
    eprintln!("[repro] select_chunks kept {} chunk(s):", chunks.len());
    let mut chunk_has_birthdate = false;
    for (i, c) in chunks.iter().enumerate() {
        let has = c.text.contains("1971");
        chunk_has_birthdate |= has;
        eprintln!(
            "[repro]   chunk {i}: score={:.3} has\"1971\"={} url={} | {}...",
            c.score,
            has,
            c.url,
            &c.text.chars().take(90).collect::<String>()
        );
    }
    eprintln!("[repro] SELECT_CHUNKS birthdate-survived={chunk_has_birthdate}");

    // Stage 3 (direct): assemble the budgeted source blocks the reuse gate would
    // hand its writer/judge. Fix-independent (does not depend on whether the
    // judge is in the path), so this stays the durable data-path assertion.
    let assembled = assemble_context(&chunks, 16384);
    let assembled_has_birthdate = assembled.iter().any(|s| s.text.contains("1971"));
    eprintln!(
        "[repro] assemble_context blocks={} birthdate-survived={assembled_has_birthdate}",
        assembled.len()
    );

    let seen = Arc::new(Mutex::new(Vec::<SourceBlock>::new()));
    let seen_prompt = Arc::new(Mutex::new(String::new()));
    let prepass = CachedPrePass;
    let judge = CapturingJudge { seen: seen.clone() };
    let synthesizer = DerivingSynthesizer {
        seen_prompt: seen_prompt.clone(),
    };
    let transport = ReqwestTransport::new().expect("transport builds");
    let health = EngineHealth::new();
    let recorder = BoundRecorder::noop_for(ConversationId::new("repro"));
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
        synthesizer: &synthesizer,
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

    let outcome = run_search(
        &deps,
        "You are a helpful assistant.",
        &[],
        "How old is Elon Musk now?",
        16384,
        "2026-07-08",
        "en-US",
        &CancellationToken::new(),
        &|phase| eprintln!("[repro] phase={phase:?}"),
    )
    .await;

    // Stage 2/3: the exact sources handed to the judge (the reuse gate's
    // assembled context after dedupe -> select_chunks -> recency -> filter ->
    // assemble). This is what the judge actually decided over.
    // Informational only: the judge capture is empty once the judge is removed
    // from the reuse path, so it is printed, never asserted.
    let judged = seen.lock().unwrap().clone();
    eprintln!(
        "[repro] sources the judge saw (empty once judge removed): {}",
        judged.len()
    );
    for s in &judged {
        eprintln!(
            "[repro]   [{}] {} | has-birthdate={}",
            s.index,
            s.url,
            s.text.contains("1971")
        );
    }

    // Stage 4: final outcome + the prompt the writer derived from.
    match &outcome {
        SearchOutcome::AnswerReused {
            content, sources, ..
        } => {
            eprintln!(
                "[repro] OUTCOME = AnswerReused ({} sources): {content}",
                sources.len()
            );
        }
        SearchOutcome::Answer { sources, .. } => {
            eprintln!(
                "[repro] OUTCOME = Answer (escalated, {} sources)",
                sources.len()
            )
        }
        SearchOutcome::NoSearch => eprintln!("[repro] OUTCOME = NoSearch"),
        SearchOutcome::Unreachable { .. } => eprintln!("[repro] OUTCOME = Unreachable"),
        SearchOutcome::Cancelled => eprintln!("[repro] OUTCOME = Cancelled"),
    }
    let prompt = seen_prompt.lock().unwrap().clone();
    eprintln!(
        "[repro] writer prompt has-birthdate={}",
        prompt.contains("1971")
    );

    // DATA-PATH VERDICT (fix-independent): with realistic pages where the byte
    // caps and per-page top-3 actually fire, the birth date still reaches the
    // assembled sources the reuse gate builds. Hypothesis (a) is falsified; the
    // observed rejection is (b), the judge model ignoring the derivation clause.
    assert!(
        chunk_has_birthdate,
        "VERDICT (a): select_chunks dropped the birth-date chunk (data path bug)"
    );
    assert!(
        assembled_has_birthdate,
        "VERDICT (a): assemble_context dropped the birth date (data path bug)"
    );
    // End-to-end: with the birth date present, the reuse path serves a derived
    // AnswerReused rather than escalating.
    assert!(
        matches!(outcome, SearchOutcome::AnswerReused { .. }),
        "expected the reuse path to serve AnswerReused"
    );
}
