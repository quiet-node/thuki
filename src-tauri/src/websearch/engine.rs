//! Keyless search-engine client with parallel racing + rank fusion.
//!
//! General queries that no vertical handles fall through to keyless engine
//! scraping from the user's device. [`web_search`] fires one request to every
//! live engine in [`ENGINES`] *concurrently* (DuckDuckGo's `html` endpoint and
//! Mojeek today), then fuses their ranked result lists with Reciprocal Rank
//! Fusion ([`rrf_fuse`]). Racing makes per-query latency `max()` of the engines
//! instead of the old sequential `sum()`, and fusion is a quality win in its own
//! right: a URL that ranks on two independent engines is far likelier to be
//! relevant and far less likely to be junk (observed live: hoax and
//! age-calculator pages that surface from a single engine's list get outranked
//! once a second engine disagrees). Adding a third engine later is one [`ENGINES`]
//! entry with no control-flow change.
//!
//! Fusion also consults a static, compiled-in domain-credibility list (see
//! [`super::credibility`]): individually verified hoax domains are hard-dropped
//! before fusion, bulk-imported spam and copycat domains take a soft rank
//! penalty, and encyclopedic and primary-reference domains are promoted. The
//! penalty and boost are rank offsets tuned so cross-engine agreement still wins,
//! so the list biases ranking without ever overriding a genuine two-engine hit.
//!
//! A tripped bot challenge (empirically IP-scoped and multi-hour on DuckDuckGo,
//! per the T1 spike) classifies as [`SerpOutcome::Blocked`] and marks that
//! engine cooling for its cooldown window; the other engines' results still fuse
//! and return, so a single engine's rate-limit is no longer fatal to the turn.
//! An engine already inside its cooldown is not requested at all. When no engine
//! returns a usable list the result is empty and the caller degrades gracefully
//! to a plain answer.
//!
//! Each engine's outcome logs to stderr under a `[search]` prefix, followed by a
//! single fused summary line, so the decision path is visible in the dev
//! console: which engines ran, whether each was blocked, empty, or returned N
//! hits, and how many URLs the fusion kept.
//!
//! All requests go through the injectable [`HttpTransport`], so the client is
//! tested against fixture SERP HTML with no network. The parsers are pure and
//! total (malformed HTML yields fewer rows, never a panic).

use scraper::{Html, Selector};

use crate::net::transport::{HttpMethod, HttpRequest, HttpTransport};
use crate::trace::EngineStat;
use crate::websearch::credibility::{classify_domain, DomainClass};
use crate::websearch::lang::{
    accept_language, ddg_region, detect_request_lang, mojeek_language_bias,
};
use crate::websearch::serp_cache::WebCache;

/// One search-engine result row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// How a raw SERP response classifies before parsing is trusted.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SerpOutcome {
    /// A 200 with at least one parsed row; its list joins the fusion.
    Ok,
    /// A bot challenge or non-200 status; the engine is marked cooling and
    /// contributes no list to the fusion.
    Blocked,
    /// A 200 that parsed to zero rows; contributes no list (no cooldown: an
    /// empty page is a bad query, not a ban).
    Empty,
}

/// Body substrings that mark a bot-detection interstitial rather than results.
/// `captcha-wrap` and `altcha-widget` are Mojeek's own Altcha proof-of-work
/// challenge markup (live-verified: a 200 response carrying `<title>Captcha</title>`
/// and these two class/tag names, zero parsed result rows). Without them a
/// captcha'd Mojeek response falls through [`classify_serp`] to
/// [`SerpOutcome::Empty`] instead of [`SerpOutcome::Blocked`], so the engine is
/// never marked cooling and gets re-hammered on every subsequent query instead
/// of backing off.
const CAPTCHA_MARKERS: &[&str] = &[
    "anomaly-modal",
    "challenge-form",
    "cf-challenge",
    "hcaptcha",
    "recaptcha",
    "captcha-wrap",
    "altcha-widget",
];

/// Browser User-Agent sent verbatim on every engine request so it is
/// indistinguishable from a real browser's; keyless SERP endpoints reject
/// obvious automation. Shared by all engines.
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const DDG_HTML_ENDPOINT: &str = "https://html.duckduckgo.com/html/";
const MOJEEK_ENDPOINT: &str = "https://www.mojeek.com/search";

/// One keyless search engine: a name for logging and cooldown keying, a request
/// builder, a pure SERP parser, and how long to skip it after it blocks.
/// [`web_search`] races every live engine and fuses their lists; this struct is
/// the per-engine plug-in that makes adding an engine a single [`ENGINES`] entry.
struct Engine {
    /// Identifier used in the `[search]` stderr log line and as the cooldown key.
    name: &'static str,
    /// Builds the outbound request for `query`. `freshness` biases the request
    /// toward recent results when the turn's standalone question carried a
    /// freshness signal (see [`super::encyclopedia::is_volatile_question`]).
    build: fn(&str, bool) -> HttpRequest,
    /// Parses this engine's SERP HTML into result rows.
    parse: fn(&str) -> Vec<SearchHit>,
    /// How long the engine is skipped after a block (seconds). Matches the
    /// engine's observed block behaviour: hours for DuckDuckGo, soft minutes for
    /// the fallbacks.
    cooldown_s: u64,
}

/// Engines raced concurrently and rank-fused for every query. DuckDuckGo has
/// the richest results; Mojeek is scraper-tolerant, keyless, and serves
/// lightweight HTML that survives a DuckDuckGo IP block, so fusing the two
/// covers each other's blind spots. Verticals (Wikipedia, weather, news) are a
/// separate
/// intent-routed layer added later; this is the general-web tier only. Other
/// candidates were probed live and rejected: Brave/Startpage/Qwant serve
/// JS-shell pages with no parseable server-side results, Ecosia and Presearch
/// return 403 to non-browser clients, and Bing's organic markup no longer ships
/// in the initial HTML.
const ENGINES: &[Engine] = &[
    Engine {
        name: "duckduckgo",
        build: ddg_html_request,
        parse: parse_ddg_html,
        cooldown_s: crate::config::defaults::ENGINE_COOLDOWN_PRIMARY_S,
    },
    Engine {
        name: "mojeek",
        build: mojeek_request,
        parse: parse_mojeek_html,
        cooldown_s: crate::config::defaults::ENGINE_COOLDOWN_FALLBACK_S,
    },
];

/// Cross-turn engine block memory. When an engine returns a bot challenge or
/// rate-limit response it is marked here and skipped for its cooldown window on
/// subsequent queries, instead of being re-hammered on every query of every
/// turn (which wastes a request, adds latency, and feeds the volume signal that
/// keeps volume-triggered blocks alive).
///
/// Thread-safe: the map is behind a `Mutex`, and the lock is held only for the
/// map lookup/insert, never across I/O. Memory-bounded by construction: keys
/// are the static engine names, so the map can never exceed [`ENGINES`] len.
pub struct EngineHealth {
    blocked_until: std::sync::Mutex<std::collections::HashMap<&'static str, std::time::Instant>>,
}

impl EngineHealth {
    /// Creates an empty registry (no engine cooling).
    pub fn new() -> Self {
        Self {
            blocked_until: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Marks `name` blocked for the next `cooldown_s` seconds.
    fn mark_blocked(&self, name: &'static str, cooldown_s: u64) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(cooldown_s);
        self.blocked_until.lock().unwrap().insert(name, until);
    }

    /// Whether `name` is inside its cooldown window. Expired entries are pruned
    /// on read so the map self-cleans.
    fn is_cooling(&self, name: &'static str) -> bool {
        let mut map = self.blocked_until.lock().unwrap();
        match map.get(name) {
            Some(until) if *until > std::time::Instant::now() => true,
            Some(_) => {
                map.remove(name);
                false
            }
            None => false,
        }
    }
}

impl Default for EngineHealth {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl EngineHealth {
    /// Test-only: force every engine into a long cooldown, so a consumer's test
    /// (the orchestrator's escalation branch) can construct an
    /// all-engines-cooling registry without driving live block responses through
    /// the transport.
    pub(crate) fn block_all_for_test(&self) {
        for engine in ENGINES {
            self.mark_blocked(engine.name, 3600);
        }
    }
}

/// The process-wide [`EngineHealth`] shared by every turn, so a block observed
/// on one message is remembered on the next. Coverage-excluded: a static
/// constructor call; the registry's behaviour is tested through instance
/// methods on locally-built registries.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn global_engine_health() -> &'static EngineHealth {
    static GLOBAL: std::sync::LazyLock<EngineHealth> = std::sync::LazyLock::new(EngineHealth::new);
    &GLOBAL
}

/// Whether at least one engine is not currently inside its block cooldown, so
/// an escalation to the scraped-engine tier has some chance of reaching a live
/// engine. When every engine is cooling, escalating a vertical's insufficient
/// answer would only add latency before an inevitable miss (and, on the burst
/// that caused the cooldowns, risk deepening the block), so the orchestrator
/// serves the vertical's partial answer instead of escalating (see
/// `crate::websearch::orchestrator`). Reads through [`EngineHealth::is_cooling`],
/// which prunes expired entries as it checks, so a lapsed cooldown counts as
/// available again.
pub fn any_engine_available(health: &EngineHealth) -> bool {
    ENGINES.iter().any(|engine| !health.is_cooling(engine.name))
}

/// The per-engine result of one raced request, carried back from the concurrent
/// tasks to the sequential (ENGINES-order) fusion step. Kept out of the async
/// tasks: cooldown marking happens after the join so `EngineHealth`'s lock is
/// never touched across an `await`.
enum EngineOutcome {
    /// A usable ranked list to feed into the fusion.
    Hits(Vec<SearchHit>),
    /// A bot challenge or non-200: the engine must be marked cooling.
    Blocked,
    /// A 200 with zero rows: contributes nothing, no cooldown.
    Empty,
    /// The transport/SSRF layer errored: contributes nothing, no cooldown.
    TransportError,
}

/// Maps one engine's raced outcome to the lean status string surfaced in
/// [`EngineStat`] for the forensic trace (see
/// [`crate::trace::RecorderEvent::SearchRetrieved`]). Pure and total: every
/// `EngineOutcome` variant maps to exactly one string. Kept separate from
/// [`web_search`]'s coverage-excluded async glue so this mapping is
/// unit-tested directly rather than only exercised indirectly through the
/// async racing tests below.
fn outcome_status(outcome: &EngineOutcome) -> &'static str {
    match outcome {
        EngineOutcome::Hits(_) => "ok",
        EngineOutcome::Blocked => "blocked",
        EngineOutcome::Empty => "empty",
        EngineOutcome::TransportError => "transport_error",
    }
}

/// Decides, from the scraped-engine tier's per-engine outcome summary (see
/// [`EngineStat`]), whether an empty result means the web was unreachable
/// rather than merely fruitless. Returns `true` only when the transport layer
/// failed for EVERY engine actually contacted this tier: at least one engine
/// was contacted, and every contacted engine came back `"transport_error"`.
///
/// The distinction drives which failure the user is shown, so it is drawn
/// conservatively toward "found nothing":
/// - `"transport_error"` is the only status that proves a network/transport
///   failure, so it is the only one that can add up to "unreachable".
/// - `"cooling"` is an engine deliberately skipped for its own prior block, not
///   a network failure happening now, so it is ignored: neither evidence of a
///   reachable web nor of an unreachable one.
/// - Any other status (`"ok"`, `"empty"`, `"blocked"`, `"cache_hit"`, or an
///   unrecognised future value) means that engine's HTTP response, or a prior
///   successful fetch, DID come back, so the web was reached; one such engine
///   is enough to make the miss "found nothing", never "could not connect".
///
/// With no engine contacted at all (every engine cooling, or no queries ran)
/// there is no transport-failure evidence, so this returns `false` and the
/// caller reports "found nothing" rather than blaming the connection.
pub(crate) fn transport_unreachable(stats: &[EngineStat]) -> bool {
    let mut contacted = false;
    for stat in stats {
        match stat.status.as_str() {
            // Skipped before any request went out: not a contact this turn.
            "cooling" => {}
            // A real network/transport failure: the only status that keeps the
            // "unreachable" verdict alive.
            "transport_error" => contacted = true,
            // Any HTTP response or cache hit proves the web was reached, so the
            // miss cannot be a connectivity failure: short-circuit to "found
            // nothing".
            _ => return false,
        }
    }
    contacted
}

/// Runs a keyless web search for `query` by racing every live engine in
/// [`ENGINES`] concurrently and fusing their ranked lists with Reciprocal Rank
/// Fusion. Engines inside their block cooldown (see [`EngineHealth`]) are not
/// requested at all. Each remaining engine is first checked against the
/// in-memory SERP cache (`cache`), UNLESS `bypass_cache` is set: a cache hit
/// contributes its stored list to the fusion WITHOUT issuing a request (the
/// strongest burst-reduction there is, since a repeat query costs the engine
/// nothing), and only the cache-missing engines are actually raced. Every
/// raced engine gets exactly one request. A bot challenge or rate-limit
/// response marks that engine blocked for its cooldown window and is NOT
/// cached (a block must never be replayed as truth); an empty SERP or
/// transport/SSRF error contributes nothing without marking (an empty page is
/// a bad query, not a ban) and is not cached either; only a
/// successfully-parsed (Ok) list is written to the cache, REGARDLESS of
/// `bypass_cache` (a fresh fetch always refreshes the entry, so the very next
/// normal turn benefits instead of re-serving what this call bypassed). The
/// surviving lists are fused ([`rrf_fuse`]), then deduped and capped
/// ([`dedupe_and_cap`]) so the final length still honours
/// [`crate::config::defaults::SERP_MAX_RESULTS_PER_QUERY`]. When no engine
/// returns a usable list the result is empty, so the caller degrades
/// gracefully rather than hanging or hallucinating. Each engine's outcome logs
/// to stderr under `[search]`, followed by one fused summary line. `freshness`
/// is forwarded to every engine's request builder, but only
/// [`ddg_html_request`] applies it (see [`mojeek_request`] for why Mojeek does
/// not) to bias results toward recent content when the turn's standalone
/// question carried a freshness signal.
///
/// `bypass_cache` is the read-bypass, write-through contract for an explicit
/// user re-search ("look it up again"): the user is telling us the result we
/// just served (possibly straight from this same cache, within its TTL) was
/// not trusted, so replaying it would silently re-serve exactly what they are
/// asking us to re-check. Setting it skips every engine's cache READ for this
/// call only (every live engine races as if cold); it does not disable the
/// cache, which still gets a fresh write below so the stale entry is replaced
/// rather than left to keep answering the next, non-explicit turn. Logged once
/// per call (not per engine) via `[search] cache bypass (explicit search)`.
///
/// Burst safety: this sends AT MOST ONE request per engine per query, exactly as
/// the old sequential path did on its failure branch. DuckDuckGo's block is
/// volume-triggered, and racing does not raise DuckDuckGo volume (still one DDG
/// request per query); it only issues the Mojeek request concurrently instead of
/// only-on-DDG-failure. So the change trades latency (`sum` -> `max`) and adds
/// fusion quality without adding any per-engine request volume. A cache-bypassing
/// call is no exception: it still sends at most one request per live engine.
///
/// Coverage-excluded: thin async glue over the injectable transport that
/// delegates every decision to the pure, directly-tested helpers (each engine's
/// request builder and parser, [`classify_serp`], [`rrf_fuse`],
/// [`dedupe_and_cap`], [`outcome_status`], [`EngineHealth`]); its racing +
/// fusion behaviour is still exercised against
/// [`crate::net::transport::FakeHttpTransport`] in the tests below. Excluded
/// only because parallel async coverage attribution is nondeterministic.
///
/// Returns the fused hit list alongside a lean per-engine outcome summary
/// (see [`EngineStat`]): one entry per engine actually consulted this call,
/// whether it was skipped for cooling, served from the SERP cache, or raced
/// live. The caller (the orchestrator's `run_engine_tier`) forwards these
/// into [`crate::trace::RecorderEvent::SearchRetrieved`] so a trace shows a
/// silently-empty or silently-blocked engine even when the overall fused
/// result looks healthy, instead of only the stderr `[search]` log below.
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn web_search(
    transport: &dyn HttpTransport,
    query: &str,
    health: &EngineHealth,
    freshness: bool,
    cache: &WebCache,
    bypass_cache: bool,
) -> (Vec<SearchHit>, Vec<EngineStat>) {
    if bypass_cache {
        eprintln!("[search] cache bypass (explicit search)");
    }
    // Cache-hit lists (served without a request) plus the tasks for the engines
    // that must actually be raced. The SERP cache is read here, before the race,
    // never inside the async tasks, so the cache lock is never held across an
    // `await` (the same discipline `health` follows). `join_all` preserves input
    // order for the raced engines, so the whole path stays deterministic.
    let mut lists: Vec<Vec<SearchHit>> = Vec::new();
    let mut stats: Vec<EngineStat> = Vec::new();
    let mut tasks = Vec::new();
    for engine in ENGINES {
        if health.is_cooling(engine.name) {
            eprintln!("[search] engine={} cooling -> skipped", engine.name);
            stats.push(EngineStat {
                name: engine.name.to_string(),
                status: "cooling".to_string(),
                hit_count: 0,
            });
            continue;
        }
        if !bypass_cache {
            if let Some(hits) = cache.serp_get(engine.name, query, freshness) {
                eprintln!(
                    "[search] engine={} serp cache hit hits={}",
                    engine.name,
                    hits.len()
                );
                stats.push(EngineStat {
                    name: engine.name.to_string(),
                    status: "cache_hit".to_string(),
                    hit_count: hits.len(),
                });
                lists.push(hits);
                continue;
            }
        }
        let request = (engine.build)(query, freshness);
        tasks.push(async move {
            let outcome = match transport.send(&request).await {
                Err(_) => EngineOutcome::TransportError,
                Ok(response) => {
                    let body = String::from_utf8_lossy(&response.body);
                    let hits = (engine.parse)(&body);
                    match classify_serp(response.status, hits.len(), &body) {
                        SerpOutcome::Ok => EngineOutcome::Hits(hits),
                        SerpOutcome::Blocked => EngineOutcome::Blocked,
                        SerpOutcome::Empty => EngineOutcome::Empty,
                    }
                }
            };
            (engine.name, engine.cooldown_s, outcome)
        });
    }

    let results = futures_util::future::join_all(tasks).await;

    for (name, cooldown_s, outcome) in results {
        // Status and hit count both derive from the pure `outcome_status`
        // classification (and a plain length read); this loop only plumbs
        // the result into the cache, the cooldown registry, and the stats
        // list, it makes no further decisions of its own.
        let status = outcome_status(&outcome);
        let hit_count = if let EngineOutcome::Hits(hits) = &outcome {
            hits.len()
        } else {
            0
        };
        stats.push(EngineStat {
            name: name.to_string(),
            status: status.to_string(),
            hit_count,
        });
        match outcome {
            EngineOutcome::Hits(mut hits) => {
                eprintln!("[search] engine={name} ok hits={}", hits.len());
                // Bound the raw per-engine row count BEFORE caching and before it
                // joins fusion. A parser pushes one row per DOM node with no row
                // cap, so an oversized or format-changed SERP could otherwise
                // cache an unbounded Vec under up to `SERP_CACHE_MAX_ENTRIES` keys
                // for the whole `SERP_CACHE_TTL_S` window. The cap is generous
                // (well above a normal ~30-row page), so a real result page is
                // never truncated and every row still reaches fusion, preserving
                // recall; only a pathologically long list is trimmed. Trimming
                // the same list that goes into `lists` keeps the cache-hit and
                // cache-miss paths symmetric: both feed the identical per-engine
                // list into fusion, so a repeat query served from the cache fuses
                // exactly what a fresh race would. The fused list is still capped
                // to the output ceiling by `dedupe_and_cap` below.
                hits.truncate(crate::config::defaults::SERP_MAX_RAW_HITS_PER_QUERY);
                // Only Ok lists are cached; a repeat of this exact query within
                // the TTL is then served from memory above.
                cache.serp_put(name, query, freshness, hits.clone());
                lists.push(hits);
            }
            EngineOutcome::Blocked => {
                health.mark_blocked(name, cooldown_s);
                eprintln!("[search] engine={name} blocked -> cooldown {cooldown_s}s");
            }
            EngineOutcome::Empty => eprintln!("[search] engine={name} empty"),
            EngineOutcome::TransportError => {
                eprintln!("[search] engine={name} transport_error")
            }
        }
    }

    // Hard-remove credibility drop-class hits (individually verified hoax and
    // impostor domains) from every engine's list before fusion. Engine count is
    // preserved (an emptied list still counts), so the fused summary line below is
    // unaffected.
    let lists: Vec<Vec<SearchHit>> = lists.into_iter().map(drop_incredible).collect();
    let fused = rrf_fuse(&lists);
    let capped = dedupe_and_cap(
        fused,
        crate::config::defaults::SERP_MAX_RESULTS_PER_QUERY,
        crate::config::defaults::SERP_MAX_RESULTS_PER_DOMAIN,
    );
    eprintln!(
        "[search] fused engines={} urls={}",
        lists.len(),
        capped.len()
    );
    (capped, stats)
}

/// Fuses per-engine ranked lists into one ranked list with Reciprocal Rank
/// Fusion: each URL's score is the sum over engines of `1 / (RRF_K + rank)`,
/// where `rank` is the URL's 1-based position in that engine's list. A URL that
/// appears on two engines therefore outscores one that is rank-1 on a single
/// engine whenever the arithmetic says so (e.g. rank 3 on both engines scores
/// `2/63 ≈ 0.0317`, beating rank 1 on one engine at `1/61 ≈ 0.0164`), which is
/// the whole point: cross-engine agreement is a strong relevance signal and a
/// strong junk filter.
///
/// The output is sorted by score descending with ties broken by first-seen order
/// across the input lists (a stable sort over a first-seen-ordered vector), so
/// the result is fully deterministic. A URL repeated within a single engine's
/// list contributes only once, at its best (first) rank, so the "sum over
/// engines" semantics is not inflated by intra-list duplicates. Fusing a single
/// list is an identity on order (every score is monotonic in rank), so the
/// one-live-engine case degrades to that engine's own ordering. Empty input
/// yields an empty list.
///
/// Fusion also consults the compiled-in domain-credibility list (see
/// [`crate::websearch::credibility`]): a boost-class URL is scored as if it were
/// rank 1 in every list it appears on, and a penalize-class URL's rank is pushed
/// down by [`crate::config::defaults::CREDIBILITY_PENALTY_RANK_OFFSET`]. Both are
/// tuning-free safety ceilings: the maximum single-list boosted score
/// `1 / (60 + 1) = 0.01639` is strictly below a dual-engine rank-10 page's
/// `0.02857`, so a boosted-but-irrelevant page can never beat a genuinely
/// relevant unboosted one on the boost alone, and the penalty offset keeps a
/// single-engine spam hit below a dual-engine legitimate one without dropping it.
pub(crate) fn rrf_fuse(lists: &[Vec<SearchHit>]) -> Vec<SearchHit> {
    rrf_fuse_classified(lists, &|url| classify_domain(&super::domain_of(url)))
}

/// The credibility-aware fusion core behind [`rrf_fuse`], with the domain
/// classifier injected so the scoring math is testable independently of the real
/// embedded credibility list (tests pass a synthetic classifier; the public
/// [`rrf_fuse`] passes the real [`classify_domain`] over the URL's host). Boost
/// URLs use an effective rank of 1 in each list; penalize URLs add the offset
/// [`crate::config::defaults::CREDIBILITY_PENALTY_RANK_OFFSET`] to their rank;
/// drop and neutral URLs use their native rank (drop-class URLs are already
/// removed upstream by [`drop_incredible`], so they never reach this step). All
/// other semantics (first-seen tiebreak, intra-list dedup, stable sort) match
/// [`rrf_fuse`].
///
/// The classifier is taken as `&dyn Fn` rather than a generic `impl Fn` so this
/// core compiles to a single instantiation. A generic bound monomorphizes once
/// per closure type (the real classifier plus each test's synthetic one), and
/// per-instantiation coverage attribution then flags match arms that a given
/// monomorphization never exercises; one shared instantiation keeps the fusion
/// math deterministic to measure and dynamic dispatch is free on this cold path.
fn rrf_fuse_classified(
    lists: &[Vec<SearchHit>],
    classify: &dyn Fn(&str) -> DomainClass,
) -> Vec<SearchHit> {
    let k = crate::config::defaults::RRF_K as f64;
    let penalty = crate::config::defaults::CREDIBILITY_PENALTY_RANK_OFFSET as f64;
    // First-seen order of URLs across all lists; the stable sort below breaks
    // score ties by this order. Both the intra-list dedup set and the score
    // map key on `canonical_url_key`, not the raw URL string, so a same-page
    // hit that two engines render with a trailing-slash or http/https
    // difference is treated as one URL and scores as genuine cross-engine
    // agreement instead of two separate, weaker single-engine hits.
    let mut order: Vec<SearchHit> = Vec::new();
    let mut scores: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for list in lists {
        // A URL may appear at most once per engine, at its first (best) rank.
        let mut seen_in_list: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (idx, hit) in list.iter().enumerate() {
            if !seen_in_list.insert(super::canonical_url_key(&hit.url)) {
                continue;
            }
            let rank = (idx + 1) as f64;
            // Boost pins the effective rank to 1; penalize pushes the rank down by
            // a fixed offset; drop and neutral keep the native rank.
            let effective_rank = match classify(&hit.url) {
                DomainClass::Boost => 1.0,
                DomainClass::Penalize => rank + penalty,
                DomainClass::Drop | DomainClass::Neutral => rank,
            };
            let contribution = 1.0 / (k + effective_rank);
            let key = super::canonical_url_key(&hit.url);
            match scores.get_mut(&key) {
                Some(score) => *score += contribution,
                None => {
                    scores.insert(key, contribution);
                    order.push(hit.clone());
                }
            }
        }
    }
    // slice::sort_by is stable, so equal scores keep their first-seen order.
    // Scores are always finite positives, so partial_cmp never returns None.
    order.sort_by(|a, b| {
        scores[&super::canonical_url_key(&b.url)]
            .partial_cmp(&scores[&super::canonical_url_key(&a.url)])
            .expect("fusion scores are finite")
    });
    order
}

/// Removes hits whose host classifies as [`DomainClass::Drop`] from one engine's
/// parsed list before fusion, logging each removal under the `[search]` prefix.
/// Drop-class domains are the individually verified hoax and impostor sources, so
/// hard removal is intended rather than the soft rank penalty applied to the
/// bulk-imported penalize set. Pure apart from the stderr log and total: a list
/// with no drop-class hits is returned unchanged.
fn drop_incredible(list: Vec<SearchHit>) -> Vec<SearchHit> {
    list.into_iter()
        .filter(|hit| {
            let dropped = matches!(
                classify_domain(&super::domain_of(&hit.url)),
                DomainClass::Drop
            );
            if dropped {
                eprintln!("[search] credibility drop url={}", hit.url);
            }
            !dropped
        })
        .collect()
}

/// Builds the DuckDuckGo `html` POST request with browser headers and the
/// form-encoded query. When `freshness` is set (the turn's standalone question
/// carried a freshness signal), the request is biased toward recent results
/// via [`crate::config::defaults::DDG_FRESHNESS_DF_VALUE`], set both as a `df`
/// form field and as a `df` cookie: the HTML endpoint honours either placement,
/// so both are set for reliability.
///
/// The query's language ([`detect_request_lang`]) drives BOTH the `kl` region
/// and the `Accept-Language` header, and it takes both because `kl` selects a
/// region, not a language: DuckDuckGo's HTML endpoint has no language selector,
/// so a fixed English header would fight a non-English region and keep the
/// results English. An unresolved language sends `wt-wt` (worldwide) rather than
/// the `us-en` that was previously forced onto every query.
pub(crate) fn ddg_html_request(query: &str, freshness: bool) -> HttpRequest {
    let lang = detect_request_lang(query);
    let mut headers = vec![
        ("User-Agent".to_string(), BROWSER_USER_AGENT.to_string()),
        (
            "Accept".to_string(),
            "text/html,application/xhtml+xml".to_string(),
        ),
        ("Accept-Language".to_string(), accept_language(lang)),
        ("Referer".to_string(), "https://duckduckgo.com/".to_string()),
    ];
    let mut form = vec![
        ("q".to_string(), query.to_string()),
        ("kl".to_string(), ddg_region(lang).to_string()),
        ("b".to_string(), String::new()),
    ];
    if freshness {
        let df = crate::config::defaults::DDG_FRESHNESS_DF_VALUE;
        form.push(("df".to_string(), df.to_string()));
        headers.push(("Cookie".to_string(), format!("df={df}")));
    }
    HttpRequest {
        method: HttpMethod::Post,
        url: DDG_HTML_ENDPOINT.to_string(),
        headers,
        form,
    }
}

/// Builds the Mojeek `search` GET request. Mojeek takes the query as a `q` URL
/// parameter and returns lightweight result HTML with direct (non-wrapped)
/// destination URLs.
///
/// `freshness` is accepted (matching [`ddg_html_request`]'s signature, since
/// both engines share [`Engine::build`]'s `fn(&str, bool) -> HttpRequest`
/// shape) but is intentionally NOT applied to the query. This used to append
/// a `since:week` operator, but that value is invalid: Mojeek's own
/// documented `since:` operator accepts only `YYYYMMDD`, `day`, `month`, or
/// `year` (no `week`), so every freshness-flagged Mojeek request was sending
/// malformed operator syntax. There is no valid substitute at week
/// granularity either (`day` is narrower than the intent, `month` broader),
/// so rather than guess a value, freshness is left to DuckDuckGo's own
/// `df=w` filter (see [`ddg_html_request`]) and Mojeek always searches the
/// plain query, keeping it in the fusion instead of sending it a query shape
/// it does not support.
///
/// The query's language ([`detect_request_lang`]) is applied through Mojeek's
/// documented `lb` (language) and `lbb` (bias strength) parameters, plus the
/// matching `Accept-Language`. An English or unresolved query sends neither
/// parameter: English is Mojeek's own default, so restating it would only change
/// a request shape that already works.
pub(crate) fn mojeek_request(query: &str, _freshness: bool) -> HttpRequest {
    let lang = detect_request_lang(query);
    // MOJEEK_ENDPOINT is a compile-time-valid absolute URL.
    let mut url = url::Url::parse(MOJEEK_ENDPOINT).expect("static endpoint");
    url.query_pairs_mut().append_pair("q", query);
    if let Some((lb, lbb)) = mojeek_language_bias(lang) {
        url.query_pairs_mut()
            .append_pair("lb", lb)
            .append_pair("lbb", lbb);
    }
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: vec![
            ("User-Agent".to_string(), BROWSER_USER_AGENT.to_string()),
            (
                "Accept".to_string(),
                "text/html,application/xhtml+xml".to_string(),
            ),
            ("Accept-Language".to_string(), accept_language(lang)),
        ],
        form: Vec::new(),
    }
}

/// Classifies a raw SERP response. A bot-challenge body or any non-200 status
/// rotates; a 200 with zero parsed rows is empty (also rotates).
pub(crate) fn classify_serp(status: u16, hit_count: usize, body: &str) -> SerpOutcome {
    let lower = body.to_lowercase();
    if CAPTCHA_MARKERS.iter().any(|m| lower.contains(m)) {
        return SerpOutcome::Blocked;
    }
    if status != 200 {
        return SerpOutcome::Blocked;
    }
    if hit_count == 0 {
        SerpOutcome::Empty
    } else {
        SerpOutcome::Ok
    }
}

/// Parses a DuckDuckGo `html` SERP into result rows: title (`a.result__a`),
/// resolved URL (redirect wrapper decoded), and snippet (`a.result__snippet`,
/// bold markup flattened to text). Rows missing a title or URL are skipped.
pub(crate) fn parse_ddg_html(body: &str) -> Vec<SearchHit> {
    // Selectors are compile-time constants and cannot fail to parse.
    let row = Selector::parse("div.result").expect("static selector");
    let title = Selector::parse("a.result__a").expect("static selector");
    let snippet = Selector::parse("a.result__snippet").expect("static selector");

    let mut hits = Vec::new();
    for result in Html::parse_document(body).select(&row) {
        let Some(anchor) = result.select(&title).next() else {
            continue;
        };
        let title_text = anchor.text().collect::<String>().trim().to_string();
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let url = decode_uddg(href);
        if title_text.is_empty() || url.is_empty() {
            continue;
        }
        if is_ad_result(result.value().attr("class"), &url) {
            continue;
        }
        let snippet_text = result
            .select(&snippet)
            .next()
            .map(|s| s.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        hits.push(SearchHit {
            title: title_text,
            url,
            snippet: snippet_text,
        });
    }
    hits
}

/// Whether a result row is a sponsored ad rather than an organic result:
/// DuckDuckGo marks ad rows with a `result--ad` class and points them at a
/// `duckduckgo.com/y.js` ad-redirect URL. Either signal drops the row so ads
/// never reach the fetch or writer stages.
fn is_ad_result(class_attr: Option<&str>, url: &str) -> bool {
    class_attr.is_some_and(|c| c.contains("result--ad")) || url.contains("duckduckgo.com/y.js")
}

/// Parses a Mojeek SERP into result rows. Mojeek lists organic results as
/// `ul.results-standard > li`, each with its title and destination in
/// `h2 a.title` (a direct absolute URL, no redirect wrapper) and its snippet in
/// `p.s` (bold markup flattened to text). Rows without a title anchor or a
/// non-http(s) href are skipped, so the "see more results" sub-links and any
/// malformed rows do not leak through.
pub(crate) fn parse_mojeek_html(body: &str) -> Vec<SearchHit> {
    // Selectors are compile-time constants and cannot fail to parse.
    let row = Selector::parse("ul.results-standard li").expect("static selector");
    let title = Selector::parse("h2 a.title").expect("static selector");
    let snippet = Selector::parse("p.s").expect("static selector");

    let mut hits = Vec::new();
    for result in Html::parse_document(body).select(&row) {
        let Some(anchor) = result.select(&title).next() else {
            continue;
        };
        let title_text = anchor.text().collect::<String>().trim().to_string();
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let url = href.trim().to_string();
        // Mojeek serves absolute destination URLs; anything else is a UI link.
        if title_text.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
            continue;
        }
        let snippet_text = result
            .select(&snippet)
            .next()
            .map(|s| s.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        hits.push(SearchHit {
            title: title_text,
            url,
            snippet: snippet_text,
        });
    }
    hits
}

/// Resolves a SERP href to an absolute URL, decoding DuckDuckGo's
/// `/l/?uddg=<encoded>` redirect wrapper and promoting protocol-relative and
/// site-relative hrefs to `https`.
pub(crate) fn decode_uddg(href: &str) -> String {
    let absolute = if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if let Some(rest) = href.strip_prefix("//") {
        format!("https://{rest}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href.to_string()
    };
    uddg_target(&absolute).unwrap_or(absolute)
}

/// Extracts the real destination from a DuckDuckGo `/l/?uddg=<encoded>`
/// redirect URL, or `None` when `absolute` is not such a wrapper.
fn uddg_target(absolute: &str) -> Option<String> {
    let parsed = url::Url::parse(absolute).ok()?;
    if parsed.path() != "/l/" {
        return None;
    }
    parsed
        .query_pairs()
        .find(|(key, _)| key == "uddg")
        .map(|(_, real)| real.into_owned())
}

/// Deduplicates by URL and caps total results and per-domain results,
/// preserving first-seen order.
///
/// Dedup keys on [`super::canonical_url_key`], not the raw URL string, so a
/// trailing-slash or http/https variant of an already-kept URL is dropped
/// here rather than surviving as a second, distinct-looking entry that then
/// fits inside `max_per_domain` right alongside the original (the domain cap
/// is deliberately allowed to admit more than one DISTINCT page per domain;
/// it must not admit the same page twice under two spellings).
pub(crate) fn dedupe_and_cap(
    hits: Vec<SearchHit>,
    max_total: usize,
    max_per_domain: usize,
) -> Vec<SearchHit> {
    let mut seen_urls = std::collections::HashSet::new();
    let mut per_domain: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for hit in hits {
        if out.len() >= max_total {
            break;
        }
        if !seen_urls.insert(super::canonical_url_key(&hit.url)) {
            continue;
        }
        let count = per_domain.entry(super::domain_of(&hit.url)).or_insert(0);
        if *count >= max_per_domain {
            continue;
        }
        *count += 1;
        out.push(hit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::transport::{FakeHttpTransport, HttpResponse};

    const DDG_HTML_FIXTURE: &str = r#"
      <div class="result results_links web-result">
        <h2 class="result__title">
          <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&amp;rut=z">Example A</a>
        </h2>
        <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa">Snippet <b>A</b> text</a>
      </div>
      <div class="result web-result">
        <h2 class="result__title">
          <a class="result__a" href="https://example.org/b">Example B</a>
        </h2>
        <a class="result__snippet">Snippet B</a>
      </div>
    "#;

    fn hit(url: &str) -> SearchHit {
        SearchHit {
            title: "t".into(),
            url: url.into(),
            snippet: "s".into(),
        }
    }

    /// A fresh, empty SERP cache for the racing tests that do not exercise
    /// caching: generous TTL and caps, so every lookup misses and every engine
    /// races exactly as it did before caching was added.
    fn empty_web_cache() -> WebCache {
        WebCache::new(
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(600),
            64,
            128,
        )
    }

    // ── decode_uddg ─────────────────────────────────────────────────────────

    #[test]
    fn decode_uddg_unwraps_redirect() {
        assert_eq!(
            decode_uddg("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&rut=z"),
            "https://example.com/a"
        );
    }

    #[test]
    fn decode_uddg_passes_direct_url_through() {
        assert_eq!(
            decode_uddg("https://example.org/b"),
            "https://example.org/b"
        );
    }

    #[test]
    fn decode_uddg_promotes_protocol_relative_non_wrapper() {
        assert_eq!(decode_uddg("//cdn.example/x"), "https://cdn.example/x");
    }

    #[test]
    fn decode_uddg_unwraps_site_relative_redirect() {
        assert_eq!(
            decode_uddg("/l/?uddg=https%3A%2F%2Fx.example%2Fy"),
            "https://x.example/y"
        );
    }

    #[test]
    fn decode_uddg_returns_unusable_href_as_is() {
        // Not http(s), not protocol- or site-relative: passed through verbatim.
        assert_eq!(decode_uddg("mailto:a@b"), "mailto:a@b");
    }

    // ── parse_ddg_html ──────────────────────────────────────────────────────

    #[test]
    fn parse_html_extracts_title_url_snippet() {
        let hits = parse_ddg_html(DDG_HTML_FIXTURE);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Example A");
        assert_eq!(hits[0].url, "https://example.com/a");
        assert_eq!(hits[0].snippet, "Snippet A text");
        assert_eq!(hits[1].url, "https://example.org/b");
    }

    #[test]
    fn parse_html_empty_on_junk() {
        assert!(parse_ddg_html("<html><body>no results</body></html>").is_empty());
    }

    #[test]
    fn parse_html_skips_ad_rows() {
        // Sponsored rows carry a `result--ad` class and/or a `y.js` ad-redirect
        // URL; neither must reach the writer.
        let body = r#"
          <div class="result result--ad">
            <a class="result__a" href="https://duckduckgo.com/y.js?ad_domain=spam.example">Sponsored</a>
            <a class="result__snippet">buy now</a>
          </div>
          <div class="result">
            <a class="result__a" href="https://real.example/">Real</a>
            <a class="result__snippet">genuine</a>
          </div>
        "#;
        let hits = parse_ddg_html(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://real.example/");
    }

    #[test]
    fn is_ad_result_flags_ad_class_and_yjs_only() {
        assert!(is_ad_result(Some("result result--ad"), "https://real/"));
        assert!(is_ad_result(
            None,
            "https://duckduckgo.com/y.js?ad_domain=x"
        ));
        assert!(!is_ad_result(Some("result web-result"), "https://real/"));
        assert!(!is_ad_result(None, "https://real/"));
    }

    #[test]
    fn parse_html_skips_malformed_rows() {
        // Row with no title anchor, a title with no href, and an empty-text
        // title are all skipped; only the well-formed row survives.
        let body = r#"
          <div class="result"><span>no anchor here</span></div>
          <div class="result"><a class="result__a">missing href</a></div>
          <div class="result"><a class="result__a" href="https://x/">   </a></div>
          <div class="result"><a class="result__a" href="https://good/">Good</a></div>
        "#;
        let hits = parse_ddg_html(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://good/");
    }

    // ── classify_serp ───────────────────────────────────────────────────────

    #[test]
    fn classify_ok_when_200_with_hits() {
        assert_eq!(
            classify_serp(200, 3, "<html>results</html>"),
            SerpOutcome::Ok
        );
    }

    #[test]
    fn classify_empty_when_200_no_hits() {
        assert_eq!(classify_serp(200, 0, "<html></html>"), SerpOutcome::Empty);
    }

    #[test]
    fn classify_blocked_on_challenge_body() {
        assert_eq!(
            classify_serp(200, 5, "<div class=\"anomaly-modal\">challenge-form</div>"),
            SerpOutcome::Blocked
        );
    }

    #[test]
    fn classify_blocked_on_non_200() {
        for status in [202u16, 403, 429, 500] {
            assert_eq!(classify_serp(status, 0, ""), SerpOutcome::Blocked);
        }
    }

    #[test]
    fn classify_blocked_on_mojeek_altcha_challenge() {
        // Live-captured Mojeek anti-bot response (200 status, zero parseable
        // result rows, an Altcha proof-of-work challenge in place of results).
        // Before CAPTCHA_MARKERS recognized "captcha-wrap"/"altcha-widget" this
        // fell through to SerpOutcome::Empty, so the engine was never marked
        // cooling and got re-hammered on every subsequent query -- the root
        // cause of "engine=mojeek empty" on 7/7 live-smoke queries.
        assert_eq!(
            classify_serp(200, 0, MOJEEK_CAPTCHA_FIXTURE),
            SerpOutcome::Blocked
        );
    }

    // ── dedupe_and_cap ──────────────────────────────────────────────────────

    #[test]
    fn dedupe_removes_duplicate_urls() {
        let hits = vec![
            hit("https://a.com/1"),
            hit("https://a.com/1"),
            hit("https://b.com/2"),
        ];
        let out = dedupe_and_cap(hits, 10, 5);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedupe_caps_per_domain() {
        let hits = vec![
            hit("https://a.com/1"),
            hit("https://a.com/2"),
            hit("https://a.com/3"),
            hit("https://b.com/1"),
        ];
        let out = dedupe_and_cap(hits, 10, 2);
        // a.com capped at 2, b.com 1 = 3 total
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn dedupe_caps_total() {
        let hits = (0..20)
            .map(|i| hit(&format!("https://d{i}.com/")))
            .collect();
        assert_eq!(dedupe_and_cap(hits, 10, 5).len(), 10);
    }

    #[test]
    fn dedupe_collapses_trailing_slash_and_scheme_variants() {
        // The live bug: DuckDuckGo and Mojeek rendered the same binance.com
        // page as two different URL strings (trailing slash, http vs https).
        // Both fit inside the per-domain cap of 2 and both passed the old
        // exact-string check, so the identical page occupied two of the ten
        // fused slots. Canonical-key dedup must collapse them to one.
        let hits = vec![
            hit("https://www.binance.com/en/price/bitcoin"),
            hit("http://www.binance.com/en/price/bitcoin/"),
            hit("https://other.example/"),
        ];
        let out = dedupe_and_cap(hits, 10, 5);
        assert_eq!(out.len(), 2);
        // The first-seen variant's exact URL text is kept verbatim; dedup
        // never rewrites the URL that gets fetched.
        assert_eq!(out[0].url, "https://www.binance.com/en/price/bitcoin");
    }

    #[test]
    fn dedupe_still_admits_two_distinct_pages_on_the_same_domain() {
        // The per-domain cap intentionally allows more than one DISTINCT page
        // per domain; canonicalization must not collapse genuinely different
        // paths just because they share a host.
        let hits = vec![hit("https://example.com/a"), hit("https://example.com/b")];
        assert_eq!(dedupe_and_cap(hits, 10, 5).len(), 2);
    }

    // ── parse_mojeek_html ───────────────────────────────────────────────────

    // Mirrors real Mojeek markup: organic results in `ul.results-standard > li`
    // with the title/URL in `h2 a.title` (direct href) and snippet in `p.s`. The
    // third row has no title anchor (a bare "more" link) and must be skipped.
    const MOJEEK_HTML_FIXTURE: &str = r#"
      <ul class="results-standard">
        <li class="r1">
          <a href="https://rust-lang.org/tools/install/" class="ob"><span class="url">rust-lang.org</span></a>
          <h2><a class="title" href="https://rust-lang.org/tools/install/">Install Rust</a></h2>
          <p class="s">Run rustc --<strong>version</strong> to check.</p>
          <p class="more"><a href="/search?q=more">See more</a></p>
        </li>
        <li class="r2">
          <h2><a class="title" href="https://blog.rust-lang.org/">Rust Blog</a></h2>
          <p class="s">Latest release news.</p>
        </li>
        <li class="r3">
          <p class="more"><a href="/search?q=x">no title here</a></p>
        </li>
      </ul>
    "#;

    /// Trimmed reproduction of a live-captured Mojeek Altcha anti-bot
    /// challenge page (200 status, `<title>Captcha</title>`, a
    /// `captcha-wrap` box hosting an `altcha-widget`, no `ul.results-standard`
    /// anywhere). See `classify_blocked_on_mojeek_altcha_challenge` and
    /// `parse_mojeek_returns_no_rows_for_altcha_challenge_page`.
    const MOJEEK_CAPTCHA_FIXTURE: &str = r#"
      <!DOCTYPE html>
      <html lang="en">
      <head><title>Captcha</title></head>
      <body data-theme="light" class="home">
      <div class="captcha-wrap">
        <div class="captcha-box">
          <h1>Verification required</h1>
          <p>Please complete the challenge to continue.</p>
          <form id="altcha-form" method="post" action="/captcha/verify">
            <altcha-widget id="altcha-widget" challenge="/captcha/challenge" name="altcha" theme="default"></altcha-widget>
          </form>
        </div>
      </div>
      </body>
      </html>
    "#;

    #[test]
    fn parse_mojeek_extracts_title_url_snippet() {
        let hits = parse_mojeek_html(MOJEEK_HTML_FIXTURE);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Install Rust");
        assert_eq!(hits[0].url, "https://rust-lang.org/tools/install/");
        assert_eq!(hits[0].snippet, "Run rustc --version to check.");
        assert_eq!(hits[1].url, "https://blog.rust-lang.org/");
    }

    #[test]
    fn parse_mojeek_skips_non_http_missing_href_and_missing_title() {
        // A relative href, a title anchor with no href, and a titleless row are
        // all dropped; only the well-formed absolute-URL row survives.
        let body = r#"
          <ul class="results-standard">
            <li><h2><a class="title" href="/settings">Relative</a></h2></li>
            <li><h2><a class="title">No href</a></h2></li>
            <li><h2><a class="title" href="https://ok.example/">Ok</a></h2></li>
            <li><p class="s">no title</p></li>
          </ul>
        "#;
        let hits = parse_mojeek_html(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://ok.example/");
        assert!(hits[0].snippet.is_empty());
    }

    #[test]
    fn parse_mojeek_empty_on_junk() {
        assert!(parse_mojeek_html("<html><body>nothing</body></html>").is_empty());
    }

    #[test]
    fn parse_mojeek_returns_no_rows_for_altcha_challenge_page() {
        // The parser was never the bug: a captcha page correctly parses to
        // zero rows (no `ul.results-standard`). The bug was in classification
        // treating that zero as "empty query" instead of "blocked" -- see
        // `classify_blocked_on_mojeek_altcha_challenge`.
        assert!(parse_mojeek_html(MOJEEK_CAPTCHA_FIXTURE).is_empty());
    }

    // ── request builders ────────────────────────────────────────────────────

    #[test]
    fn ddg_request_is_post_with_query_form() {
        let req = ddg_html_request("rust bm25", false);
        assert_eq!(req.method, HttpMethod::Post);
        assert_eq!(req.url, DDG_HTML_ENDPOINT);
        assert!(req.form.iter().any(|(k, v)| k == "q" && v == "rust bm25"));
        // SERP deliberately keeps a browser UA (honest bot UA is blocked on
        // DDG /html). Must not be the product THUKI_USER_AGENT used on API verticals.
        assert!(req.headers.iter().any(|(k, v)| {
            k == "User-Agent" && v == BROWSER_USER_AGENT && !v.starts_with("Thuki/")
        }));
    }

    #[test]
    fn mojeek_request_is_get_with_query_param() {
        let req = mojeek_request("rust version", false);
        assert_eq!(req.method, HttpMethod::Get);
        assert!(req.url.starts_with(MOJEEK_ENDPOINT));
        assert!(req.url.contains("q=rust+version") || req.url.contains("q=rust%20version"));
        assert!(req.form.is_empty());
        assert!(req.headers.iter().any(|(k, v)| {
            k == "User-Agent" && v == BROWSER_USER_AGENT && !v.starts_with("Thuki/")
        }));
    }

    // ── language parity ─────────────────────────────────────────────────────

    #[test]
    fn ddg_request_carries_the_region_and_header_of_the_query_language() {
        // A script-detectable query, so the assertion does not depend on the
        // machine's locale. Vietnamese needs BOTH: `vn-en` is a region, and
        // DuckDuckGo's HTML endpoint has no language selector, so only the
        // header can ask for Vietnamese-language results.
        let req = ddg_html_request("thời tiết Hà Nội hôm nay", false);
        assert!(req.form.iter().any(|(k, v)| k == "kl" && v == "vn-en"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Accept-Language" && v == "vi,en;q=0.5"));

        let req = ddg_html_request("東京の天気は", false);
        assert!(req.form.iter().any(|(k, v)| k == "kl" && v == "jp-jp"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Accept-Language" && v == "ja,en;q=0.5"));
    }

    #[test]
    fn mojeek_request_carries_the_language_bias_of_the_query_language() {
        let req = mojeek_request("thời tiết Hà Nội hôm nay", false);
        assert!(req.url.contains("lb=vi"));
        assert!(req.url.contains("lbb=100"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Accept-Language" && v == "vi,en;q=0.5"));
    }

    // ── freshness operators ──────────────────────────────────────────────────

    #[test]
    fn ddg_request_carries_no_freshness_operator_by_default() {
        let req = ddg_html_request("rust bm25", false);
        assert!(!req.form.iter().any(|(k, _)| k == "df"));
        assert!(!req.headers.iter().any(|(k, _)| k == "Cookie"));
    }

    #[test]
    fn ddg_request_adds_df_form_field_and_cookie_when_fresh() {
        let req = ddg_html_request("rust bm25", true);
        assert!(req.form.iter().any(|(k, v)| k == "df" && v == "w"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Cookie" && v == "df=w"));
        // The query itself is untouched; only the date-filter dimension changes.
        assert!(req.form.iter().any(|(k, v)| k == "q" && v == "rust bm25"));
    }

    #[test]
    fn mojeek_request_carries_no_freshness_operator_by_default() {
        let req = mojeek_request("rust version", false);
        assert!(!req.url.contains("since"));
    }

    #[test]
    fn mojeek_request_ignores_freshness_flag() {
        // Mojeek's `since:` operator does not support week granularity (see
        // `mojeek_request`'s rustdoc), so freshness=true must produce the exact
        // same request as freshness=false: no operator, no query mutation.
        let fresh = mojeek_request("rust version", true);
        let plain = mojeek_request("rust version", false);
        assert_eq!(fresh.url, plain.url);
        assert!(!fresh.url.contains("since"));
    }

    // ── rrf_fuse ────────────────────────────────────────────────────────────

    #[test]
    fn rrf_fuse_ranks_cross_engine_agreement_over_single_rank1() {
        // list A: y is rank 1, x is rank 3. list B: x is rank 3.
        // With RRF_K = 60:
        //   score(x) = 1/(60+3) + 1/(60+3) = 2/63 ≈ 0.031746
        //   score(y) = 1/(60+1)           = 1/61 ≈ 0.016393
        // So x (agreed on by both engines at mid rank) outranks y (rank 1 on one
        // engine only), which is the whole reason to fuse.
        let x = hit("https://x.example/");
        let y = hit("https://y.example/");
        let a1 = hit("https://a1.example/");
        let b1 = hit("https://b1.example/");
        let b2 = hit("https://b2.example/");
        let list_a = vec![y.clone(), a1, x.clone()];
        let list_b = vec![b1, b2, x.clone()];
        let fused = rrf_fuse(&[list_a, list_b]);
        assert_eq!(fused[0].url, "https://x.example/");
        // y still appears, just below x.
        assert!(fused.iter().any(|h| h.url == "https://y.example/"));
    }

    #[test]
    fn rrf_fuse_single_list_preserves_order() {
        // One live engine degrades to that engine's own ordering: score is
        // strictly monotonic in rank, so the input order is preserved verbatim.
        let list = vec![
            hit("https://one.example/"),
            hit("https://two.example/"),
            hit("https://three.example/"),
        ];
        let fused = rrf_fuse(std::slice::from_ref(&list));
        assert_eq!(
            fused.iter().map(|h| h.url.as_str()).collect::<Vec<_>>(),
            vec![
                "https://one.example/",
                "https://two.example/",
                "https://three.example/",
            ]
        );
    }

    #[test]
    fn rrf_fuse_counts_intra_list_duplicate_once() {
        // A URL repeated inside one engine's list contributes only once, at its
        // first (best) rank, so the "sum over engines" semantics is not inflated
        // by an engine listing the same URL twice.
        let list = vec![
            hit("https://a.example/"),
            hit("https://b.example/"),
            hit("https://a.example/"),
        ];
        let fused = rrf_fuse(std::slice::from_ref(&list));
        // Deduped to two URLs, first-seen order (a before b), a scored from
        // rank 1 only.
        assert_eq!(
            fused.iter().map(|h| h.url.as_str()).collect::<Vec<_>>(),
            vec!["https://a.example/", "https://b.example/"]
        );
    }

    #[test]
    fn rrf_fuse_counts_canonical_duplicate_within_list_once() {
        // Same as the exact-match case above, but the repeat is a
        // trailing-slash variant of the first occurrence: still one URL, one
        // contribution, at the first (best) rank.
        let list = vec![
            hit("https://a.example/"),
            hit("https://b.example/"),
            hit("https://a.example"),
        ];
        let fused = rrf_fuse(std::slice::from_ref(&list));
        assert_eq!(
            fused.iter().map(|h| h.url.as_str()).collect::<Vec<_>>(),
            vec!["https://a.example/", "https://b.example/"]
        );
    }

    #[test]
    fn rrf_fuse_merges_cross_engine_url_variants_as_agreement() {
        // The live bug's fusion-level counterpart: DDG renders a page with a
        // trailing slash, Mojeek renders the same page without one (and over
        // http instead of https). Canonical-key scoring must treat this as
        // genuine two-engine agreement (one merged entry, boosted score) not
        // two separate weaker single-engine hits.
        let ddg = vec![
            hit("https://other-ddg/"),
            hit("https://www.binance.com/en/price/bitcoin/"),
        ];
        let mojeek = vec![hit("http://www.binance.com/en/price/bitcoin")];
        let fused = rrf_fuse(&[ddg, mojeek]);
        let urls: Vec<&str> = fused.iter().map(|h| h.url.as_str()).collect();
        // One entry, not two, for the shared page.
        assert_eq!(
            urls.iter()
                .filter(|u| u.contains("binance.com/en/price/bitcoin"))
                .count(),
            1
        );
        // And cross-engine agreement at rank 2 beats a single-engine rank 1:
        // the merged binance entry outranks the DDG-only "other-ddg" hit.
        assert_eq!(urls[0], "https://www.binance.com/en/price/bitcoin/");
    }

    #[test]
    fn rrf_fuse_empty_input_is_empty() {
        assert!(rrf_fuse(&[]).is_empty());
        assert!(rrf_fuse(&[Vec::new()]).is_empty());
    }

    #[test]
    fn rrf_fuse_is_deterministic() {
        // Same inputs -> same output order, every time.
        let list_a = vec![hit("https://p.example/"), hit("https://q.example/")];
        let list_b = vec![hit("https://q.example/"), hit("https://r.example/")];
        let first = rrf_fuse(&[list_a.clone(), list_b.clone()]);
        let second = rrf_fuse(&[list_a, list_b]);
        assert_eq!(
            first.iter().map(|h| &h.url).collect::<Vec<_>>(),
            second.iter().map(|h| &h.url).collect::<Vec<_>>()
        );
        // q is on both engines (ranks 2 and 1) -> highest score -> first.
        assert_eq!(first[0].url, "https://q.example/");
    }

    // ── credibility-aware fusion ────────────────────────────────────────────

    #[test]
    fn drop_incredible_removes_drop_class_and_keeps_others() {
        // now8news.com is a real drop-class entry in the embedded list; the other
        // host is neutral and survives. Covers both filter branches.
        let list = vec![
            hit("https://now8news.com/hoax"),
            hit("https://example.com/ok"),
        ];
        let kept = drop_incredible(list);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].url, "https://example.com/ok");
    }

    #[test]
    fn rrf_fuse_penalizes_below_neutral_dual_engine() {
        // A penalized rank-1 single-engine hit must rank below a neutral page that
        // two engines agree on at rank 10:
        //   spam:  1 / (60 + 1 + 40) = 1/101 ≈ 0.009901
        //   legit: 1/(60+10) + 1/(60+10) = 2/70 ≈ 0.028571
        // 0.0099 < 0.0286, so the soft penalty does not erase the hit, it just
        // seats a lone spam page below cross-engine agreement.
        let classify = |url: &str| {
            if url == "https://spam/" {
                DomainClass::Penalize
            } else {
                DomainClass::Neutral
            }
        };
        let mut list_a = vec![hit("https://spam/")];
        for i in 1..=8 {
            list_a.push(hit(&format!("https://a{i}/")));
        }
        list_a.push(hit("https://legit/")); // rank 10 in list A
        let mut list_b = Vec::new();
        for i in 1..=9 {
            list_b.push(hit(&format!("https://b{i}/")));
        }
        list_b.push(hit("https://legit/")); // rank 10 in list B
        let fused = rrf_fuse_classified(&[list_a, list_b], &classify);
        let pos = |u: &str| fused.iter().position(|h| h.url == u).expect("present");
        assert!(pos("https://legit/") < pos("https://spam/"));
    }

    #[test]
    fn rrf_fuse_boost_beats_same_rank_neutral_but_not_dual_engine() {
        // A boosted hit is scored as rank 1 in its list, so it outranks a neutral
        // hit sitting at the same native rank, yet the tuning-free ceiling holds:
        //   boost@5    = 1 / (60 + 1) = 0.016393
        //   neutral5@5 = 1 / (60 + 5) = 0.015385   -> boost > neutral5
        //   dual@10 x2 = 2 / (60 + 10) = 0.028571   -> dual > boost
        // so a boosted-but-thin page can never beat genuine cross-engine agreement.
        let classify = |url: &str| {
            if url == "https://boost/" {
                DomainClass::Boost
            } else {
                DomainClass::Neutral
            }
        };
        let list_a = vec![
            hit("https://a1/"),
            hit("https://a2/"),
            hit("https://a3/"),
            hit("https://a4/"),
            hit("https://boost/"), // rank 5
            hit("https://a6/"),
            hit("https://a7/"),
            hit("https://a8/"),
            hit("https://a9/"),
            hit("https://dual/"), // rank 10
        ];
        let list_b = vec![
            hit("https://b1/"),
            hit("https://b2/"),
            hit("https://b3/"),
            hit("https://b4/"),
            hit("https://neutral5/"), // rank 5
            hit("https://b6/"),
            hit("https://b7/"),
            hit("https://b8/"),
            hit("https://b9/"),
            hit("https://dual/"), // rank 10
        ];
        let fused = rrf_fuse_classified(&[list_a, list_b], &classify);
        let pos = |u: &str| fused.iter().position(|h| h.url == u).expect("present");
        assert!(pos("https://boost/") < pos("https://neutral5/"));
        assert!(pos("https://dual/") < pos("https://boost/"));
    }

    #[test]
    fn rrf_fuse_classified_is_deterministic() {
        // Same inputs and classifier -> identical output order, every time.
        let classify = |url: &str| {
            if url == "https://b/" {
                DomainClass::Boost
            } else {
                DomainClass::Neutral
            }
        };
        let list = vec![hit("https://a/"), hit("https://b/")];
        let first = rrf_fuse_classified(std::slice::from_ref(&list), &classify);
        let second = rrf_fuse_classified(std::slice::from_ref(&list), &classify);
        assert_eq!(
            first.iter().map(|h| &h.url).collect::<Vec<_>>(),
            second.iter().map(|h| &h.url).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn web_search_drops_incredible_domain_from_results() {
        // An end-to-end check that a drop-class hit vanishes from the fused output.
        // DuckDuckGo is forced cooling so only Mojeek runs, keeping the result
        // deterministic; its list carries one hoax domain and one legitimate one.
        let mojeek_url = mojeek_request("q", false).url;
        let body = r#"
          <ul class="results-standard">
            <li><h2><a class="title" href="https://now8news.com/hoax">Hoax</a></h2></li>
            <li><h2><a class="title" href="https://legit.example/real">Real</a></h2></li>
          </ul>
        "#;
        let transport = FakeHttpTransport::new().with_response(&mojeek_url, ok(&mojeek_url, body));
        let health = EngineHealth::new();
        health.mark_blocked("duckduckgo", 3600);
        let (hits, _stats) =
            web_search(&transport, "q", &health, false, &empty_web_cache(), false).await;
        let urls: Vec<&str> = hits.iter().map(|h| h.url.as_str()).collect();
        assert!(!urls.iter().any(|u| u.contains("now8news")));
        assert!(urls.contains(&"https://legit.example/real"));
    }

    // ── outcome_status ──────────────────────────────────────────────────────

    #[test]
    fn outcome_status_maps_every_variant() {
        assert_eq!(outcome_status(&EngineOutcome::Hits(vec![])), "ok");
        assert_eq!(outcome_status(&EngineOutcome::Blocked), "blocked");
        assert_eq!(outcome_status(&EngineOutcome::Empty), "empty");
        assert_eq!(
            outcome_status(&EngineOutcome::TransportError),
            "transport_error"
        );
    }

    // ── transport_unreachable ───────────────────────────────────────────────

    /// Builds an `EngineStat` with just the status set; `name`/`hit_count` do
    /// not affect [`transport_unreachable`].
    fn stat(status: &str) -> EngineStat {
        EngineStat {
            name: "engine".to_string(),
            status: status.to_string(),
            hit_count: 0,
        }
    }

    #[test]
    fn transport_unreachable_true_when_every_contacted_engine_transport_errored() {
        assert!(transport_unreachable(&[
            stat("transport_error"),
            stat("transport_error"),
        ]));
    }

    #[test]
    fn transport_unreachable_ignores_cooling_engines() {
        // Cooling engines are skipped, not contacted: a lone transport error
        // among them still reads as unreachable.
        assert!(transport_unreachable(&[
            stat("cooling"),
            stat("transport_error"),
        ]));
    }

    #[test]
    fn transport_unreachable_false_when_any_engine_reached_the_web() {
        // One HTTP response (even an empty or blocked one) proves the web was
        // reached, so the miss is "found nothing", not "unreachable".
        assert!(!transport_unreachable(&[
            stat("transport_error"),
            stat("empty"),
        ]));
        assert!(!transport_unreachable(&[stat("blocked")]));
        assert!(!transport_unreachable(&[stat("cache_hit")]));
        assert!(!transport_unreachable(&[stat("ok")]));
    }

    #[test]
    fn transport_unreachable_false_when_no_engine_was_contacted() {
        // No engines at all, and all-cooling, both lack transport-failure
        // evidence: never blame the connection.
        assert!(!transport_unreachable(&[]));
        assert!(!transport_unreachable(&[stat("cooling"), stat("cooling")]));
    }

    // ── web_search racing + fusion over the fake transport ──────────────────

    fn ok(url: &str, body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            final_url: url.into(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[tokio::test]
    async fn web_search_forwards_freshness_to_the_engine_request() {
        // freshness=true must reach the DDG request builder: the recorded call
        // carries the df=w form field and the df=w cookie.
        let transport = FakeHttpTransport::new()
            .with_response(DDG_HTML_ENDPOINT, ok(DDG_HTML_ENDPOINT, DDG_HTML_FIXTURE));
        let _ = web_search(
            &transport,
            "q",
            &EngineHealth::new(),
            true,
            &empty_web_cache(),
            false,
        )
        .await;
        let call = transport
            .calls()
            .into_iter()
            .find(|c| c.url == DDG_HTML_ENDPOINT)
            .expect("ddg request recorded");
        assert!(call.form.iter().any(|(k, v)| k == "df" && v == "w"));
        assert!(call
            .headers
            .iter()
            .any(|(k, v)| k == "Cookie" && v == "df=w"));
    }

    #[tokio::test]
    async fn web_search_races_and_fuses_both_engines() {
        // Both engines answer Ok with disjoint URLs: the fused list carries hits
        // from both, and each engine got exactly one request (burst-safe).
        let mojeek_url = mojeek_request("q", false).url;
        let transport = FakeHttpTransport::new()
            .with_response(DDG_HTML_ENDPOINT, ok(DDG_HTML_ENDPOINT, DDG_HTML_FIXTURE))
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let (hits, stats) = web_search(
            &transport,
            "q",
            &EngineHealth::new(),
            false,
            &empty_web_cache(),
            false,
        )
        .await;
        let urls: Vec<&str> = hits.iter().map(|h| h.url.as_str()).collect();
        // A DuckDuckGo result and a Mojeek result both survive the fusion.
        assert!(urls.contains(&"https://example.com/a"));
        assert!(urls.contains(&"https://rust-lang.org/tools/install/"));
        // Exactly one request per engine.
        let calls = transport.calls();
        assert_eq!(
            calls.iter().filter(|c| c.url == DDG_HTML_ENDPOINT).count(),
            1
        );
        assert_eq!(calls.iter().filter(|c| c.url == mojeek_url).count(), 1);
        // Both engines' per-query outcome is surfaced for the trace: "ok"
        // with their real hit counts.
        assert_eq!(
            stats,
            vec![
                EngineStat {
                    name: "duckduckgo".into(),
                    status: "ok".into(),
                    hit_count: 2,
                },
                EngineStat {
                    name: "mojeek".into(),
                    status: "ok".into(),
                    hit_count: 2,
                },
            ]
        );
    }

    #[tokio::test]
    async fn web_search_cools_blocked_engine_and_still_fuses_the_other() {
        // The live failure mode: DuckDuckGo hard-blocks (HTTP 202 challenge). It
        // is marked cooling, and Mojeek's results still come back fused.
        let mojeek_url = mojeek_request("q", false).url;
        let health = EngineHealth::new();
        let transport = FakeHttpTransport::new()
            .with_response(
                DDG_HTML_ENDPOINT,
                HttpResponse {
                    status: 202,
                    final_url: DDG_HTML_ENDPOINT.into(),
                    body: b"<div class=\"anomaly-modal\">challenge-form</div>".to_vec(),
                },
            )
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let (hits, stats) =
            web_search(&transport, "q", &health, false, &empty_web_cache(), false).await;
        // Blocked engine contributes no list; Mojeek's two hits survive.
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://rust-lang.org/tools/install/");
        // And the block was recorded for the next query.
        assert!(health.is_cooling("duckduckgo"));
        // The trace-facing stats distinguish the blocked engine (zero hits,
        // status "blocked") from the one that actually answered.
        assert_eq!(
            stats,
            vec![
                EngineStat {
                    name: "duckduckgo".into(),
                    status: "blocked".into(),
                    hit_count: 0,
                },
                EngineStat {
                    name: "mojeek".into(),
                    status: "ok".into(),
                    hit_count: 2,
                },
            ]
        );
    }

    #[tokio::test]
    async fn web_search_empty_engine_contributes_nothing_and_other_fuses() {
        // One engine returns 200 with zero parsed rows (Empty): it adds no list
        // and is NOT cooled (an empty page is a bad query, not a ban), while the
        // other engine's results still come back fused.
        let mojeek_url = mojeek_request("q", false).url;
        let health = EngineHealth::new();
        let transport = FakeHttpTransport::new()
            .with_response(
                DDG_HTML_ENDPOINT,
                ok(DDG_HTML_ENDPOINT, "<html><body>no results</body></html>"),
            )
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let (hits, stats) =
            web_search(&transport, "q", &health, false, &empty_web_cache(), false).await;
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://rust-lang.org/tools/install/");
        // Empty is not a block: DuckDuckGo stays available for the next query.
        assert!(!health.is_cooling("duckduckgo"));
        // The trace-facing stats show "empty" (not "blocked") with zero hits.
        assert_eq!(
            stats,
            vec![
                EngineStat {
                    name: "duckduckgo".into(),
                    status: "empty".into(),
                    hit_count: 0,
                },
                EngineStat {
                    name: "mojeek".into(),
                    status: "ok".into(),
                    hit_count: 2,
                },
            ]
        );
    }

    #[tokio::test]
    async fn web_search_empty_when_all_engines_blocked() {
        let mojeek_url = mojeek_request("q", false).url;
        let blocked = |url: &str| HttpResponse {
            status: 429,
            final_url: url.into(),
            body: b"rate limited".to_vec(),
        };
        let transport = FakeHttpTransport::new()
            .with_response(DDG_HTML_ENDPOINT, blocked(DDG_HTML_ENDPOINT))
            .with_response(&mojeek_url, blocked(&mojeek_url));
        assert!(web_search(
            &transport,
            "q",
            &EngineHealth::new(),
            false,
            &empty_web_cache(),
            false,
        )
        .await
        .0
        .is_empty());
    }

    #[tokio::test]
    async fn web_search_empty_when_all_engines_transport_error() {
        // No canned responses -> every engine's send errors -> empty.
        let transport = FakeHttpTransport::new();
        let (hits, stats) = web_search(
            &transport,
            "q",
            &EngineHealth::new(),
            false,
            &empty_web_cache(),
            false,
        )
        .await;
        assert!(hits.is_empty());
        assert_eq!(
            stats,
            vec![
                EngineStat {
                    name: "duckduckgo".into(),
                    status: "transport_error".into(),
                    hit_count: 0,
                },
                EngineStat {
                    name: "mojeek".into(),
                    status: "transport_error".into(),
                    hit_count: 0,
                },
            ]
        );
    }

    // ── EngineHealth cooldown ───────────────────────────────────────────────

    #[test]
    fn health_marks_and_reports_cooling_until_expiry() {
        let health = EngineHealth::new();
        assert!(!health.is_cooling("duckduckgo"));
        health.mark_blocked("duckduckgo", 3600);
        assert!(health.is_cooling("duckduckgo"));
        // A zero-second cooldown is expired on the next read and pruned.
        health.mark_blocked("mojeek", 0);
        assert!(!health.is_cooling("mojeek"));
        assert!(!health.is_cooling("mojeek"));
    }

    #[test]
    fn health_default_is_empty() {
        assert!(!EngineHealth::default().is_cooling("duckduckgo"));
    }

    #[test]
    fn any_engine_available_true_when_nothing_cooling() {
        assert!(any_engine_available(&EngineHealth::new()));
    }

    #[test]
    fn any_engine_available_true_when_one_engine_still_live() {
        // One engine cooling, the other free: escalation still has a target.
        let health = EngineHealth::new();
        health.mark_blocked("duckduckgo", 3600);
        assert!(any_engine_available(&health));
    }

    #[test]
    fn any_engine_available_false_when_every_engine_cooling() {
        // All engines cooling: escalation is futile, serve the partial instead.
        let health = EngineHealth::new();
        for engine in ENGINES {
            health.mark_blocked(engine.name, 3600);
        }
        assert!(!any_engine_available(&health));
    }

    #[tokio::test]
    async fn web_search_skips_cooling_engine_entirely() {
        // DuckDuckGo is inside its cooldown: no request goes to it at all, and
        // Mojeek serves the query.
        let health = EngineHealth::new();
        health.mark_blocked("duckduckgo", 3600);
        let mojeek_url = mojeek_request("q", false).url;
        let transport = FakeHttpTransport::new()
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let (hits, stats) =
            web_search(&transport, "q", &health, false, &empty_web_cache(), false).await;
        assert_eq!(hits.len(), 2);
        // Single live engine: fusion is an identity on order, so Mojeek's own
        // ordering is preserved verbatim.
        assert_eq!(hits[0].url, "https://rust-lang.org/tools/install/");
        assert_eq!(hits[1].url, "https://blog.rust-lang.org/");
        assert!(!transport.calls().iter().any(|c| c.url == DDG_HTML_ENDPOINT));
        // The cooling engine is surfaced in the stats too (status "cooling",
        // never requested), not silently dropped from the trace.
        assert_eq!(
            stats,
            vec![
                EngineStat {
                    name: "duckduckgo".into(),
                    status: "cooling".into(),
                    hit_count: 0,
                },
                EngineStat {
                    name: "mojeek".into(),
                    status: "ok".into(),
                    hit_count: 2,
                },
            ]
        );
    }

    #[tokio::test]
    async fn web_search_block_marks_cooldown_for_next_query() {
        // First query: DuckDuckGo answers with a challenge -> marked blocked.
        // Second query: DuckDuckGo is skipped without a request (one DDG POST
        // total across both queries).
        let health = EngineHealth::new();
        let mojeek_url = mojeek_request("q", false).url;
        let transport = FakeHttpTransport::new()
            .with_response(
                DDG_HTML_ENDPOINT,
                HttpResponse {
                    status: 202,
                    final_url: DDG_HTML_ENDPOINT.into(),
                    body: b"<div class=\"anomaly-modal\">challenge-form</div>".to_vec(),
                },
            )
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let cache = empty_web_cache();
        let _ = web_search(&transport, "q", &health, false, &cache, false).await;
        let _ = web_search(&transport, "q", &health, false, &cache, false).await;
        let ddg_posts = transport
            .calls()
            .into_iter()
            .filter(|c| c.url == DDG_HTML_ENDPOINT)
            .count();
        assert_eq!(ddg_posts, 1, "blocked engine must not be re-queried");
        assert!(health.is_cooling("duckduckgo"));
    }

    // ── SERP cache integration ──────────────────────────────────────────────

    #[tokio::test]
    async fn web_search_serp_cache_hit_skips_request_and_still_fuses() {
        // DuckDuckGo's list is already cached for this exact query: it must NOT be
        // requested, its cached list must still join the fusion, and the live
        // (uncached) Mojeek engine must still race and contribute.
        let mojeek_url = mojeek_request("q", false).url;
        let cache = empty_web_cache();
        cache.serp_put("duckduckgo", "q", false, vec![hit("https://cached-ddg/")]);
        // Only Mojeek is canned; DuckDuckGo has no response because it must never
        // be requested.
        let transport = FakeHttpTransport::new()
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let health = EngineHealth::new();
        let (hits, stats) = web_search(&transport, "q", &health, false, &cache, false).await;
        let urls: Vec<&str> = hits.iter().map(|h| h.url.as_str()).collect();
        // Cached DuckDuckGo hit and a live Mojeek hit both survive the fusion.
        assert!(urls.contains(&"https://cached-ddg/"));
        assert!(urls.contains(&"https://rust-lang.org/tools/install/"));
        // Exactly one request total, and none of it to DuckDuckGo.
        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        assert!(!calls.iter().any(|c| c.url == DDG_HTML_ENDPOINT));
        // The cache-served engine is surfaced as "cache_hit" (not "ok"), so a
        // trace can distinguish a live answer from a replayed one.
        assert_eq!(
            stats,
            vec![
                EngineStat {
                    name: "duckduckgo".into(),
                    status: "cache_hit".into(),
                    hit_count: 1,
                },
                EngineStat {
                    name: "mojeek".into(),
                    status: "ok".into(),
                    hit_count: 2,
                },
            ]
        );
    }

    #[tokio::test]
    async fn web_search_blocked_engine_is_not_cached() {
        // A block must never be written to the cache (it must not be replayable as
        // truth). Each call uses a FRESH health so cooldown never masks the cache
        // behaviour: because the block was not cached, the second call re-requests
        // DuckDuckGo (two DDG requests total), rather than serving a cached block.
        let transport = FakeHttpTransport::new().with_response(
            DDG_HTML_ENDPOINT,
            HttpResponse {
                status: 202,
                final_url: DDG_HTML_ENDPOINT.into(),
                body: b"<div class=\"anomaly-modal\">challenge-form</div>".to_vec(),
            },
        );
        let cache = empty_web_cache();
        let _ = web_search(&transport, "q", &EngineHealth::new(), false, &cache, false).await;
        let _ = web_search(&transport, "q", &EngineHealth::new(), false, &cache, false).await;
        let ddg_posts = transport
            .calls()
            .into_iter()
            .filter(|c| c.url == DDG_HTML_ENDPOINT)
            .count();
        assert_eq!(
            ddg_posts, 2,
            "a blocked engine must not be served from cache"
        );
    }

    // ── cache bypass (explicit re-search) ───────────────────────────────────

    #[tokio::test]
    async fn web_search_bypass_cache_requests_engine_despite_warm_serp_cache() {
        // A warm SERP cache entry would normally be served without a request
        // (see `web_search_serp_cache_hit_skips_request_and_still_fuses`).
        // `bypass_cache=true` must skip that read and still issue the request,
        // exactly as an explicit user re-search demands.
        let mojeek_url = mojeek_request("q", false).url;
        let cache = empty_web_cache();
        cache.serp_put("duckduckgo", "q", false, vec![hit("https://stale-ddg/")]);
        let transport = FakeHttpTransport::new()
            .with_response(DDG_HTML_ENDPOINT, ok(DDG_HTML_ENDPOINT, DDG_HTML_FIXTURE))
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let health = EngineHealth::new();
        let (hits, _stats) = web_search(&transport, "q", &health, false, &cache, true).await;
        // The stale cached hit is gone; the freshly fetched DDG fixture hit is
        // there instead.
        let urls: Vec<&str> = hits.iter().map(|h| h.url.as_str()).collect();
        assert!(!urls.contains(&"https://stale-ddg/"));
        assert!(urls.contains(&"https://example.com/a"));
        // Both engines were actually requested: the cache read was skipped.
        let calls = transport.calls();
        assert_eq!(
            calls.iter().filter(|c| c.url == DDG_HTML_ENDPOINT).count(),
            1
        );
        assert_eq!(calls.iter().filter(|c| c.url == mojeek_url).count(), 1);
    }

    #[tokio::test]
    async fn web_search_bypass_cache_replaces_the_stale_entry() {
        // The fresh result from a bypassing call must overwrite the cache, so
        // the very next NON-bypassing call is served the refreshed list rather
        // than the stale one the user just distrusted. Mojeek is forced cooling
        // for the first (bypassing) call so its outcome is deterministic
        // (DuckDuckGo alone), isolating the DuckDuckGo cache-write behaviour.
        let cache = empty_web_cache();
        cache.serp_put("duckduckgo", "q", false, vec![hit("https://stale-ddg/")]);
        let health = EngineHealth::new();
        health.mark_blocked("mojeek", 3600);
        let transport = FakeHttpTransport::new()
            .with_response(DDG_HTML_ENDPOINT, ok(DDG_HTML_ENDPOINT, DDG_HTML_FIXTURE));
        let (bypassed, _stats) = web_search(&transport, "q", &health, false, &cache, true).await;
        assert!(bypassed.iter().any(|h| h.url == "https://example.com/a"));
        // A second, non-bypassing call with a fresh (non-cooling) health
        // registry reads the cache: DuckDuckGo must now be served from the
        // REPLACED (fresh) entry, not the original stale one, with no further
        // DuckDuckGo request.
        let (served, _stats) =
            web_search(&transport, "q", &EngineHealth::new(), false, &cache, false).await;
        assert!(!served.iter().any(|h| h.url == "https://stale-ddg/"));
        assert!(served.iter().any(|h| h.url == "https://example.com/a"));
        // Exactly one DuckDuckGo request total: the first (bypassing) call
        // fetched and wrote through; the second call was served entirely from
        // the refreshed cache entry.
        assert_eq!(
            transport
                .calls()
                .iter()
                .filter(|c| c.url == DDG_HTML_ENDPOINT)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn web_search_bypass_cache_false_keeps_serp_cache_hit_behavior() {
        // bypass_cache=false must behave exactly like the pre-existing
        // cache-hit path: a warm entry is served with zero requests.
        let cache = empty_web_cache();
        cache.serp_put("duckduckgo", "q", false, vec![hit("https://cached-ddg/")]);
        let mojeek_url = mojeek_request("q", false).url;
        let transport = FakeHttpTransport::new()
            .with_response(&mojeek_url, ok(&mojeek_url, MOJEEK_HTML_FIXTURE));
        let health = EngineHealth::new();
        let (hits, _stats) = web_search(&transport, "q", &health, false, &cache, false).await;
        let urls: Vec<&str> = hits.iter().map(|h| h.url.as_str()).collect();
        assert!(urls.contains(&"https://cached-ddg/"));
        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        assert!(!calls.iter().any(|c| c.url == DDG_HTML_ENDPOINT));
    }

    /// Builds a DuckDuckGo SERP body with `n` distinct-domain organic rows.
    fn ddg_body_with_rows(n: usize) -> String {
        let rows: String = (0..n)
            .map(|i| {
                format!(
                    r#"<div class="result"><h2><a class="result__a" href="https://ex{i}.example/p">T{i}</a></h2></div>"#
                )
            })
            .collect();
        format!("<div>{rows}</div>")
    }

    #[tokio::test]
    async fn web_search_caches_bounded_list_even_when_parser_yields_many_rows() {
        // A pathological or format-changed SERP that parses into far more rows
        // than any real page must NOT cache an unbounded Vec: the cached (and
        // fused-input) list is bounded to `SERP_MAX_RAW_HITS_PER_QUERY`. Mojeek is
        // forced cooling so the oversized DuckDuckGo list is the only thing under
        // test. Feed well above the raw cap to prove the trim fires.
        let raw_cap = crate::config::defaults::SERP_MAX_RAW_HITS_PER_QUERY;
        let transport = FakeHttpTransport::new().with_response(
            DDG_HTML_ENDPOINT,
            ok(DDG_HTML_ENDPOINT, &ddg_body_with_rows(raw_cap + 20)),
        );
        let health = EngineHealth::new();
        health.mark_blocked("mojeek", 3600);
        let cache = empty_web_cache();
        let _ = web_search(&transport, "q", &health, false, &cache, false).await;
        let cached = cache
            .serp_get("duckduckgo", "q", false)
            .expect("ddg list cached");
        assert_eq!(cached.len(), raw_cap);
    }

    #[tokio::test]
    async fn web_search_does_not_truncate_a_normal_sized_serp() {
        // Recall guard: a normal ~30-row result page (larger than the final
        // `SERP_MAX_RESULTS_PER_QUERY` output ceiling, but under the raw cap) must
        // reach fusion untruncated. The cached fusion-input list keeps every row,
        // proving the bound does not shrink real pages down to the output ceiling.
        let normal_rows = 30;
        assert!(normal_rows > crate::config::defaults::SERP_MAX_RESULTS_PER_QUERY);
        assert!(normal_rows < crate::config::defaults::SERP_MAX_RAW_HITS_PER_QUERY);
        let transport = FakeHttpTransport::new().with_response(
            DDG_HTML_ENDPOINT,
            ok(DDG_HTML_ENDPOINT, &ddg_body_with_rows(normal_rows)),
        );
        let health = EngineHealth::new();
        health.mark_blocked("mojeek", 3600);
        let cache = empty_web_cache();
        let _ = web_search(&transport, "q", &health, false, &cache, false).await;
        let cached = cache
            .serp_get("duckduckgo", "q", false)
            .expect("ddg list cached");
        assert_eq!(cached.len(), normal_rows);
    }
}
