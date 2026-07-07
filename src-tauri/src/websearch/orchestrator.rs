//! The search orchestrator: the fixed, non-agentic pipeline that turns one user
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
//! - `no` decision, empty results, or nothing worth citing after ranking →
//!   [`SearchOutcome::NoSearch`];
//! - cancellation mid-pipeline → [`SearchOutcome::Cancelled`].
//!
//! `cached` is mapped to `web` for now (a correct re-search); the TTL'd
//! multi-turn source cache is a later optimisation.

use tokio_util::sync::CancellationToken;

use crate::commands::ChatMessage;
use crate::net::transport::HttpTransport;
use crate::websearch::assemble::{assemble_context, SourceBlock};
use crate::websearch::engine::{web_search, SearchHit};
use crate::websearch::fetch::fetch_pages;
use crate::websearch::prefilter::{prefilter, PreFilterVerdict};
use crate::websearch::prepass::{InferenceError, PrePass, PrePassDecision, SearchDecision};
use crate::websearch::rank::{select_chunks, Scorer};
use crate::websearch::writer::writer_messages;

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
}

/// What the orchestrator resolved for this turn.
pub enum SearchOutcome {
    /// Answer the user directly with the plain chat prompt (no decision, an
    /// infra failure, or a search that found nothing worth citing).
    NoSearch,
    /// Answer with the source-grounded writer prompt. `messages` already embeds
    /// the delimited sources; `sources` is the citation metadata for the UI.
    Answer {
        messages: Vec<ChatMessage>,
        sources: Vec<SourceBlock>,
    },
    /// The user cancelled during the pipeline; the caller emits `Cancelled` and
    /// streams nothing.
    Cancelled,
}

/// The injected effectful dependencies of the pipeline.
pub struct SearchDeps<'a> {
    pub prepass: &'a dyn PrePass,
    pub transport: &'a dyn HttpTransport,
    pub scorer: &'a dyn Scorer,
}

/// Runs the pipeline for one turn. `chat_system_prompt`, `history`, and
/// `latest_user` MUST be byte-identical to the plain chat prompt so the
/// pre-pass and writer share llama-server's warm KV prefix.
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
    // Stage one: deterministic pre-filter, no model call.
    let verdict = prefilter(latest_user, today);
    eprintln!("[search] prefilter={verdict:?}");
    if verdict == PreFilterVerdict::ForceNo {
        return SearchOutcome::NoSearch;
    }
    // Stage two: the persona-free classifier decides the ambiguous middle and
    // rewrites the query. A `ForceWeb` verdict still runs it, for the rewrite.
    status(SearchPhase::Deciding);
    let classified = match deps
        .prepass
        .decide(history, latest_user, today, cancel)
        .await
    {
        Ok(classified) => classified,
        Err(InferenceError::Cancelled) => return SearchOutcome::Cancelled,
        // A search-infra failure must never block the answer: fall back to a
        // plain, model-only response.
        Err(InferenceError::Request(_)) => return SearchOutcome::NoSearch,
    };
    let decision = resolve_decision(verdict, classified);
    eprintln!(
        "[search] decision={:?} queries={}",
        decision.decision,
        decision.queries.len()
    );
    match decision.decision {
        SearchDecision::No => SearchOutcome::NoSearch,
        // `cached` is mapped to `web` for now (a correct re-search).
        SearchDecision::Web | SearchDecision::Cached => {
            run_web(
                deps,
                chat_system_prompt,
                history,
                latest_user,
                &decision.standalone_question,
                &decision.queries,
                num_ctx,
                today,
                locale,
                cancel,
                status,
            )
            .await
        }
    }
}

/// Combines the pre-filter verdict with the classifier's result into the final
/// decision. `ForceWeb` overrides the classifier's own `search` value to `web`
/// (the deterministic freshness signal is authoritative), keeping the
/// classifier's standalone rewrite and queries and backfilling a query from the
/// rewrite when the classifier, having leaned "no", produced none. Any other
/// verdict leaves the classifier's decision untouched.
fn resolve_decision(verdict: PreFilterVerdict, classified: PrePassDecision) -> PrePassDecision {
    match verdict {
        PreFilterVerdict::ForceWeb => {
            let queries = if classified.queries.is_empty() {
                vec![classified.standalone_question.clone()]
            } else {
                classified.queries
            };
            PrePassDecision {
                decision: SearchDecision::Web,
                standalone_question: classified.standalone_question,
                queries,
            }
        }
        // Ambiguous honours the classifier verbatim; ForceNo never reaches here
        // (it short-circuits before the model call).
        PreFilterVerdict::Ambiguous | PreFilterVerdict::ForceNo => classified,
    }
}

/// The `web`/`cached` branch: search every query, fetch and rank the pages,
/// assemble a budgeted source set, and build the writer prompt. Degrades to
/// [`SearchOutcome::NoSearch`] when nothing citable survives and to
/// [`SearchOutcome::Cancelled`] on cancellation.
#[allow(clippy::too_many_arguments)]
async fn run_web(
    deps: &SearchDeps<'_>,
    chat_system_prompt: &str,
    history: &[ChatMessage],
    latest_user: &str,
    standalone_question: &str,
    queries: &[String],
    num_ctx: u32,
    today: &str,
    locale: &str,
    cancel: &CancellationToken,
    status: &(dyn Fn(SearchPhase) + Send + Sync),
) -> SearchOutcome {
    if cancel.is_cancelled() {
        return SearchOutcome::Cancelled;
    }
    status(SearchPhase::Searching);
    let mut hits: Vec<SearchHit> = Vec::new();
    for query in queries {
        if cancel.is_cancelled() {
            return SearchOutcome::Cancelled;
        }
        hits.extend(web_search(deps.transport, query).await);
    }
    let hits = dedupe_hits(hits);
    if hits.is_empty() {
        return SearchOutcome::NoSearch;
    }
    if cancel.is_cancelled() {
        return SearchOutcome::Cancelled;
    }
    status(SearchPhase::Reading);
    let pages = fetch_pages(deps.transport, &hits, num_ctx).await;
    let chunks = select_chunks(&pages, standalone_question, deps.scorer);
    let sources = assemble_context(&chunks, num_ctx);
    if sources.is_empty() {
        return SearchOutcome::NoSearch;
    }
    let messages = writer_messages(
        chat_system_prompt,
        history,
        latest_user,
        &sources,
        today,
        locale,
    );
    SearchOutcome::Answer { messages, sources }
}

/// Removes cross-query duplicate hits by URL, preserving first-seen order.
fn dedupe_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen = std::collections::HashSet::new();
    hits.into_iter()
        .filter(|h| seen.insert(h.url.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{FakeHttpTransport, HttpRequest, HttpResponse, TransportError};
    use crate::websearch::prepass::{FakePrePass, PrePassDecision};
    use crate::websearch::rank::Bm25Scorer;
    use async_trait::async_trait;
    use std::sync::Mutex;

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
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: queries.into_iter().map(String::from).collect(),
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

    fn deps<'a>(
        prepass: &'a dyn PrePass,
        transport: &'a dyn HttpTransport,
        scorer: &'a dyn Scorer,
    ) -> SearchDeps<'a> {
        SearchDeps {
            prepass,
            transport,
            scorer,
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

    // ── run_search: decision branches ─────────────────────────────────────────

    #[tokio::test]
    async fn classifier_no_decision_yields_no_search() {
        // An ambiguous turn ("tell me a joke") reaches the classifier, which
        // returns `no`: the Deciding phase is emitted, then no search runs.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::No,
            standalone_question: "tell me a joke".into(),
            queries: vec![],
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
            standalone_question: "when was the treaty of versailles signed in paris".into(),
            queries: vec![],
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

    // ── resolve_decision (pure) ───────────────────────────────────────────────

    #[test]
    fn resolve_force_web_overrides_no_and_backfills_queries() {
        let out = resolve_decision(
            PreFilterVerdict::ForceWeb,
            PrePassDecision {
                decision: SearchDecision::No,
                standalone_question: "current tokyo weather".into(),
                queries: vec![],
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
                standalone_question: "q".into(),
                queries: vec!["a".into(), "b".into()],
            },
        );
        assert_eq!(out.decision, SearchDecision::Web);
        assert_eq!(out.queries, vec!["a", "b"]);
    }

    #[test]
    fn resolve_ambiguous_keeps_classifier_decision() {
        let classified = PrePassDecision {
            decision: SearchDecision::No,
            standalone_question: "q".into(),
            queries: vec![],
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
            standalone_question: "q".into(),
            queries: vec!["a".into()],
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
                && messages.last().is_some_and(|m| m.content.starts_with("when signed")
                    && m.content.contains("UNTRUSTED_WEB_CONTENT")
                    && m.content.contains("treaty")))
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
    async fn web_with_no_results_degrades_to_no_search() {
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q"])));
        // SERP returns a challenge page -> zero hits.
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

    #[tokio::test]
    async fn web_with_no_relevant_chunks_degrades_to_no_search() {
        // The page has real text but shares no term with the standalone question,
        // so BM25 keeps nothing and there is nothing to cite.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Web,
            standalone_question: "quantum chromodynamics lagrangian".into(),
            queries: vec!["q".into()],
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
        assert!(matches!(outcome, SearchOutcome::NoSearch));
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
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q one", "q two"])));
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
        // post-search cancellation check aborts before fetching pages.
        let prepass = FakePrePass::returning(Ok(web_decision(vec!["q"])));
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
    async fn cached_decision_runs_the_web_pipeline() {
        // Cached is mapped to Web for now, so it still grounds the answer.
        let prepass = FakePrePass::returning(Ok(PrePassDecision {
            decision: SearchDecision::Cached,
            standalone_question: "treaty of versailles signed paris".into(),
            queries: vec!["treaty versailles".into()],
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
    }
}
