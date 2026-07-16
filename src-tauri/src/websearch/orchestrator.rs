//! The search orchestrator: fixed-budget web retrieval that turns one user
//! turn into either a plain answer or a source-grounded answer.
//!
//! All external effects are injected through traits ([`PrePass`],
//! [`HttpTransport`], [`Scorer`]), so the whole decision tree — the
//! `no｜cached｜web` branch, cancellation at every step, and degradation when
//! search yields nothing — is unit-tested against fakes with no live model or
//! network. The caller (the built-in chat route) supplies the real
//! implementations and a status callback that forwards progress to the UI.
//!
//! The decision is two-stage. The deterministic [`super::prefilter`] runs first,
//! with no model call: it forces the obvious turns (a greeting needs no web, a
//! "latest ..." question always does) and defers only the ambiguous middle to
//! the persona-free classifier ([`PrePass`]). A `ForceWeb` verdict overrides the
//! classifier's own decision (see [`resolve_decision`]) but still uses its
//! standalone rewrite and queries.
//!
//! Failure policy, in order of how it degrades:
//! - `ForceNo` from the pre-filter → [`SearchOutcome::NoSearch`], no model call;
//! - classifier cancelled → [`SearchOutcome::Cancelled`] (the caller stops);
//! - classifier infra error → [`SearchOutcome::NoSearch`] (answer from the
//!   model, never block the user on a search-infra failure);
//! - `no` decision → [`SearchOutcome::NoSearch`];
//! - a `web` decision whose retrieval yields nothing (every engine blocked or
//!   empty, or nothing worth citing after ranking) →
//!   [`SearchOutcome::Unreachable`]: the model still answers, but is explicitly
//!   told to disclose that it could not verify current information. Silently
//!   presenting stale memory as current on a turn that wanted the web is the
//!   pipeline's worst failure mode, so it is never allowed to happen silently;
//! - cancellation mid-pipeline → [`SearchOutcome::Cancelled`].
//!
//! Query volume is bounded two ways: the loop stops issuing further queries
//! once one has returned enough hits ([`SERP_EARLY_STOP_HITS`]), and blocked
//! engines sit out a cooldown window ([`EngineHealth`]) instead of being
//! re-hammered on every query. Both exist because the keyless engines'
//! rate-limits are volume-triggered: the pipeline's own burst was observed
//! live tripping them.
//!
//! A `cached` decision is only a routing hint: it enters the gated reuse arm
//! ([`reuse_or_escalate`]), which reuses the conversation's stored sources from
//! [`SearchDeps::cache`] (the TTL'd, bounded multi-turn source cache of the
//! recent successful searches for the turn's [`SearchDeps::cache_scope`]) only
//! when the route is eligible AND the sufficiency judge confirms the union of
//! those sources answers the question. An ineligible route (weather, news,
//! sports), an empty cache, an insufficient union, or a judge failure all
//! escalate to the same `web` retrieval a `web` decision runs, using the
//! classifier's route, standalone rewrite, and queries.
//!
//! Whenever the general engine tier itself assembles sources (whether reached
//! directly or via a vertical's escalation, see [`commit_or_escalate`]), one
//! extra sufficiency-judge call checks the result against the standalone
//! question ([`judge_and_requery`]). An insufficient verdict naming what is
//! missing fires exactly one bounded requery
//! ([`crate::config::defaults::ENGINE_REQUERY_MAX`]); its new sources merge
//! with round one's rather than replace them. After that merge a **second**
//! judge call sets `still_missing` / conflict on the merged set; there is no
//! third requery. A first-round judge failure, timeout, or unparseable body
//! fails toward committing round one's sources unchanged, the same posture
//! [`commit_or_escalate`] takes on a vertical judge failure.

use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::commands::ChatMessage;
use crate::config::defaults::{
    REQUERY_MISSING_MAX_CHARS, SEARCH_LANG_DEFAULT, SERP_EARLY_STOP_HITS,
    TRACE_SOURCE_TEXT_MAX_BYTES,
};
use crate::net::reachability::{offline_cutoff, Reachability};
use crate::net::transport::HttpTransport;
use crate::trace::{truncate_for_trace, BoundRecorder, EngineStat, RecorderEvent, RetrievedSource};
use crate::websearch::assemble::{assemble_context, SourceBlock};
use crate::websearch::cache::{CachedSearch, SourceCache};
use crate::websearch::encyclopedia::{
    fetch_encyclopedia, is_price_intent_question, is_volatile_question,
};
use crate::websearch::engine::{
    any_engine_available, transport_unreachable, web_search, EngineHealth, SearchHit,
};
use crate::websearch::evidence::filter_evidence_chunks;
use crate::websearch::fetch::{fetch_pages, FetchedPage};
use crate::websearch::judge::{deterministic_sufficiency, SufficiencyJudge, SufficiencyVerdict};
use crate::websearch::lang::{detect_script_lang, resolve_lang, supported_lang};
use crate::websearch::news::{fetch_news, is_news_intent};
use crate::websearch::prefilter::{prefilter, PreFilterVerdict};
use crate::websearch::prepass::{
    InferenceError, PrePass, PrePassDecision, SearchDecision, SearchRoute,
};
use crate::websearch::rank::{rerank_by_score, select_chunks, ScoredChunk, Scorer};
use crate::websearch::recency::recency_reorder;
use crate::websearch::serp_cache::WebCache;
use crate::websearch::sports::{fetch_sports, is_sports_intent};
use crate::websearch::stage_timing::{
    queries_near_duplicate, TimingBag, STAGE_CLASSIFIER, STAGE_FETCH, STAGE_JUDGE,
    STAGE_JUDGE_POST_REQUERY, STAGE_RANK_ASSEMBLY, STAGE_RAW_RACE_SERP, STAGE_SERP,
    STAGE_WRITER_PREPARE,
};
use crate::websearch::weather::{fetch_weather, is_weather_intent};
use crate::websearch::writer::{unreachable_messages, writer_messages};

/// Progress phase reported to the UI while the pipeline runs, before any answer
/// token streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchPhase {
    /// Running the pre-pass decision on the warm slot.
    Deciding,
    /// Querying the keyless search engine.
    Searching,
    /// Fetching and reading the top result pages.
    Reading,
    /// Citation audit (± repair) after the answer has finished streaming.
    /// UI shows a compact sources pill so the gap does not look hung.
    Verifying,
}

/// What the orchestrator resolved for this turn.
pub enum SearchOutcome {
    /// Answer the user directly with the plain chat prompt (a `no` decision or
    /// a search-infra failure before any retrieval was wanted).
    NoSearch,
    /// Answer with the source-grounded writer prompt. `messages` already embeds
    /// the delimited sources; `sources` is the citation metadata for the UI.
    Answer {
        messages: Vec<ChatMessage>,
        sources: Vec<SourceBlock>,
    },
    /// A search was wanted but retrieval produced nothing. `messages` is the
    /// plain chat prompt plus a reason-specific appendix telling the model to
    /// disclose the failed verification, so a stale answer is never presented as
    /// current. `reason` distinguishes a true transport failure from a search
    /// that ran but found nothing, so the frontend can show the right note and
    /// the writer prompt can caveat with the right words (see
    /// [`SearchFailReason`]).
    Unreachable {
        messages: Vec<ChatMessage>,
        reason: SearchFailReason,
    },
    /// The user cancelled during the pipeline; the caller emits `Cancelled` and
    /// streams nothing.
    Cancelled,
}

/// Why a wanted web search produced no citable answer. Distinguishes the two
/// failures that used to collapse into one so both the user-facing note and the
/// model's self-disclosure appendix can name the accurate cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchFailReason {
    /// Every engine actually contacted this turn failed at the network/transport
    /// layer: the web could not be reached, so "check your connection" is
    /// accurate.
    Unreachable,
    /// At least one engine returned an HTTP response (or a cached list) but
    /// nothing usable survived fusion and ranking: the web was searched and
    /// simply had nothing current. Includes the blocked-but-online case.
    NoResults,
    /// Weather vertical wanted live Open-Meteo conditions but geocode/forecast
    /// missed. Must not fall back to SEO or to model-invented seasonal RH/temp
    /// (2026-07-15 Hanoi humidity smoke). Distinct disclosure from general
    /// [`Self::NoResults`].
    WeatherUnavailable,
}

/// The injected effectful dependencies of the pipeline.
pub struct SearchDeps<'a> {
    pub prepass: &'a dyn PrePass,
    /// The sufficiency judge, run both after a keyless vertical answers (see
    /// [`commit_or_escalate`]) and after the general engine tier assembles its
    /// own sources (see [`judge_and_requery`]), deciding whether the block in
    /// hand actually contains what the question asked before the pipeline
    /// commits to it. A vertical's insufficient block escalates to the
    /// scraped engines; an insufficient engine-tier block gets one bounded
    /// requery. Injected like the other effectful dependencies so both
    /// branches are tested with a fake judge.
    pub judge: &'a dyn SufficiencyJudge,
    pub transport: &'a dyn HttpTransport,
    /// The offline fast-fail signal, RACED against the engine tier's live
    /// requests (never used as a pre-flight gate; see
    /// [`crate::net::reachability`]). Only a proven-unreachable probe, with no
    /// engine response inside the grace window, short-circuits the turn to the
    /// existing "can't reach the web" disclosure; a reachable or inconclusive
    /// probe leaves the requests their full budget. Injected like the other
    /// effectful dependencies so both sides of that decision are tested without
    /// a resolver or a network.
    pub reachability: &'a dyn Reachability,
    pub scorer: &'a dyn Scorer,
    /// Cross-turn engine block memory; blocked engines sit out their cooldown.
    pub health: &'a EngineHealth,
    /// Conversation-bound forensic recorder. The pipeline emits the resolved
    /// search decision and the retrieval tier to the chat-domain trace so a bad
    /// route or dead-end is diagnosable after the fact. A `NoopRecorder`-backed
    /// bound recorder makes every emission a constant-time no-op when tracing is
    /// off (the production default).
    pub recorder: &'a BoundRecorder,
    /// The bounded multi-turn source cache backing a `cached` decision (see
    /// module docs). Every successful answer of any tier writes its sources
    /// here (recording the route that produced them); only a `cached` decision
    /// reads from it, through the reuse gate ([`reuse_or_escalate`]).
    pub cache: &'a dyn SourceCache,
    /// Opaque scope key for `cache` reads/writes this turn (in production,
    /// the backend's conversation epoch at the start of the turn), so a
    /// cache entry from one conversation is never served to another.
    pub cache_scope: u64,
    /// Process-lifetime, in-memory result cache for the scraped-engine tier: a
    /// repeat SERP scrape or page fetch is served from memory instead of hitting
    /// a keyless engine again, cutting latency and the volume that triggers the
    /// engines' rate limits. Distinct from `cache` (the cross-turn SOURCE cache
    /// backing a `cached` decision): this one holds raw engine SERP lists and
    /// extracted page bodies, is not conversation-scoped (its contents are public
    /// web pages, not the user's resolved question), and is read/written only
    /// inside [`run_engine_tier`]. See [`crate::websearch::serp_cache`].
    pub web_cache: &'a WebCache,
    /// The user's device IANA timezone (e.g. `"America/Chicago"`), when known,
    /// used by the sports vertical to localize scheduled kickoff times. `None`
    /// falls back to date-only event lines. An environmental value the caller
    /// snapshots per turn, like `cache_scope`; `today`/`locale` are passed
    /// positionally to `run_search`, but the zone is only ever read deep in the
    /// pipeline (the sports tier), so it rides in `deps` rather than threading
    /// an extra positional argument through every `run_search` call site.
    pub local_zone: Option<&'a str>,
    /// Force a web search this turn regardless of what the pre-filter and
    /// classifier decide, with the exact "look it up again" semantics: skip
    /// every fast path (the source cache and all verticals) and go straight to
    /// the scraped engines with a cache read-bypass, write-through fetch. Set by
    /// the `/search` slash command (see `commands::run_builtin_search`); `false`
    /// for the invisible auto-search, which lets the pre-pass decide. Rides in
    /// `deps` rather than as a positional argument for the same reason
    /// `local_zone` does: it is a per-turn caller input the call sites should not
    /// each have to thread.
    pub force_search: bool,
    /// Base64 image payloads for the latest user turn when the active model is
    /// vision-capable. Passed to the classifier and re-attached on the writer
    /// so grounded answers keep the photo. `None` or empty keeps the text-only
    /// path (no multimodal prefill). Never sent to search engines.
    pub latest_images: Option<&'a [String]>,
    /// Per-stage wall-clock bag for this turn. Callers create one bag per
    /// search; the orchestrator records stage ms without holding locks across
    /// awaits and flushes a [`RecorderEvent::SearchTimings`] before return.
    pub timings: &'a TimingBag,
}

/// Runs the pipeline for one turn. `chat_system_prompt`, `history`, and
/// `latest_user` MUST be byte-identical to the plain chat prompt so the
/// pre-pass and writer share llama-server's warm KV prefix.
///
/// Production and unit tests call this with auto-routing (no force). Slash
/// `/search` uses [`run_search_forced`] so the user override always hits the
/// engines-only path.
#[allow(clippy::too_many_arguments)]
pub async fn run_search(
    deps: &SearchDeps<'_>,
    chat_system_prompt: &str,
    history: &[ChatMessage],
    latest_user: &str,
    num_ctx: u32,
    today: &str,
    locale: &str,
    cancel: &CancellationToken,
    status: &(dyn Fn(SearchPhase) + Send + Sync),
) -> SearchOutcome {
    run_search_inner(
        deps,
        chat_system_prompt,
        history,
        latest_user,
        num_ctx,
        today,
        locale,
        cancel,
        status,
        false,
    )
    .await
}

/// Like [`run_search`], but forces the engines-only `explicit_search` path
/// (skip cache and verticals), even when the classifier would say no. Used by
/// the `/search` slash alias as the user's recovery hatch for false negatives.
#[allow(clippy::too_many_arguments)]
pub async fn run_search_forced(
    deps: &SearchDeps<'_>,
    chat_system_prompt: &str,
    history: &[ChatMessage],
    latest_user: &str,
    num_ctx: u32,
    today: &str,
    locale: &str,
    cancel: &CancellationToken,
    status: &(dyn Fn(SearchPhase) + Send + Sync),
) -> SearchOutcome {
    run_search_inner(
        deps,
        chat_system_prompt,
        history,
        latest_user,
        num_ctx,
        today,
        locale,
        cancel,
        status,
        true,
    )
    .await
}

/// Shared body for [`run_search`] / [`run_search_forced`].
#[allow(clippy::too_many_arguments)]
async fn run_search_inner(
    deps: &SearchDeps<'_>,
    chat_system_prompt: &str,
    history: &[ChatMessage],
    latest_user: &str,
    num_ctx: u32,
    today: &str,
    locale: &str,
    cancel: &CancellationToken,
    status: &(dyn Fn(SearchPhase) + Send + Sync),
    force_search: bool,
) -> SearchOutcome {
    // Flush stage timings on every exit path (including cancel and early NoSearch).
    // Drop runs after the function body returns, so awaits complete first; the bag
    // never holds its mutex across those awaits (see TimingBag::record).
    let _timing_flush = TimingFlushGuard {
        bag: deps.timings,
        recorder: deps.recorder,
    };

    // Slash `/search` (and any other explicit force): skip ForceNo and go
    // engines-only. Still runs the classifier when possible for a better
    // rewrite, then stamps explicit_search so cache/verticals never win.
    if force_search {
        return force_explicit_web(
            deps,
            chat_system_prompt,
            history,
            latest_user,
            num_ctx,
            today,
            locale,
            cancel,
            status,
        )
        .await;
    }

    // Stage one: deterministic pre-filter, no model call.
    let verdict = prefilter(latest_user, today);
    eprintln!("[search] prefilter={verdict:?}");
    // The `/search` command forces a search even when the pre-filter would
    // force-skip (e.g. a message that reads like a greeting): the user asked
    // for a web search explicitly, so the deterministic skip is overridden and
    // the classifier still runs for the query rewrite.
    if verdict == PreFilterVerdict::ForceNo && !deps.force_search {
        deps.recorder.record(RecorderEvent::SearchDecided {
            prefilter: prefilter_label(verdict).to_string(),
            decision: "no".to_string(),
            force: false,
            route: String::new(),
            standalone_question: latest_user.trim().to_string(),
            queries: Vec::new(),
        });
        return SearchOutcome::NoSearch;
    }
    // Stage two: the persona-free classifier decides the ambiguous middle and
    // rewrites the query. A `ForceWeb` verdict still runs it, for the rewrite.
    // On ForceWeb only, race a raw-query SERP against the classifier so the
    // common near-duplicate rewrite path pays zero SERP wait after the model
    // returns (≤1 DDG when the race is kept; divergent rewrites re-SERP).
    status(SearchPhase::Deciding);
    let classified_and_race =
        classify_maybe_race_raw(deps, history, latest_user, today, cancel, verdict).await;
    let (classified, raced_serp) = match classified_and_race {
        ClassifyRaceResult::Cancelled => return SearchOutcome::Cancelled,
        ClassifyRaceResult::NoSearch => return SearchOutcome::NoSearch,
        ClassifyRaceResult::Offline => {
            // ForceWeb raw-SERP race lost to a proven-unreachable probe: same
            // Unreachable disclosure as `run_engine_tier`'s offline cut, without
            // waiting for stalled engines under the classifier join.
            return SearchOutcome::Unreachable {
                messages: unreachable_messages(
                    chat_system_prompt,
                    history,
                    latest_user,
                    deps.latest_images,
                    SearchFailReason::Unreachable,
                ),
                reason: SearchFailReason::Unreachable,
            };
        }
        ClassifyRaceResult::Ready {
            classified,
            raced_serp,
        } => (classified, raced_serp),
    };
    let decision = sanitize_search_decision(latest_user, resolve_decision(verdict, classified));
    // The turn's language, resolved ONCE and threaded down into every channel.
    // Resolved from `latest_user`, the message the USER actually wrote, never from
    // the classifier's rewrite: the rewrite is tuned for retrieval (it may carry an
    // English companion query on a Vietnamese turn), so its wording is an artifact,
    // not a language signal. The classifier's own `lang` judgement rides alongside
    // the raw text and is validated inside `resolve_lang` before it can influence
    // anything.
    let lang = resolve_lang(latest_user, &decision.lang, locale);
    eprintln!(
        "[search] decision={:?} route={:?} queries={} lang={lang}",
        decision.decision,
        decision.route,
        decision.queries.len()
    );
    deps.recorder.record(RecorderEvent::SearchDecided {
        prefilter: prefilter_label(verdict).to_string(),
        decision: decision_label(decision.decision).to_string(),
        force: deps.force_search,
        route: route_label(decision.route).to_string(),
        standalone_question: decision.standalone_question.clone(),
        queries: decision.queries.clone(),
    });
    // Keep raced ForceWeb SERP only when rewrite ≈ raw, decision is web, and the
    // race language matches the final resolved language (else re-SERP under final).
    let preloaded_serp = preloaded_serp_for_decision(&decision, latest_user, lang, raced_serp);
    // Explicit "look it up / verify / double-check" request, or the `/search`
    // command (`deps.force_search`): re-serving ANY fast path is forbidden. Skip
    // the cache AND every vertical and go straight to the scraped engines with
    // the classifier's resolved question/queries and a cache read-bypass,
    // write-through fetch. Forced even over a `No` decision: an explicit
    // look-it-up, and an explicit `/search`, are both always a search.
    if decision.explicit_search || deps.force_search {
        return run_web(
            deps,
            chat_system_prompt,
            history,
            latest_user,
            decision.route,
            &decision.standalone_question,
            &decision.queries,
            num_ctx,
            today,
            locale,
            lang,
            cancel,
            status,
            true,
            preloaded_serp,
        )
        .await;
    }
    match decision.decision {
        SearchDecision::No => SearchOutcome::NoSearch,
        // A `cached` decision is only a hint: the reuse gate verifies the
        // stored sources against the actual question (the sufficiency judge)
        // and escalates to a fresh search when they do not carry the answer,
        // so a recall-biased classifier never serves a wrong or stale reply.
        SearchDecision::Cached => {
            reuse_or_escalate(
                deps,
                chat_system_prompt,
                history,
                latest_user,
                decision.route,
                &decision.standalone_question,
                &decision.queries,
                num_ctx,
                today,
                locale,
                lang,
                cancel,
                status,
                preloaded_serp,
            )
            .await
        }
        SearchDecision::Web => {
            run_web(
                deps,
                chat_system_prompt,
                history,
                latest_user,
                decision.route,
                &decision.standalone_question,
                &decision.queries,
                num_ctx,
                today,
                locale,
                lang,
                cancel,
                status,
                false,
                preloaded_serp,
            )
            .await
        }
    }
}

/// Flushes [`TimingBag`] into the trace recorder when dropped. Placed at the
/// top of [`run_search_inner`] so every exit path (including early returns and
/// cancellation) emits one [`RecorderEvent::SearchTimings`].
struct TimingFlushGuard<'a> {
    bag: &'a TimingBag,
    recorder: &'a BoundRecorder,
}

impl Drop for TimingFlushGuard<'_> {
    /// Records pipeline total + stage list to the forensic recorder.
    fn drop(&mut self) {
        self.bag.flush(self.recorder);
    }
}

/// Result of the classifier stage, optionally concurrent with a ForceWeb raw SERP race.
enum ClassifyRaceResult {
    /// User cancelled during the classifier (or race is abandoned on cancel).
    Cancelled,
    /// Ambiguous-turn classifier infra failure: answer without search.
    NoSearch,
    /// Offline probe proved no network path while the ForceWeb raw SERP was
    /// racing the classifier. Same disclosure as an engine-tier offline cut.
    Offline,
    /// Classifier finished (or ForceWeb fallback synthesized); optional raced SERP.
    ///
    /// The raced triple is `(hits, engine stats, race_lang)`: `race_lang` is the
    /// script-only language the concurrent SERP used, so
    /// [`preloaded_serp_for_decision`] can discard hits when the final resolved
    /// language differs.
    Ready {
        classified: PrePassDecision,
        raced_serp: Option<(Vec<SearchHit>, Vec<EngineStat>, &'static str)>,
    },
}

/// Language for a ForceWeb concurrent raw-query SERP race.
///
/// Script only: the classifier has not returned yet, and locale must not bias
/// the race. A `vi_VN` machine racing an English query under `vi` was the live
/// regression this path exists to prevent. No supported script signal →
/// [`SEARCH_LANG_DEFAULT`].
fn race_lang_for_force_web(latest_user: &str) -> &'static str {
    detect_script_lang(latest_user)
        .and_then(supported_lang)
        .unwrap_or(SEARCH_LANG_DEFAULT)
}

/// Whether a ForceWeb turn should race a raw-query SERP with the classifier.
///
/// Skips vertical-shaped questions (weather / sports / news keywords) so the
/// race never spends DDG quota on turns a keyless vertical is about to answer.
/// Engine-shaped ForceWeb turns race; that is the latency win the package
/// targets. Cached decisions on engine-shaped ForceWeb may still race once and
/// discard hits (rare follow-up path); see [`preloaded_serp_for_decision`].
fn force_web_should_race_raw(latest_user: &str) -> bool {
    if is_sports_intent(latest_user) || is_news_intent(latest_user) {
        return false;
    }
    // Share the weather vertical's intent detector so VI phrases ("độ ẩm",
    // "thời tiết") also skip the raw SERP race the same way English does.
    !is_weather_intent(latest_user)
}

/// Runs the classifier; on [`PreFilterVerdict::ForceWeb`] only (and when the
/// turn looks engine-shaped, see [`force_web_should_race_raw`]), races a
/// raw-query SERP in parallel so a near-duplicate rewrite can reuse those hits
/// (common path stays at one DDG request). Non-ForceWeb paths never race.
async fn classify_maybe_race_raw(
    deps: &SearchDeps<'_>,
    history: &[ChatMessage],
    latest_user: &str,
    today: &str,
    cancel: &CancellationToken,
    verdict: PreFilterVerdict,
) -> ClassifyRaceResult {
    let raw = latest_user.trim().to_string();
    if verdict == PreFilterVerdict::ForceWeb && force_web_should_race_raw(latest_user) {
        // Freshness bias matches run_web: volatile raw phrasing gets the date
        // filter; stable ForceWeb still races without it.
        let freshness = is_volatile_question(&raw);
        // Time each future from its own Instant, recorded when that future
        // completes, not after tokio::join returns. Recording both stages from
        // pre-join start until post-join end would make both report
        // max(classifier, serp) and hide which side dominated.
        let decide = async {
            let start = Instant::now();
            let res = deps
                .prepass
                .decide(history, latest_user, deps.latest_images, today, cancel)
                .await;
            deps.timings.record(STAGE_CLASSIFIER, start);
            res
        };
        // Race SERP cannot wait for the classifier's lang field. Script only
        // (default en); never locale: locale is the weak signal that raced
        // English queries under a non-English $LANG on ForceWeb.
        let race_lang = race_lang_for_force_web(latest_user);
        let race = async {
            let start = Instant::now();
            // Race the raw SERP against the offline cutoff, same contract as
            // `run_engine_tier`: never a preflight gate; proven-unreachable only;
            // taking the cutoff DROPS the in-flight SERP so a late engine cannot
            // contradict the disclosure. Without this, ForceWeb offline turns
            // paid the full dual-engine stall under `tokio::join!` before the
            // engine-tier race ever ran.
            let res = tokio::select! {
                biased;
                res = web_search(
                    deps.transport,
                    &raw,
                    deps.health,
                    freshness,
                    race_lang,
                    deps.web_cache,
                    false,
                ) => Some(res),
                () = offline_cutoff(deps.reachability) => None,
            };
            deps.timings.record(STAGE_RAW_RACE_SERP, start);
            res
        };
        let (classified_res, race_res) = tokio::join!(decide, race);
        // Offline proved during the raw race. Prefer user cancel if the
        // classifier also reported it; otherwise short-circuit before any
        // further SERP round.
        let Some(race_res) = race_res else {
            if matches!(classified_res, Err(InferenceError::Cancelled)) {
                return ClassifyRaceResult::Cancelled;
            }
            eprintln!("[search] offline short-circuit: reachability probe proved no network path");
            return ClassifyRaceResult::Offline;
        };
        let classified = match classified_res {
            Ok(c) => c,
            Err(InferenceError::Cancelled) => return ClassifyRaceResult::Cancelled,
            Err(InferenceError::Request(reason)) => {
                eprintln!("[search] classifier error: {reason}");
                // ForceWeb: fall through to raw-message search.
                PrePassDecision {
                    decision: SearchDecision::Web,
                    route: SearchRoute::Web,
                    standalone_question: raw.clone(),
                    queries: vec![raw.clone()],
                    explicit_search: false,
                    // No classifier output: resolve_lang falls back to script/locale.
                    lang: String::new(),
                }
            }
        };
        let (hits, stats) = race_res;
        return ClassifyRaceResult::Ready {
            classified,
            raced_serp: Some((hits, stats, race_lang)),
        };
    }

    // Ambiguous (and ForceNo never reaches here): sequential classifier only.
    let classifier_start = Instant::now();
    match deps
        .prepass
        .decide(history, latest_user, deps.latest_images, today, cancel)
        .await
    {
        Ok(classified) => {
            deps.timings.record(STAGE_CLASSIFIER, classifier_start);
            ClassifyRaceResult::Ready {
                classified,
                raced_serp: None,
            }
        }
        Err(InferenceError::Cancelled) => {
            deps.timings.record(STAGE_CLASSIFIER, classifier_start);
            ClassifyRaceResult::Cancelled
        }
        Err(InferenceError::Request(reason)) => {
            deps.timings.record(STAGE_CLASSIFIER, classifier_start);
            eprintln!("[search] classifier error: {reason}");
            if !deps.force_search {
                deps.recorder.record(RecorderEvent::SearchDecided {
                    prefilter: prefilter_label(verdict).to_string(),
                    decision: "no".to_string(),
                    force: false,
                    route: String::new(),
                    standalone_question: raw,
                    queries: Vec::new(),
                });
                return ClassifyRaceResult::NoSearch;
            }
            ClassifyRaceResult::Ready {
                classified: PrePassDecision {
                    decision: SearchDecision::Web,
                    route: SearchRoute::Web,
                    standalone_question: latest_user.trim().to_string(),
                    queries: vec![latest_user.trim().to_string()],
                    explicit_search: false,
                    // No classifier output: resolve_lang falls back to script/locale.
                    lang: String::new(),
                },
                raced_serp: None,
            }
        }
    }
}

/// Keeps a ForceWeb raced SERP only when the classifier's first query is a
/// near-duplicate of the raw user message, the decision is still `web`, and the
/// race language matches the final resolved language. Empty race results are
/// kept too when those hold (same miss, no second DDG round that would re-touch
/// engines already cooling from the race). Divergent rewrites or a language
/// mismatch return `None` so the engine tier re-SERPs under the final language.
fn preloaded_serp_for_decision(
    decision: &PrePassDecision,
    latest_user: &str,
    final_lang: &str,
    raced_serp: Option<(Vec<SearchHit>, Vec<EngineStat>, &'static str)>,
) -> Option<(Vec<SearchHit>, Vec<EngineStat>)> {
    if decision.decision != SearchDecision::Web {
        return None;
    }
    let (hits, stats, race_lang) = raced_serp?;
    if race_lang != final_lang {
        eprintln!(
            "[search] force_web race: discard raw SERP (lang mismatch race={race_lang} final={final_lang})"
        );
        return None;
    }
    let first = decision
        .queries
        .first()
        .map(String::as_str)
        .unwrap_or(decision.standalone_question.as_str());
    if queries_near_duplicate(first, latest_user.trim()) {
        eprintln!(
            "[search] force_web race: keeping raw SERP (near-duplicate rewrite, hits={})",
            hits.len()
        );
        Some((hits, stats))
    } else {
        eprintln!("[search] force_web race: discard raw SERP (rewrite diverged)");
        None
    }
}

/// Wire label for a resolved [`SearchDecision`] on [`RecorderEvent::SearchDecided`].
fn decision_label(decision: SearchDecision) -> &'static str {
    match decision {
        SearchDecision::No => "no",
        SearchDecision::Cached => "cached",
        SearchDecision::Web => "web",
    }
}

/// Stable snake_case label for a pre-filter verdict, for the trace.
fn prefilter_label(verdict: PreFilterVerdict) -> &'static str {
    match verdict {
        PreFilterVerdict::ForceNo => "force_no",
        PreFilterVerdict::ForceWeb => "force_web",
        PreFilterVerdict::Ambiguous => "ambiguous",
    }
}

/// Stable lowercase label for a classifier route, for the trace.
fn route_label(route: SearchRoute) -> &'static str {
    match route {
        SearchRoute::Weather => "weather",
        SearchRoute::News => "news",
        SearchRoute::Wiki => "wiki",
        SearchRoute::Sports => "sports",
        SearchRoute::Web => "web",
    }
}

/// User text after stripping a leading `/search` command token, trimmed.
///
/// The slash command is handled as `force_search` upstream; the message body
/// that remains is what intent clamps must read. Pure and total.
fn message_body_without_search_command(latest_user: &str) -> &str {
    let trimmed = latest_user.trim();
    let lower = trimmed.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("/search") {
        // Map the lowercased prefix length back onto the original so casing of
        // the remainder is preserved when present.
        let prefix_len = trimmed.len() - rest.len();
        trimmed[prefix_len..].trim()
    } else {
        trimmed
    }
}

/// True when the user's message is only the bare token `SJC` (optional
/// `/search` prefix). That token is both Vietnam's dominant gold brand and
/// San Jose's IATA code; without an explicit weather/airport cue, Thuki
/// prefers gold for this product (local-first VN market). Pure and total.
fn is_bare_sjc_token(latest_user: &str) -> bool {
    message_body_without_search_command(latest_user).eq_ignore_ascii_case("sjc")
}

/// True when the user explicitly asked for airport / aviation context (so bare
/// `SJC` may mean San Jose, not gold). Pure and total.
fn user_signals_airport_context(latest_user: &str) -> bool {
    let lower = latest_user.to_lowercase();
    lower.split(|c: char| !c.is_alphanumeric()).any(|t| {
        matches!(
            t,
            "airport"
                | "iata"
                | "icao"
                | "flight"
                | "flights"
                | "airline"
                | "airlines"
                | "runway"
                | "terminal"
                | "sfo"
                | "oak"
        ) || t == "san" // "San Jose" usually co-occurs; weak alone, paired with jose below
    }) || lower.contains("san jose")
        || lower.contains("mineta")
}

/// Clamps classifier hallucinations that invent weather (or airport) for
/// messages the user never framed that way, and rewrites bare `SJC` to a gold
/// price ask.
///
/// 2026-07-15 smoke: `/search SJC` → classifier rewrote to San Jose airport
/// weather → SEO heat advisory. Reliability first: only honour a weather route
/// when the user's own text carries weather or explicit airport intent.
///
/// Pure over its inputs (no I/O). Decision fields not rewritten are preserved.
fn sanitize_search_decision(latest_user: &str, mut decision: PrePassDecision) -> PrePassDecision {
    if is_bare_sjc_token(latest_user) && !user_signals_airport_context(latest_user) {
        // Bare SJC without airport cue: gold brand, not KSJC weather.
        let vi_query = "giá vàng SJC hôm nay";
        let en_query = "SJC gold price today";
        // Prefer the Vietnamese gold phrasing when the classifier (or locale
        // path) already marked the turn as VI; otherwise lead with English
        // and still fan out both queries so SERP hits either brand page.
        let prefer_vi = decision.lang.eq_ignore_ascii_case("vi");
        let primary = if prefer_vi { vi_query } else { en_query };
        decision.decision = SearchDecision::Web;
        decision.route = SearchRoute::Web;
        decision.standalone_question = primary.to_string();
        decision.queries = vec![vi_query.to_string(), en_query.to_string()];
        eprintln!("[search] sanitize: bare SJC -> gold web ({primary})");
        return decision;
    }

    // Classifier invented a weather route (or weather rewrite) the user's own
    // message does not support: demote to web and restore the user text so
    // engines do not chase the fabricated airport/weather SERP.
    if matches!(decision.route, SearchRoute::Weather)
        && !is_weather_intent(latest_user)
        && !user_signals_airport_context(latest_user)
    {
        eprintln!(
            "[search] sanitize: drop invented weather route (user={:?})",
            message_body_without_search_command(latest_user)
        );
        decision.route = SearchRoute::Web;
        if is_weather_intent(&decision.standalone_question) {
            let body = message_body_without_search_command(latest_user);
            if !body.is_empty() {
                decision.standalone_question = body.to_string();
                decision.queries = vec![body.to_string()];
            }
        }
    }
    decision
}

/// Combines the pre-filter verdict with the classifier's result into the final
/// decision. `ForceWeb` overrides a classifier `no` to `web` (the deterministic
/// freshness signal is authoritative over the model declining to search), but
/// preserves a classifier `cached`: a repeated or rephrased "latest ..."
/// question (the exact turn shape `ForceWeb` exists to catch) is the case the
/// multi-turn cache is for, and sources fetched moments ago this same
/// conversation are already at least as fresh as a re-search would find. The
/// classifier's standalone rewrite and queries are kept either way, backfilling
/// a query from the rewrite when the classifier produced none. Any other
/// verdict leaves the classifier's decision untouched.
fn resolve_decision(verdict: PreFilterVerdict, classified: PrePassDecision) -> PrePassDecision {
    match verdict {
        PreFilterVerdict::ForceWeb => {
            let queries = if classified.queries.is_empty() {
                vec![classified.standalone_question.clone()]
            } else {
                classified.queries
            };
            let decision = match classified.decision {
                SearchDecision::Cached => SearchDecision::Cached,
                SearchDecision::No | SearchDecision::Web => SearchDecision::Web,
            };
            PrePassDecision {
                decision,
                // Keep the classifier's route hint: ForceWeb overrides only the
                // yes/no decision, not which source tier best answers the turn.
                route: classified.route,
                standalone_question: classified.standalone_question,
                queries,
                // Preserve the explicit look-it-up signal: a ForceWeb turn can
                // still be an explicit "look it up" request, and dropping it
                // here would silently re-enable the fast paths it must skip.
                explicit_search: classified.explicit_search,
                // ForceWeb overrides the yes/no decision, never what language the
                // user wrote in: that is an observation, not a decision.
                lang: classified.lang,
            }
        }
        // Ambiguous honours the classifier verbatim; ForceNo never reaches here
        // (it short-circuits before the model call).
        PreFilterVerdict::Ambiguous | PreFilterVerdict::ForceNo => classified,
    }
}

/// Engines-only path for an explicit user force (`/search` alias). Runs the
/// classifier for rewrite quality when possible; on infra failure uses the raw
/// message. Always sets `explicit_search` and `Web` so cache and verticals are
/// skipped (same contract as a classifier `explicit_search: true` decision).
#[allow(clippy::too_many_arguments)]
async fn force_explicit_web(
    deps: &SearchDeps<'_>,
    chat_system_prompt: &str,
    history: &[ChatMessage],
    latest_user: &str,
    num_ctx: u32,
    today: &str,
    locale: &str,
    cancel: &CancellationToken,
    status: &(dyn Fn(SearchPhase) + Send + Sync),
) -> SearchOutcome {
    status(SearchPhase::Deciding);
    let classifier_start = Instant::now();
    let classified = match deps
        .prepass
        .decide(history, latest_user, deps.latest_images, today, cancel)
        .await
    {
        Ok(classified) => {
            deps.timings.record(STAGE_CLASSIFIER, classifier_start);
            classified
        }
        Err(InferenceError::Cancelled) => {
            deps.timings.record(STAGE_CLASSIFIER, classifier_start);
            return SearchOutcome::Cancelled;
        }
        Err(InferenceError::Request(reason)) => {
            deps.timings.record(STAGE_CLASSIFIER, classifier_start);
            eprintln!("[search] force_search classifier error: {reason} -> raw query");
            PrePassDecision {
                decision: SearchDecision::Web,
                route: SearchRoute::Web,
                standalone_question: latest_user.trim().to_string(),
                queries: vec![latest_user.trim().to_string()],
                explicit_search: true,
                // No classifier output, so no language judgement (see the same
                // fallback in `run_search_inner`).
                lang: String::new(),
            }
        }
    };
    let lang = resolve_lang(latest_user, &classified.lang, locale);
    // Same classifier-hallucination clamp as the auto path: bare `SJC` must not
    // become San Jose airport weather under `/search` either.
    let classified = sanitize_search_decision(latest_user, classified);
    let queries = if classified.queries.is_empty() {
        vec![classified.standalone_question.clone()]
    } else {
        classified.queries
    };
    let standalone = if classified.standalone_question.trim().is_empty() {
        latest_user.trim().to_string()
    } else {
        classified.standalone_question
    };
    eprintln!(
        "[search] force_search route={:?} queries={}",
        classified.route,
        queries.len()
    );
    deps.recorder.record(RecorderEvent::SearchDecided {
        prefilter: "force_search".to_string(),
        decision: "web".to_string(),
        force: true,
        route: route_label(classified.route).to_string(),
        standalone_question: standalone.clone(),
        queries: queries.clone(),
    });
    run_web(
        deps,
        chat_system_prompt,
        history,
        latest_user,
        classified.route,
        &standalone,
        &queries,
        num_ctx,
        today,
        locale,
        lang,
        cancel,
        status,
        true, /* explicit_search: skip cache + verticals */
        // `/search` force path does not race the raw query (no ForceWeb
        // prefilter gate); always SERP the classifier rewrite.
        None,
    )
    .await
}

/// The `cached` decision's gated reuse arm: reuse the conversation's stored
/// sources when they actually answer the follow-up, else escalate to a fresh
/// web search in the same turn. The `cached` classifier decision is only a
/// hint; this function is where it is verified against real evidence.
///
/// The gate:
/// 1. Eligibility. Reuse runs only for the general `web` and `wiki` routes. A
///    weather, news, or sports `cached` decision bypasses reuse entirely and
///    flows straight through the fresh path (as a `web` decision would), so its
///    live vertical always runs: those tiers answer volatile questions stored
///    sources must never re-serve.
/// 2. Evidence. The live cache entries for the turn's scope whose OWN producing
///    route was a stable tier (web or wiki) are unioned, most recent first, and
///    re-budgeted to the context allowance via [`merge_sources`] (the oldest
///    tail that does not fit is dropped). Entries produced by a volatile
///    vertical (weather, news, sports) are excluded, since their content was
///    fetched for a live question. An empty cache, or one with no stable-tier
///    entry, escalates.
/// 3. Grounding judge. The same [`crate::websearch::judge::SufficiencyJudge`]
///    the vertical fast paths use decides whether the union carries the answer.
///    Its prompt fences the untrusted source text exactly as the writer's does,
///    so reused web content flows through the same spotlighting as a fresh
///    fetch. A sufficient verdict commits the reuse (a `cache`-tier grounded
///    answer, no retrieval); an insufficient verdict or a judge transport
///    failure both escalate; only a cancellation short-circuits. The bias is
///    always toward a fresh search, never toward a weakly grounded reply.
///
/// An ineligible route, an empty cache, an insufficient reuse, and a judge
/// failure all converge on one identical fresh-search call (the classifier's own
/// route, rewrite, and queries, no second classification), so the return value
/// is indistinguishable from a plain `web`-decision turn on every escalation
/// path.
#[allow(clippy::too_many_arguments)]
async fn reuse_or_escalate(
    deps: &SearchDeps<'_>,
    chat_system_prompt: &str,
    history: &[ChatMessage],
    latest_user: &str,
    route: SearchRoute,
    standalone_question: &str,
    queries: &[String],
    num_ctx: u32,
    today: &str,
    locale: &str,
    lang: &str,
    cancel: &CancellationToken,
    status: &(dyn Fn(SearchPhase) + Send + Sync),
    preloaded_serp: Option<(Vec<SearchHit>, Vec<EngineStat>)>,
) -> SearchOutcome {
    if matches!(route, SearchRoute::Web | SearchRoute::Wiki) {
        // Reuse only entries whose OWN producing route was a stable tier (web or
        // wiki). A stored source produced by a volatile vertical (weather, news,
        // sports) must never ground a later turn even on an eligible route: it
        // was fetched to answer a live question and may already be stale. The
        // provenance route recorded on each entry (see `CachedSearch::route`) is
        // what makes this exclusion possible.
        let eligible: Vec<CachedSearch> = deps
            .cache
            .entries(deps.cache_scope)
            .into_iter()
            .filter(|entry| matches!(entry.route, SearchRoute::Web | SearchRoute::Wiki))
            .collect();
        if !eligible.is_empty() {
            // Fold the entries newest-first: `merge_sources` keeps its first
            // argument whole and renumbers the combined list, so accumulating
            // from the most recent entry drops the oldest sources first when the
            // union exceeds the context budget.
            let mut union: Vec<SourceBlock> = Vec::new();
            for entry in eligible {
                union = merge_sources(union, entry.sources, num_ctx);
            }
            let judge_start = Instant::now();
            let verdict = deps.judge.judge(standalone_question, &union, cancel).await;
            deps.timings.record(STAGE_JUDGE, judge_start);
            match verdict {
                Ok(v) if v.sufficient => {
                    eprintln!("[search] cache reuse sufficient -> serving from stored sources");
                    // Records the reuse commit on the same escalation trace the
                    // verticals use, with the synthetic tier "cache": sufficient,
                    // not escalated. `grounded_answer` then emits the terminal
                    // SearchRetrieved{tier:"cache"} and skips the cache write (a
                    // reuse is not a new search).
                    deps.recorder.record(RecorderEvent::SearchEscalated {
                        from_tier: "cache".to_string(),
                        sufficient: true,
                        missing: String::new(),
                        escalated: false,
                        escalation_hit: false,
                    });
                    return grounded_answer(
                        deps,
                        "cache",
                        chat_system_prompt,
                        history,
                        latest_user,
                        standalone_question,
                        union,
                        today,
                        locale,
                        lang,
                        Vec::new(),
                        // A cache reuse is never a conflict commit: the reuse
                        // judge is a plain sufficiency check, not the engine
                        // tier's conflict-aware judge.
                        false,
                        None,
                    );
                }
                Ok(v) => {
                    eprintln!(
                        "[search] cache reuse insufficient (missing: {}) -> fresh search",
                        v.missing
                    );
                    // `escalated: true` records that the turn left the cache for
                    // a fresh search; `escalation_hit` stays false here because
                    // this event is emitted at the gate, before the fresh search
                    // resolves. That search records its own SearchRetrieved.
                    deps.recorder.record(RecorderEvent::SearchEscalated {
                        from_tier: "cache".to_string(),
                        sufficient: false,
                        missing: v.missing,
                        escalated: true,
                        escalation_hit: false,
                    });
                }
                Err(InferenceError::Cancelled) => return SearchOutcome::Cancelled,
                // A judge transport failure fails toward a FRESH search, the
                // opposite of the vertical fast path's fail-toward-commit: here
                // committing would mean serving a reply the gate could not
                // actually vouch for, and a fresh search is always safe.
                Err(InferenceError::Request(reason)) => {
                    eprintln!("[search] cache reuse judge error: {reason} -> fresh search");
                    deps.recorder.record(RecorderEvent::SearchEscalated {
                        from_tier: "cache".to_string(),
                        sufficient: false,
                        missing: String::new(),
                        escalated: true,
                        escalation_hit: false,
                    });
                }
            }
        }
    }
    run_web(
        deps,
        chat_system_prompt,
        history,
        latest_user,
        route,
        standalone_question,
        queries,
        num_ctx,
        today,
        locale,
        lang,
        cancel,
        status,
        false,
        preloaded_serp,
    )
    .await
}

/// Maps a retrieval tier label (the answering source, as passed to
/// [`grounded_answer`]) to the [`SearchRoute`] recorded on the cache entry that
/// answer produces. The general scraped-engine tier ("engine"), and the cache
/// tier itself, map to [`SearchRoute::Web`]; every keyless vertical maps to its
/// own route. Total by construction (an unknown tier is the general engine
/// route), so a new tier label never fails the store.
fn route_for_tier(tier: &str) -> SearchRoute {
    match tier {
        "weather" => SearchRoute::Weather,
        "news" => SearchRoute::News,
        "wiki" => SearchRoute::Wiki,
        "sports" => SearchRoute::Sports,
        _ => SearchRoute::Web,
    }
}

/// The `web`/`cached` branch: search every query, fetch and rank the pages,
/// assemble a budgeted source set, and build the writer prompt. Degrades to
/// [`SearchOutcome::NoSearch`] when nothing citable survives and to
/// [`SearchOutcome::Cancelled`] on cancellation.
///
/// `engines_only` is `true` exactly when the caller reached this fn via an
/// explicit look-it-up request (see the `decision.explicit_search` branch in
/// [`run_search`], its only call site with `true`); besides skipping every
/// vertical below, it is forwarded to [`run_engine_tier`] as its cache-bypass
/// signal, so an explicit re-search is never silently re-served a SERP or page
/// pulled from the in-memory cache within its TTL. The cache is still written
/// through on a fresh fetch either way, so the entry the user just distrusted
/// is replaced rather than left to keep answering later, non-explicit turns.
///
/// `lang` is the turn's language, resolved once by the caller from the user's
/// ORIGINAL message (see [`crate::websearch::lang::resolve_lang`]) and forwarded
/// to every channel with a language-dependent request shape: the weather
/// geocoder, the news feed's locale triple, the Wikipedia edition, and both
/// scraped engines. No stage below re-derives it.
#[allow(clippy::too_many_arguments)]
async fn run_web(
    deps: &SearchDeps<'_>,
    chat_system_prompt: &str,
    history: &[ChatMessage],
    latest_user: &str,
    route: SearchRoute,
    standalone_question: &str,
    queries: &[String],
    num_ctx: u32,
    today: &str,
    locale: &str,
    lang: &str,
    cancel: &CancellationToken,
    status: &(dyn Fn(SearchPhase) + Send + Sync),
    engines_only: bool,
    // ForceWeb raw-query race hits to reuse when the rewrite matched; `None`
    // runs the normal SERP loop inside `run_engine_tier`.
    preloaded_serp: Option<(Vec<SearchHit>, Vec<EngineStat>)>,
) -> SearchOutcome {
    if cancel.is_cancelled() {
        return SearchOutcome::Cancelled;
    }
    status(SearchPhase::Searching);
    // Whether the standalone question carries a freshness signal (reusing the
    // Wikipedia vertical's volatility guard as the freshness detector, see
    // `encyclopedia::is_volatile_question`). Computed once and reused below to
    // gate the wiki tier AND to bias the news feed and scraped-engine queries
    // toward recent results, since a needless date filter costs nothing on a
    // stable question but a missing one on a volatile one risks a stale answer.
    let freshness = is_volatile_question(standalone_question);
    // An explicit look-it-up request (`engines_only`) skips *cached* and most
    // vertical fast paths: the user already asked to re-check. Weather is the
    // exception: Open-Meteo is live structured truth, not a stale cache, and
    // SEO weather widgets invent impossible RH / wrong cities. On any weather
    // intent (user or classifier route), try Open-Meteo and, on a miss, refuse
    // with NoResults rather than scraping engines.
    //
    // Weather keeps its own precise gate (location extraction inside
    // `fetch_weather` self-gates: a non-weather question yields no location and
    // returns `None`).
    let weather_turn =
        is_weather_intent(standalone_question) || matches!(route, SearchRoute::Weather);
    if weather_turn {
        if let Some(block) = fetch_weather(deps.transport, standalone_question, lang).await {
            return commit_or_escalate(
                deps,
                "weather",
                block,
                chat_system_prompt,
                history,
                latest_user,
                standalone_question,
                queries,
                num_ctx,
                today,
                locale,
                lang,
                freshness,
                cancel,
                status,
            )
            .await;
        }
        // Honour cancel that landed during the Open-Meteo attempt before the
        // honest-refuse path; otherwise a mid-geocode cancel would surface as
        // NoResults instead of Cancelled.
        if cancel.is_cancelled() {
            return SearchOutcome::Cancelled;
        }
        eprintln!("[search] weather exclusive miss -> WeatherUnavailable (no SEO fallback)");
        return SearchOutcome::Unreachable {
            messages: unreachable_messages(
                chat_system_prompt,
                history,
                latest_user,
                deps.latest_images,
                SearchFailReason::WeatherUnavailable,
            ),
            reason: SearchFailReason::WeatherUnavailable,
        };
    }
    // Cancel already checked at `run_web` entry and inside the weather-miss
    // path above; nothing async runs between those and here on a non-weather
    // turn, so a third cancel probe would be dead.
    // Route-respect hint gating (see `hint_claims_turn`): the classifier's
    // explicit route outranks a vertical's deterministic keyword hint. A hint
    // only claims a turn the model routed to that same vertical, or to the
    // general Web tier (a classifier miss the hint can rescue), plus the one
    // deliberate cross-tier upgrade below.
    //
    // Sports tier: ESPN's public scoreboard API, positioned ahead of the news
    // feed because a scoreboard beats headlines for a live score/fixture/
    // standings question (the feed's dated headlines can lag the live state by
    // minutes). Runs when the classifier routed to sports, OR its league-keyword
    // hint matches on a News/Web-routed turn: the news->sports upgrade is kept on
    // purpose (a score-shaped question the model called "news" is still better
    // served by a scoreboard), but the hint must never hijack a weather- or
    // wiki-routed turn. `fetch_sports` self-gates on the league match internally,
    // so a route hit with no keyword match falls through cleanly with a logged
    // reason. A miss falls through to the news feed / engines.
    let sports_hint = is_sports_intent(standalone_question)
        && matches!(route, SearchRoute::News | SearchRoute::Web);
    if !engines_only && (matches!(route, SearchRoute::Sports) || sports_hint) {
        if let Some(block) =
            fetch_sports(deps.transport, standalone_question, today, deps.local_zone).await
        {
            return commit_or_escalate(
                deps,
                "sports",
                block,
                chat_system_prompt,
                history,
                latest_user,
                standalone_question,
                queries,
                num_ctx,
                today,
                locale,
                lang,
                freshness,
                cancel,
                status,
            )
            .await;
        }
    }
    if cancel.is_cancelled() {
        return SearchOutcome::Cancelled;
    }
    // News tier: keyless Google News RSS, not SERP-bot-gated, whose dated
    // headlines answer the who-won / what-happened / latest-status class
    // directly. Runs when the classifier routed to news, OR its token hint
    // matches on a Web-routed turn ONLY: news must never claim a turn the model
    // routed to Sports/Wiki/Weather. This is the fix for the observed F1-points
    // dead end, where route=sports but the "race"/"championship" token let news
    // steal the turn and dead-end on headlines that had no standings. Each
    // classifier query is tried in order until one yields a block (mirroring the
    // engine tier), so a first query that returns nothing no longer strands the
    // turn on an empty first result. Queries are biased toward recent results
    // when the standalone question carried a freshness signal.
    let news_hint = is_news_intent(standalone_question) && matches!(route, SearchRoute::Web);
    if !engines_only && (matches!(route, SearchRoute::News) || news_hint) {
        for query in queries {
            if let Some(block) = fetch_news(deps.transport, query, freshness, lang).await {
                return commit_or_escalate(
                    deps,
                    "news",
                    block,
                    chat_system_prompt,
                    history,
                    latest_user,
                    standalone_question,
                    queries,
                    num_ctx,
                    today,
                    locale,
                    lang,
                    freshness,
                    cancel,
                    status,
                )
                .await;
            }
        }
    }
    if cancel.is_cancelled() {
        return SearchOutcome::Cancelled;
    }
    // Wikipedia tier: runs ONLY when the classifier routed to wiki AND the
    // deterministic volatility guard passes. Wikipedia's lead summary answers a
    // stable subject, never its live state, so a volatile question (a freshness
    // marker or a present/future year) must never be served from it. The
    // vertical itself applies a second, year-mismatch guard after resolving the
    // article title. A miss or a refusal falls through to the engines.
    if !engines_only && matches!(route, SearchRoute::Wiki) && !freshness {
        if let Some(block) = fetch_encyclopedia(deps.transport, standalone_question, lang).await {
            return commit_or_escalate(
                deps,
                "wiki",
                block,
                chat_system_prompt,
                history,
                latest_user,
                standalone_question,
                queries,
                num_ctx,
                today,
                locale,
                lang,
                freshness,
                cancel,
                status,
            )
            .await;
        }
    }
    if cancel.is_cancelled() {
        return SearchOutcome::Cancelled;
    }
    // The general scraped-engine tier: the terminal source, so there is
    // nothing further to escalate to. Its own sufficiency judge still runs on
    // a Ranked result (see `judge_and_requery`), with one bounded requery on
    // an insufficient verdict; the writer's graceful-partial contract covers
    // whatever the result still lacks after that. A miss is the pipeline's
    // honest could-not-verify disclosure, never a silent stale answer.
    match run_engine_tier(
        deps,
        standalone_question,
        queries,
        num_ctx,
        freshness,
        lang,
        // `engines_only` is set exactly when this call is reached via an
        // explicit look-it-up request (see the module-level comment above the
        // `decision.explicit_search` branch in `run_search`, the only place
        // `run_web` is ever called with `engines_only = true`), so it doubles
        // as this tier's cache-bypass signal.
        engines_only,
        cancel,
        status,
        preloaded_serp,
    )
    .await
    {
        EngineTierOutcome::Cancelled => SearchOutcome::Cancelled,
        EngineTierOutcome::Empty { reason } => SearchOutcome::Unreachable {
            messages: unreachable_messages(
                chat_system_prompt,
                history,
                latest_user,
                deps.latest_images,
                reason,
            ),
            reason,
        },
        EngineTierOutcome::Ranked {
            sources,
            engine_stats,
            chunks,
            pages,
        } => {
            // Round one produced sources: run the engine tier's own sufficiency
            // judge and one bounded requery over them (see `judge_and_requery`)
            // before answering. The requery, when it fires, races the keyless
            // engines again, so its per-engine outcomes are folded into the same
            // `SearchRetrieved` trace `engine_stats` the primary query populated
            // (additive, so one retrieval record shows both rounds' engines).
            //
            // This is the engine-tier requery merge: pass round one's chunks and
            // pages so the merge re-ranks round one and round two together by
            // fused score (`Some` rerank) rather than pinning round one ahead.
            match judge_and_requery(
                deps,
                standalone_question,
                sources,
                Some(RequeryRerank {
                    round_one_chunks: chunks,
                    round_one_pages: pages,
                }),
                num_ctx,
                freshness,
                lang,
                engines_only,
                cancel,
            )
            .await
            {
                EngineJudgeOutcome::Cancelled => SearchOutcome::Cancelled,
                EngineJudgeOutcome::Sources(sources, requery_stats, conflict, still_missing) => {
                    let mut engine_stats = engine_stats;
                    engine_stats.extend(requery_stats);
                    grounded_answer(
                        deps,
                        "engine",
                        chat_system_prompt,
                        history,
                        latest_user,
                        standalone_question,
                        sources,
                        today,
                        locale,
                        lang,
                        engine_stats,
                        conflict,
                        still_missing,
                    )
                }
            }
        }
    }
}

/// What the general scraped-engine tier produced. Distinguishes a user cancel
/// from an honest empty result so both the normal `web` path and an escalation
/// from an insufficient vertical can map it to the right outcome.
enum EngineTierOutcome {
    /// The user cancelled mid-retrieval.
    Cancelled,
    /// Every engine was blocked or empty, or nothing survived ranking: there is
    /// nothing citable. `reason` records whether the tier never reached the web
    /// (every contacted engine transport-failed) or reached it and found nothing
    /// usable, derived from the per-engine outcome summary (see
    /// [`transport_unreachable`]).
    Empty { reason: SearchFailReason },
    /// The budgeted, ranked source blocks ready for the writer, plus the
    /// byproducts a downstream requery merge needs.
    Ranked {
        /// The budgeted, ranked source blocks ready for the writer.
        sources: Vec<SourceBlock>,
        /// The per-query, per-engine outcome summary (see [`EngineStat`])
        /// collected across every [`web_search`] call this tier made, for the
        /// caller to forward into [`RecorderEvent::SearchRetrieved`].
        engine_stats: Vec<EngineStat>,
        /// The ranked chunks `sources` was assembled from, kept so the
        /// engine-tier requery merge can re-rank round one and round two
        /// together by fused score instead of pinning round one ahead (see
        /// [`judge_and_requery`]'s [`RequeryRerank`]). The vertical-escalation
        /// caller ignores it (its merge stays vertical-first pinned).
        chunks: Vec<ScoredChunk>,
        /// The fetched pages `chunks` came from, kept for their extracted
        /// publish dates so the requery merge's freshness-gated recency fusion
        /// can score round-one URLs, not just round-two ones.
        pages: Vec<FetchedPage>,
    },
}

/// Runs the general scraped-engine tier for `queries`: for each query race the
/// keyless engines and rank-fuse their results (skipping any inside their
/// cooldown), dedupe hits across queries, stop early once enough are gathered,
/// then fetch, rank, and budget the pages into source
/// blocks. Shared by the normal `web` path and by [`commit_or_escalate`] when an
/// insufficient vertical block escalates, so escalation inherits the exact same
/// cooldown-skip and early-stop volume controls the engines' rate limits
/// require. Emits [`SearchPhase::Reading`] before the page fetch; the caller
/// owns the [`SearchPhase::Searching`] phase (already emitted once per turn).
///
/// `bypass_cache` is forwarded verbatim to both [`web_search`] and
/// [`fetch_pages`]: it is the read-bypass, write-through contract for an
/// explicit user re-search (see their docs). Set only when this tier is
/// reached because the user explicitly asked to re-check a result (see
/// [`run_web`]'s `engines_only` parameter, its only source today); a
/// judge-driven escalation from [`commit_or_escalate`] is not a user distrust
/// signal, so that call site always passes `false` and a cached-but-fresh
/// result is fine there.
/// Maps the scraped-engine tier's per-engine outcome summary to the
/// [`SearchFailReason`] for an empty result: [`SearchFailReason::Unreachable`]
/// when every contacted engine transport-failed, else
/// [`SearchFailReason::NoResults`]. The transport-failure test lives in
/// [`transport_unreachable`] (it owns the engine-status vocabulary); this thin
/// wrapper only turns that verdict into the outcome enum, kept separate so both
/// the derivation and the mapping stay directly unit-tested.
fn empty_reason(engine_stats: &[EngineStat]) -> SearchFailReason {
    if transport_unreachable(engine_stats) {
        SearchFailReason::Unreachable
    } else {
        SearchFailReason::NoResults
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_engine_tier(
    deps: &SearchDeps<'_>,
    standalone_question: &str,
    queries: &[String],
    num_ctx: u32,
    freshness: bool,
    lang: &str,
    bypass_cache: bool,
    cancel: &CancellationToken,
    status: &(dyn Fn(SearchPhase) + Send + Sync),
    preloaded_serp: Option<(Vec<SearchHit>, Vec<EngineStat>)>,
) -> EngineTierOutcome {
    // Per-(query, engine) outcome summary accumulated across every web_search
    // call this tier makes, for the caller to surface in the trace (see
    // `EngineTierOutcome::Ranked`'s docs). Collected even on a path that ends
    // up `Empty`-content-wise being discarded with the outcome: only the
    // `Ranked` arm below actually forwards it, since that is the only path a
    // `SearchRetrieved` event gets recorded for this tier.
    //
    // ForceWeb race: when `preloaded_serp` is `Some`, the SERP already ran
    // concurrent with the classifier on the raw (near-duplicate first) query.
    // Seed hits/stats from that race, then still SERP remaining queries so a
    // multi-query classifier fan-out is not dropped on the keep path.
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut engine_stats: Vec<EngineStat> = Vec::new();
    let queries_to_serp: &[String] = if let Some((pre_hits, pre_stats)) = preloaded_serp {
        hits = pre_hits;
        engine_stats = pre_stats;
        if queries.len() > 1 {
            &queries[1..]
        } else {
            &[]
        }
    } else {
        queries
    };
    // Skip the SERP loop when race alone already hit early-stop, or when the
    // keep path has no remaining queries (single near-dupe rewrite).
    if !queries_to_serp.is_empty() && hits.len() < SERP_EARLY_STOP_HITS {
        // Offline fast-fail races the WHOLE remaining SERP round (not each
        // query): an offline turn runs every remaining query (no hit → early
        // stop never fires), and a per-query race would cost one grace window
        // per query. `biased` polls the real round first so a round that
        // completes wins even against a cutoff due in the same poll; taking
        // the cutoff arm DROPS in-flight requests so a late engine cannot
        // contradict the disclosure. Only proven-unreachable short-circuits
        // (see `offline_cutoff`); Unknown/hang never does. Only the SERP
        // round is raced, not page fetch: past that point an engine already
        // answered, so the device is demonstrably online.
        let serp_start = Instant::now();
        // Seed early-stop accounting with any ForceWeb-preloaded hits so a
        // partial race seed still stops the remaining loop at the same
        // threshold as the non-offline path.
        let preloaded_hit_count = hits.len();
        let round = async {
            let mut round_hits: Vec<SearchHit> = Vec::new();
            let mut round_stats: Vec<EngineStat> = Vec::new();
            let mut total_hits = preloaded_hit_count;
            for query in queries_to_serp {
                if cancel.is_cancelled() {
                    return None;
                }
                let (query_hits, query_stats) = web_search(
                    deps.transport,
                    query,
                    deps.health,
                    freshness,
                    lang,
                    deps.web_cache,
                    bypass_cache,
                )
                .await;
                total_hits += query_hits.len();
                round_hits.extend(query_hits);
                round_stats.extend(query_stats);
                // Early stop: once one query has produced enough hits, further
                // queries add third-party burst (the engines' rate limits are
                // volume-triggered) and latency for marginal recall.
                if total_hits >= SERP_EARLY_STOP_HITS {
                    break;
                }
            }
            Some((round_hits, round_stats))
        };
        let (round_hits, round_stats) = tokio::select! {
            biased;
            round = round => match round {
                Some(round) => round,
                None => {
                    deps.timings.record(STAGE_SERP, serp_start);
                    return EngineTierOutcome::Cancelled;
                }
            },
            () = offline_cutoff(deps.reachability) => {
                eprintln!(
                    "[search] offline short-circuit: reachability probe proved no network path"
                );
                deps.timings.record(STAGE_SERP, serp_start);
                // Straight into the existing unreachable path: same outcome the
                // stacked-timeout route reached ~46 s later (see
                // `engine::transport_unreachable`), just honest about it in ~1 s.
                return EngineTierOutcome::Empty {
                    reason: SearchFailReason::Unreachable,
                };
            }
        };
        hits.extend(round_hits);
        engine_stats.extend(round_stats);
        deps.timings.record(STAGE_SERP, serp_start);
    }
    let hits = dedupe_hits(hits);
    if hits.is_empty() {
        // No engine yielded a hit. The reason turns on WHY every engine drew a
        // blank: an all-transport-error round is "unreachable", any HTTP
        // response (empty/blocked) is "found nothing" (see
        // `transport_unreachable`).
        return EngineTierOutcome::Empty {
            reason: empty_reason(&engine_stats),
        };
    }
    if cancel.is_cancelled() {
        return EngineTierOutcome::Cancelled;
    }
    status(SearchPhase::Reading);
    // Honor cancellation DURING the fetch, not only at the pre-fetch check above:
    // the fetch stage can wait up to `FETCH_SOFT_DEADLINE_MS`, so a cancel that
    // lands mid-fetch would otherwise be ignored for that whole window. Racing the
    // await against the token (biased, matching `openai::request_openai_json`)
    // drops the in-flight `FuturesUnordered` the instant the token trips, which
    // cancels every still-pending page fetch cleanly.
    let fetch_start = Instant::now();
    let pages = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            deps.timings.record(STAGE_FETCH, fetch_start);
            return EngineTierOutcome::Cancelled;
        }
        pages = fetch_pages(
            deps.transport,
            &hits,
            num_ctx,
            freshness,
            deps.web_cache,
            bypass_cache,
        ) => pages,
    };
    deps.timings.record(STAGE_FETCH, fetch_start);
    let rank_start = Instant::now();
    let chunks = select_chunks(&pages, standalone_question, deps.scorer);
    // Freshness-gated recency-prior fusion (see `recency::recency_reorder`):
    // only reorders sources on a turn the freshness signal already flagged, so
    // a non-fresh turn pays zero extra cost and its ranking is untouched.
    let chunks = if freshness {
        recency_reorder(&chunks, &pages, time::OffsetDateTime::now_utc())
    } else {
        chunks
    };
    // Evidence bar for live-price / freshness turns: drop multi-year archive
    // URL paths and, on price intent, require price-like numbers (else empty
    // → NoResults refuse rather than confident scrapes). See `evidence`.
    let price_intent = is_price_intent_question(standalone_question);
    let now_year = time::OffsetDateTime::now_utc().year() as u32;
    let chunks = filter_evidence_chunks(chunks, freshness, price_intent, now_year);
    let sources = assemble_context(&chunks, num_ctx);
    deps.timings.record(STAGE_RANK_ASSEMBLY, rank_start);
    if sources.is_empty() {
        // Engines returned hits (this path is past the `hits.is_empty()` guard)
        // but nothing survived fetch, ranking, and budgeting: the web was
        // reached, so `empty_reason` resolves this to `NoResults`.
        return EngineTierOutcome::Empty {
            reason: empty_reason(&engine_stats),
        };
    }
    // `chunks` and `pages` ride along so the engine-tier requery merge can fuse
    // round one and round two by score (see `judge_and_requery`); the
    // vertical-escalation caller drops them.
    EngineTierOutcome::Ranked {
        sources,
        engine_stats,
        chunks,
        pages,
    }
}

/// The judge gate applied to every keyless vertical answer: decide whether the
/// vertical's `block` actually contains what the question asked, and either
/// commit it or escalate to the scraped engines.
///
/// A vertical returning a block is not the same as the block answering the
/// question (a scoreboard carries today's fixture, not the full bracket; a news
/// feed carries headlines, not a specific figure). Before this gate an
/// irrelevant vertical block was committed unconditionally and the writer, right
/// to refuse to invent, produced a bare "the sources do not contain that" dead
/// end. Here the [`SufficiencyJudge`] reads that verdict one call earlier:
/// - **sufficient** (or a judge failure, which fails toward committing): answer
///   from the vertical block, exactly as before.
/// - **insufficient, an engine available**: run the scraped-engine tier and, on
///   a hit, answer from the vertical block merged with those sources (merge,
///   not replace: an insufficient block is still partially relevant, and
///   dropping it loses data the engines may lack — see [`merge_sources`]). On
///   an engine miss, fall back to the vertical block as a partial answer (the
///   writer caveats what is missing) rather than a wall.
/// - **insufficient, every engine cooling**: escalation is futile and, on the
///   burst that caused the cooldowns, would risk deepening the block, so serve
///   the vertical block as a partial answer directly.
///
/// Every path records a [`RecorderEvent::SearchEscalated`] so a trace shows the
/// judge's verdict and what the orchestrator did with it.
#[allow(clippy::too_many_arguments)]
async fn commit_or_escalate(
    deps: &SearchDeps<'_>,
    tier: &str,
    block: SourceBlock,
    chat_system_prompt: &str,
    history: &[ChatMessage],
    latest_user: &str,
    standalone_question: &str,
    queries: &[String],
    num_ctx: u32,
    today: &str,
    locale: &str,
    lang: &str,
    freshness: bool,
    cancel: &CancellationToken,
    status: &(dyn Fn(SearchPhase) + Send + Sync),
) -> SearchOutcome {
    // Deterministic pre-check first (see `judge::deterministic_sufficiency`): an
    // empty vertical block is mechanically insufficient and a populated weather
    // block is mechanically sufficient, both decided in pure code with no model
    // call. Only the ambiguous middle (a populated scoreboard, news feed, or wiki
    // summary) falls through to the LLM judge, so the pre-check REPLACES the
    // judge call rather than adding one: the per-turn LLM-call budget never rises.
    let verdict = match deterministic_sufficiency(tier, &block) {
        Some(verdict) => verdict,
        None => {
            let judge_start = Instant::now();
            let result = deps
                .judge
                .judge(standalone_question, std::slice::from_ref(&block), cancel)
                .await;
            deps.timings.record(STAGE_JUDGE, judge_start);
            match result {
                Ok(verdict) => verdict,
                Err(InferenceError::Cancelled) => return SearchOutcome::Cancelled,
                // A judge infra failure fails toward committing (see the judge module
                // docs): serve the vertical block rather than stall the user or spend
                // an engine request on a decision the judge could not actually make.
                Err(InferenceError::Request(reason)) => {
                    eprintln!("[search] judge error: {reason} -> committing {tier} block");
                    SufficiencyVerdict::commit()
                }
            }
        }
    };
    if verdict.sufficient {
        eprintln!("[search] {tier} sufficient -> committing");
        deps.recorder.record(RecorderEvent::SearchEscalated {
            from_tier: tier.to_string(),
            sufficient: true,
            missing: String::new(),
            escalated: false,
            escalation_hit: false,
        });
        return grounded_answer(
            deps,
            tier,
            chat_system_prompt,
            history,
            latest_user,
            standalone_question,
            vec![block],
            today,
            locale,
            lang,
            Vec::new(),
            // The vertical judge (single block) never yields a conflict verdict.
            false,
            None,
        );
    }
    eprintln!(
        "[search] {tier} insufficient (missing: {}) -> escalating to engines",
        verdict.missing
    );
    // Every engine cooling: escalation is futile, serve the vertical block as a
    // partial answer (the writer caveats what is missing) rather than a wall.
    if !any_engine_available(deps.health) {
        eprintln!("[search] all engines cooling -> serving {tier} partial");
        deps.recorder.record(RecorderEvent::SearchEscalated {
            from_tier: tier.to_string(),
            sufficient: false,
            missing: verdict.missing,
            escalated: false,
            escalation_hit: false,
        });
        return grounded_answer(
            deps,
            tier,
            chat_system_prompt,
            history,
            latest_user,
            standalone_question,
            vec![block],
            today,
            locale,
            lang,
            Vec::new(),
            // The vertical judge (single block) never yields a conflict verdict.
            false,
            None,
        );
    }
    // Prefer judge-authored gap-targeted keyword queries when escalating a
    // vertical miss: the classifier's original queries already produced the
    // related-but-wrong block; reusing them only re-ranks the same cluster.
    // Fall back to the classifier queries when the judge omitted requery_queries.
    let escalation_queries: Vec<String> = if !verdict.requery_queries.is_empty() {
        verdict.requery_queries.clone()
    } else {
        queries.to_vec()
    };
    match run_engine_tier(
        deps,
        standalone_question,
        &escalation_queries,
        num_ctx,
        freshness,
        lang,
        // A judge-driven escalation is not a user distrust signal (the user
        // never asked to re-check anything this turn), so a cached-but-fresh
        // result is fine here: never bypass on this path.
        false,
        cancel,
        status,
        // Escalation never reuses a ForceWeb raw-query race: vertical miss
        // paths always SERP the rewrite queries fresh.
        None,
    )
    .await
    {
        EngineTierOutcome::Cancelled => SearchOutcome::Cancelled,
        // The engines answered: merge the vertical block with their sources
        // rather than discard it (see `merge_sources`), then run the
        // engine-tier's own sufficiency judge and bounded requery over the
        // merged result (see `judge_and_requery`).
        // The engines' own chunks and pages are ignored here: this merge pins
        // the vertical block ahead of the engines (contract locked), so it never
        // takes the engine-tier requery merge's fused re-rank (`None` rerank).
        EngineTierOutcome::Ranked {
            sources,
            engine_stats,
            ..
        } => {
            deps.recorder.record(RecorderEvent::SearchEscalated {
                from_tier: tier.to_string(),
                sufficient: false,
                missing: verdict.missing.clone(),
                escalated: true,
                escalation_hit: true,
            });
            let merged = merge_sources(vec![block], sources, num_ctx);
            match judge_and_requery(
                deps,
                standalone_question,
                merged,
                None,
                num_ctx,
                freshness,
                lang,
                // A judge-driven escalation is not a user distrust signal, the
                // same rationale as the `run_engine_tier` call above: never
                // bypass the cache on this path.
                false,
                cancel,
            )
            .await
            {
                EngineJudgeOutcome::Cancelled => SearchOutcome::Cancelled,
                EngineJudgeOutcome::Sources(sources, requery_stats, conflict, still_missing) => {
                    let mut engine_stats = engine_stats;
                    engine_stats.extend(requery_stats);
                    grounded_answer(
                        deps,
                        "engine",
                        chat_system_prompt,
                        history,
                        latest_user,
                        standalone_question,
                        sources,
                        today,
                        locale,
                        lang,
                        engine_stats,
                        conflict,
                        still_missing,
                    )
                }
            }
        }
        // The engines came up empty too: fall back to the vertical block as a
        // partial answer rather than a wall. The empty reason is irrelevant
        // here (a grounded vertical block still answers), so it is dropped.
        EngineTierOutcome::Empty { .. } => {
            deps.recorder.record(RecorderEvent::SearchEscalated {
                from_tier: tier.to_string(),
                sufficient: false,
                missing: verdict.missing,
                escalated: true,
                escalation_hit: false,
            });
            grounded_answer(
                deps,
                tier,
                chat_system_prompt,
                history,
                latest_user,
                standalone_question,
                vec![block],
                today,
                locale,
                lang,
                Vec::new(),
                // Engine-miss fallback to the vertical block: no conflict verdict.
                false,
                None,
            )
        }
    }
}

/// Records the retrieval tier to the trace, caches a freshly retrieved
/// source set for a later `cached` decision to reuse, and builds the
/// source-grounded [`SearchOutcome::Answer`]. `tier` is the source that
/// answered the turn ("weather", "sports", "news", "wiki", "engine", or
/// "cache"); the recorded URLs are the cited sources', so a trace shows
/// exactly what grounded the reply. `engine_stats` is the general
/// scraped-engine tier's per-query, per-engine outcome summary (see
/// [`EngineStat`]); callers answering from a vertical or the cache pass an
/// empty vec, since only the engine tier ever races the keyless engines.
///
/// The cache write is skipped for tier `"cache"` itself (an answer served
/// from the cache is not a new search, so it must not reset the entry's TTL
/// or overwrite it with the same sources).
///
/// `conflict` and `still_missing` are forwarded to the writer prompt (see
/// [`crate::websearch::writer::writer_messages`]): `conflict` is `true` only on
/// the engine tier's conflict commit (see [`judge_and_requery`]); `still_missing`
/// is set when a requery still could not surface the asked fact. Every other
/// caller passes `false` / `None`.
#[allow(clippy::too_many_arguments)]
fn grounded_answer(
    deps: &SearchDeps<'_>,
    tier: &str,
    chat_system_prompt: &str,
    history: &[ChatMessage],
    latest_user: &str,
    standalone_question: &str,
    sources: Vec<SourceBlock>,
    today: &str,
    locale: &str,
    lang: &str,
    engine_stats: Vec<EngineStat>,
    conflict: bool,
    still_missing: Option<String>,
) -> SearchOutcome {
    deps.recorder.record(RecorderEvent::SearchRetrieved {
        tier: tier.to_string(),
        sources: sources
            .iter()
            .map(|s| RetrievedSource {
                index: s.index,
                url: s.url.clone(),
                title: s.title.clone(),
                text: truncate_for_trace(&s.text, TRACE_SOURCE_TEXT_MAX_BYTES),
            })
            .collect(),
        engine_stats,
        // Always the turn's terminal record (see `RecorderEvent::SearchRetrieved`'s
        // rustdoc): the only round, or the post-merge result that follows a
        // round-one record `judge_and_requery` emits itself when a requery
        // fires (see its docs).
        round: None,
    });
    let is_cache_tier = tier == "cache";
    if !is_cache_tier {
        deps.cache.store(
            deps.cache_scope,
            CachedSearch {
                standalone_question: standalone_question.to_string(),
                sources: sources.clone(),
                // The route the answering tier maps to, recorded as provenance
                // on the entry (see `CachedSearch::route`).
                route: route_for_tier(tier),
            },
        );
    }
    let writer_start = Instant::now();
    let messages = writer_messages(
        chat_system_prompt,
        history,
        latest_user,
        &sources,
        today,
        locale,
        lang,
        is_cache_tier,
        conflict,
        still_missing.as_deref(),
        deps.latest_images,
    );
    deps.timings.record(STAGE_WRITER_PREPARE, writer_start);
    SearchOutcome::Answer { messages, sources }
}

/// Merges two source-block lists, `first` kept whole and ahead of `second`,
/// and re-assigns contiguous 1-based indices across the combined list.
///
/// `first`'s blocks are never dropped for budget reasons; only `second`'s can
/// be. Two call sites rely on that asymmetry: [`commit_or_escalate`] passes an
/// insufficient vertical's single judged-relevant block as `first` (an
/// insufficient verdict means the block does not fully answer the question,
/// not that it is irrelevant: it can still carry a fact the engines miss,
/// observed live with a sports block's exact kickoff time, so escalation
/// merges instead of replaces), and [`judge_and_requery`] passes round-one's
/// already-assembled engine-tier sources as `first` (the product invariant
/// that retrieved information is never discarded applies just as much to a
/// requery as to an escalation).
///
/// The merged list is re-budgeted to the same token allowance the engine tier
/// was assembled under (`assemble::budget_tokens(num_ctx)`, same
/// `estimate_tokens` accounting): `second` already filled that budget on its
/// own in both call sites, so appending `first` unchecked would overflow the
/// documented source budget and silently eat conversation-history headroom.
/// `second`'s blocks are dropped from the tail, the fusion's lowest-ranked
/// end, until the combined list fits.
fn merge_sources(
    first: Vec<SourceBlock>,
    second: Vec<SourceBlock>,
    num_ctx: u32,
) -> Vec<SourceBlock> {
    let budget = crate::websearch::assemble::budget_tokens(num_ctx);
    let mut spent: usize = first
        .iter()
        .map(|b| crate::websearch::assemble::estimate_tokens(&b.text))
        .sum();
    let mut merged = first;
    for block in second {
        let cost = crate::websearch::assemble::estimate_tokens(&block.text);
        if spent + cost > budget {
            break;
        }
        spent += cost;
        merged.push(block);
    }
    for (i, block) in merged.iter_mut().enumerate() {
        block.index = i + 1;
    }
    merged
}

/// Removes cross-query duplicate hits by URL, preserving first-seen order.
fn dedupe_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    // Same canonical-key dedup as `engine::dedupe_and_cap` (see its docs and
    // `super::canonical_url_key`): each query's hits already went through
    // that per-query dedup, but this pass merges hits ACROSS the (up to two)
    // queries one turn can run, so the same trailing-slash/scheme variant
    // could otherwise slip back in from a second query's results.
    let mut seen = std::collections::HashSet::new();
    hits.into_iter()
        .filter(|h| seen.insert(super::canonical_url_key(&h.url)))
        .collect()
}

/// Filters `hits` down to the URLs not already present in `existing`, so
/// [`judge_and_requery`]'s bounded requery only fetches pages round one has
/// not already assembled (see its docs' "fetch only new URLs" contract).
/// Preserves input order.
fn new_urls_only(hits: Vec<SearchHit>, existing: &[SourceBlock]) -> Vec<SearchHit> {
    let seen: std::collections::HashSet<&str> = existing.iter().map(|s| s.url.as_str()).collect();
    hits.into_iter()
        .filter(|h| !seen.contains(h.url.as_str()))
        .collect()
}

/// Truncates the sufficiency judge's `missing` phrase to at most `max_chars`
/// characters before [`judge_and_requery`] appends it to the standalone
/// question, cutting at the last whitespace boundary at or before the cap so
/// the text a requery actually searches never ends mid-word. `missing` is
/// free-form model prose (see `crate::websearch::judge`) and can run to a
/// full sentence; a long tail of prose degrades keyless-engine SERP quality
/// far more than a whole trailing word dropped does (see
/// [`crate::config::defaults::REQUERY_MISSING_MAX_CHARS`]).
///
/// Counts by `char`, not byte, so a multi-byte codepoint is never split; a
/// `missing` with no whitespace before the cap (a single run longer than
/// `max_chars`) falls back to a hard cut on that same char boundary, since no
/// better split exists. `missing` at or under the cap is returned unchanged.
fn truncate_missing(missing: &str, max_chars: usize) -> &str {
    let mut cut_at = None;
    let mut last_boundary = None;
    for (count, (byte_idx, ch)) in missing.char_indices().enumerate() {
        if count == max_chars {
            cut_at = Some(byte_idx);
            break;
        }
        if ch.is_whitespace() {
            last_boundary = Some(byte_idx);
        }
    }
    let Some(cut_at) = cut_at else {
        return missing;
    };
    match last_boundary {
        Some(boundary) => &missing[..boundary],
        None => &missing[..cut_at],
    }
}

/// Resolves the keyword SERP string(s) for the one bounded engine-tier requery.
///
/// Prefers the judge's already-normalized [`SufficiencyVerdict::requery_queries`]
/// when non-empty (horizontal: gap-targeted keywords, not a prose concat that
/// re-ranks the same related-facet cluster). Falls back to the legacy
/// `{standalone_question} {capped_missing}` single query so a model that omits
/// `requery_queries` still requeries.
///
/// Always returns at least one non-empty query when `missing` or `standalone`
/// has content; callers only invoke this on the insufficient-missing path.
fn resolve_requery_search_queries(
    standalone_question: &str,
    missing: &str,
    judge_queries: &[String],
) -> Vec<String> {
    if !judge_queries.is_empty() {
        return judge_queries.to_vec();
    }
    let standalone = standalone_question.trim();
    let missing_capped = truncate_missing(missing, REQUERY_MISSING_MAX_CHARS);
    if missing_capped.is_empty() {
        if standalone.is_empty() {
            return Vec::new();
        }
        return vec![standalone.to_string()];
    }
    if standalone.is_empty() {
        return vec![missing_capped.to_string()];
    }
    vec![format!("{standalone} {missing_capped}")]
}

/// What [`judge_and_requery`] resolved to: the user cancelling mid-judge or
/// mid-requery, or the final source list to answer from paired with the
/// requery's per-engine outcome summary.
enum EngineJudgeOutcome {
    /// The user cancelled mid-judge or mid-requery.
    Cancelled,
    /// The sources to answer from, paired with the per-query, per-engine outcome
    /// summary of the one requery this call may have fired (see [`EngineStat`]),
    /// a conflict flag, and an optional still-missing phrase.
    ///
    /// The sources are `sources` unchanged (a sufficient verdict, a judge
    /// failure, an empty `missing` phrase, a conflict verdict, or a requery that
    /// found no new URLs) or merged with the one requery's new sources; the stats
    /// are that requery's engines, empty on every path where no requery ran, for
    /// the caller to fold into the primary query's [`RecorderEvent::SearchRetrieved`]
    /// so one retrieval record shows both rounds' engines.
    ///
    /// `conflict` is `true` only when the judge returned an
    /// insufficient-because-conflicting verdict (see [`judge_and_requery`]), so
    /// the caller forwards it to [`grounded_answer`] and the writer presents the
    /// disagreement rather than hedging.
    ///
    /// `still_missing` is `Some(phrase)` when a requery ran (or found no new
    /// URLs) and the asked fact is still absent: the writer gets a partial
    /// directive so it does not treat a related metric as the full answer.
    /// Mutually exclusive with `conflict` in practice (conflict skips requery).
    Sources(Vec<SourceBlock>, Vec<EngineStat>, bool, Option<String>),
}

/// Round one's ranked chunks and fetched pages, supplied only by the
/// engine-tier requery caller so [`judge_and_requery`] can fuse round one and
/// round two into one score-ranked set instead of pinning round one ahead (see
/// its docs' "how round two is merged" contract).
struct RequeryRerank {
    /// Round one's ranked chunks (carrying their relevance scores), fused with
    /// round two's before re-budgeting.
    round_one_chunks: Vec<ScoredChunk>,
    /// Round one's fetched pages, kept for their extracted publish dates so the
    /// freshness-gated recency fusion scores round-one URLs, not just round-two.
    round_one_pages: Vec<FetchedPage>,
}

/// The engine tier's own sufficiency judge, plus the one bounded requery it
/// can trigger (see the module docs and
/// [`crate::config::defaults::ENGINE_REQUERY_MAX`]).
///
/// Runs whenever the general engine tier itself produced `sources`: both
/// [`run_web`]'s terminal engine-tier branch and [`commit_or_escalate`]'s
/// escalation-merge branch reach here, since either path ends in a freshly
/// assembled engine-tier block no prior judge call (if any) ever saw.
///
/// A `conflicting` verdict (see [`crate::websearch::judge::InsufficiencyReason`])
/// short-circuits before the requery: the sources already hold the asked value,
/// they just disagree on it, and a requery searches for a value that is not
/// missing, so it cannot resolve the disagreement (observed live wasting 28.5s).
/// The sources are committed unchanged with the conflict flag set on
/// [`EngineJudgeOutcome::Sources`], so the writer presents the spread. This is
/// why the per-turn LLM-call budget cannot rise: the conflict path spends fewer
/// calls, never more.
///
/// On a confident insufficient verdict naming what is missing, this records
/// `sources` as a round-one [`RecorderEvent::SearchRetrieved`] (`round:
/// Some(1)`, see its docs) before firing exactly one requery round: up to
/// [`crate::config::defaults::REQUERY_QUERY_MAX`] keyword SERPs from
/// [`resolve_requery_search_queries`] (judge `requery_queries` when present,
/// else standalone + capped `missing`, see
/// [`crate::config::defaults::REQUERY_MISSING_MAX_CHARS`] /
/// [`truncate_missing`]), through the normal engine path (`web_search` then
/// `fetch_pages`, so cache read/write and `bypass_cache` behave exactly as
/// they do for any other engine-tier call). Its hits are deduped by URL
/// against `sources` ([`new_urls_only`]) so only genuinely new pages are
/// fetched and ranked. The round-one record is emitted whenever the requery
/// fires, even when it turns up nothing new, so a trace always shows what
/// round one held right before the merge below may change it; the caller's
/// own [`RecorderEvent::SearchRetrieved`] (`round: None`) then records the
/// turn's terminal result, unchanged whether or not a requery ran.
///
/// How round two is merged depends on `rerank`, which encodes the two callers'
/// different merge contracts:
/// - `Some` (the [`run_web`] engine-tier caller): round one and round two are
///   fused into one set and re-ranked by fused score before the token
///   re-budget, so a stronger round-two source can outrank a weak round-one one
///   and the budget truncation drops the fused tail rather than categorically
///   round two. The re-rank reuses the exact ranking the pipeline already
///   produces: [`recency_reorder`] over the combined chunks when the turn is
///   fresh, a plain relevance ([`rerank_by_score`]) sort otherwise, then
///   [`assemble_context`]. This is cleaner than teaching [`merge_sources`] a
///   score order, since [`SourceBlock`] carries no score to sort on.
/// - `None` (the [`commit_or_escalate`] vertical-escalation caller): round one
///   (the vertical block ahead of the engines) is pinned whole and only round
///   two is dropped for budget ([`merge_sources`]); its vertical-block-rides-
///   first contract is locked, so it never takes the fused re-rank. Round two
///   is still recency-reordered on a fresh turn before the merge.
///
/// After the requery merges new sources (or finds none), a **second** judge
/// call checks whether the asked fact is now present. If still missing, the
/// commit carries `still_missing` so the writer partial-directive path fires
/// (related metrics must not masquerade as the asked total/level). A post-
/// requery judge error fails toward committing with the first-round
/// `still_missing` phrase (safe: better a partial caveat than a false full
/// answer). A [`InferenceError::Cancelled`] or an observed cancellation
/// before the requery's network calls yields [`EngineJudgeOutcome::Cancelled`].
#[allow(clippy::too_many_arguments)]
async fn judge_and_requery(
    deps: &SearchDeps<'_>,
    standalone_question: &str,
    sources: Vec<SourceBlock>,
    rerank: Option<RequeryRerank>,
    num_ctx: u32,
    freshness: bool,
    lang: &str,
    bypass_cache: bool,
    cancel: &CancellationToken,
) -> EngineJudgeOutcome {
    let judge_start = Instant::now();
    let verdict = match deps
        .judge
        .judge(standalone_question, &sources, cancel)
        .await
    {
        Ok(verdict) => {
            deps.timings.record(STAGE_JUDGE, judge_start);
            verdict
        }
        Err(InferenceError::Cancelled) => {
            deps.timings.record(STAGE_JUDGE, judge_start);
            return EngineJudgeOutcome::Cancelled;
        }
        Err(InferenceError::Request(reason)) => {
            deps.timings.record(STAGE_JUDGE, judge_start);
            eprintln!("[search] engine-tier judge error: {reason} -> committing round-one sources");
            return EngineJudgeOutcome::Sources(sources, Vec::new(), false, None);
        }
    };
    // Conflict: the sources DO hold the asked value but disagree on it (see
    // `judge::InsufficiencyReason::Conflicting`). A requery searches for a value
    // that is not missing, so it cannot resolve a disagreement (observed live
    // wasting 28.5s doing exactly that); commit round one's sources and raise the
    // conflict flag so the writer presents the spread instead of re-searching. No
    // requery ran, so the engine-stats companion is empty.
    if verdict.conflicting() {
        eprintln!(
            "[search] engine tier conflicting (missing: {}) -> committing, flagging writer",
            verdict.missing
        );
        return EngineJudgeOutcome::Sources(sources, Vec::new(), true, None);
    }
    // Sufficient, or insufficient with nothing to search for: commit
    // round-one's sources. `ENGINE_REQUERY_MAX == 0` also lands here (the
    // requery is disabled outright), since the flow has no loop back into
    // itself and cannot fire more than the one requery below regardless. No
    // requery ran, so the engine-stats companion is empty.
    if verdict.sufficient
        || verdict.missing.is_empty()
        || crate::config::defaults::ENGINE_REQUERY_MAX == 0
    {
        return EngineJudgeOutcome::Sources(sources, Vec::new(), false, None);
    }
    // Keep the gap phrase for post-requery partial-missing if the second
    // search still cannot fill it (or finds no new URLs).
    let first_missing = verdict.missing.clone();
    if cancel.is_cancelled() {
        return EngineJudgeOutcome::Cancelled;
    }
    // Round one's assembled sources, recorded now (round=1) so a trace can
    // audit exactly what round one held, including whatever detail the judge
    // found insufficient, before it is merged away below: the FINAL
    // `SearchRetrieved` this call's caller emits only shows the post-requery
    // merged set. `engine_stats` stays empty here; round one's own per-engine
    // stats are the caller's to fold into that final event alongside the
    // requery's (see `EngineJudgeOutcome::Sources`'s docs), so repeating them
    // here would double-count them in the trace.
    deps.recorder.record(RecorderEvent::SearchRetrieved {
        tier: "engine".to_string(),
        sources: sources
            .iter()
            .map(|s| RetrievedSource {
                index: s.index,
                url: s.url.clone(),
                title: s.title.clone(),
                text: truncate_for_trace(&s.text, TRACE_SOURCE_TEXT_MAX_BYTES),
            })
            .collect(),
        engine_stats: Vec::new(),
        round: Some(1),
    });
    let requery_queries: Vec<String> = resolve_requery_search_queries(
        standalone_question,
        &verdict.missing,
        &verdict.requery_queries,
    )
    .into_iter()
    // Drop pure-whitespace judge strings that bypassed normalize (defensive):
    // never issue a SERP with an empty `q=`.
    .filter(|q| !q.trim().is_empty())
    .collect();
    if requery_queries.is_empty() {
        // No searchable string at all: commit with partial flag so the writer
        // does not treat related metrics as the asked answer.
        return EngineJudgeOutcome::Sources(sources, Vec::new(), false, Some(first_missing));
    }
    // Forensic: first query is the primary; join extras so multi-query requeries
    // remain readable in one field without a schema bump on SearchRequeried.
    let requery_trace = requery_queries.join(" | ");
    eprintln!(
        "[search] engine tier insufficient (missing: {}) -> requerying once: {requery_trace}",
        first_missing
    );
    deps.recorder.record(RecorderEvent::SearchRequeried {
        missing: first_missing.clone(),
        requery: requery_trace,
    });
    // Multi-query requery: same fan-out shape as `run_engine_tier`'s primary
    // loop (early-stop at SERP_EARLY_STOP_HITS). Prefer judge keyword queries
    // so the second SERP aims at the gap, not a prose restatement of round one.
    // The requery races the keyless engines exactly as the primary query does,
    // so it produces its own per-engine outcome summary. Carry it back to the
    // caller regardless of whether new pages survive below, so the trace
    // records the requery's engines even when it found no new URLs.
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut requery_stats: Vec<EngineStat> = Vec::new();
    for query in &requery_queries {
        if cancel.is_cancelled() {
            return EngineJudgeOutcome::Cancelled;
        }
        let (query_hits, query_stats) = web_search(
            deps.transport,
            query,
            deps.health,
            freshness,
            lang,
            deps.web_cache,
            bypass_cache,
        )
        .await;
        hits.extend(query_hits);
        requery_stats.extend(query_stats);
        if hits.len() >= SERP_EARLY_STOP_HITS {
            break;
        }
    }
    let new_hits = new_urls_only(hits, &sources);
    if new_hits.is_empty() {
        // Requery found nothing new: still missing the asked fact.
        eprintln!(
            "[search] engine tier requery found no new URLs -> committing with still_missing={first_missing}"
        );
        return EngineJudgeOutcome::Sources(sources, requery_stats, false, Some(first_missing));
    }
    if cancel.is_cancelled() {
        return EngineJudgeOutcome::Cancelled;
    }
    let pages = fetch_pages(
        deps.transport,
        &new_hits,
        num_ctx,
        // Fix A: the recency merge added `freshness` to `fetch_pages` (it gates
        // publish-date extraction). Thread the turn's signal through so the
        // requery's pages carry dates on a fresh turn, feeding the fusion below.
        freshness,
        deps.web_cache,
        bypass_cache,
    )
    .await;
    // Sampled once so both merge branches age publish dates against a single
    // consistent instant (the plain-relevance branch ignores it).
    let now = time::OffsetDateTime::now_utc();
    let merged = match rerank {
        // Engine-tier requery merge (fix D, contract in this fn's docs): round
        // one and round two are fused at chunk level and re-ranked as one set,
        // so a stronger round-two source can outrank a weak round-one one and
        // the token budget truncates the fused tail rather than categorically
        // round two. Round one's `SourceBlock`s are rebuilt from its chunks here
        // (a `SourceBlock` carries no score to sort on), so the assembled set is
        // ordered by fused score, not by round-one-then-round-two append order.
        Some(RequeryRerank {
            round_one_chunks,
            round_one_pages,
        }) => {
            // Round two's own chunks, scored against the same standalone question
            // and scorer as round one, so the two rounds' scores are directly
            // comparable once fused.
            let round_two_chunks = select_chunks(&pages, standalone_question, deps.scorer);
            let mut combined_chunks = round_one_chunks;
            combined_chunks.extend(round_two_chunks);
            // Both rounds' pages, so the freshness-gated recency pass can read a
            // publish date for round-one URLs too, not only round-two ones.
            let mut combined_pages = round_one_pages;
            combined_pages.extend(pages);
            // Fix C over the fused set: freshness-gated recency ordering, the
            // recency-prior fusion on a fresh turn (identical to
            // `run_engine_tier`'s), a plain relevance sort otherwise. Either way
            // every chunk is kept and only reordered; `assemble_context` is the
            // one place the tail is dropped, and only for the token budget.
            let ordered = if freshness {
                recency_reorder(&combined_chunks, &combined_pages, now)
            } else {
                rerank_by_score(combined_chunks)
            };
            let ordered = filter_evidence_chunks(
                ordered,
                freshness,
                is_price_intent_question(standalone_question),
                now.year() as u32,
            );
            assemble_context(&ordered, num_ctx)
        }
        // Vertical-escalation merge (contract locked): the vertical block stays
        // pinned ahead of the engines (`merge_sources`), so round two is not
        // fused into round one here. Fix C still applies to round two among
        // itself: it is recency-reordered on a fresh turn before the merge,
        // matching `run_engine_tier`, so the requeried engine sources are never
        // the one path that skips recency ranking.
        None => {
            let chunks = select_chunks(&pages, standalone_question, deps.scorer);
            let chunks = if freshness {
                recency_reorder(&chunks, &pages, now)
            } else {
                chunks
            };
            let chunks = filter_evidence_chunks(
                chunks,
                freshness,
                is_price_intent_question(standalone_question),
                now.year() as u32,
            );
            let new_sources = assemble_context(&chunks, num_ctx);
            merge_sources(sources, new_sources, num_ctx)
        }
    };
    // Post-requery judge: did the merged set finally contain the asked fact?
    // Without this, a related-facet corpus (growth % only) was committed as a
    // full answer. One extra LLM call, only on the requery path.
    if cancel.is_cancelled() {
        return EngineJudgeOutcome::Cancelled;
    }
    let post_start = Instant::now();
    let post = match deps.judge.judge(standalone_question, &merged, cancel).await {
        Ok(v) => {
            deps.timings.record(STAGE_JUDGE_POST_REQUERY, post_start);
            v
        }
        Err(InferenceError::Cancelled) => {
            deps.timings.record(STAGE_JUDGE_POST_REQUERY, post_start);
            return EngineJudgeOutcome::Cancelled;
        }
        Err(InferenceError::Request(reason)) => {
            deps.timings.record(STAGE_JUDGE_POST_REQUERY, post_start);
            // Fail toward commit with the original gap: never claim sufficiency
            // we did not verify.
            eprintln!(
                "[search] post-requery judge error: {reason} -> committing with still_missing"
            );
            return EngineJudgeOutcome::Sources(merged, requery_stats, false, Some(first_missing));
        }
    };
    if post.conflicting() {
        eprintln!(
            "[search] post-requery conflicting (missing: {}) -> committing, flagging writer",
            post.missing
        );
        return EngineJudgeOutcome::Sources(merged, requery_stats, true, None);
    }
    if post.sufficient {
        eprintln!("[search] post-requery sufficient -> committing");
        return EngineJudgeOutcome::Sources(merged, requery_stats, false, None);
    }
    let still = if post.missing.is_empty() {
        first_missing
    } else {
        post.missing
    };
    eprintln!("[search] post-requery still insufficient (missing: {still}) -> partial writer path");
    EngineJudgeOutcome::Sources(merged, requery_stats, false, Some(still))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::reachability::{FakeReachability, ReachabilityVerdict};
    use crate::net::transport::{FakeHttpTransport, HttpRequest, HttpResponse, TransportError};
    use crate::websearch::cache::TtlSourceCache;
    use crate::websearch::judge::{FakeSufficiencyJudge, InsufficiencyReason};
    use crate::websearch::prepass::{FakePrePass, PrePassDecision};
    use crate::websearch::rank::Bm25Scorer;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    /// A transport that cancels `token` on every send (after returning the SERP),
    /// so the orchestrator's mid-pipeline cancellation checks are exercised.
    struct CancelOnSend {
        token: CancellationToken,
        serp: Vec<u8>,
    }

    #[async_trait]
    impl HttpTransport for CancelOnSend {
        async fn send(&self, _req: &HttpRequest) -> Result<HttpResponse, TransportError> {
            self.token.cancel();
            Ok(HttpResponse {
                status: 200,
                final_url: DDG_ENDPOINT.into(),
                body: self.serp.clone(),
            })
        }
    }

    /// Cancels on the requery page fetch only (not Mojeek/SERP), after returning
    /// the inner response. Exercises cancel between requery merge and the
    /// post-requery judge without tripping the mid-SERP cancel check.
    struct CancelOnPageFetch {
        token: CancellationToken,
        inner: FakeHttpTransport,
    }

    #[async_trait]
    impl HttpTransport for CancelOnPageFetch {
        async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
            let resp = self.inner.send(req).await;
            // Only the page URL: engine SERP races also hit non-DDG hosts.
            if req.url.starts_with("https://requery.example") {
                self.token.cancel();
            }
            resp
        }
    }

    const DDG_ENDPOINT: &str = "https://html.duckduckgo.com/html/";

    /// A SERP with one organic result pointing at `match.example`.
    const SERP_HTML: &str = r#"
      <div class="result">
        <a class="result__a" href="https://match.example/">Treaty of Versailles</a>
        <a class="result__snippet">the treaty signed in paris</a>
      </div>
    "#;

    /// A dense article readability will extract, about the query subject.
    const PAGE_HTML: &str = r#"
      <html><body><article><h1>Treaty of Versailles</h1>
      <p>The treaty was signed in paris in 1919, formally ending the state of war
      between Germany and the Allied Powers after the armistice of 1918. The
      negotiations stretched across many months and reshaped the borders of
      Europe in ways that echoed for a generation afterward.</p>
      <p>Its territorial and financial terms were debated fiercely at the paris
      peace conference, and historians still argue about whether the 1919
      settlement made another war more or less likely in the decades that
      followed the signing of the treaty itself.</p>
      </article></body></html>
    "#;

    fn web_decision(queries: Vec<&str>) -> PrePassDecision {
        PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: queries.into_iter().map(String::from).collect(),
            explicit_search: false,
            lang: "en".into(),
        }
    }

    fn transport_with_serp_and_page() -> FakeHttpTransport {
        FakeHttpTransport::new()
            .with_response(
                DDG_ENDPOINT,
                HttpResponse {
                    status: 200,
                    final_url: DDG_ENDPOINT.into(),
                    body: SERP_HTML.as_bytes().to_vec(),
                },
            )
            .with_response(
                "https://match.example/",
                HttpResponse {
                    status: 200,
                    final_url: "https://match.example/".into(),
                    body: PAGE_HTML.as_bytes().to_vec(),
                },
            )
    }

    /// The reachability probe every default-path test injects: a network that
    /// reports itself up, so no existing test can short-circuit. The offline
    /// fast-fail tests below opt in explicitly via [`deps_with_probe`].
    fn reachable() -> &'static dyn Reachability {
        Box::leak(Box::new(FakeReachability::returning(
            ReachabilityVerdict::Reachable,
        )))
    }

    /// The default [`deps`] with the reachability probe swapped, so the offline
    /// fast-fail tests can script what the network says while everything else
    /// stays on the shared default path.
    fn deps_with_probe<'a>(
        prepass: &'a dyn PrePass,
        transport: &'a dyn HttpTransport,
        scorer: &'a dyn Scorer,
        reachability: &'a dyn Reachability,
    ) -> SearchDeps<'a> {
        let mut deps = deps(prepass, transport, scorer);
        deps.reachability = reachability;
        deps
    }

    fn deps<'a>(
        prepass: &'a dyn PrePass,
        transport: &'a dyn HttpTransport,
        scorer: &'a dyn Scorer,
    ) -> SearchDeps<'a> {
        deps_with_recorder(
            prepass,
            transport,
            scorer,
            Box::leak(Box::new(crate::trace::BoundRecorder::noop_for(
                crate::trace::ConversationId::new("test"),
            ))),
        )
    }

    fn deps_with_recorder<'a>(
        prepass: &'a dyn PrePass,
        transport: &'a dyn HttpTransport,
        scorer: &'a dyn Scorer,
        recorder: &'a crate::trace::BoundRecorder,
    ) -> SearchDeps<'a> {
        // A fresh, empty cache per test (no test in this default path relies
        // on a cache hit), scope 1 by convention.
        deps_with_cache(
            prepass,
            transport,
            scorer,
            recorder,
            Box::leak(Box::new(TtlSourceCache::new(
                std::time::Duration::from_secs(600),
                crate::config::defaults::SEARCH_CACHE_MAX_ENTRIES,
            ))),
            1,
        )
    }

    fn deps_with_cache<'a>(
        prepass: &'a dyn PrePass,
        transport: &'a dyn HttpTransport,
        scorer: &'a dyn Scorer,
        recorder: &'a crate::trace::BoundRecorder,
        cache: &'a dyn SourceCache,
        cache_scope: u64,
    ) -> SearchDeps<'a> {
        SearchDeps {
            prepass,
            // The default path never escalates: a judge that always finds the
            // vertical block sufficient keeps every existing vertical test
            // committing its block exactly as before. Escalation tests build
            // `SearchDeps` directly with a scripted judge and health (see
            // `deps_for_escalation`).
            judge: Box::leak(Box::new(FakeSufficiencyJudge::sufficient())),
            transport,
            reachability: reachable(),
            scorer,
            // Each test gets its own leaked registry so a block marked in one
            // test can never poison a parallel test's rotation.
            health: Box::leak(Box::new(EngineHealth::new())),
            recorder,
            cache,
            cache_scope,
            // A fresh, empty web cache per test (leaked so it outlives the
            // returned `SearchDeps`), so parallel tests never share cache state.
            web_cache: Box::leak(Box::new(WebCache::new(
                std::time::Duration::from_secs(600),
                std::time::Duration::from_secs(600),
                64,
                128,
            ))),
            // The sports vertical's kickoff-time localization is unit-tested in
            // `websearch::sports`; the orchestrator only threads the zone
            // through, so tests here run without one (date-only event lines).
            local_zone: None,
            force_search: false,
            latest_images: None,
            timings: Box::leak(Box::new(TimingBag::new())),
        }
    }

    /// Builds `SearchDeps` for the commit-or-escalate tests with full control of
    /// the judge verdict and engine health (the two inputs that decide whether a
    /// vertical answer is committed, escalated, or served as a partial). A fresh
    /// empty cache and no local zone, like the default helpers.
    fn deps_for_escalation<'a>(
        prepass: &'a dyn PrePass,
        transport: &'a dyn HttpTransport,
        scorer: &'a dyn Scorer,
        judge: &'a dyn SufficiencyJudge,
        health: &'a EngineHealth,
        recorder: &'a crate::trace::BoundRecorder,
    ) -> SearchDeps<'a> {
        SearchDeps {
            prepass,
            judge,
            transport,
            reachability: reachable(),
            scorer,
            health,
            recorder,
            cache: Box::leak(Box::new(TtlSourceCache::new(
                std::time::Duration::from_secs(600),
                crate::config::defaults::SEARCH_CACHE_MAX_ENTRIES,
            ))),
            cache_scope: 1,
            web_cache: Box::leak(Box::new(WebCache::new(
                std::time::Duration::from_secs(600),
                std::time::Duration::from_secs(600),
                64,
                128,
            ))),
            local_zone: None,
            force_search: false,
            latest_images: None,
            timings: Box::leak(Box::new(TimingBag::new())),
        }
    }

    /// Builds `SearchDeps` like [`deps_for_escalation`], but with a
    /// caller-supplied `web_cache` instead of a fresh one, so the cache-bypass
    /// tests can pre-warm a SERP entry before the pipeline runs and assert on
    /// its state (or on the transport's calls) afterward.
    fn deps_with_web_cache<'a>(
        prepass: &'a dyn PrePass,
        transport: &'a dyn HttpTransport,
        scorer: &'a dyn Scorer,
        judge: &'a dyn SufficiencyJudge,
        health: &'a EngineHealth,
        recorder: &'a crate::trace::BoundRecorder,
        web_cache: &'a WebCache,
    ) -> SearchDeps<'a> {
        SearchDeps {
            prepass,
            judge,
            transport,
            reachability: reachable(),
            scorer,
            health,
            recorder,
            cache: Box::leak(Box::new(TtlSourceCache::new(
                std::time::Duration::from_secs(600),
                crate::config::defaults::SEARCH_CACHE_MAX_ENTRIES,
            ))),
            cache_scope: 1,
            web_cache,
            local_zone: None,
            force_search: false,
            latest_images: None,
            timings: Box::leak(Box::new(TimingBag::new())),
        }
    }

    /// `SearchDeps` for the multi-turn reuse-gate tests: a caller-supplied cache
    /// (pre-loaded with stored searches), sufficiency judge (the reuse grounding
    /// gate), engine health (whether an escalation can reach the engines), and
    /// scope, the inputs that decide whether a `cached` decision reuses stored
    /// sources or escalates to a fresh search.
    #[allow(clippy::too_many_arguments)]
    fn deps_for_reuse<'a>(
        prepass: &'a dyn PrePass,
        transport: &'a dyn HttpTransport,
        scorer: &'a dyn Scorer,
        judge: &'a dyn SufficiencyJudge,
        health: &'a EngineHealth,
        recorder: &'a crate::trace::BoundRecorder,
        cache: &'a dyn SourceCache,
        cache_scope: u64,
    ) -> SearchDeps<'a> {
        SearchDeps {
            prepass,
            judge,
            transport,
            reachability: reachable(),
            scorer,
            health,
            recorder,
            cache,
            cache_scope,
            web_cache: Box::leak(Box::new(WebCache::new(
                std::time::Duration::from_secs(600),
                std::time::Duration::from_secs(600),
                64,
                128,
            ))),
            local_zone: None,
            force_search: false,
            latest_images: None,
            timings: Box::leak(Box::new(TimingBag::new())),
        }
    }

    fn recorder() -> (
        std::sync::Arc<Mutex<Vec<SearchPhase>>>,
        impl Fn(SearchPhase) + Send + Sync,
    ) {
        let phases = std::sync::Arc::new(Mutex::new(Vec::new()));
        let clone = std::sync::Arc::clone(&phases);
        (phases, move |p| clone.lock().unwrap().push(p))
    }

    // ── dedupe_hits ───────────────────────────────────────────────────────────

    #[test]
    fn dedupe_hits_removes_cross_query_duplicates() {
        let hit = |u: &str| SearchHit {
            title: "t".into(),
            url: u.into(),
            snippet: "s".into(),
        };
        let out = dedupe_hits(vec![
            hit("https://a/"),
            hit("https://b/"),
            hit("https://a/"),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].url, "https://a/");
        assert_eq!(out[1].url, "https://b/");
    }

    #[test]
    fn dedupe_hits_collapses_cross_query_canonical_variants() {
        // A second query in the same turn can surface a trailing-slash or
        // http/https variant of a URL the first query already returned; this
        // cross-query pass must collapse it exactly like the per-query
        // `engine::dedupe_and_cap` does.
        let hit = |u: &str| SearchHit {
            title: "t".into(),
            url: u.into(),
            snippet: "s".into(),
        };
        let out = dedupe_hits(vec![
            hit("https://www.binance.com/en/price/bitcoin"),
            hit("http://www.binance.com/en/price/bitcoin/"),
        ]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn merge_sources_puts_vertical_first_and_reindexes_contiguously() {
        let source = |index: usize, url: &str| SourceBlock {
            index,
            url: url.into(),
            title: "t".into(),
            text: "x".into(),
        };
        // Stale index (1) on the vertical block, as it arrives from the
        // vertical tier; two engine blocks already indexed 1, 2.
        let vertical = source(1, "https://vertical.example/");
        let engines = vec![
            source(1, "https://engine-a.example/"),
            source(2, "https://engine-b.example/"),
        ];
        let merged = merge_sources(vec![vertical], engines, 16384);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].url, "https://vertical.example/");
        assert_eq!(merged[0].index, 1);
        assert_eq!(merged[1].url, "https://engine-a.example/");
        assert_eq!(merged[1].index, 2);
        assert_eq!(merged[2].url, "https://engine-b.example/");
        assert_eq!(merged[2].index, 3);
    }

    #[test]
    fn merge_sources_drops_tail_engine_blocks_that_overflow_the_budget() {
        // Regression pin for the combine-then-overflow hazard: the engine
        // sources already fill the whole token budget on their own, so a merge
        // that never re-budgets would exceed the documented source allowance.
        // The vertical block must be kept whole and engine blocks must drop
        // from the tail (the fusion's lowest-ranked end) until the merged list
        // fits, with indices still contiguous.
        let budget = crate::websearch::assemble::budget_tokens(16384);
        // Three engine blocks of ~half a budget each: the first fits next to
        // the vertical block, the second and third must be dropped.
        let big_text = "y".repeat(budget / 2 * crate::config::defaults::CHARS_PER_TOKEN);
        let block = |index: usize, url: &str, text: &str| SourceBlock {
            index,
            url: url.into(),
            title: "t".into(),
            text: text.into(),
        };
        let vertical = block(1, "https://vertical.example/", "small vertical block");
        let engines = vec![
            block(1, "https://engine-a.example/", &big_text),
            block(2, "https://engine-b.example/", &big_text),
            block(3, "https://engine-c.example/", &big_text),
        ];
        let merged = merge_sources(vec![vertical], engines, 16384);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].url, "https://vertical.example/");
        assert_eq!(merged[0].index, 1);
        assert_eq!(merged[1].url, "https://engine-a.example/");
        assert_eq!(merged[1].index, 2);
        // The kept set fits the same budget the engine tier assembles under.
        let spent: usize = merged
            .iter()
            .map(|b| crate::websearch::assemble::estimate_tokens(&b.text))
            .sum();
        assert!(spent <= budget);
    }

    // ── resolve_requery_search_queries ────────────────────────────────────────

    #[test]
    fn resolve_requery_prefers_judge_keyword_queries() {
        // Horizontal gap-targeting: judge queries win over the legacy concat.
        let q = resolve_requery_search_queries(
            "what is Vietnam's total GDP in 2026",
            "total GDP value for 2026",
            &[
                "Vietnam nominal GDP USD billion".into(),
                "Vietnam GDP current US$".into(),
            ],
        );
        assert_eq!(
            q,
            vec![
                "Vietnam nominal GDP USD billion".to_string(),
                "Vietnam GDP current US$".to_string()
            ]
        );
    }

    #[test]
    fn resolve_requery_falls_back_to_standalone_plus_capped_missing() {
        let q = resolve_requery_search_queries(
            "what is Vietnam's total GDP in 2026",
            "total GDP value for 2026",
            &[],
        );
        assert_eq!(
            q,
            vec!["what is Vietnam's total GDP in 2026 total GDP value for 2026".to_string()]
        );
    }

    #[test]
    fn resolve_requery_empty_both_yields_empty_vec() {
        assert!(resolve_requery_search_queries("", "", &[]).is_empty());
        // Standalone trims empty; empty missing → no searchable string.
        assert!(resolve_requery_search_queries("   \t", "", &[]).is_empty());
    }

    #[test]
    fn resolve_requery_standalone_only_or_missing_only() {
        assert_eq!(
            resolve_requery_search_queries("standalone alone", "", &[]),
            vec!["standalone alone".to_string()]
        );
        assert_eq!(
            resolve_requery_search_queries("", "missing alone", &[]),
            vec!["missing alone".to_string()]
        );
        assert_eq!(
            resolve_requery_search_queries("   ", "gap phrase", &[]),
            vec!["gap phrase".to_string()]
        );
    }

    // ── truncate_missing ──────────────────────────────────────────────────────

    #[test]
    fn truncate_missing_leaves_short_text_unchanged() {
        let missing = "the treaty terms";
        assert_eq!(truncate_missing(missing, 80), missing);
    }

    #[test]
    fn truncate_missing_returns_unchanged_at_exactly_the_cap() {
        // Exactly `max_chars` characters: the loop never sees `count ==
        // max_chars`, so this must take the same unchanged path as text
        // strictly shorter than the cap.
        let missing = "a".repeat(80);
        assert_eq!(truncate_missing(&missing, 80), missing);
    }

    #[test]
    fn truncate_missing_cuts_at_the_last_word_boundary_within_the_cap() {
        // 105 characters; the 80-char cap falls inside "settlement", so the
        // cut must back up to the last space before it rather than split the
        // word mid-way.
        let missing = "the full territorial and financial terms and conditions of the treaty \
                        settlement and reparations schedule";
        assert_eq!(missing.chars().count(), 105);
        let truncated = truncate_missing(missing, 80);
        assert_eq!(
            truncated,
            "the full territorial and financial terms and conditions of the treaty"
        );
        assert!(truncated.chars().count() <= 80);
        assert!(!truncated.ends_with(' '));
    }

    #[test]
    fn truncate_missing_hard_cuts_a_single_word_with_no_whitespace() {
        // No whitespace anywhere before the cap: there is no better split, so
        // this falls back to a hard cut on the char boundary at `max_chars`.
        let missing = "a".repeat(100);
        let truncated = truncate_missing(&missing, 80);
        assert_eq!(truncated.chars().count(), 80);
        assert_eq!(truncated, "a".repeat(80));
    }

    #[test]
    fn truncate_missing_never_panics_on_multibyte_text_with_no_whitespace() {
        // A run of 4-byte codepoints (emoji) longer than the cap, with no
        // whitespace: the hard-cut fallback must land on a char boundary, not
        // split a codepoint, panicking on the byte-slice.
        let missing = "🎉".repeat(100);
        let truncated = truncate_missing(&missing, 80);
        assert_eq!(truncated.chars().count(), 80);
        assert_eq!(truncated, "🎉".repeat(80));
    }

    // ── run_search: decision branches ─────────────────────────────────────────

    #[tokio::test]
    async fn classifier_no_decision_yields_no_search() {
        // An ambiguous turn ("tell me a joke") reaches the classifier, which
        // returns `no`: the Deciding phase is emitted, then no search runs.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::No,
            route: SearchRoute::Web,
            standalone_question: "tell me a joke".into(),
            queries: vec![],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = FakeHttpTransport::new();
        let (phases, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "tell me a joke",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::NoSearch));
        assert_eq!(*phases.lock().unwrap(), vec![SearchPhase::Deciding]);
    }

    #[tokio::test]
    async fn force_search_overrides_classifier_no_and_searches_engines() {
        // `/search` force path: even when the classifier would say No, we still
        // run engines-only with explicit_search semantics.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::No,
            route: SearchRoute::Web,
            standalone_question: "who owns Figma".into(),
            queries: vec!["Figma ownership".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (phases, status) = recorder();
        let outcome = run_search_forced(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "who owns Figma",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // Engines ran (not NoSearch). Answer vs Unreachable depends on page
        // fetch fixtures; both prove force overrode classifier No.
        assert!(
            !matches!(outcome, SearchOutcome::NoSearch | SearchOutcome::Cancelled),
            "force_search must not skip search when the classifier says No"
        );
        let phases = phases.lock().unwrap().clone();
        assert!(phases.contains(&SearchPhase::Deciding));
        assert!(phases
            .iter()
            .any(|p| matches!(p, SearchPhase::Searching | SearchPhase::Reading)));
    }

    #[tokio::test]
    async fn force_search_classifier_cancel_yields_cancelled() {
        let prepass = FakePrePass::returning(Err(InferenceError::Cancelled));
        let transport = FakeHttpTransport::new();
        let (_phases, status) = recorder();
        let outcome = run_search_forced(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "force me",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    #[tokio::test]
    async fn force_search_classifier_error_uses_raw_query_and_empty_rewrite_fallback() {
        // Infra failure: raw message becomes standalone + query.
        let prepass = FakePrePass::returning(Err(InferenceError::Request("timeout".into())));
        let transport = transport_with_serp_and_page();
        let (_phases, status) = recorder();
        let outcome = run_search_forced(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "  raw forced query  ",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(!matches!(
            outcome,
            SearchOutcome::NoSearch | SearchOutcome::Cancelled
        ));
    }

    #[tokio::test]
    async fn force_search_backfills_empty_queries_and_standalone() {
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::No,
            route: SearchRoute::Web,
            standalone_question: "   ".into(),
            queries: vec![],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_phases, status) = recorder();
        let outcome = run_search_forced(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "user typed this",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(!matches!(
            outcome,
            SearchOutcome::NoSearch | SearchOutcome::Cancelled
        ));
    }

    #[tokio::test]
    async fn prefilter_force_no_short_circuits_without_classifier() {
        // A greeting is force-skipped by the pre-filter: no Deciding phase, and
        // the classifier is never consulted (it would have returned Web).
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["should not run"])));
        let transport = transport_with_serp_and_page();
        let (phases, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "hi there, thanks!",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::NoSearch));
        assert!(phases.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn prefilter_force_web_overrides_classifier_no() {
        // The pre-filter forces web on "latest ..."; the classifier leaned `no`
        // with no queries, yet the pipeline still searches, using the
        // classifier's standalone rewrite (which matches the fixture page).
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::No,
            route: SearchRoute::Web,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec![],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (phases, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "the latest on the treaty",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Answer { .. }));
        assert_eq!(
            *phases.lock().unwrap(),
            vec![
                SearchPhase::Deciding,
                SearchPhase::Searching,
                SearchPhase::Reading
            ]
        );
    }

    #[tokio::test]
    async fn cancel_during_page_fetch_yields_cancelled() {
        // A cancel raised WHILE the page-fetch stage is awaiting (after the
        // pre-fetch check, not before it) must still abort the turn: the fetch
        // await is raced against the token. `SearchPhase::Reading` is emitted
        // immediately before that await, so a status callback that cancels on
        // Reading reproduces a mid-fetch cancellation deterministically.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec!["treaty versailles paris".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let cancel = CancellationToken::new();
        let cancel_on_read = cancel.clone();
        let status = move |phase: SearchPhase| {
            if matches!(phase, SearchPhase::Reading) {
                cancel_on_read.cancel();
            }
        };
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &cancel,
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    // ── news vertical routing ─────────────────────────────────────────────────

    #[tokio::test]
    async fn news_question_answers_via_headlines_without_engines() {
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::News,
            standalone_question: "who won the most recent F1 race".into(),
            queries: vec!["f1 race winner".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let feed_url = crate::websearch::news::news_request("f1 race winner", true, "en").url;
        let feed = r#"<rss><channel><item><title>Leclerc wins British GP - Formula 1</title><pubDate>Wed, 08 Jul 2026 01:11:35 GMT</pubDate></item></channel></rss>"#;
        let transport = FakeHttpTransport::new().with_response(
            &feed_url,
            HttpResponse {
                status: 200,
                final_url: feed_url.clone(),
                body: feed.as_bytes().to_vec(),
            },
        );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "who won the most recent F1 race",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(
            matches!(&outcome, SearchOutcome::Answer { messages, sources }
            if sources.len() == 1
                && sources[0].url == "https://news.google.com/"
                && messages.first().is_some_and(|m| m.content.contains("Leclerc wins British GP")))
        );
        assert!(!transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn news_intent_with_no_queries_skips_feed() {
        // Totality: an ambiguous turn (no ForceWeb backfill) whose classifier
        // answered Web with an empty query list cannot query the feed or the
        // engines and resolves to the unreachable disclosure.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::News,
            standalone_question: "who won the game".into(),
            queries: vec![],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = FakeHttpTransport::new();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "and that game?",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Unreachable { .. }));
        assert!(transport.calls().is_empty());
    }

    #[tokio::test]
    async fn cancel_during_news_miss_yields_cancelled() {
        // The feed request cancels the token and returns junk: the news vertical
        // misses, and the post-news cancellation check aborts before engines.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::News,
            standalone_question: "who won the race".into(),
            queries: vec!["race winner".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let cancel = CancellationToken::new();
        let transport = CancelOnSend {
            token: cancel.clone(),
            serp: b"not a feed".to_vec(),
        };
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &cancel,
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    #[tokio::test]
    async fn news_feed_miss_falls_through_to_engines() {
        // News intent but the feed errors (no canned response): the engines run
        // and ground the answer as usual.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::News,
            standalone_question: "who won the treaty of versailles game".into(),
            queries: vec!["treaty versailles paris".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Answer { .. }));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn news_tries_each_query_until_one_yields_headlines() {
        // Fix 4b: the news tier loops every classifier query, not just the
        // first. Here the first query's feed is empty (no items) and the second
        // query's feed carries a headline, so the turn grounds on the second
        // query instead of dead-ending on the empty first result.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::News,
            standalone_question: "who won the race".into(),
            queries: vec!["empty first query".into(), "race winner".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let first_url = crate::websearch::news::news_request("empty first query", false, "en").url;
        let second_url = crate::websearch::news::news_request("race winner", false, "en").url;
        let transport = FakeHttpTransport::new()
            .with_response(
                &first_url,
                HttpResponse {
                    status: 200,
                    final_url: first_url.clone(),
                    body: b"<rss><channel></channel></rss>".to_vec(),
                },
            )
            .with_response(
                &second_url,
                HttpResponse {
                    status: 200,
                    final_url: second_url.clone(),
                    body: br#"<rss><channel><item><title>Leclerc wins the race - Formula 1</title></item></channel></rss>"#.to_vec(),
                },
            );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "who won the race",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(
            matches!(&outcome, SearchOutcome::Answer { messages, sources }
            if sources[0].url == "https://news.google.com/"
                && messages.first().is_some_and(|m| m.content.contains("Leclerc wins the race")))
        );
        // Both feed queries were attempted, in order.
        assert!(transport.calls().iter().any(|c| c.url == first_url));
        assert!(transport.calls().iter().any(|c| c.url == second_url));
    }

    // ── weather vertical routing ──────────────────────────────────────────────

    #[tokio::test]
    async fn weather_question_answers_via_vertical_without_engines() {
        // The standalone question is a weather question: the vertical resolves
        // geocode + forecast and the scraped engines are never queried.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Weather,
            standalone_question: "weather in Tokyo".into(),
            queries: vec!["tokyo weather".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let geo_url = crate::websearch::weather::geocode_request("Tokyo", "en").url;
        let geo_body = r#"{"results":[{"name":"Tokyo","latitude":35.6895,"longitude":139.69171,"country":"Japan"}]}"#;
        let place = crate::websearch::weather::parse_geocode(geo_body).unwrap();
        let fc_url = crate::websearch::weather::forecast_request(&place).url;
        let fc_body = r#"{"current":{"temperature_2m":25.5,"relative_humidity_2m":61,"apparent_temperature":27.9,"weather_code":1,"wind_speed_10m":2.6}}"#;
        let transport = FakeHttpTransport::new()
            .with_response(
                &geo_url,
                HttpResponse {
                    status: 200,
                    final_url: geo_url.clone(),
                    body: geo_body.as_bytes().to_vec(),
                },
            )
            .with_response(
                &fc_url,
                HttpResponse {
                    status: 200,
                    final_url: fc_url.clone(),
                    body: fc_body.as_bytes().to_vec(),
                },
            );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "weather in Tokyo",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(
            matches!(&outcome, SearchOutcome::Answer { messages, sources }
            if sources.len() == 1
                && sources[0].url == "https://open-meteo.com/"
                && messages.first().is_some_and(|m| m.content.contains("Current weather in Tokyo")))
        );
        // No scraped engine was touched.
        assert!(!transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn cancel_during_vertical_miss_yields_cancelled() {
        // A weather-shaped question whose geocode call cancels the token and
        // returns junk: the vertical misses, and the post-Open-Meteo cancel
        // check aborts (weather exclusive does not fall through to engines).
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Weather,
            standalone_question: "weather in Xyzzyplace".into(),
            queries: vec!["xyzzyplace weather".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let cancel = CancellationToken::new();
        let transport = CancelOnSend {
            token: cancel.clone(),
            serp: b"not geocode json".to_vec(),
        };
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            // User text must itself be weather-shaped so sanitize keeps the
            // weather route (bare "q" would demote to web engines).
            "weather in Xyzzyplace",
            16384,
            "2026-07-05",
            "en-US",
            &cancel,
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    #[tokio::test]
    async fn weather_miss_refuses_without_engine_seo() {
        // Weather exclusive: Open-Meteo miss must NOT fall through to scraped
        // engines (SEO widgets invent humidity > 100% and wrong cities).
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Weather,
            standalone_question: "weather in Xyzzyplace".into(),
            queries: vec!["xyzzyplace weather".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "weather in Xyzzyplace",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(
            matches!(
                &outcome,
                SearchOutcome::Unreachable {
                    reason: SearchFailReason::WeatherUnavailable,
                    messages,
                } if messages.first().is_some_and(|m| {
                    m.content.contains("Do NOT invent") && m.content.contains("live weather")
                })
            ),
            "expected WeatherUnavailable with no-invent system note"
        );
        assert!(
            !transport.calls().iter().any(|c| c.url == DDG_ENDPOINT),
            "weather exclusive must not touch scraped engines"
        );
    }

    #[test]
    fn sanitize_bare_sjc_rewrites_to_gold_web() {
        let classified = PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Weather,
            standalone_question:
                "what is the current weather at San Jose International Airport (SJC)".into(),
            queries: vec!["SJC weather".into()],
            explicit_search: true,
            lang: "en".into(),
        };
        let out = sanitize_search_decision("SJC", classified);
        assert_eq!(out.route, SearchRoute::Web);
        assert!(out.standalone_question.to_lowercase().contains("gold"));
        assert!(out.queries.iter().any(|q| q.contains("SJC")));
        // `/search` prefix still counts as bare SJC; VI lang prefers vàng.
        let with_cmd = sanitize_search_decision(
            "/search SJC",
            PrePassDecision {
                decision: SearchDecision::Web,
                route: SearchRoute::Weather,
                standalone_question: "SJC weather".into(),
                queries: vec!["SJC weather".into()],
                explicit_search: true,
                lang: "vi".into(),
            },
        );
        assert_eq!(with_cmd.route, SearchRoute::Web);
        assert!(with_cmd.standalone_question.contains("vàng"));
        // Explicit airport context still allows weather.
        let airport = PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Weather,
            standalone_question: "SJC airport weather".into(),
            queries: vec!["SJC airport weather".into()],
            explicit_search: false,
            lang: "en".into(),
        };
        let kept = sanitize_search_decision("SJC airport weather", airport);
        assert_eq!(kept.route, SearchRoute::Weather);
        // Flight/IATA tokens also mark airport context (not gold rewrite).
        assert!(user_signals_airport_context("SJC flight status"));
        // San Jose / mineta phrasing is also airport context.
        let jose = sanitize_search_decision(
            "weather at San Jose SJC",
            PrePassDecision {
                decision: SearchDecision::Web,
                route: SearchRoute::Weather,
                standalone_question: "SJC weather".into(),
                queries: vec!["SJC weather".into()],
                explicit_search: false,
                lang: "en".into(),
            },
        );
        assert_eq!(jose.route, SearchRoute::Weather);
        // Weather route with weather rewrite but empty body after `/search` alone:
        // demote route without rewriting queries onto empty text.
        let empty_body = sanitize_search_decision(
            "/search",
            PrePassDecision {
                decision: SearchDecision::Web,
                route: SearchRoute::Weather,
                standalone_question: "weather in Paris".into(),
                queries: vec!["paris weather".into()],
                explicit_search: true,
                lang: "en".into(),
            },
        );
        assert_eq!(empty_body.route, SearchRoute::Web);
        assert_eq!(empty_body.standalone_question, "weather in Paris");
        // Weather route but non-weather rewrite: demote route, leave standalone.
        let no_weather_rewrite = sanitize_search_decision(
            "capital of France",
            PrePassDecision {
                decision: SearchDecision::Web,
                route: SearchRoute::Weather,
                standalone_question: "capital of France".into(),
                queries: vec!["capital of France".into()],
                explicit_search: false,
                lang: "en".into(),
            },
        );
        assert_eq!(no_weather_rewrite.route, SearchRoute::Web);
        assert_eq!(no_weather_rewrite.standalone_question, "capital of France");
    }

    #[test]
    fn sanitize_drops_invented_weather_route_without_user_signal() {
        let classified = PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Weather,
            standalone_question: "weather in Paris".into(),
            queries: vec!["paris weather".into()],
            explicit_search: false,
            lang: "en".into(),
        };
        let out = sanitize_search_decision("tell me about Paris", classified);
        assert_eq!(out.route, SearchRoute::Web);
        assert_eq!(out.standalone_question, "tell me about Paris");
    }

    // ── sports vertical routing ────────────────────────────────────────────────

    /// A minimal valid ESPN scoreboard fixture: one completed match.
    const ESPN_SCOREBOARD_FIXTURE: &str = r#"{"leagues":[{"name":"National Basketball Association"}],"events":[{"name":"Lakers at Celtics","status":{"type":{"state":"post","completed":true,"shortDetail":"Final"}},"competitions":[{"competitors":[{"homeAway":"home","score":"110","team":{"displayName":"Boston Celtics"}},{"homeAway":"away","score":"102","team":{"displayName":"Los Angeles Lakers"}}]}]}]}"#;

    #[tokio::test]
    async fn sports_route_hit_answers_via_vertical_without_engines() {
        // The classifier routed to sports; the league keyword ("nba") also
        // matches. The vertical resolves the scoreboard and neither the news
        // feed nor the scraped engines are ever queried.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Sports,
            standalone_question: "what's the score of the nba game".into(),
            queries: vec!["nba score".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let espn_url =
            crate::websearch::sports::scoreboard_request("basketball", "nba", "2026-07-05").url;
        let transport = FakeHttpTransport::new().with_response(
            &espn_url,
            HttpResponse {
                status: 200,
                final_url: espn_url.clone(),
                body: ESPN_SCOREBOARD_FIXTURE.as_bytes().to_vec(),
            },
        );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "what's the score of the nba game",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(
            matches!(&outcome, SearchOutcome::Answer { messages, sources }
            if sources.len() == 1
                && sources[0].url == "https://www.espn.com/"
                && messages.first().is_some_and(|m| m.content.contains("Celtics")))
        );
        assert!(!transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn sports_keyword_hit_triggers_vertical_even_on_web_route() {
        // The classifier missed and routed to the general web tier, but the
        // deterministic league-keyword map still matches ("nfl"): the sports
        // vertical runs on its own signal, mirroring the weather/news verticals'
        // "own signal OR classifier route" gate.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "nfl scores this week".into(),
            queries: vec!["nfl scores".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let espn_url =
            crate::websearch::sports::scoreboard_request("football", "nfl", "2026-07-05").url;
        let transport = FakeHttpTransport::new().with_response(
            &espn_url,
            HttpResponse {
                status: 200,
                final_url: espn_url.clone(),
                body: ESPN_SCOREBOARD_FIXTURE.as_bytes().to_vec(),
            },
        );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "nfl scores this week",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://www.espn.com/"));
    }

    // ── explicit search (look-it-up) override ─────────────────────────────────

    #[tokio::test]
    async fn explicit_search_skips_verticals_and_reaches_engines() {
        // route=Sports with a matching league keyword WOULD normally answer from
        // the ESPN scoreboard. An explicit look-it-up request must skip it (and
        // every other vertical) and go straight to the scraped engines: the user
        // told us the vertical's answer was insufficient. The ForceWeb prefilter
        // signal ("score") also exercises resolve_decision's explicit_search
        // preservation.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Sports,
            standalone_question: "what's the nba score".into(),
            queries: vec!["nba score".into()],
            explicit_search: true,
            lang: "en".into(),
        }));
        let espn_url =
            crate::websearch::sports::scoreboard_request("basketball", "nba", "2026-07-05").url;
        let transport = transport_with_serp_and_page().with_response(
            &espn_url,
            HttpResponse {
                status: 200,
                final_url: espn_url.clone(),
                body: ESPN_SCOREBOARD_FIXTURE.as_bytes().to_vec(),
            },
        );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "nba score, can you look it up",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // The sports vertical was skipped: its scoreboard endpoint was never hit.
        assert!(
            !transport.calls().iter().any(|c| c.url == espn_url),
            "an explicit search must skip the sports vertical"
        );
        // The scraped engines ran instead.
        assert!(
            transport.calls().iter().any(|c| c.url == DDG_ENDPOINT),
            "an explicit search must reach the engines"
        );
        assert!(matches!(
            &outcome,
            SearchOutcome::Answer { .. } | SearchOutcome::Unreachable { .. }
        ));
    }

    #[tokio::test]
    async fn explicit_search_skips_the_cache_and_re_searches() {
        // A populated cache would normally answer a `cached` decision instantly.
        // An explicit look-it-up request must bypass the cache entirely and
        // re-search from the engines.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        cache.store(
            7,
            CachedSearch {
                standalone_question: "when was the treaty of versailles signed in paris".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://cached.example/".into(),
                    title: "Cached".into(),
                    text: "a stale cached answer".into(),
                }],
                route: SearchRoute::Web,
            },
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec!["treaty versailles paris".into()],
            explicit_search: true,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_with_cache(&prepass, &transport, &Bm25Scorer, &bound, &cache, 7),
            "sys",
            &[],
            "look it up please",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // Answered from the engines (the scraped page), never the cached source.
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://match.example/"));
        assert!(
            transport.calls().iter().any(|c| c.url == DDG_ENDPOINT),
            "an explicit search must bypass the cache and re-search"
        );
    }

    #[tokio::test]
    async fn explicit_search_forces_search_even_over_a_no_decision() {
        // The classifier returned `no` but flagged explicit_search: an explicit
        // look-it-up request is always a search, straight from the engines.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::No,
            route: SearchRoute::Web,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec!["treaty versailles paris".into()],
            explicit_search: true,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "look it up",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://match.example/"));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn force_search_overrides_a_force_no_greeting_and_reaches_the_engines() {
        // The `/search` command sets `deps.force_search`. Even a bare greeting,
        // which the deterministic pre-filter force-skips, must still search: the
        // ForceNo short-circuit is overridden, the classifier runs for the query
        // rewrite, and the turn goes straight to the scraped engines with the
        // "look it up again" cache-bypass semantics.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::No,
            route: SearchRoute::Web,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec!["treaty versailles paris".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let mut deps = deps(&prepass, &transport, &Bm25Scorer);
        deps.force_search = true;
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps,
            "sys",
            &[],
            "hi",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://match.example/"));
        assert!(
            transport.calls().iter().any(|c| c.url == DDG_ENDPOINT),
            "a forced /search must reach the engines even over a ForceNo greeting"
        );
    }

    #[tokio::test]
    async fn force_search_survives_a_classifier_error_on_a_force_no_turn() {
        // `/search` on a greeting whose classifier call also fails: the forced
        // search must fall back to searching the raw message from the engines
        // rather than dropping to a plain answer. Exercises both the ForceNo
        // override and the classifier-error `force_search` fall-through.
        let prepass = FakePrePass::returning(Err(InferenceError::Request("timeout".into())));
        let transport = transport_with_serp_and_page();
        let mut deps = deps(&prepass, &transport, &Bm25Scorer);
        deps.force_search = true;
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps,
            "sys",
            &[],
            "hi",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // The forced search reached the engines with the raw message rather than
        // abandoning to a plain answer; the loosely-relevant raw query may or may
        // not clear the relevance bar, so either a grounded answer or an
        // unreachable (searched-but-nothing-kept) outcome is correct here, never
        // NoSearch.
        assert!(matches!(
            &outcome,
            SearchOutcome::Answer { .. } | SearchOutcome::Unreachable { .. }
        ));
        assert!(
            transport.calls().iter().any(|c| c.url == DDG_ENDPOINT),
            "a forced /search must reach the engines even when the classifier errors"
        );
    }

    #[tokio::test]
    async fn explicit_search_bypasses_a_warm_serp_cache_and_still_requests_the_engine() {
        // A warm SERP cache entry for the exact query the classifier resolved
        // would normally be served with zero requests (see
        // `crate::websearch::engine`'s cache-hit tests). An explicit look-it-up
        // request must never be silently re-served that entry: the engine tier
        // is reached with `bypass_cache = true`, so the cache read is skipped
        // and DuckDuckGo is actually requested.
        let standalone = "when was the treaty of versailles signed in paris";
        let query = "treaty versailles paris";
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: standalone.into(),
            queries: vec![query.into()],
            explicit_search: true,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let web_cache = WebCache::new(
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(600),
            64,
            128,
        );
        // Pre-warm the cache with a stale hit that must NOT be what answers
        // this turn.
        let freshness = is_volatile_question(standalone);
        web_cache.serp_put(
            "duckduckgo",
            query,
            freshness,
            "en",
            vec![SearchHit {
                title: "Stale".into(),
                url: "https://stale.example/".into(),
                snippet: "a stale cached result".into(),
            }],
        );
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let judge = FakeSufficiencyJudge::sufficient();
        let health = EngineHealth::new();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_with_web_cache(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &web_cache,
            ),
            "sys",
            &[],
            "look it up again",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // The engine WAS requested despite the warm cache entry.
        assert!(
            transport.calls().iter().any(|c| c.url == DDG_ENDPOINT),
            "an explicit search must bypass the SERP cache and request the engine"
        );
        // The stale cached hit never reached the answer; the freshly fetched
        // page did.
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.iter().any(|s| s.url == "https://match.example/")
                && !sources.iter().any(|s| s.url == "https://stale.example/")));
    }

    #[tokio::test]
    async fn sports_tier_wins_over_news_when_both_signals_match() {
        // "game" trips the news intent gate AND "nba" trips the sports keyword
        // map. Both the ESPN scoreboard and the news feed have canned
        // responses, but the sports tier is positioned first and returns before
        // the news feed is ever queried, proving the tier ordering.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "what's the score of the nba game".into(),
            queries: vec!["nba game score".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let espn_url =
            crate::websearch::sports::scoreboard_request("basketball", "nba", "2026-07-05").url;
        let feed_url = crate::websearch::news::news_request("nba game score", false, "en").url;
        let transport = FakeHttpTransport::new()
            .with_response(
                &espn_url,
                HttpResponse {
                    status: 200,
                    final_url: espn_url.clone(),
                    body: ESPN_SCOREBOARD_FIXTURE.as_bytes().to_vec(),
                },
            )
            .with_response(
                &feed_url,
                HttpResponse {
                    status: 200,
                    final_url: feed_url.clone(),
                    body: br#"<rss><channel><item><title>Lakers fall to Celtics - NBA</title></item></channel></rss>"#.to_vec(),
                },
            );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "what's the score of the nba game",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://www.espn.com/"));
        assert!(
            !transport.calls().iter().any(|c| c.url == feed_url),
            "sports tier must intercept before the news feed is queried"
        );
    }

    #[tokio::test]
    async fn sports_miss_falls_through_to_news_tier() {
        // The league keyword matches ("nba"), but the ESPN endpoint has no
        // canned response (transport error -> miss). The turn still grounds via
        // the news feed, proving the fallthrough ordering.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::News,
            standalone_question: "what's the score of the nba game".into(),
            queries: vec!["nba game score".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let feed_url = crate::websearch::news::news_request("nba game score", false, "en").url;
        let transport = FakeHttpTransport::new().with_response(
            &feed_url,
            HttpResponse {
                status: 200,
                final_url: feed_url.clone(),
                body: br#"<rss><channel><item><title>Lakers fall to Celtics - NBA</title></item></channel></rss>"#.to_vec(),
            },
        );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "what's the score of the nba game",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://news.google.com/"));
    }

    #[tokio::test]
    async fn sports_miss_falls_through_to_engines_when_news_also_misses() {
        // League keyword matches but neither ESPN nor the news feed has a
        // canned response: falls all the way through to the scraped engines.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "nhl scores treaty versailles paris".into(),
            queries: vec!["treaty versailles paris".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Answer { .. }));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn cancel_during_sports_miss_yields_cancelled() {
        // A sports-shaped question whose scoreboard call cancels the token and
        // returns junk: the vertical misses, and the post-sports cancellation
        // check aborts before the news feed or any engine is queried.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Sports,
            standalone_question: "nba scores tonight".into(),
            queries: vec!["q".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let cancel = CancellationToken::new();
        let transport = CancelOnSend {
            token: cancel.clone(),
            serp: b"not scoreboard json".to_vec(),
        };
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &cancel,
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    // ── encyclopedia vertical routing ─────────────────────────────────────────

    #[tokio::test]
    async fn the_turns_language_comes_from_the_user_not_the_rewrite_or_the_locale() {
        // THE end-to-end regression. Every ingredient is deliberate:
        // - the user's Vietnamese carries NO Vietnamese-distinctive character,
        //   so script detection names nothing and only the classifier's `lang`
        //   can see it (this is the local-price question class, the highest
        //   value there is);
        // - the classifier ALSO emits an English companion query beside the
        //   native one (measured live: it does this on its own), and that
        //   English rewrite must not drag the turn back to the English feed;
        // - the machine's locale is `en-US`, so the locale cannot supply `vi`
        //   either. Only the classifier can, and it does.
        let question = "giá vàng hôm nay bao nhiêu";
        assert_eq!(
            crate::websearch::lang::detect_script_lang(question),
            None,
            "fixture invalid: this question must be invisible to script detection"
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::News,
            standalone_question: question.into(),
            queries: vec!["giá vàng hôm nay".into(), "gold price today".into()],
            explicit_search: false,
            lang: "vi".into(),
        }));
        let freshness = is_volatile_question(question);
        let feed_url =
            crate::websearch::news::news_request("giá vàng hôm nay", freshness, "vi").url;
        // The Vietnamese feed, derived from one allowlist row, so the triple
        // cannot disagree and silently serve English.
        assert!(feed_url.contains("ceid=VN%3Avi"), "{feed_url}");
        let feed = r#"<rss><channel><item><title>Giá vàng hôm nay tăng mạnh - VnExpress</title><pubDate>Tue, 14 Jul 2026 01:11:35 GMT</pubDate></item></channel></rss>"#;
        let transport = FakeHttpTransport::new().with_response(
            &feed_url,
            HttpResponse {
                status: 200,
                final_url: feed_url.clone(),
                body: feed.as_bytes().to_vec(),
            },
        );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            question,
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // The Vietnamese feed answered, so the request went out under `vi`.
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.len() == 1 && sources[0].url == "https://news.google.com/"));
        assert!(transport.calls().iter().any(|c| c.url == feed_url));
    }

    #[tokio::test]
    async fn encyclopedia_question_answers_via_vertical_without_engines() {
        // A stable factual question: Wikipedia search + summary resolve it and
        // the scraped engines are never queried.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Wiki,
            standalone_question: "what is photosynthesis".into(),
            queries: vec!["photosynthesis".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        // The turn resolves English (English message, English classifier `lang`,
        // `en-US` locale below), so the wiki vertical rides the English edition
        // and the canned URLs are keyed to it.
        let lang = "en";
        let search_url =
            crate::websearch::encyclopedia::search_request("what is photosynthesis", lang).url;
        let search_body = r#"{"query":{"search":[{"title":"Photosynthesis"}]}}"#;
        let summary_url =
            crate::websearch::encyclopedia::summary_request("Photosynthesis", lang).url;
        let summary_body = r#"{"type":"standard","title":"Photosynthesis","extract":"Photosynthesis is a system of biological processes.","content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Photosynthesis"}}}"#;
        let transport = FakeHttpTransport::new()
            .with_response(
                &search_url,
                HttpResponse {
                    status: 200,
                    final_url: search_url.clone(),
                    body: search_body.as_bytes().to_vec(),
                },
            )
            .with_response(
                &summary_url,
                HttpResponse {
                    status: 200,
                    final_url: summary_url.clone(),
                    body: summary_body.as_bytes().to_vec(),
                },
            );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "what is photosynthesis",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(
            matches!(&outcome, SearchOutcome::Answer { messages, sources }
            if sources.len() == 1
                && sources[0].url == "https://en.wikipedia.org/wiki/Photosynthesis"
                && messages.first().is_some_and(|m| m.content.contains("Photosynthesis is a system")))
        );
        assert!(!transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn cancel_during_encyclopedia_miss_yields_cancelled() {
        // An encyclopedic-shaped question whose search call cancels the token
        // and returns junk: the vertical misses, and the post-encyclopedia
        // cancellation check aborts before any engine is queried.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Wiki,
            standalone_question: "what is xyzzyplace".into(),
            queries: vec!["q".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let cancel = CancellationToken::new();
        let transport = CancelOnSend {
            token: cancel.clone(),
            serp: b"not search json".to_vec(),
        };
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &cancel,
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    #[tokio::test]
    async fn encyclopedia_miss_falls_through_to_engines() {
        // Encyclopedic-shaped question but the search call fails (no canned
        // response): the engines run and ground the answer as usual.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Wiki,
            standalone_question: "what is the treaty of versailles game".into(),
            queries: vec!["treaty versailles paris".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Answer { .. }));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    // ── route-respect hint gating ─────────────────────────────────────────────
    // The classifier's explicit route outranks a vertical's keyword hint. A hint
    // only claims a turn routed to that same vertical or to Web (plus the one
    // news->sports upgrade). These four cover the decision matrix.

    #[tokio::test]
    async fn route_sports_with_news_keyword_skips_news_and_reaches_engines() {
        // The exact observed F1 shape: route=sports, but the question carries a
        // news token ("won"/"championship") and NO league keyword, so the sports
        // vertical self-misses (no_league_match). News must NOT steal the turn
        // (route is sports, not news/web); the engines run instead.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Sports,
            standalone_question: "who won the championship treaty versailles paris".into(),
            queries: vec!["treaty versailles paris".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Answer { .. }));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
        // News was never consulted: the route-respect rule kept it out.
        assert!(
            !transport
                .calls()
                .iter()
                .any(|c| c.url.starts_with("https://news.google.com/rss")),
            "news must not claim a sports-routed turn"
        );
    }

    #[tokio::test]
    async fn route_web_with_news_keyword_runs_news() {
        // route=web + a news token: the news hint legitimately rescues a
        // classifier miss on a web-routed turn, so the feed answers.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "who won the big game tonight".into(),
            queries: vec!["big game result".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let feed_url = crate::websearch::news::news_request("big game result", false, "en").url;
        let transport = FakeHttpTransport::new().with_response(
            &feed_url,
            HttpResponse {
                status: 200,
                final_url: feed_url.clone(),
                body: br#"<rss><channel><item><title>Home team wins big - Sports Daily</title></item></channel></rss>"#.to_vec(),
            },
        );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "who won the big game tonight",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://news.google.com/"));
    }

    #[tokio::test]
    async fn route_news_with_sports_keyword_upgrades_to_sports() {
        // route=news + a league keyword ("nba"): the deliberate news->sports
        // upgrade fires (a scoreboard beats headlines for a score question), so
        // the sports vertical answers and the news feed is never queried.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::News,
            standalone_question: "what's the nba score tonight".into(),
            queries: vec!["nba score".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let espn_url =
            crate::websearch::sports::scoreboard_request("basketball", "nba", "2026-07-05").url;
        let transport = FakeHttpTransport::new().with_response(
            &espn_url,
            HttpResponse {
                status: 200,
                final_url: espn_url.clone(),
                body: ESPN_SCOREBOARD_FIXTURE.as_bytes().to_vec(),
            },
        );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "what's the nba score tonight",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://www.espn.com/"));
        assert!(
            !transport
                .calls()
                .iter()
                .any(|c| c.url.starts_with("https://news.google.com/rss")),
            "the news->sports upgrade must intercept before the feed"
        );
    }

    #[tokio::test]
    async fn route_wiki_with_news_keyword_skips_news() {
        // route=wiki + a news token: news must NOT claim the turn. The wiki
        // vertical misses (no canned Wikipedia response) and the turn falls to
        // the engines; the news feed is never queried.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Wiki,
            standalone_question: "who won the election treaty versailles paris".into(),
            queries: vec!["treaty versailles paris".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Answer { .. }));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
        assert!(
            !transport
                .calls()
                .iter()
                .any(|c| c.url.starts_with("https://news.google.com/rss")),
            "news must not claim a wiki-routed turn"
        );
    }

    // ── ForceWeb raw-query race helpers (pure) ───────────────────────────────

    #[test]
    fn force_web_should_race_raw_skips_vertical_shaped() {
        assert!(!force_web_should_race_raw("weather in Tokyo"));
        assert!(!force_web_should_race_raw("độ ẩm Hà Nội"));
        assert!(!force_web_should_race_raw("NBA Lakers score tonight"));
        assert!(!force_web_should_race_raw("latest news about tariffs"));
        assert!(force_web_should_race_raw("what is the latest rust version"));
        assert!(force_web_should_race_raw("SJC"));
    }

    #[test]
    fn race_lang_for_force_web_is_script_only() {
        // No locale arg: English text defaults to en even when the machine is
        // vi_VN (that was the live race regression).
        assert_eq!(
            race_lang_for_force_web("what is the latest rust version"),
            "en"
        );
        assert_eq!(race_lang_for_force_web("thời tiết Hà Nội hôm nay"), "vi");
        assert_eq!(race_lang_for_force_web("東京の天気は"), "ja");
        assert_eq!(race_lang_for_force_web("SJC"), "en");
    }

    #[test]
    fn preloaded_serp_keeps_near_duplicate_web_including_empty() {
        let decision = PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "latest rust version".into(),
            queries: vec!["latest rust version".into()],
            explicit_search: false,
            lang: "en".into(),
        };
        let empty = preloaded_serp_for_decision(
            &decision,
            "latest rust version",
            "en",
            Some((vec![], vec![], "en")),
        );
        assert!(empty.is_some());
        let hit = SearchHit {
            title: "t".into(),
            url: "https://example.com/".into(),
            snippet: "s".into(),
        };
        let kept = preloaded_serp_for_decision(
            &decision,
            "  Latest  Rust  Version ",
            "en",
            Some((vec![hit], vec![], "en")),
        );
        assert_eq!(kept.unwrap().0.len(), 1);
    }

    #[test]
    fn preloaded_serp_drops_divergent_and_non_web() {
        let web = PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "adobe figma deal".into(),
            queries: vec!["adobe figma deal".into()],
            explicit_search: false,
            lang: "en".into(),
        };
        assert!(preloaded_serp_for_decision(
            &web,
            "latest figma ownership",
            "en",
            Some((
                vec![SearchHit {
                    title: "t".into(),
                    url: "https://example.com/".into(),
                    snippet: "s".into(),
                }],
                vec![],
                "en"
            )),
        )
        .is_none());
        let cached = PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "latest rust version".into(),
            queries: vec!["latest rust version".into()],
            explicit_search: false,
            lang: "en".into(),
        };
        assert!(preloaded_serp_for_decision(
            &cached,
            "latest rust version",
            "en",
            Some((vec![], vec![], "en")),
        )
        .is_none());
    }

    #[test]
    fn preloaded_serp_drops_when_race_lang_mismatches_final() {
        let decision = PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "latest rust version".into(),
            queries: vec!["latest rust version".into()],
            explicit_search: false,
            lang: "en".into(),
        };
        // Near-duplicate rewrite would keep, but race ran under vi and final is en.
        assert!(preloaded_serp_for_decision(
            &decision,
            "latest rust version",
            "en",
            Some((
                vec![SearchHit {
                    title: "t".into(),
                    url: "https://example.com/".into(),
                    snippet: "s".into(),
                }],
                vec![],
                "vi"
            )),
        )
        .is_none());
    }

    #[test]
    fn preloaded_serp_keeps_when_race_lang_matches_final() {
        let decision = PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "latest rust version".into(),
            queries: vec!["latest rust version".into()],
            explicit_search: false,
            lang: "en".into(),
        };
        let kept = preloaded_serp_for_decision(
            &decision,
            "latest rust version",
            "en",
            Some((
                vec![SearchHit {
                    title: "t".into(),
                    url: "https://example.com/".into(),
                    snippet: "s".into(),
                }],
                vec![],
                "en",
            )),
        );
        assert_eq!(kept.unwrap().0.len(), 1);
    }

    #[tokio::test]
    async fn force_web_race_classifier_cancel_yields_cancelled() {
        // ForceWeb + engine-shaped race: classifier cancel must surface Cancelled
        // even though the concurrent SERP may still complete.
        let prepass = FakePrePass::returning(Err(InferenceError::Cancelled));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "what is the latest rust version",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    #[tokio::test]
    async fn force_web_near_duplicate_race_uses_one_ddg_round() {
        // Engine-shaped ForceWeb with rewrite ≈ raw: race SERP is kept; engines
        // must not be hit a second time for the same query.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "what is the latest rust version".into(),
            queries: vec!["what is the latest rust version".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let _ = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "what is the latest rust version",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let ddg_calls = transport
            .calls()
            .into_iter()
            .filter(|c| c.url == DDG_ENDPOINT)
            .count();
        assert_eq!(
            ddg_calls, 1,
            "near-duplicate ForceWeb race must keep common path at 1 DDG"
        );
    }

    #[tokio::test]
    async fn force_web_keep_still_serps_secondary_distinct_query() {
        // H1: race keep seeds first near-dupe query, but a distinct second
        // classifier query must still hit the engine when early-stop is not met.
        let raw = "what is the latest rust version";
        let secondary = "rust 1.80 release notes changelog";
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: raw.into(),
            queries: vec![raw.into(), secondary.into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let _ = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            raw,
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let ddg_qs: Vec<String> = transport
            .calls()
            .into_iter()
            .filter(|c| c.url == DDG_ENDPOINT)
            .filter_map(|c| c.form.into_iter().find(|(k, _)| k == "q").map(|(_, v)| v))
            .collect();
        assert_eq!(
            ddg_qs.len(),
            2,
            "race keep must still SERP the distinct secondary query, got {ddg_qs:?}"
        );
        assert!(
            ddg_qs
                .iter()
                .any(|q| q == raw || queries_near_duplicate(q, raw)),
            "race raw query should appear, got {ddg_qs:?}"
        );
        assert!(
            ddg_qs.iter().any(|q| q == secondary),
            "distinct secondary must be SERPed, got {ddg_qs:?}"
        );
    }

    /// Classifier that sleeps before returning, so concurrent race timing tests
    /// can prove classifier wall time is measured independently of SERP.
    struct DelayedPrePass {
        delay: std::time::Duration,
        result: Result<PrePassDecision, InferenceError>,
    }

    #[async_trait]
    impl PrePass for DelayedPrePass {
        /// Sleeps `delay`, then returns the scripted result.
        async fn decide(
            &self,
            _history: &[ChatMessage],
            _latest_user_message: &str,
            _latest_images: Option<&[String]>,
            _today: &str,
            _cancel: &CancellationToken,
        ) -> Result<PrePassDecision, InferenceError> {
            tokio::time::sleep(self.delay).await;
            self.result.clone()
        }
    }

    /// Transport that sleeps before each send, so SERP wall time can be made
    /// much larger than a fast classifier without relying on real network.
    struct DelayedTransport {
        inner: FakeHttpTransport,
        delay: std::time::Duration,
    }

    #[async_trait]
    impl HttpTransport for DelayedTransport {
        /// Sleeps `delay`, then delegates to the inner fake transport.
        async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
            tokio::time::sleep(self.delay).await;
            self.inner.send(req).await
        }
    }

    #[tokio::test]
    async fn force_web_race_records_independent_classifier_and_serp_times() {
        // Regression: stage Instant must live inside each concurrent future.
        // If both stages snap Instant before join and record after join, both
        // report max(classifier, serp). With a ~20ms classifier and ~120ms SERP,
        // classifier ms must stay well below serp ms (not both ≈ serp).
        let prepass = DelayedPrePass {
            delay: std::time::Duration::from_millis(20),
            result: Ok(PrePassDecision {
                decision: SearchDecision::Web,
                route: SearchRoute::Web,
                standalone_question: "what is the latest rust version".into(),
                queries: vec!["what is the latest rust version".into()],
                explicit_search: false,
                lang: "en".into(),
            }),
        };
        let transport = DelayedTransport {
            inner: transport_with_serp_and_page(),
            delay: std::time::Duration::from_millis(120),
        };
        let timings = TimingBag::new();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let mut d = deps_with_recorder(&prepass, &transport, &Bm25Scorer, &bound);
        d.timings = &timings;
        let _ = run_search(
            &d,
            "sys",
            &[],
            "what is the latest rust version",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let stages = timings.snapshot();
        let classifier_ms = stages
            .iter()
            .find(|s| s.stage == STAGE_CLASSIFIER)
            .map(|s| s.ms)
            .expect("classifier stage");
        let race_ms = stages
            .iter()
            .find(|s| s.stage == STAGE_RAW_RACE_SERP)
            .map(|s| s.ms)
            .expect("raw_race_serp stage");
        // Allow scheduling jitter: classifier should be far below SERP delay.
        assert!(
            classifier_ms < race_ms.saturating_sub(40),
            "classifier={classifier_ms}ms must be independent of serp={race_ms}ms (not both max)"
        );
        assert!(
            race_ms >= 80,
            "serp stage should reflect its own ~120ms delay, got {race_ms}ms"
        );
        // Trace flush also carries both stage names.
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        let flushed = events
            .iter()
            .find_map(|e| match e {
                RecorderEvent::SearchTimings { stages } => Some(stages),
                _ => None,
            })
            .expect("SearchTimings");
        assert!(flushed.iter().any(|s| s.stage == STAGE_CLASSIFIER));
        assert!(flushed.iter().any(|s| s.stage == STAGE_RAW_RACE_SERP));
    }

    #[tokio::test]
    async fn non_force_web_does_not_race_raw_serp() {
        // Ambiguous turn: sequential classifier only; one SERP after decision.
        let prepass = FakePrePass::returning(Ok(web_decision(vec![
            "when was the treaty of versailles signed in paris",
        ])));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let _ = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "when was the treaty of versailles signed in paris",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // No raw_race stage: only the post-decision SERP (and no double race).
        let ddg_calls = transport
            .calls()
            .into_iter()
            .filter(|c| c.url == DDG_ENDPOINT)
            .count();
        assert_eq!(ddg_calls, 1);
    }

    // ── resolve_decision (pure) ───────────────────────────────────────────────

    #[test]
    fn resolve_force_web_overrides_no_and_backfills_queries() {
        let out = resolve_decision(
            PreFilterVerdict::ForceWeb,
            PrePassDecision {
                decision: SearchDecision::No,
                route: SearchRoute::Weather,
                standalone_question: "current tokyo weather".into(),
                queries: vec![],
                explicit_search: false,
                lang: "en".into(),
            },
        );
        assert_eq!(out.decision, SearchDecision::Web);
        assert_eq!(out.queries, vec!["current tokyo weather"]);
    }

    #[test]
    fn resolve_force_web_keeps_existing_queries() {
        let out = resolve_decision(
            PreFilterVerdict::ForceWeb,
            PrePassDecision {
                decision: SearchDecision::No,
                route: SearchRoute::News,
                standalone_question: "q".into(),
                queries: vec!["a".into(), "b".into()],
                explicit_search: false,
                lang: "en".into(),
            },
        );
        assert_eq!(out.decision, SearchDecision::Web);
        assert_eq!(out.queries, vec!["a", "b"]);
        // ForceWeb overrides only the yes/no decision, never the route hint.
        assert_eq!(out.route, SearchRoute::News);
    }

    #[test]
    fn resolve_force_web_preserves_cached_decision() {
        // The exact bug this preserves against: "what's the latest stable
        // Rust version" carries the deterministic "latest" freshness word, so
        // the pre-filter forces `ForceWeb` on every turn that asks it, even a
        // repeat of the same question. If the classifier judges the repeat
        // answerable from what was just fetched (`cached`), `ForceWeb` must
        // NOT downgrade that back to a fresh `web` search: sources fetched
        // moments ago this same conversation are already at least as fresh as
        // a re-search would find.
        let out = resolve_decision(
            PreFilterVerdict::ForceWeb,
            PrePassDecision {
                decision: SearchDecision::Cached,
                route: SearchRoute::Web,
                standalone_question: "what's the latest stable rust version".into(),
                queries: vec!["rust latest stable version".into()],
                explicit_search: false,
                lang: "en".into(),
            },
        );
        assert_eq!(out.decision, SearchDecision::Cached);
    }

    #[test]
    fn resolve_ambiguous_keeps_classifier_decision() {
        let classified = PrePassDecision {
            decision: SearchDecision::No,
            route: SearchRoute::Wiki,
            standalone_question: "q".into(),
            queries: vec![],
            explicit_search: false,
            lang: "en".into(),
        };
        let out = resolve_decision(PreFilterVerdict::Ambiguous, classified.clone());
        assert_eq!(out, classified);
    }

    #[test]
    fn resolve_force_no_passes_classifier_through_unchanged() {
        // Totality: ForceNo does not reach this in run_search, but the function
        // is total and leaves the input untouched.
        let classified = PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Weather,
            standalone_question: "q".into(),
            queries: vec!["a".into()],
            explicit_search: false,
            lang: "en".into(),
        };
        let out = resolve_decision(PreFilterVerdict::ForceNo, classified.clone());
        assert_eq!(out, classified);
    }

    #[tokio::test]
    async fn prepass_cancelled_yields_cancelled() {
        let prepass = FakePrePass::returning(Err(InferenceError::Cancelled));
        let transport = FakeHttpTransport::new();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    #[tokio::test]
    async fn classifier_error_on_force_web_turn_still_searches() {
        // The deterministic ForceWeb signal survives a classifier infra failure
        // (e.g. a timed-out reasoning-heavy call): the raw message becomes the
        // query and the search proceeds. The fixture SERP/page ground it.
        let prepass = FakePrePass::returning(Err(InferenceError::Request("timeout".into())));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            // ForceWeb via "latest"; words overlap the fixture page so BM25
            // keeps chunks and the answer grounds.
            "latest treaty versailles paris",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Answer { .. }));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn prepass_infra_error_degrades_to_no_search() {
        let prepass = FakePrePass::returning(Err(InferenceError::Request("boom".into())));
        let transport = FakeHttpTransport::new();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::NoSearch));
    }

    // ── run_search: web pipeline ──────────────────────────────────────────────

    #[tokio::test]
    async fn web_decision_produces_grounded_answer() {
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["treaty versailles paris"])));
        let transport = transport_with_serp_and_page();
        let (phases, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "when signed",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(
            matches!(&outcome, SearchOutcome::Answer { messages, sources }
            if sources.len() == 1
                && sources[0].url == "https://match.example/"
                && messages.first().is_some_and(|m| m.role == "system"
                    && m.content.contains("UNTRUSTED_WEB_CONTENT")
                    && m.content.contains("treaty"))
                && messages.last().is_some_and(|m| m.role == "user"
                    && m.content == "when signed"))
        );
        assert_eq!(
            *phases.lock().unwrap(),
            vec![
                SearchPhase::Deciding,
                SearchPhase::Searching,
                SearchPhase::Reading
            ]
        );
    }

    #[tokio::test]
    async fn conflicting_engine_verdict_flags_writer_and_skips_requery() {
        // End-to-end seam for item (a): a conflicting engine-tier verdict must
        // reach the writer prompt as the conflict directive, and must NOT fire a
        // requery (a disagreement is not searchable). Drives the full run_search
        // engine path with a judge that returns reason=Conflicting, proving the
        // judge_and_requery -> grounded_answer -> writer_messages forwarding.
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["treaty versailles paris"])));
        let transport = transport_with_serp_and_page();
        let judge = FakeSufficiencyJudge::returning(Ok(SufficiencyVerdict {
            sufficient: false,
            missing: "the exact figure".into(),
            reason: InsufficiencyReason::Conflicting,
            requery_queries: Vec::new(),
        }));
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let (_phases, status) = recorder();
        let outcome = run_search(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            "sys",
            &[],
            "when signed",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // The system turn carries the conflict directive alongside the
        // untrusted-source region: the forwarded flag survived every hop.
        assert!(matches!(&outcome, SearchOutcome::Answer { messages, .. }
            if messages.first().is_some_and(|m| m.role == "system"
                && m.content.contains("The sources disagree on a value")
                && m.content.contains("UNTRUSTED_WEB_CONTENT"))
                && messages.last().is_some_and(|m| m.content == "when signed")));
        // No requery fired: a conflict commits directly, so no SearchRequeried.
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(!events
            .iter()
            .any(|e| matches!(e, RecorderEvent::SearchRequeried { .. })));
    }

    /// Two hits whose extracted article bodies are byte-identical (so BM25
    /// gives them equal relevance), differing only by URL and an embedded
    /// JSON-LD `datePublished`, `old` published far further in the past than
    /// `new`. Used to prove the recency pass actually reorders the engine
    /// tier's `sources` end to end, gated by the turn's freshness signal.
    fn transport_with_two_dated_pages(
        old: OffsetDateTime,
        new: OffsetDateTime,
    ) -> FakeHttpTransport {
        let serp = r#"
          <div class="result">
            <a class="result__a" href="https://old.example/">Springfield population</a>
            <a class="result__snippet">springfield population figures</a>
          </div>
          <div class="result">
            <a class="result__a" href="https://new.example/">Springfield population</a>
            <a class="result__snippet">springfield population figures</a>
          </div>
        "#;
        let article = |published: OffsetDateTime| {
            format!(
                r#"<html><head>
                  <script type="application/ld+json">{{"datePublished":"{}"}}</script>
                </head><body><article><h1>Springfield population</h1>
                <p>Officials released updated population figures for Springfield this
                week, citing new housing developments and job growth as the main
                drivers behind the change across the metro region.</p>
                <p>Local planners expect the population trend to continue as more
                residents move in seeking affordable housing and employment in the
                growing local economy of Springfield.</p>
                </article></body></html>"#,
                published.format(&Rfc3339).unwrap()
            )
        };
        FakeHttpTransport::new()
            .with_response(
                DDG_ENDPOINT,
                HttpResponse {
                    status: 200,
                    final_url: DDG_ENDPOINT.into(),
                    body: serp.as_bytes().to_vec(),
                },
            )
            .with_response(
                "https://old.example/",
                HttpResponse {
                    status: 200,
                    final_url: "https://old.example/".into(),
                    body: article(old).into_bytes(),
                },
            )
            .with_response(
                "https://new.example/",
                HttpResponse {
                    status: 200,
                    final_url: "https://new.example/".into(),
                    body: article(new).into_bytes(),
                },
            )
    }

    #[tokio::test]
    async fn fresh_query_reorders_equally_relevant_sources_by_recency() {
        // "latest" is a WIKI_VOLATILITY_MARKERS token, so is_volatile_question
        // (the freshness gate `run_engine_tier` reuses) is true for this
        // standalone question.
        let standalone = "what is the latest population of springfield";
        let now = OffsetDateTime::now_utc();
        let old = now - time::Duration::days(90);
        let new = now - time::Duration::days(1);
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: standalone.into(),
            queries: vec!["springfield population".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_two_dated_pages(old, new);
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            standalone,
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // Both sources are equally relevant (identical article text): the
        // freshness-gated recency pass must put the newer one first.
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://new.example/"
                && sources[1].url == "https://old.example/"));
    }

    #[tokio::test]
    async fn non_fresh_query_leaves_the_same_dated_sources_unreordered() {
        // Same two dated, equally relevant pages as the fresh-query test
        // above, but a standalone question carrying no freshness marker: the
        // recency pass must not run, so the order stays exactly the
        // relevance/SERP order (old.example first, matching its SERP rank).
        let standalone = "history of the springfield population";
        let now = OffsetDateTime::now_utc();
        let old = now - time::Duration::days(90);
        let new = now - time::Duration::days(1);
        assert!(!is_volatile_question(standalone));
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: standalone.into(),
            queries: vec!["springfield population".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_two_dated_pages(old, new);
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            standalone,
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://old.example/"
                && sources[1].url == "https://new.example/"));
    }

    #[tokio::test]
    async fn web_dedupes_repeated_queries() {
        // Two identical queries hit the same SERP; the duplicate hit is deduped,
        // so only the one page is fetched.
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q one", "q two"])));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let _ = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // Two SERP POSTs + exactly one page GET (deduped).
        let page_gets = transport
            .calls()
            .into_iter()
            .filter(|c| c.url == "https://match.example/")
            .count();
        assert_eq!(page_gets, 1);
    }

    #[tokio::test]
    async fn web_with_all_engines_transport_error_yields_unreachable_disclosure() {
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q"])));
        // Every engine transport-fails: an empty FakeHttpTransport has no canned
        // response for either engine, so each `send` errors. Zero engines reached
        // the web -> the reason is `Unreachable` and the disclosure says the web
        // could not be reached.
        let transport = FakeHttpTransport::new();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "what is the latest rust version",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome,
            SearchOutcome::Unreachable { messages, reason: SearchFailReason::Unreachable }
            if messages.first().is_some_and(|m| m.role == "system"
                && m.content.contains("no web sources could be retrieved"))
                && messages.last().is_some_and(|m|
                    m.content == "what is the latest rust version")));
    }

    /// A transport whose ENGINE (SERP) requests stall for `delay` before
    /// resolving through `inner`; page fetches are served instantly. Models the
    /// two networks the offline fast-fail exists to tell apart: a dead link
    /// (`inner` has no canned SERP, so the stalled request ends in a transport
    /// error, exactly as a real connect/request timeout does) and a
    /// slow-but-working one (`inner` serves a real SERP and page, just late).
    struct StalledEngineTransport {
        delay: std::time::Duration,
        inner: FakeHttpTransport,
    }

    #[async_trait]
    impl HttpTransport for StalledEngineTransport {
        async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
            if req.url.contains("duckduckgo") || req.url.contains("mojeek") {
                tokio::time::sleep(self.delay).await;
            }
            self.inner.send(req).await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn offline_turn_short_circuits_to_unreachable_inside_the_grace_window() {
        // Two queries, every engine request stalling for the full per-request
        // timeout: without the fast-fail this turn burns the stacked
        // per-engine, per-query timeouts (~30 s here, ~46 s in production with
        // the connect timeout on top) before concluding what the probe already
        // proved. With it, the same honest Unreachable disclosure lands inside
        // the grace window.
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q one", "q two"])));
        let transport = StalledEngineTransport {
            delay: std::time::Duration::from_secs(crate::config::defaults::HTTP_REQUEST_TIMEOUT_S),
            inner: FakeHttpTransport::new(),
        };
        let probe = FakeReachability::returning(ReachabilityVerdict::Unreachable);
        let (_p, status) = recorder();
        let started = tokio::time::Instant::now();
        let outcome = run_search(
            &deps_with_probe(&prepass, &transport, &Bm25Scorer, &probe),
            "sys",
            &[],
            "what is the latest rust version",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let elapsed = started.elapsed();
        assert!(matches!(&outcome,
            SearchOutcome::Unreachable { messages, reason: SearchFailReason::Unreachable }
            if messages.first().is_some_and(|m|
                m.content.contains("no web sources could be retrieved"))));
        // The whole point: about a window, not tens of seconds of timeouts.
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "offline turn took {elapsed:?}, expected the ~{}ms grace window",
            crate::config::defaults::OFFLINE_SHORTCIRCUIT_WINDOW_MS
        );
    }

    /// Non-ForceWeb path: no raw-SERP race under the classifier, so the offline
    /// cut lands inside [`run_engine_tier`] itself (the SERP round race). Covers
    /// the post-#324 merge of reachability into the preloaded-SERP engine tier.
    #[tokio::test(start_paused = true)]
    async fn offline_engine_tier_short_circuits_on_ambiguous_turn() {
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q one", "q two"])));
        let transport = StalledEngineTransport {
            delay: std::time::Duration::from_secs(crate::config::defaults::HTTP_REQUEST_TIMEOUT_S),
            inner: FakeHttpTransport::new(),
        };
        let probe = FakeReachability::returning(ReachabilityVerdict::Unreachable);
        let (_p, status) = recorder();
        let started = tokio::time::Instant::now();
        let outcome = run_search(
            &deps_with_probe(&prepass, &transport, &Bm25Scorer, &probe),
            "sys",
            &[],
            // No ForceWeb prefilter keywords: Ambiguous → classifier only, then
            // engine tier (where the offline race lives for this shape).
            "when was the treaty of versailles signed",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let elapsed = started.elapsed();
        assert!(matches!(
            &outcome,
            SearchOutcome::Unreachable {
                reason: SearchFailReason::Unreachable,
                ..
            }
        ));
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "engine-tier offline cut took {elapsed:?}, expected the grace window"
        );
    }

    /// ForceWeb raw race offline + classifier Cancelled: user cancel wins over
    /// the offline disclosure (same preference as a cancel mid-classifier).
    #[tokio::test(start_paused = true)]
    async fn offline_force_web_race_prefers_classifier_cancel() {
        let prepass = FakePrePass::returning(Err(InferenceError::Cancelled));
        let transport = StalledEngineTransport {
            delay: std::time::Duration::from_secs(crate::config::defaults::HTTP_REQUEST_TIMEOUT_S),
            inner: FakeHttpTransport::new(),
        };
        let probe = FakeReachability::returning(ReachabilityVerdict::Unreachable);
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_with_probe(&prepass, &transport, &Bm25Scorer, &probe),
            "sys",
            &[],
            "what is the latest rust version",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    /// THE false-positive guard: a working-but-slow network (hotel Wi-Fi, a
    /// congested tether) must never be told it is offline. The probe says
    /// reachable, so no cutoff fires however long the engines take, and the real
    /// result wins: an Answer, not an Unreachable disclosure.
    #[tokio::test(start_paused = true)]
    async fn slow_but_reachable_network_is_never_told_it_is_offline() {
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q"])));
        // Far longer than the grace window, so a cutoff that wrongly fired would
        // beat the real fetch and be caught here.
        let slow = std::time::Duration::from_secs(5);
        let transport = StalledEngineTransport {
            delay: slow,
            inner: transport_with_serp_and_page(),
        };
        let probe = FakeReachability::returning(ReachabilityVerdict::Reachable);
        let (_p, status) = recorder();
        let started = tokio::time::Instant::now();
        let outcome = run_search(
            &deps_with_probe(&prepass, &transport, &Bm25Scorer, &probe),
            "sys",
            &[],
            "when was the treaty of versailles signed",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(
            matches!(&outcome, SearchOutcome::Answer { .. }),
            "a slow but reachable network must still get its real answer"
        );
        // And it really did wait for the slow engines rather than short-circuit.
        assert!(started.elapsed() >= slow);
    }

    /// An inconclusive probe (one that never answers, so [`offline_cutoff`] hits
    /// its deadline) is not evidence of being offline either: same
    /// slow-but-working network, same real answer.
    #[tokio::test(start_paused = true)]
    async fn inconclusive_probe_does_not_short_circuit() {
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q"])));
        let slow = std::time::Duration::from_secs(5);
        let transport = StalledEngineTransport {
            delay: slow,
            inner: transport_with_serp_and_page(),
        };
        let probe = FakeReachability::hanging();
        let (_p, status) = recorder();
        let started = tokio::time::Instant::now();
        let outcome = run_search(
            &deps_with_probe(&prepass, &transport, &Bm25Scorer, &probe),
            "sys",
            &[],
            "when was the treaty of versailles signed",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { .. }));
        assert!(started.elapsed() >= slow);
    }

    #[tokio::test]
    async fn web_with_blocked_engine_yields_no_results_disclosure() {
        // Query matches the raw user message so a ForceWeb race (if fired)
        // is near-duplicate-kept and its blocked-engine stats drive NoResults
        // without a second SERP that would only see DDG cooling.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "what is the latest rust version".into(),
            queries: vec!["what is the latest rust version".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        // DDG returns a bot challenge (blocked, but online) and Mojeek transport-
        // fails. At least one engine reached the web, so the miss is `NoResults`,
        // not `Unreachable`: the disclosure says no usable current sources were
        // found rather than blaming the connection.
        let transport = FakeHttpTransport::new().with_response(
            DDG_ENDPOINT,
            HttpResponse {
                status: 202,
                final_url: DDG_ENDPOINT.into(),
                body: b"<div class=\"anomaly-modal\">challenge-form</div>".to_vec(),
            },
        );
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "what is the latest rust version",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome,
            SearchOutcome::Unreachable { messages, reason: SearchFailReason::NoResults }
            if messages.first().is_some_and(|m| m.role == "system"
                && m.content.contains("no usable current sources"))
                && messages.last().is_some_and(|m|
                    m.content == "what is the latest rust version")));
    }

    #[tokio::test]
    async fn web_with_no_relevant_chunks_yields_unreachable_disclosure() {
        // The page has real text but shares no term with the standalone question,
        // so BM25 keeps nothing: search was wanted, nothing citable -> disclose.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "quantum chromodynamics lagrangian".into(),
            queries: vec!["q".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // Engines returned hits but nothing survived ranking: reached the web,
        // so this resolves to `NoResults`, not a connectivity failure.
        assert!(matches!(
            outcome,
            SearchOutcome::Unreachable {
                reason: SearchFailReason::NoResults,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn query_loop_stops_early_once_enough_hits() {
        // A SERP with more rows than the early-stop threshold: the first query
        // satisfies it, so the second query is never sent (one SERP POST total).
        let many_rows: String = (0..SERP_EARLY_STOP_HITS + 2)
            .map(|i| {
                format!(
                    "<div class=\"result\"><a class=\"result__a\" href=\"https://s{i}.example/\">Treaty {i}</a>\
                     <a class=\"result__snippet\">the treaty signed in paris</a></div>"
                )
            })
            .collect();
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q one", "q two"])));
        let transport = FakeHttpTransport::new()
            .with_response(
                DDG_ENDPOINT,
                HttpResponse {
                    status: 200,
                    final_url: DDG_ENDPOINT.into(),
                    body: many_rows.into_bytes(),
                },
            )
            .with_response(
                "https://s0.example/",
                HttpResponse {
                    status: 200,
                    final_url: "https://s0.example/".into(),
                    body: PAGE_HTML.as_bytes().to_vec(),
                },
            );
        let (_p, status) = recorder();
        let _ = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let serp_posts = transport
            .calls()
            .into_iter()
            .filter(|c| c.url == DDG_ENDPOINT)
            .count();
        assert_eq!(
            serp_posts, 1,
            "second query should be skipped by early stop"
        );
    }

    #[tokio::test]
    async fn cancel_before_search_yields_cancelled() {
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q"])));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &cancel,
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    #[tokio::test]
    async fn cancel_during_query_loop_yields_cancelled() {
        // Two queries; the transport cancels on the first send, so the second
        // loop iteration's cancellation check aborts before searching again.
        // The standalone question deliberately matches no vertical's intent
        // (weather/news/encyclopedia), so the turn reaches the query loop
        // instead of being absorbed by a vertical's own cancellation check.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "xyzzyplace status report".into(),
            queries: vec!["q one".into(), "q two".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let cancel = CancellationToken::new();
        let transport = CancelOnSend {
            token: cancel.clone(),
            serp: SERP_HTML.as_bytes().to_vec(),
        };
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &cancel,
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    #[tokio::test]
    async fn cancel_after_search_before_fetch_yields_cancelled() {
        // One query returning a hit; the transport cancels on that send, so the
        // post-search cancellation check aborts before fetching pages. The
        // standalone question deliberately matches no vertical's intent, so
        // the turn reaches the engine query instead of being absorbed by a
        // vertical's own cancellation check.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::Web,
            standalone_question: "xyzzyplace status report".into(),
            queries: vec!["q".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let cancel = CancellationToken::new();
        let transport = CancelOnSend {
            token: cancel.clone(),
            serp: SERP_HTML.as_bytes().to_vec(),
        };
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &cancel,
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    #[tokio::test]
    async fn cached_decision_with_empty_cache_falls_back_to_web_pipeline() {
        // No prior search this conversation (or this test's fresh cache): the
        // `cached` decision finds nothing to reuse and degrades to exactly the
        // `web` pipeline, using the classifier's own route/rewrite/queries.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "treaty of versailles signed paris".into(),
            queries: vec!["treaty versailles".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Answer { .. }));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn cached_decision_with_matching_scope_answers_from_cache_without_retrieval() {
        // A prior search stored sources under scope 7. A follow-up turn in the
        // same conversation (same scope) with a `cached` decision must answer
        // straight from those sources: no transport call at all, and the
        // writer prompt embeds the cached source text.
        //
        // Message is Ambiguous (no ForceWeb freshness word) so the ForceWeb
        // raw-query race does not fire; a ForceWeb+Cached turn may still race
        // once and discard (see force_web_should_race_raw), which is outside
        // this unit's contract.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        cache.store(
            7,
            CachedSearch {
                standalone_question: "what is the stable rust version we discussed".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://blog.rust-lang.org/".into(),
                    title: "Rust 1.90.0".into(),
                    text: "Rust 1.90.0 is the latest stable release.".into(),
                }],
                route: SearchRoute::Web,
            },
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "what is the stable rust version we discussed".into(),
            queries: vec!["rust stable version".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        // No canned responses at all: a network call would fail the test by
        // returning a transport error the pipeline would otherwise degrade on.
        let transport = FakeHttpTransport::new();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_with_cache(&prepass, &transport, &Bm25Scorer, &bound, &cache, 7),
            "sys",
            &[],
            "what is the stable rust version we discussed",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(
            matches!(&outcome, SearchOutcome::Answer { messages, sources }
            if sources.len() == 1
                && sources[0].url == "https://blog.rust-lang.org/"
                && messages.first().is_some_and(|m| m.role == "system"
                    && m.content.contains("Rust 1.90.0 is the latest stable release"))
                && messages.last().is_some_and(|m|
                    m.content == "what is the stable rust version we discussed"))
        );
        assert!(
            transport.calls().is_empty(),
            "a cache hit must not retrieve"
        );
        // A `cached`-tier answer carries the cache-brevity directive on system:
        // the user is re-asking about the answer just given.
        assert!(matches!(&outcome, SearchOutcome::Answer { messages, .. }
            if messages.first().is_some_and(|m| m.content
                .contains("asking again about the answer you just gave"))));
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(
            e,
            RecorderEvent::SearchRetrieved { tier, sources, .. }
            if tier == "cache"
                && sources.len() == 1
                && sources[0].index == 1
                && sources[0].url == "https://blog.rust-lang.org/"
                && sources[0].title == "Rust 1.90.0"
                && sources[0].text.contains("Rust 1.90.0")
        )));
    }

    #[tokio::test]
    async fn web_tier_answer_omits_the_cache_brevity_directive() {
        // A fresh `web`-tier retrieval (not served from the cache) must never
        // carry the cache-tier brevity directive: only a `cached`-decision
        // answer is a repeat of an earlier reply.
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["treaty versailles paris"])));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps(&prepass, &transport, &Bm25Scorer),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { messages, .. }
            if messages.first().is_some_and(|m| m.role == "system"
                && m.content.contains("UNTRUSTED_WEB_CONTENT")
                && !m.content.contains("asking again about the answer you just gave"))));
    }

    #[tokio::test]
    async fn cached_decision_with_different_scope_falls_back_to_web_pipeline() {
        // Sources cached under scope 1 (an earlier conversation, or the same
        // conversation before a reset) must never answer a turn scoped to a
        // different epoch: the cache read misses and the pipeline searches
        // fresh, exactly like an empty cache.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        cache.store(
            1,
            CachedSearch {
                standalone_question: "stale question".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://stale.example/".into(),
                    title: "Stale".into(),
                    text: "stale text".into(),
                }],
                route: SearchRoute::Web,
            },
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "treaty of versailles signed paris".into(),
            queries: vec!["treaty versailles".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            // Scope 2: does not match the scope 1 entry above.
            &deps_with_cache(
                &prepass,
                &transport,
                &Bm25Scorer,
                Box::leak(Box::new(crate::trace::BoundRecorder::noop_for(
                    crate::trace::ConversationId::new("test"),
                ))),
                &cache,
                2,
            ),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://match.example/"));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn cached_decision_after_ttl_elapses_falls_back_to_web_pipeline() {
        // A zero-TTL cache: the entry is stored, but by the time it is read it
        // has already expired, so the `cached` decision must degrade to a
        // fresh search rather than serving the stale sources.
        let cache = TtlSourceCache::new(std::time::Duration::ZERO, 4);
        cache.store(
            7,
            CachedSearch {
                standalone_question: "stale question".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://stale.example/".into(),
                    title: "Stale".into(),
                    text: "stale text".into(),
                }],
                route: SearchRoute::Web,
            },
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "treaty of versailles signed paris".into(),
            queries: vec!["treaty versailles".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_with_cache(
                &prepass,
                &transport,
                &Bm25Scorer,
                Box::leak(Box::new(crate::trace::BoundRecorder::noop_for(
                    crate::trace::ConversationId::new("test"),
                ))),
                &cache,
                7,
            ),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources[0].url == "https://match.example/"));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn successful_web_answer_populates_the_cache_for_a_later_cached_turn() {
        // A plain `web` decision that grounds an answer must leave the cache
        // holding those exact sources, under the turn's scope, so the very
        // next turn's `cached` decision (if the classifier judges the
        // follow-up answerable from them) can reuse them without retrieving.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["treaty versailles paris"])));
        let transport = transport_with_serp_and_page();
        let (_p, status) = recorder();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let _ = run_search(
            &deps_with_cache(&prepass, &transport, &Bm25Scorer, &bound, &cache, 9),
            "sys",
            &[],
            "when signed",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let entries = cache.entries(9);
        assert_eq!(
            entries.len(),
            1,
            "a successful search must populate the cache"
        );
        let cached = &entries[0];
        assert_eq!(cached.sources.len(), 1);
        assert_eq!(cached.sources[0].url, "https://match.example/");
        assert_eq!(
            cached.standalone_question,
            "when was the treaty of versailles signed in paris"
        );
        // The general engine tier that produced this answer maps to the Web
        // route, recorded on the entry as provenance.
        assert_eq!(cached.route, crate::websearch::prepass::SearchRoute::Web);
    }

    #[test]
    fn route_for_tier_maps_every_answering_tier_to_a_route() {
        assert_eq!(route_for_tier("weather"), SearchRoute::Weather);
        assert_eq!(route_for_tier("news"), SearchRoute::News);
        assert_eq!(route_for_tier("wiki"), SearchRoute::Wiki);
        assert_eq!(route_for_tier("sports"), SearchRoute::Sports);
        assert_eq!(route_for_tier("engine"), SearchRoute::Web);
        // The cache tier is never stored, but the mapping is total: it and any
        // unknown label fall back to the general engine route.
        assert_eq!(route_for_tier("cache"), SearchRoute::Web);
    }

    #[tokio::test]
    async fn cached_decision_reuses_the_union_of_live_entries_when_the_judge_finds_them_sufficient()
    {
        // Two searches earlier this conversation (scope 7). A `cached` follow-up
        // on an eligible (web) route reuses the UNION of their sources, most
        // recent first and renumbered, with no retrieval, when the grounding
        // judge agrees they answer the question.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        let entry = |url: &str, text: &str| CachedSearch {
            standalone_question: "elon musk profile".into(),
            sources: vec![SourceBlock {
                index: 1,
                url: url.into(),
                title: "Profile".into(),
                text: text.into(),
            }],
            route: SearchRoute::Web,
        };
        cache.store(7, entry("https://older.example/", "older stored source"));
        cache.store(7, entry("https://newer.example/", "newer stored source"));
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "how old is elon musk".into(),
            queries: vec!["elon musk age".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        // No canned responses: any retrieval would fail the test with a
        // transport error instead of the silent no-op a true reuse produces.
        let transport = FakeHttpTransport::new();
        let health = EngineHealth::new();
        let judge = FakeSufficiencyJudge::sufficient();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_reuse(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &cache,
                7,
            ),
            "sys",
            &[],
            "and how old is he now?",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // The union, most recent first, renumbered contiguously from 1.
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.len() == 2
                && sources[0].index == 1
                && sources[0].url == "https://newer.example/"
                && sources[1].index == 2
                && sources[1].url == "https://older.example/"));
        assert!(transport.calls().is_empty(), "a reuse must not retrieve");
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(
            e,
            RecorderEvent::SearchEscalated { from_tier, sufficient, escalated, .. }
            if from_tier == "cache" && *sufficient && !*escalated
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            RecorderEvent::SearchRetrieved { tier, sources, .. }
            if tier == "cache" && sources.len() == 2
        )));
    }

    #[tokio::test]
    async fn cached_decision_excludes_volatile_route_entries_from_the_reuse_union() {
        // Two stored searches under scope 7: one produced by the stable web
        // tier, one by the volatile news vertical. A web-routed `cached`
        // follow-up must reuse ONLY the web-tier source; the news-tier source is
        // excluded from the union because its content was fetched for a live
        // question and may be stale.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        cache.store(
            7,
            CachedSearch {
                standalone_question: "stable topic".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://web.example/".into(),
                    title: "Web".into(),
                    text: "stable web source".into(),
                }],
                route: SearchRoute::Web,
            },
        );
        cache.store(
            7,
            CachedSearch {
                standalone_question: "volatile topic".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://news.example/".into(),
                    title: "News".into(),
                    text: "volatile news source".into(),
                }],
                route: SearchRoute::News,
            },
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "the stable topic detail".into(),
            queries: vec!["stable topic".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = FakeHttpTransport::new();
        let health = EngineHealth::new();
        let judge = FakeSufficiencyJudge::sufficient();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_reuse(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &cache,
                7,
            ),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        // Only the web-tier source survives; the news-tier source is excluded.
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.len() == 1 && sources[0].url == "https://web.example/"));
        assert!(transport.calls().is_empty(), "a reuse must not retrieve");
    }

    #[tokio::test]
    async fn cached_decision_escalates_when_every_stored_entry_is_volatile_route() {
        // The only stored entry was produced by a volatile vertical (sports), so
        // even though the follow-up route is eligible and the judge would find it
        // sufficient, no entry survives the route filter and the turn escalates
        // to a fresh search rather than grounding on stale live data.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        cache.store(
            7,
            CachedSearch {
                standalone_question: "yesterday's score".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://scores.example/".into(),
                    title: "Scores".into(),
                    text: "a stale scoreboard".into(),
                }],
                route: SearchRoute::Sports,
            },
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec!["treaty versailles".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let health = EngineHealth::new();
        // Would reuse if any eligible entry survived the filter.
        let judge = FakeSufficiencyJudge::sufficient();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_reuse(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &cache,
                7,
            ),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.iter().all(|s| s.url != "https://scores.example/")
                && sources.iter().any(|s| s.url == "https://match.example/")));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn cached_decision_escalates_to_fresh_search_when_the_judge_finds_the_union_insufficient()
    {
        // A stored source that no longer answers the drilled-down follow-up: the
        // grounding judge says insufficient, so the turn escalates to a fresh
        // web search and answers from the new result, never the stale entry.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        cache.store(
            7,
            CachedSearch {
                standalone_question: "prior".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://cached.example/".into(),
                    title: "Cached".into(),
                    text: "a stale cached answer".into(),
                }],
                route: SearchRoute::Web,
            },
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec!["treaty versailles".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let health = EngineHealth::new();
        // The reuse gate (first judge call) says insufficient -> escalate; the
        // engine tier (second call) then commits the fresh block.
        let judge = insufficient_then_sufficient();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_reuse(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &cache,
                7,
            ),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.iter().all(|s| s.url != "https://cached.example/")
                && sources.iter().any(|s| s.url == "https://match.example/")));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(
            e,
            RecorderEvent::SearchEscalated { from_tier, sufficient, escalated, .. }
            if from_tier == "cache" && !*sufficient && *escalated
        )));
    }

    #[tokio::test]
    async fn cached_decision_on_a_non_eligible_route_bypasses_reuse_and_searches_fresh() {
        // A news-routed `cached` decision must never reuse stored sources, even
        // when a sufficient entry exists: the live news vertical (then the
        // engines) owns volatile questions. The judge is `sufficient()`, so had
        // the reuse gate run it would have served the stored source with no
        // retrieval. A fresh search instead (transport hit, cached source
        // absent) proves the eligibility check skipped the gate entirely.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        cache.store(
            7,
            CachedSearch {
                standalone_question: "prior".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://cached.example/".into(),
                    title: "Cached".into(),
                    text: "a stale cached answer".into(),
                }],
                route: SearchRoute::Web,
            },
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::News,
            standalone_question: "what is the latest economic news".into(),
            queries: vec!["economy news".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let health = EngineHealth::new();
        let judge = FakeSufficiencyJudge::sufficient();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_reuse(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &cache,
                7,
            ),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.iter().all(|s| s.url != "https://cached.example/")));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn cached_decision_escalates_to_fresh_search_when_the_grounding_judge_errors() {
        // A judge transport failure fails toward a FRESH search (the opposite of
        // the vertical fast path's fail-toward-commit): committing would serve a
        // reply the gate could not vouch for.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        cache.store(
            7,
            CachedSearch {
                standalone_question: "prior".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://cached.example/".into(),
                    title: "Cached".into(),
                    text: "a stale cached answer".into(),
                }],
                route: SearchRoute::Web,
            },
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec!["treaty versailles".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let transport = transport_with_serp_and_page();
        let health = EngineHealth::new();
        let judge = FakeSufficiencyJudge::returning(Err(InferenceError::Request("boom".into())));
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_reuse(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &cache,
                7,
            ),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.iter().all(|s| s.url != "https://cached.example/")));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(
            e,
            RecorderEvent::SearchEscalated { from_tier, sufficient, escalated, .. }
            if from_tier == "cache" && !*sufficient && *escalated
        )));
    }

    #[tokio::test]
    async fn cached_decision_is_cancelled_when_the_grounding_judge_is_cancelled() {
        // A cancellation during the reuse judge short-circuits the whole turn:
        // no fresh search runs after a cancel.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600), 4);
        cache.store(
            7,
            CachedSearch {
                standalone_question: "prior".into(),
                sources: vec![SourceBlock {
                    index: 1,
                    url: "https://cached.example/".into(),
                    title: "Cached".into(),
                    text: "a stale cached answer".into(),
                }],
                route: SearchRoute::Web,
            },
        );
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            route: SearchRoute::Web,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec!["treaty versailles".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        // No canned responses: a fresh search after cancel would error here.
        let transport = FakeHttpTransport::new();
        let health = EngineHealth::new();
        let judge = FakeSufficiencyJudge::returning(Err(InferenceError::Cancelled));
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_reuse(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &cache,
                7,
            ),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
        assert!(
            transport.calls().is_empty(),
            "a cancelled reuse must not search fresh"
        );
    }

    // ── trace emission ────────────────────────────────────────────────────────

    /// Builds a `BoundRecorder` over a `MockRecorder` and returns both so a test
    /// can drive the pipeline and then assert on the emitted events.
    fn mock_recorder() -> (
        std::sync::Arc<crate::trace::recorder::MockRecorder>,
        crate::trace::BoundRecorder,
    ) {
        let mock = std::sync::Arc::new(crate::trace::recorder::MockRecorder::new());
        let bound = crate::trace::BoundRecorder::new(
            mock.clone(),
            crate::trace::ConversationId::new("conv-search"),
        );
        (mock, bound)
    }

    #[tokio::test]
    async fn trace_records_decision_and_retrieval_on_news_answer() {
        // A news-routed turn: the trace must carry one SearchDecided (route
        // "news", the standalone rewrite, the queries) and one SearchRetrieved
        // (tier "news", the cited feed URL).
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::News,
            standalone_question: "who won the most recent F1 race".into(),
            queries: vec!["f1 race winner".into()],
            explicit_search: false,
            lang: "en".into(),
        }));
        let feed_url = crate::websearch::news::news_request("f1 race winner", true, "en").url;
        let feed = r#"<rss><channel><item><title>Leclerc wins British GP - Formula 1</title><pubDate>Wed, 08 Jul 2026 01:11:35 GMT</pubDate></item></channel></rss>"#;
        let transport = FakeHttpTransport::new().with_response(
            &feed_url,
            HttpResponse {
                status: 200,
                final_url: feed_url.clone(),
                body: feed.as_bytes().to_vec(),
            },
        );
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let _ = run_search(
            &deps_with_recorder(&prepass, &transport, &Bm25Scorer, &bound),
            "sys",
            &[],
            "who won the most recent F1 race",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        // The resolved decision is recorded first, carrying the classifier's
        // route hint, the standalone rewrite, and the queries.
        assert!(matches!(
            &events[0],
            RecorderEvent::SearchDecided {
                prefilter,
                decision,
                force,
                route,
                standalone_question,
                queries
            } if prefilter == "force_web"
                && decision == "web"
                && !*force
                && route == "news"
                && standalone_question == "who won the most recent F1 race"
                && queries == &vec!["f1 race winner".to_string()]
        ));
        // The retrieval tier follows, carrying the cited source's URL AND title:
        // the generic feed homepage URL is uninformative without the title.
        assert!(events.iter().any(|e| matches!(
            e,
            RecorderEvent::SearchRetrieved { tier, sources, .. }
            if tier == "news"
                && sources.len() == 1
                && sources[0].url == "https://news.google.com/"
                && sources[0].title == "Google News headlines: f1 race winner"
                && !sources[0].text.is_empty()
        )));
    }

    #[tokio::test]
    async fn trace_records_decision_only_on_force_no_turn() {
        // A greeting is force-skipped: SearchDecided with the "force_no"
        // pre-filter label and empty route, plus SearchTimings (pipeline only);
        // no SearchRetrieved.
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["unused"])));
        let transport = FakeHttpTransport::new();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let _ = run_search(
            &deps_with_recorder(&prepass, &transport, &Bm25Scorer, &bound),
            "sys",
            &[],
            "hi there, thanks!",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(
            events.iter().any(|e| matches!(
                e,
                RecorderEvent::SearchDecided {
                    prefilter,
                    decision,
                    force,
                    route,
                    ..
                } if prefilter == "force_no" && decision == "no" && !*force && route.is_empty()
            )),
            "missing SearchDecided force_no: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, RecorderEvent::SearchTimings { .. })),
            "missing SearchTimings: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, RecorderEvent::SearchRetrieved { .. })),
            "ForceNo must not retrieve: {events:?}"
        );
    }

    #[tokio::test]
    async fn trace_records_stage_timings_on_engine_web_answer() {
        // A full engine path records classifier, serp, fetch, rank_assembly,
        // writer_prepare, and pipeline on SearchTimings (stderr format covered
        // by stage_timing unit tests).
        use crate::websearch::stage_timing::{
            STAGE_CLASSIFIER, STAGE_FETCH, STAGE_PIPELINE, STAGE_RANK_ASSEMBLY, STAGE_SERP,
            STAGE_WRITER_PREPARE,
        };
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["treaty versailles paris"])));
        let transport = transport_with_serp_and_page();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let _ = run_search(
            &deps_with_recorder(&prepass, &transport, &Bm25Scorer, &bound),
            "sys",
            &[],
            "when was the treaty of versailles signed in paris",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        let stages = events
            .iter()
            .find_map(|e| match e {
                RecorderEvent::SearchTimings { stages } => Some(stages.clone()),
                _ => None,
            })
            .expect("SearchTimings event");
        for name in [
            STAGE_CLASSIFIER,
            STAGE_SERP,
            STAGE_FETCH,
            STAGE_RANK_ASSEMBLY,
            STAGE_WRITER_PREPARE,
            STAGE_PIPELINE,
        ] {
            assert!(
                stages.iter().any(|s| s.stage == name),
                "missing stage {name} in {stages:?}"
            );
        }
    }

    #[tokio::test]
    async fn trace_records_engine_tier_on_grounded_web_answer() {
        // A plain web turn that reaches the engines: SearchRetrieved tier is
        // "engine" and carries the cited source URL.
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["treaty versailles paris"])));
        let transport = transport_with_serp_and_page();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let _ = run_search(
            &deps_with_recorder(&prepass, &transport, &Bm25Scorer, &bound),
            "sys",
            &[],
            "when signed",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(
            e,
            RecorderEvent::SearchRetrieved { tier, sources, .. }
            if tier == "engine" && sources.iter().any(|s| s.url == "https://match.example/")
        )));
        // The engine tier's per-engine outcome summary reaches the trace too:
        // DuckDuckGo (canned) answered "ok", Mojeek (no canned response, so the
        // fake transport errors it) shows "transport_error" rather than
        // silently vanishing from the record.
        let engine_stats = events
            .iter()
            .find_map(|e| match e {
                RecorderEvent::SearchRetrieved {
                    tier, engine_stats, ..
                } if tier == "engine" => Some(engine_stats),
                _ => None,
            })
            .expect("engine SearchRetrieved event recorded");
        assert!(engine_stats
            .iter()
            .any(|s| s.name == "duckduckgo" && s.status == "ok" && s.hit_count > 0));
        assert!(engine_stats
            .iter()
            .any(|s| s.name == "mojeek" && s.status == "transport_error"));
    }

    // ── commit_or_escalate: the sufficiency judge on vertical answers ──────────

    /// A News-routed decision whose non-volatile standalone matches the treaty
    /// fixture page, so an escalation to the engines produces a ranked source.
    fn news_route_decision() -> PrePassDecision {
        PrePassDecision {
            decision: SearchDecision::Web,
            route: SearchRoute::News,
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec!["treaty versailles paris".into()],
            explicit_search: false,
            lang: "en".into(),
        }
    }

    /// The Google News feed response for the escalation tests' query (non-
    /// volatile, so `freshness` is false), yielding a `news.google.com` block.
    fn news_feed_response() -> (String, HttpResponse) {
        let feed_url =
            crate::websearch::news::news_request("treaty versailles paris", false, "en").url;
        let resp = HttpResponse {
            status: 200,
            final_url: feed_url.clone(),
            body: br#"<rss><channel><item><title>Treaty of Versailles anniversary marked - History Today</title></item></channel></rss>"#.to_vec(),
        };
        (feed_url, resp)
    }

    /// Transport serving only the News feed: a vertical hit with no engine
    /// fallback reachable in the transport.
    fn news_only_transport() -> FakeHttpTransport {
        let (feed_url, resp) = news_feed_response();
        FakeHttpTransport::new().with_response(&feed_url, resp)
    }

    /// Transport serving the News feed plus the SERP + page, so an escalation to
    /// the engines can rank and replace the vertical block.
    fn news_and_engine_transport() -> FakeHttpTransport {
        let (feed_url, resp) = news_feed_response();
        transport_with_serp_and_page().with_response(&feed_url, resp)
    }

    /// An insufficient judge verdict with a `missing` phrase.
    fn insufficient() -> FakeSufficiencyJudge {
        FakeSufficiencyJudge::returning(Ok(SufficiencyVerdict {
            sufficient: false,
            missing: "the treaty terms".into(),
            reason: InsufficiencyReason::Missing,
            requery_queries: Vec::new(),
        }))
    }

    /// A judge that returns `first` on its 1st call and `second` on every call
    /// after. Every escalation turn now calls the judge twice (once on the
    /// vertical block in `commit_or_escalate`, once more on the merged
    /// engine-tier result in `judge_and_requery`), so tests that only want to
    /// drive the FIRST (vertical) verdict without also triggering the
    /// engine-tier judge's own requery use this instead of
    /// [`FakeSufficiencyJudge`]'s single fixed result.
    struct SequencedJudge {
        calls: std::sync::atomic::AtomicUsize,
        first: SufficiencyVerdict,
        second: SufficiencyVerdict,
    }

    #[async_trait]
    impl SufficiencyJudge for SequencedJudge {
        async fn judge(
            &self,
            _standalone_question: &str,
            _sources: &[SourceBlock],
            _cancel: &CancellationToken,
        ) -> Result<SufficiencyVerdict, InferenceError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(if n == 0 {
                self.first.clone()
            } else {
                self.second.clone()
            })
        }
    }

    /// Insufficient on the vertical judge call, sufficient on every call after
    /// (the engine-tier judge's own verdict), so an escalation test can assert
    /// on the escalation-merge behaviour without the engine-tier judge firing
    /// its own requery on top.
    fn insufficient_then_sufficient() -> SequencedJudge {
        SequencedJudge {
            calls: std::sync::atomic::AtomicUsize::new(0),
            first: SufficiencyVerdict {
                sufficient: false,
                missing: "the treaty terms".into(),
                reason: InsufficiencyReason::Missing,
                requery_queries: Vec::new(),
            },
            second: SufficiencyVerdict {
                sufficient: true,
                missing: String::new(),
                reason: InsufficiencyReason::Missing,
                requery_queries: Vec::new(),
            },
        }
    }

    /// Scripted judge that pops results from a queue (last result repeats).
    /// Covers post-requery branches that need Error / conflict / still-missing
    /// on the second call after a first-round insufficient.
    struct QueueJudge {
        results: Mutex<Vec<Result<SufficiencyVerdict, InferenceError>>>,
    }

    impl QueueJudge {
        /// Builds a queue that yields `results` in order; after the last entry,
        /// further calls clone that last entry.
        fn new(results: Vec<Result<SufficiencyVerdict, InferenceError>>) -> Self {
            assert!(!results.is_empty(), "QueueJudge needs at least one result");
            Self {
                results: Mutex::new(results),
            }
        }
    }

    #[async_trait]
    impl SufficiencyJudge for QueueJudge {
        async fn judge(
            &self,
            _standalone_question: &str,
            _sources: &[SourceBlock],
            _cancel: &CancellationToken,
        ) -> Result<SufficiencyVerdict, InferenceError> {
            let mut guard = self.results.lock().unwrap();
            if guard.len() == 1 {
                return guard[0].clone();
            }
            guard.remove(0)
        }
    }

    fn insufficient_verdict(missing: &str) -> SufficiencyVerdict {
        SufficiencyVerdict {
            sufficient: false,
            missing: missing.into(),
            reason: InsufficiencyReason::Missing,
            requery_queries: Vec::new(),
        }
    }

    fn insufficient_with_queries(missing: &str, queries: Vec<&str>) -> SufficiencyVerdict {
        SufficiencyVerdict {
            sufficient: false,
            missing: missing.into(),
            reason: InsufficiencyReason::Missing,
            requery_queries: queries.into_iter().map(String::from).collect(),
        }
    }

    #[tokio::test]
    async fn insufficient_vertical_escalates_and_merges_vertical_with_engine_sources() {
        // The news vertical answers, the judge finds the block insufficient, and
        // an engine is available: the pipeline escalates and answers from the
        // vertical block merged with the engine sources (vertical first),
        // rather than discarding the vertical's data.
        let prepass = FakePrePass::returning(Ok(news_route_decision()));
        let transport = news_and_engine_transport();
        let judge = insufficient();
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.len() == 2
                && sources[0].url == "https://news.google.com/"
                && sources[0].index == 1
                && sources[1].url == "https://match.example/"
                && sources[1].index == 2));
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(e,
            RecorderEvent::SearchEscalated { from_tier, sufficient, escalated, escalation_hit, .. }
            if from_tier == "news" && !*sufficient && *escalated && *escalation_hit)));
    }

    #[tokio::test]
    async fn escalation_from_an_insufficient_vertical_does_not_bypass_a_warm_serp_cache() {
        // A judge-driven escalation is not a user distrust signal (the user
        // never asked to re-check anything), so it must always pass
        // `bypass_cache = false`: a warm SERP cache entry for the escalation
        // query is served with NO engine request, unlike the explicit-search
        // path above.
        let prepass = FakePrePass::returning(Ok(news_route_decision()));
        let transport = news_and_engine_transport();
        // Sufficient on the engine-tier judge's own call (2nd), so the
        // engine-tier's requery never fires and this test stays scoped to the
        // escalation-cache behaviour it exists to check.
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let web_cache = WebCache::new(
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(600),
            64,
            128,
        );
        // Both raced engines are pre-warmed (freshness = false for this
        // non-volatile question, matching `news_route_decision`'s query), each
        // pointing at the fixture page already served by
        // `news_and_engine_transport`, so the escalation still ranks a source
        // without ever touching either engine's endpoint.
        let cached_hit = || SearchHit {
            title: "Treaty of Versailles".into(),
            url: "https://match.example/".into(),
            snippet: "the treaty signed in paris".into(),
        };
        web_cache.serp_put(
            "duckduckgo",
            "treaty versailles paris",
            false,
            "en",
            vec![cached_hit()],
        );
        web_cache.serp_put(
            "mojeek",
            "treaty versailles paris",
            false,
            "en",
            vec![cached_hit()],
        );
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_with_web_cache(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &web_cache,
            ),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.iter().any(|s| s.url == "https://match.example/")));
        // Neither engine's SERP endpoint was ever contacted: both were served
        // from the warm cache.
        let mojeek_url =
            crate::websearch::engine::mojeek_request("treaty versailles paris", false, "en").url;
        assert!(!transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
        assert!(!transport.calls().iter().any(|c| c.url == mojeek_url));
    }

    #[tokio::test]
    async fn insufficient_vertical_with_engine_miss_serves_partial() {
        // Insufficient block, an engine is available but the SERP is unreachable
        // (no canned response): the engines are tried, come up empty, and the
        // vertical block is served as a partial answer rather than a wall.
        let prepass = FakePrePass::returning(Ok(news_route_decision()));
        let transport = news_only_transport();
        let judge = insufficient();
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.len() == 1 && sources[0].url == "https://news.google.com/"));
        // The engines were attempted (and missed) before the fallback.
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(e,
            RecorderEvent::SearchEscalated { escalated, escalation_hit, .. }
            if *escalated && !*escalation_hit)));
    }

    #[tokio::test]
    async fn insufficient_vertical_with_all_engines_cooling_serves_partial_without_calling_engines()
    {
        // Insufficient block, but every engine is inside its cooldown:
        // escalating is futile (and would risk deepening the block), so the
        // vertical block is served directly, with no engine request issued.
        let prepass = FakePrePass::returning(Ok(news_route_decision()));
        let transport = news_and_engine_transport();
        let judge = insufficient();
        let health = EngineHealth::new();
        health.block_all_for_test();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.len() == 1 && sources[0].url == "https://news.google.com/"));
        // No engine was contacted: escalation was skipped as futile.
        assert!(!transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(e,
            RecorderEvent::SearchEscalated { escalated, .. } if !*escalated)));
    }

    #[tokio::test]
    async fn judge_infra_failure_commits_the_vertical_block() {
        // A judge transport failure fails toward committing: the vertical block
        // is served without spending an engine request on a verdict the judge
        // could not actually make.
        let prepass = FakePrePass::returning(Ok(news_route_decision()));
        let transport = news_and_engine_transport();
        let judge = FakeSufficiencyJudge::returning(Err(InferenceError::Request("boom".into())));
        let health = EngineHealth::new();
        let (_mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(&outcome, SearchOutcome::Answer { sources, .. }
            if sources.len() == 1 && sources[0].url == "https://news.google.com/"));
        assert!(!transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    #[tokio::test]
    async fn judge_cancellation_yields_cancelled() {
        // The user cancelled while the judge was deciding: the pipeline aborts.
        let prepass = FakePrePass::returning(Ok(news_route_decision()));
        let transport = news_only_transport();
        let judge = FakeSufficiencyJudge::returning(Err(InferenceError::Cancelled));
        let health = EngineHealth::new();
        let (_mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    /// Transport that serves the News feed, then cancels the token on the
    /// escalation SERP request (returning a parseable SERP), so the engine
    /// tier's post-retrieval cancellation check fires mid-escalation.
    struct FeedThenCancelOnSerp {
        feed_url: String,
        token: CancellationToken,
    }

    #[async_trait]
    impl HttpTransport for FeedThenCancelOnSerp {
        async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
            if req.url == self.feed_url {
                return Ok(HttpResponse {
                    status: 200,
                    final_url: self.feed_url.clone(),
                    body: br#"<rss><channel><item><title>Treaty of Versailles anniversary - History Today</title></item></channel></rss>"#.to_vec(),
                });
            }
            self.token.cancel();
            Ok(HttpResponse {
                status: 200,
                final_url: DDG_ENDPOINT.into(),
                body: SERP_HTML.as_bytes().to_vec(),
            })
        }
    }

    #[tokio::test]
    async fn cancel_during_escalation_engine_tier_yields_cancelled() {
        // The vertical answers, the judge escalates, and the user cancels while
        // the engine tier retrieves: the escalation aborts with Cancelled.
        let prepass = FakePrePass::returning(Ok(news_route_decision()));
        let (feed_url, _resp) = news_feed_response();
        let cancel = CancellationToken::new();
        let transport = FeedThenCancelOnSerp {
            feed_url,
            token: cancel.clone(),
        };
        let judge = insufficient();
        let health = EngineHealth::new();
        let (_mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &cancel,
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    // ── judge_and_requery: the engine tier's own judge + bounded requery ───────

    /// The standalone question `judge_and_requery` tests judge round-one
    /// sources against.
    const REQUERY_QUESTION: &str = "when was the treaty of versailles signed in paris";

    /// Round-one's already-assembled engine-tier sources: what a real
    /// `run_engine_tier` call would have produced for [`REQUERY_QUESTION`].
    fn round_one_sources() -> Vec<SourceBlock> {
        vec![SourceBlock {
            index: 1,
            url: "https://match.example/".into(),
            title: "Treaty of Versailles".into(),
            text: "the treaty signed in paris".into(),
        }]
    }

    /// A SERP with one organic result at a URL distinct from
    /// [`round_one_sources`], so the requery's hit is genuinely new.
    const REQUERY_SERP_HTML: &str = r#"
      <div class="result">
        <a class="result__a" href="https://requery.example/">Treaty Terms</a>
        <a class="result__snippet">the exact terms of the treaty</a>
      </div>
    "#;

    /// A dense article readability will extract, about the requery's missing
    /// phrase, with strong lexical overlap on [`REQUERY_QUESTION`] so BM25
    /// reliably keeps it.
    const REQUERY_PAGE_HTML: &str = r#"
      <html><body><article><h1>Treaty of Versailles Terms</h1>
      <p>The financial and territorial terms of the Treaty of Versailles
      required Germany to accept responsibility for the war and pay
      substantial reparations to the Allied powers over a period of decades,
      alongside major territorial concessions signed in paris and across
      Europe.</p>
      <p>Military restrictions, including strict caps on army size and a ban
      on maintaining an air force, were imposed as part of the same 1919
      settlement, reshaping the postwar balance of power for a generation of
      European diplomacy that followed the signing of the treaty of
      versailles.</p>
      </article></body></html>
    "#;

    /// Transport serving the requery's SERP and page, distinct from
    /// [`transport_with_serp_and_page`]'s round-one fixtures.
    fn requery_transport() -> FakeHttpTransport {
        FakeHttpTransport::new()
            .with_response(
                DDG_ENDPOINT,
                HttpResponse {
                    status: 200,
                    final_url: DDG_ENDPOINT.into(),
                    body: REQUERY_SERP_HTML.as_bytes().to_vec(),
                },
            )
            .with_response(
                "https://requery.example/",
                HttpResponse {
                    status: 200,
                    final_url: "https://requery.example/".into(),
                    body: REQUERY_PAGE_HTML.as_bytes().to_vec(),
                },
            )
    }

    /// A `PrePass` `judge_and_requery` tests never call: they drive
    /// `judge_and_requery` directly rather than through `run_search`, but the
    /// `SearchDeps` builders still require one.
    fn dummy_prepass() -> FakePrePass {
        FakePrePass::returning(Ok(web_decision(vec![])))
    }

    #[tokio::test]
    async fn judge_and_requery_sufficient_skips_requery() {
        // A sufficient verdict on round-one's own sources must never trigger
        // the requery: the transport sees no calls at all, and nothing is
        // recorded either (the round-one `SearchRetrieved` only exists to
        // audit a round the requery is about to fire).
        let prepass = dummy_prepass();
        let transport = FakeHttpTransport::new();
        let judge = FakeSufficiencyJudge::sufficient();
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let sources = round_one_sources();
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            sources.clone(),
            // Vertical-escalation merge contract (round-one pinned,
            // `merge_sources`), byte-identical to this test's pre-fix behaviour.
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(&outcome, EngineJudgeOutcome::Sources(s, _, _, None) if s == &sources));
        assert!(
            transport.calls().is_empty(),
            "a sufficient verdict must never requery"
        );
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(
            events.is_empty(),
            "no requery fired, so nothing is recorded"
        );
    }

    #[tokio::test]
    async fn judge_and_requery_post_judge_still_insufficient_sets_still_missing() {
        // Same insufficient on both rounds: merge runs, second judge keeps gap.
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = FakeSufficiencyJudge::returning(Ok(insufficient_verdict("the treaty terms")));
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            &outcome,
            EngineJudgeOutcome::Sources(s, _, false, Some(m))
                if s.len() == 2 && m == "the treaty terms"
        ));
    }

    #[tokio::test]
    async fn judge_and_requery_post_judge_conflict_flags_writer() {
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = QueueJudge::new(vec![
            Ok(insufficient_verdict("the treaty terms")),
            Ok(SufficiencyVerdict {
                sufficient: false,
                missing: "attendance figure".into(),
                reason: InsufficiencyReason::Conflicting,
                requery_queries: Vec::new(),
            }),
        ]);
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            &outcome,
            EngineJudgeOutcome::Sources(s, _, true, None) if s.len() == 2
        ));
    }

    #[tokio::test]
    async fn judge_and_requery_post_judge_request_error_keeps_first_missing() {
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = QueueJudge::new(vec![
            Ok(insufficient_verdict("the treaty terms")),
            Err(InferenceError::Request("post boom".into())),
        ]);
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            &outcome,
            EngineJudgeOutcome::Sources(s, _, false, Some(m))
                if s.len() == 2 && m == "the treaty terms"
        ));
    }

    #[tokio::test]
    async fn judge_and_requery_post_judge_cancelled_yields_cancelled() {
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = QueueJudge::new(vec![
            Ok(insufficient_verdict("the treaty terms")),
            Err(InferenceError::Cancelled),
        ]);
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(outcome, EngineJudgeOutcome::Cancelled));
    }

    #[tokio::test]
    async fn judge_and_requery_post_judge_empty_missing_falls_back_to_first() {
        // Post-requery insufficient with empty missing → keep first_missing.
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = QueueJudge::new(vec![
            Ok(insufficient_verdict("the treaty terms")),
            Ok(SufficiencyVerdict {
                sufficient: false,
                missing: String::new(),
                reason: InsufficiencyReason::Missing,
                requery_queries: Vec::new(),
            }),
        ]);
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            &outcome,
            EngineJudgeOutcome::Sources(_, _, false, Some(m)) if m == "the treaty terms"
        ));
    }

    #[tokio::test]
    async fn judge_and_requery_whitespace_only_requery_queries_commit_partial() {
        // Judge authored only whitespace `requery_queries` (bypassing normalize):
        // after the empty filter there is nothing to SERP → still_missing, no DDG.
        let prepass = dummy_prepass();
        let transport = FakeHttpTransport::new();
        let judge = FakeSufficiencyJudge::returning(Ok(SufficiencyVerdict {
            sufficient: false,
            missing: "the treaty terms".into(),
            reason: InsufficiencyReason::Missing,
            requery_queries: vec!["   ".into(), "\t".into()],
        }));
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let sources = round_one_sources();
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            sources.clone(),
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            &outcome,
            EngineJudgeOutcome::Sources(s, _, false, Some(m))
                if s == &sources && m == "the treaty terms"
        ));
        assert!(
            transport.calls().is_empty(),
            "whitespace-only requery queries must not hit engines"
        );
    }

    #[tokio::test]
    async fn judge_and_requery_cancel_after_merge_before_post_judge() {
        // Cancel during the requery page fetch: merge may complete, then the
        // pre-post-judge cancel check must abort.
        let prepass = dummy_prepass();
        let cancel = CancellationToken::new();
        let transport = CancelOnPageFetch {
            token: cancel.clone(),
            inner: requery_transport(),
        };
        let judge = FakeSufficiencyJudge::returning(Ok(insufficient_verdict("the treaty terms")));
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            None,
            16384,
            false,
            "en",
            false,
            &cancel,
        )
        .await;
        assert!(matches!(outcome, EngineJudgeOutcome::Cancelled));
    }

    #[tokio::test]
    async fn judge_and_requery_multi_query_cancel_between_iterations() {
        // Two requery queries: CancelOnSend trips on the first SERP; the
        // second iteration's pre-check must yield Cancelled (line 1943 path).
        let prepass = dummy_prepass();
        let cancel = CancellationToken::new();
        let transport = CancelOnSend {
            token: cancel.clone(),
            serp: REQUERY_SERP_HTML.as_bytes().to_vec(),
        };
        let judge = FakeSufficiencyJudge::returning(Ok(insufficient_with_queries(
            "the treaty terms",
            vec!["first gap query", "second gap query"],
        )));
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            None,
            16384,
            false,
            "en",
            false,
            &cancel,
        )
        .await;
        assert!(matches!(outcome, EngineJudgeOutcome::Cancelled));
    }

    #[tokio::test]
    async fn judge_and_requery_requery_early_stops_when_hits_enough() {
        // SERP with many organic results: requery loop must early-stop and not
        // issue a second query when SERP_EARLY_STOP_HITS is already met.
        let mut serp = String::new();
        for i in 0..SERP_EARLY_STOP_HITS + 2 {
            serp.push_str(&format!(
                r#"<div class="result">
                  <a class="result__a" href="https://requery{i}.example/">Hit {i}</a>
                  <a class="result__snippet">the treaty terms extra {i}</a>
                </div>"#
            ));
        }
        let transport = FakeHttpTransport::new()
            .with_response(
                DDG_ENDPOINT,
                HttpResponse {
                    status: 200,
                    final_url: DDG_ENDPOINT.into(),
                    body: serp.into_bytes(),
                },
            )
            .with_response(
                "https://requery0.example/",
                HttpResponse {
                    status: 200,
                    final_url: "https://requery0.example/".into(),
                    body: REQUERY_PAGE_HTML.as_bytes().to_vec(),
                },
            );
        let prepass = dummy_prepass();
        let judge = FakeSufficiencyJudge::returning(Ok(insufficient_with_queries(
            "the treaty terms",
            vec!["first gap query", "second gap query should not run"],
        )));
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let _ = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        let ddg_qs: Vec<String> = transport
            .calls()
            .into_iter()
            .filter(|c| c.url == DDG_ENDPOINT)
            .filter_map(|c| c.form.into_iter().find(|(k, _)| k == "q").map(|(_, v)| v))
            .collect();
        assert_eq!(
            ddg_qs,
            vec!["first gap query".to_string()],
            "early-stop must skip the second requery query, got {ddg_qs:?}"
        );
    }

    #[tokio::test]
    async fn vertical_escalation_uses_judge_requery_queries() {
        // Escalation with non-empty verdict.requery_queries must SERP those
        // keywords (not only the classifier's original query).
        let prepass = FakePrePass::returning(Ok(news_route_decision()));
        let transport = news_and_engine_transport();
        let judge = QueueJudge::new(vec![
            Ok(insufficient_with_queries(
                "the treaty terms",
                vec!["versailles reparations schedule keywords"],
            )),
            Ok(SufficiencyVerdict {
                sufficient: true,
                missing: String::new(),
                reason: InsufficiencyReason::Missing,
                requery_queries: Vec::new(),
            }),
        ]);
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let _ = run_search(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        let ddg_qs: Vec<String> = transport
            .calls()
            .into_iter()
            .filter(|c| c.url == DDG_ENDPOINT)
            .filter_map(|c| c.form.into_iter().find(|(k, _)| k == "q").map(|(_, v)| v))
            .collect();
        assert!(
            ddg_qs
                .iter()
                .any(|q| q == "versailles reparations schedule keywords"),
            "escalation must SERP judge requery_queries, got {ddg_qs:?}"
        );
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(
            e,
            RecorderEvent::SearchEscalated {
                escalated: true,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn judge_and_requery_insufficient_with_missing_fires_one_requery_and_merges() {
        // Insufficient with a `missing` phrase: exactly one requery fires,
        // and its new source is merged in alongside round-one's, never
        // replacing it.
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let sources = round_one_sources();
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            sources,
            // Vertical-escalation merge contract (round-one pinned,
            // `merge_sources`), byte-identical to this test's pre-fix behaviour.
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(&outcome, EngineJudgeOutcome::Sources(s, _, _, None)
            if s.len() == 2
                && s[0].url == "https://match.example/"
                && s[0].index == 1
                && s[1].url == "https://requery.example/"
                && s[1].index == 2)
        );
        // Exactly one requery: the DDG POST carried the missing phrase
        // appended to the standalone question.
        let ddg_call = transport
            .calls()
            .into_iter()
            .find(|c| c.url == DDG_ENDPOINT)
            .expect("the requery must hit DDG exactly once");
        assert!(ddg_call.form.iter().any(|(k, v)| k == "q"
            && v == "when was the treaty of versailles signed in paris the treaty terms"));
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(e,
            RecorderEvent::SearchRequeried { missing, requery }
            if missing == "the treaty terms"
                && requery == "when was the treaty of versailles signed in paris the treaty terms")));
        // A requery fired, so round-one's own (pre-merge) sources must be
        // auditable in the trace: one `SearchRetrieved` tagged `round: Some(1)`
        // carrying exactly round-one's source, recorded before the requery's
        // `SearchRequeried` above.
        let round_one_index = events
            .iter()
            .position(|e| {
                matches!(e,
                RecorderEvent::SearchRetrieved { tier, sources, round: Some(1), .. }
                if tier == "engine"
                    && sources.len() == 1
                    && sources[0].url == "https://match.example/"
                    && sources[0].title == "Treaty of Versailles"
                    && !sources[0].text.is_empty())
            })
            .expect("round-one SearchRetrieved must be recorded when a requery fires");
        let requeried_index = events
            .iter()
            .position(|e| matches!(e, RecorderEvent::SearchRequeried { .. }))
            .expect("SearchRequeried must be recorded");
        assert!(
            round_one_index < requeried_index,
            "round-one's SearchRetrieved must be recorded before SearchRequeried"
        );
    }

    #[tokio::test]
    async fn judge_and_requery_uses_judge_requery_queries_not_concat() {
        // When the judge authors keyword SERPs, the DDG `q=` must be those
        // keywords (gap-targeted), not the legacy standalone+missing concat.
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = FakeSufficiencyJudge::returning(Ok(SufficiencyVerdict {
            sufficient: false,
            missing: "nominal GDP total in USD".into(),
            reason: InsufficiencyReason::Missing,
            requery_queries: vec!["Vietnam nominal GDP USD billion".into()],
        }));
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let sources = round_one_sources();
        let _ = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            sources,
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        let ddg_call = transport
            .calls()
            .into_iter()
            .find(|c| c.url == DDG_ENDPOINT)
            .expect("the requery must hit DDG");
        assert!(
            ddg_call
                .form
                .iter()
                .any(|(k, v)| k == "q" && v == "Vietnam nominal GDP USD billion"),
            "requery must search the judge keyword query, got {:?}",
            ddg_call.form
        );
        // Must not fall back to the prose concat when judge queries exist.
        assert!(!ddg_call
            .form
            .iter()
            .any(|(k, v)| k == "q" && v.contains("nominal GDP total in USD")));
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(e,
            RecorderEvent::SearchRequeried { missing, requery }
            if missing == "nominal GDP total in USD"
                && requery == "Vietnam nominal GDP USD billion")));
    }

    #[tokio::test]
    async fn judge_and_requery_caps_a_long_missing_phrase_at_a_word_boundary() {
        // The judge's `missing` can run to a full prose sentence; the text
        // actually searched must be capped to
        // `REQUERY_MISSING_MAX_CHARS` at a word boundary, while the trace's
        // `SearchRequeried::missing` field keeps the judge's full phrase.
        let long_missing = "the full territorial and financial terms and conditions of the treaty \
                             settlement and reparations schedule";
        assert_eq!(long_missing.chars().count(), 105);
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = FakeSufficiencyJudge::returning(Ok(SufficiencyVerdict {
            sufficient: false,
            missing: long_missing.into(),
            reason: InsufficiencyReason::Missing,
            requery_queries: Vec::new(),
        }));
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let sources = round_one_sources();
        let _ = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            sources,
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        let capped = "the full territorial and financial terms and conditions of the treaty";
        let expected_requery = format!("{REQUERY_QUESTION} {capped}");
        let ddg_call = transport
            .calls()
            .into_iter()
            .find(|c| c.url == DDG_ENDPOINT)
            .expect("the requery must hit DDG");
        assert!(
            ddg_call
                .form
                .iter()
                .any(|(k, v)| k == "q" && v == &expected_requery),
            "the requery's search text must be capped at a word boundary"
        );
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.iter().any(|e| matches!(e,
            RecorderEvent::SearchRequeried { missing, requery }
            if missing == long_missing && requery == &expected_requery)));
    }

    #[tokio::test]
    async fn judge_and_requery_dedupes_requery_hits_by_url() {
        // The requery's only hit points at the SAME URL round-one already
        // has: it must be filtered out before any fetch, so no new source is
        // added and the page is never re-fetched.
        let prepass = dummy_prepass();
        let dup_serp = r#"
          <div class="result">
            <a class="result__a" href="https://match.example/">Treaty of Versailles</a>
            <a class="result__snippet">already retrieved</a>
          </div>
        "#;
        let transport = FakeHttpTransport::new().with_response(
            DDG_ENDPOINT,
            HttpResponse {
                status: 200,
                final_url: DDG_ENDPOINT.into(),
                body: dup_serp.as_bytes().to_vec(),
            },
        );
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let sources = round_one_sources();
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            sources.clone(),
            // Vertical-escalation merge contract (round-one pinned,
            // `merge_sources`), byte-identical to this test's pre-fix behaviour.
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        // No new URLs: sources unchanged, still_missing flags partial writer path.
        assert!(matches!(
            &outcome,
            EngineJudgeOutcome::Sources(s, _, false, Some(m))
                if s == &sources && m == "the treaty terms"
        ));
        assert!(!transport
            .calls()
            .iter()
            .any(|c| c.url == "https://match.example/"));
        // The requery still fired (the judge found round one insufficient),
        // even though it turned up no genuinely new URL: round-one's
        // SearchRetrieved must still exist, so a trace shows what was judged
        // insufficient even on this no-new-sources outcome.
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events
            .iter()
            .any(|e| matches!(e, RecorderEvent::SearchRetrieved { round: Some(1), .. })));
    }

    #[tokio::test]
    async fn judge_and_requery_judge_failure_commits_round_one() {
        // A judge transport failure fails toward committing, the same
        // posture the vertical judge takes: round-one's sources are served
        // without spending a requery on a verdict the judge could not make,
        // and nothing is recorded (no requery fired).
        let prepass = dummy_prepass();
        let transport = FakeHttpTransport::new();
        let judge = FakeSufficiencyJudge::returning(Err(InferenceError::Request("boom".into())));
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let sources = round_one_sources();
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            sources.clone(),
            // Vertical-escalation merge contract (round-one pinned,
            // `merge_sources`), byte-identical to this test's pre-fix behaviour.
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(&outcome, EngineJudgeOutcome::Sources(s, _, _, None) if s == &sources));
        assert!(transport.calls().is_empty());
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(
            events.is_empty(),
            "no requery fired, so nothing is recorded"
        );
    }

    #[tokio::test]
    async fn judge_and_requery_conflict_commits_without_requery() {
        // A conflicting verdict: the sources hold the asked value but disagree on
        // it. A requery cannot resolve a disagreement, so round one's sources are
        // committed unchanged with the conflict flag raised, and NO requery
        // fires. The conflict branch returns before the round-one record, so a
        // trace records nothing (no requery ran).
        let prepass = dummy_prepass();
        let transport = FakeHttpTransport::new();
        let judge = FakeSufficiencyJudge::returning(Ok(SufficiencyVerdict {
            sufficient: false,
            missing: "attendance figure".into(),
            reason: InsufficiencyReason::Conflicting,
            requery_queries: Vec::new(),
        }));
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let sources = round_one_sources();
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            sources.clone(),
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        // Sources unchanged, conflict flag raised, so the writer presents the
        // spread rather than re-searching.
        assert!(matches!(&outcome, EngineJudgeOutcome::Sources(s, _, true, None) if s == &sources));
        // No requery fired: a disagreement is not searchable.
        assert!(transport.calls().is_empty());
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(
            events.is_empty(),
            "conflict commits without a requery, so nothing is recorded"
        );
    }

    #[tokio::test]
    async fn judge_and_requery_judge_cancellation_yields_cancelled() {
        // The user cancelled while the engine-tier judge was deciding: the
        // requery never fires, and the pipeline aborts.
        let prepass = dummy_prepass();
        let transport = FakeHttpTransport::new();
        let judge = FakeSufficiencyJudge::returning(Err(InferenceError::Cancelled));
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            // Vertical-escalation merge contract (round-one pinned,
            // `merge_sources`), byte-identical to this test's pre-fix behaviour.
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(outcome, EngineJudgeOutcome::Cancelled));
    }

    #[tokio::test]
    async fn judge_and_requery_cancelled_before_firing_yields_cancelled() {
        // The user cancelled right after the judge returned insufficient,
        // before the requery's network calls: caught by the post-verdict
        // cancellation check, never reaching the transport, and never
        // reaching the round-one `SearchRetrieved` record either (that
        // record and `SearchRequeried` sit on the far side of this check).
        let prepass = dummy_prepass();
        let transport = FakeHttpTransport::new();
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let (mock, bound) = mock_recorder();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            // Vertical-escalation merge contract (round-one pinned,
            // `merge_sources`), byte-identical to this test's pre-fix behaviour.
            None,
            16384,
            false,
            "en",
            false,
            &cancel,
        )
        .await;
        assert!(matches!(outcome, EngineJudgeOutcome::Cancelled));
        assert!(transport.calls().is_empty());
        let events: Vec<RecorderEvent> = mock.snapshot().into_iter().map(|(_, e)| e).collect();
        assert!(events.is_empty(), "a pre-firing cancel must record nothing");
    }

    #[tokio::test]
    async fn judge_and_requery_cancelled_during_requery_search_yields_cancelled() {
        // The user cancels while the requery's own SERP request is in
        // flight: caught by the check between the search and the fetch.
        let prepass = dummy_prepass();
        let cancel = CancellationToken::new();
        let transport = CancelOnSend {
            token: cancel.clone(),
            serp: REQUERY_SERP_HTML.as_bytes().to_vec(),
        };
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            round_one_sources(),
            // Vertical-escalation merge contract (round-one pinned,
            // `merge_sources`), byte-identical to this test's pre-fix behaviour.
            None,
            16384,
            false,
            "en",
            false,
            &cancel,
        )
        .await;
        assert!(matches!(outcome, EngineJudgeOutcome::Cancelled));
    }

    #[tokio::test]
    async fn judge_and_requery_drops_an_overflowing_requery_source_after_re_budgeting() {
        // A tiny num_ctx budget: round-one's small block fits, but the
        // requery's extracted article alone would already fill the whole
        // budget, so the merge must drop it rather than append unchecked.
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let sources = round_one_sources();
        let outcome = judge_and_requery(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            REQUERY_QUESTION,
            sources.clone(),
            // Vertical-escalation merge contract (round-one pinned,
            // `merge_sources`), byte-identical to this test's pre-fix behaviour.
            None,
            100,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(&outcome, EngineJudgeOutcome::Sources(s, _, _, None) if s == &sources),
            "the oversized requery source must be dropped, leaving round-one sources unchanged"
        );
    }

    #[tokio::test]
    async fn judge_and_requery_bypass_cache_false_serves_the_requery_from_a_warm_serp_cache() {
        // bypass_cache=false: a warm SERP entry for the EXACT requery text
        // (the standalone question plus the judge's missing phrase) is
        // served with no engine request, the same contract every other
        // engine-tier call honours under this flag.
        let prepass = dummy_prepass();
        let requery_text = format!("{REQUERY_QUESTION} the treaty terms");
        let cached_hit = SearchHit {
            title: "Treaty Terms".into(),
            url: "https://cached-requery.example/".into(),
            snippet: "the exact terms of the treaty".into(),
        };
        let web_cache = WebCache::new(
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(600),
            64,
            128,
        );
        web_cache.serp_put(
            "duckduckgo",
            &requery_text,
            false,
            "en",
            vec![cached_hit.clone()],
        );
        web_cache.serp_put("mojeek", &requery_text, false, "en", vec![cached_hit]);
        // Only the page fetch hits the network; both engines' SERP endpoints
        // are served from the warm cache.
        let transport = FakeHttpTransport::new().with_response(
            "https://cached-requery.example/",
            HttpResponse {
                status: 200,
                final_url: "https://cached-requery.example/".into(),
                body: REQUERY_PAGE_HTML.as_bytes().to_vec(),
            },
        );
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_with_web_cache(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &web_cache,
            ),
            REQUERY_QUESTION,
            round_one_sources(),
            // Vertical-escalation merge contract (round-one pinned,
            // `merge_sources`), byte-identical to this test's pre-fix behaviour.
            None,
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(&outcome, EngineJudgeOutcome::Sources(s, _, _, None)
            if s.iter().any(|b| b.url == "https://cached-requery.example/"))
        );
        assert!(!transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
        let mojeek_url = crate::websearch::engine::mojeek_request(&requery_text, false, "en").url;
        assert!(!transport.calls().iter().any(|c| c.url == mojeek_url));
    }

    #[tokio::test]
    async fn judge_and_requery_bypass_cache_true_skips_the_warm_serp_cache() {
        // Same warm cache, but bypass_cache=true: the requery must skip the
        // read and hit the network engines instead, the flag threaded
        // through unchanged from whichever call site invoked it.
        let prepass = dummy_prepass();
        let requery_text = format!("{REQUERY_QUESTION} the treaty terms");
        let cached_hit = SearchHit {
            title: "Cached".into(),
            url: "https://cached-requery.example/".into(),
            snippet: "stale".into(),
        };
        let web_cache = WebCache::new(
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(600),
            64,
            128,
        );
        web_cache.serp_put("duckduckgo", &requery_text, false, "en", vec![cached_hit]);
        let transport = requery_transport();
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_with_web_cache(
                &prepass,
                &transport,
                &Bm25Scorer,
                &judge,
                &health,
                &bound,
                &web_cache,
            ),
            REQUERY_QUESTION,
            round_one_sources(),
            // Vertical-escalation merge contract (round-one pinned,
            // `merge_sources`), byte-identical to this test's pre-fix behaviour.
            None,
            16384,
            false,
            "en",
            true,
            &CancellationToken::new(),
        )
        .await;
        // The network's hit (requery.example), not the stale cached one, made
        // it through: the cache read was genuinely skipped, not
        // coincidentally equal.
        assert!(
            matches!(&outcome, EngineJudgeOutcome::Sources(s, _, _, None)
            if s.iter().any(|b| b.url == "https://requery.example/")
                && !s.iter().any(|b| b.url == "https://cached-requery.example/"))
        );
        assert!(transport.calls().iter().any(|c| c.url == DDG_ENDPOINT));
    }

    // ── engine-tier requery merge (fix D): fused chunk-level re-rank ───────────

    /// A scorer that assigns the same fixed score to every chunk, so a test can
    /// control round two's relevance exactly (round one's chunks carry their own
    /// injected scores) and assert on the fused ordering without depending on
    /// BM25's corpus-sensitive output.
    struct ConstScorer(f64);
    impl Scorer for ConstScorer {
        fn score(&self, _query: &str, chunks: &[String]) -> Vec<f64> {
            vec![self.0; chunks.len()]
        }
    }

    /// Round-one byproducts (`sources`, plus the `RequeryRerank` chunks and
    /// pages) for a URL distinct from the requery's `requery.example`, so the
    /// engine-tier fusion path has a genuine round one to fuse round two against.
    /// `score` is round one's injected relevance and `published` its extracted
    /// date (the two signals the fused re-rank reads).
    fn round_one_rerank(
        score: f64,
        text: &str,
        published: Option<OffsetDateTime>,
    ) -> (Vec<SourceBlock>, RequeryRerank) {
        let url = "https://round-one.example/";
        let sources = vec![SourceBlock {
            index: 1,
            url: url.into(),
            title: "Round One".into(),
            text: text.into(),
        }];
        let rerank = RequeryRerank {
            round_one_chunks: vec![ScoredChunk {
                url: url.into(),
                title: "Round One".into(),
                text: text.into(),
                score,
            }],
            round_one_pages: vec![FetchedPage {
                url: url.into(),
                title: "Round One".into(),
                text: text.into(),
                published,
            }],
        };
        (sources, rerank)
    }

    #[tokio::test]
    async fn judge_and_requery_engine_tier_fuses_stronger_round_two_ahead_of_weak_round_one() {
        // Engine-tier requery merge (`Some` rerank), non-fresh turn: round two's
        // chunks score 10.0 and round one's a weak 0.5, so the fused re-rank must
        // put the stronger round-two source FIRST rather than pinning round one
        // ahead the way the vertical-escalation (`None`) contract would.
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let (sources, rerank) = round_one_rerank(0.5, "round one is weakly relevant here", None);
        let outcome = judge_and_requery(
            &deps_for_escalation(
                &prepass,
                &transport,
                &ConstScorer(10.0),
                &judge,
                &health,
                &bound,
            ),
            REQUERY_QUESTION,
            sources,
            Some(rerank),
            16384,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(&outcome, EngineJudgeOutcome::Sources(s, _, _, None)
                if s.len() == 2
                    && s[0].url == "https://requery.example/"
                    && s[0].index == 1
                    && s[1].url == "https://round-one.example/"
                    && s[1].index == 2),
            "the stronger round-two source must outrank the weak round-one one"
        );
    }

    #[tokio::test]
    async fn judge_and_requery_engine_tier_recency_reorders_fused_sources_when_fresh() {
        // Engine-tier requery merge on a FRESH turn: round one and round two
        // carry the SAME relevance (5.0), but round one's page is ancient while
        // round two's is undated (neutral, still fresher than an ancient date).
        // Without the recency pass the equal scores would leave round one first
        // (stable append order); a round-two-first result therefore proves the
        // freshness-gated recency fusion ran over the combined set.
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let ancient = time::macros::datetime!(2000-01-01 00:00:00 UTC);
        let (sources, rerank) =
            round_one_rerank(5.0, "round one is equally relevant here", Some(ancient));
        let outcome = judge_and_requery(
            &deps_for_escalation(
                &prepass,
                &transport,
                &ConstScorer(5.0),
                &judge,
                &health,
                &bound,
            ),
            REQUERY_QUESTION,
            sources,
            Some(rerank),
            16384,
            true,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(&outcome, EngineJudgeOutcome::Sources(s, _, _, None)
                if s.first().map(|b| b.url.as_str()) == Some("https://requery.example/")),
            "the fresher round-two source must be recency-reordered ahead of the ancient round one"
        );
    }

    #[tokio::test]
    async fn judge_and_requery_engine_tier_budget_truncation_drops_the_fused_tail_not_round_two() {
        // Engine-tier requery merge, tiny budget (num_ctx 2048 -> 819 tokens):
        // round two scores 10.0 and is small; round one scores a weak 0.1 and is
        // huge. Fused best-first, round two leads and fits, so the weak oversized
        // round-one source is the fused TAIL that overflows and is dropped. The
        // old round-one-pinned merge would instead have kept round one and
        // dropped round two, so a result carrying round two but NOT round one
        // proves truncation follows fused score, not round order.
        let prepass = dummy_prepass();
        let transport = requery_transport();
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let huge = "round one padding ".repeat(400); // ~7200 chars, far over budget
        let (sources, rerank) = round_one_rerank(0.1, &huge, None);
        let outcome = judge_and_requery(
            &deps_for_escalation(
                &prepass,
                &transport,
                &ConstScorer(10.0),
                &judge,
                &health,
                &bound,
            ),
            REQUERY_QUESTION,
            sources,
            Some(rerank),
            2048,
            false,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(&outcome, EngineJudgeOutcome::Sources(s, _, _, None)
                if s.iter().any(|b| b.url == "https://requery.example/")
                    && !s.iter().any(|b| b.url == "https://round-one.example/")),
            "the fused tail (weak, oversized round one) must be dropped, not round two"
        );
    }

    /// A round-two SERP with two organic results, both distinct from round one's
    /// `match.example`, so the requery has two genuinely new sources to order.
    const TWO_RESULT_REQUERY_SERP_HTML: &str = r#"
      <div class="result">
        <a class="result__a" href="https://newer.example/">Newer Terms</a>
        <a class="result__snippet">the exact terms of the treaty</a>
      </div>
      <div class="result">
        <a class="result__a" href="https://older.example/">Older Terms</a>
        <a class="result__snippet">the exact terms of the treaty</a>
      </div>
    "#;

    /// A dense, readability-extractable requery article carrying `date_iso` as
    /// its JSON-LD publish date, so a fresh-turn fetch extracts a real date for
    /// the recency pass to sort on.
    fn dated_requery_page(date_iso: &str) -> String {
        format!(
            r#"<html><head>
            <script type="application/ld+json">{{"datePublished":"{date_iso}"}}</script>
            </head><body><article><h1>Treaty of Versailles Terms</h1>
            <p>The financial and territorial terms of the Treaty of Versailles required
            Germany to accept responsibility for the war and pay substantial reparations
            to the Allied powers over a period of decades, alongside major territorial
            concessions signed in paris and across Europe.</p>
            <p>Military restrictions, including strict caps on army size and a ban on
            maintaining an air force, were imposed as part of the same 1919 settlement,
            reshaping the postwar balance of power for a generation of European diplomacy
            that followed the signing of the treaty of versailles.</p>
            </article></body></html>"#
        )
    }

    #[tokio::test]
    async fn judge_and_requery_vertical_escalation_recency_reorders_round_two_when_fresh() {
        // Vertical-escalation merge (`None` rerank) on a FRESH turn: round one
        // (the vertical block) stays pinned first, but the two requeried round-
        // two sources must be recency-reordered among themselves before the merge
        // (`fix C` on the `None` path), so the newer precedes the older.
        let prepass = dummy_prepass();
        let newer = dated_requery_page("2026-07-01T00:00:00Z");
        let older = dated_requery_page("2001-01-01T00:00:00Z");
        let transport = FakeHttpTransport::new()
            .with_response(
                DDG_ENDPOINT,
                HttpResponse {
                    status: 200,
                    final_url: DDG_ENDPOINT.into(),
                    body: TWO_RESULT_REQUERY_SERP_HTML.as_bytes().to_vec(),
                },
            )
            .with_response(
                "https://newer.example/",
                HttpResponse {
                    status: 200,
                    final_url: "https://newer.example/".into(),
                    body: newer.into_bytes(),
                },
            )
            .with_response(
                "https://older.example/",
                HttpResponse {
                    status: 200,
                    final_url: "https://older.example/".into(),
                    body: older.into_bytes(),
                },
            );
        let judge = insufficient_then_sufficient();
        let health = EngineHealth::new();
        let bound = crate::trace::BoundRecorder::noop_for(crate::trace::ConversationId::new("t"));
        let outcome = judge_and_requery(
            &deps_for_escalation(
                &prepass,
                &transport,
                &ConstScorer(5.0),
                &judge,
                &health,
                &bound,
            ),
            REQUERY_QUESTION,
            round_one_sources(),
            None,
            16384,
            true,
            "en",
            false,
            &CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(&outcome, EngineJudgeOutcome::Sources(s, _, _, None)
                if s.len() == 3
                    && s[0].url == "https://match.example/"
                    && s[1].url == "https://newer.example/"
                    && s[2].url == "https://older.example/"),
            "round one stays pinned first; round two is recency-reordered newer-before-older"
        );
    }

    // ── full-pipeline coverage of judge_and_requery's Cancelled branch at
    //    both call sites (run_web's direct tier and commit_or_escalate's
    //    escalation-merge tier), distinct from judge_and_requery's own unit
    //    tests above which call it directly, not through run_search ────────

    #[tokio::test]
    async fn engine_tier_judge_cancellation_yields_cancelled_on_the_direct_web_path() {
        // The engine tier ran directly (no vertical), and its own judge call
        // was cancelled: `run_web`'s terminal `EngineJudgeOutcome::Cancelled`
        // branch, not just `judge_and_requery` in isolation, must abort the
        // turn.
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["treaty versailles paris"])));
        let transport = transport_with_serp_and_page();
        let judge = FakeSufficiencyJudge::returning(Err(InferenceError::Cancelled));
        let health = EngineHealth::new();
        let (_mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            "sys",
            &[],
            "when signed",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }

    /// A judge returning an insufficient verdict on its 1st call (driving a
    /// vertical's escalation in `commit_or_escalate`) and a cancellation on
    /// every call after (the engine-tier's own call in `judge_and_requery`),
    /// so a test can drive `commit_or_escalate`'s terminal
    /// `EngineJudgeOutcome::Cancelled` branch specifically.
    struct InsufficientThenCancelled {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl SufficiencyJudge for InsufficientThenCancelled {
        async fn judge(
            &self,
            _standalone_question: &str,
            _sources: &[SourceBlock],
            _cancel: &CancellationToken,
        ) -> Result<SufficiencyVerdict, InferenceError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(SufficiencyVerdict {
                    sufficient: false,
                    missing: "the treaty terms".into(),
                    reason: InsufficiencyReason::Missing,
                    requery_queries: Vec::new(),
                })
            } else {
                Err(InferenceError::Cancelled)
            }
        }
    }

    #[tokio::test]
    async fn engine_tier_judge_cancellation_yields_cancelled_on_the_escalation_merge_path() {
        // The vertical escalates, the engines answer, and the engine-tier's
        // OWN judge call (the 2nd) is cancelled: `commit_or_escalate`'s
        // terminal `EngineJudgeOutcome::Cancelled` branch must abort the
        // turn.
        let prepass = FakePrePass::returning(Ok(news_route_decision()));
        let transport = news_and_engine_transport();
        let judge = InsufficientThenCancelled {
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let health = EngineHealth::new();
        let (_mock, bound) = mock_recorder();
        let (_p, status) = recorder();
        let outcome = run_search(
            &deps_for_escalation(&prepass, &transport, &Bm25Scorer, &judge, &health, &bound),
            "sys",
            &[],
            "q",
            16384,
            "2026-07-05",
            "en-US",
            &CancellationToken::new(),
            &status,
        )
        .await;
        assert!(matches!(outcome, SearchOutcome::Cancelled));
    }
}
