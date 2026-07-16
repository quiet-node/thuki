//! Multi-turn source cache: the sources of the recent successful searches of a
//! conversation, reused when the classifier judges a follow-up answerable from
//! what was already fetched (a `cached` decision, see
//! [`crate::websearch::prepass::SearchDecision`]) AND the reuse gate's
//! sufficiency judge agrees the stored sources actually carry the answer (see
//! [`crate::websearch::orchestrator`]).
//!
//! This is a chat follow-up cache, not a general results cache: it holds a
//! small, bounded FIFO of entries per conversation
//! ([`crate::config::defaults::SEARCH_CACHE_MAX_ENTRIES`]), each entry the
//! source set of one successful search of any tier (weather, sports, news,
//! wiki, or the scraped engines). The oldest entry is evicted once the cap is
//! exceeded, so a follow-up can be grounded in the last few searches, not only
//! the most recent one. Injected like the pipeline's other effectful
//! dependencies ([`crate::websearch::prepass::PrePass`],
//! [`crate::net::transport::HttpTransport`], [`crate::websearch::rank::Scorer`])
//! so the orchestrator's reuse/escalate branch is unit-tested without a live
//! wall clock.
//!
//! ## Scoping
//!
//! The caller supplies an opaque `scope` key on every read and write (in
//! production, the backend's conversation epoch, see
//! `crate::commands::ConversationHistory`, which increments its epoch on
//! every reset: "New conversation", loading a different conversation from
//! history, or clearing the current one). An entry is only ever returned for
//! the exact `scope` it was stored under, so a new conversation, or a reset of
//! the current one, can never reuse another conversation's sources: the epoch
//! that scoped the entries no longer matches the epoch of any turn that follows
//! a reset. A write under a new scope additionally drops every entry from any
//! other scope, so the cache never accumulates orphaned entries from
//! conversations that have already ended and never spends its bounded capacity
//! on entries no read can ever see.

use crate::websearch::assemble::SourceBlock;
use crate::websearch::prepass::SearchRoute;
use std::collections::VecDeque;

/// The sources and standalone question of one successful search, cached for a
/// follow-up turn to reuse without re-retrieving.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedSearch {
    /// The resolved standalone question the stored search answered.
    pub standalone_question: String,
    /// The source blocks that grounded the stored answer.
    pub sources: Vec<SourceBlock>,
    /// The retrieval route that produced this entry (the answering tier mapped
    /// to a [`SearchRoute`]). Recorded as provenance metadata; the reuse gate
    /// reads every live entry regardless of route and lets the sufficiency
    /// judge decide, so this field documents what kind of source grounded the
    /// entry rather than filtering which entries a follow-up may reuse.
    pub route: SearchRoute,
}

/// Cross-turn, bounded multi-entry source cache. See the module docs for the
/// scoping, TTL, and entry-cap contract.
pub trait SourceCache: Send + Sync {
    /// Returns every live entry (matching `scope` and unexpired) for reuse,
    /// most recent first. Empty when nothing is cached for `scope`.
    fn entries(&self, scope: u64) -> Vec<CachedSearch>;
    /// Stores `entry` under `scope`. Drops every entry from any other scope
    /// first (a scope change orphans prior conversations, see module docs),
    /// appends the new entry, then evicts the oldest entries beyond
    /// [`crate::config::defaults::SEARCH_CACHE_MAX_ENTRIES`].
    fn store(&self, scope: u64, entry: CachedSearch);
}

/// One held entry, with the scope key and fetch time an expiry check needs.
struct Entry {
    scope: u64,
    fetched_at: std::time::Instant,
    search: CachedSearch,
}

/// The production [`SourceCache`]: a mutex-guarded FIFO of entries with a TTL
/// and an entry cap supplied at construction, so tests can exercise expiry and
/// eviction deterministically (a `Duration::ZERO` TTL expires immediately, a
/// small cap forces eviction) rather than sleeping a real wall-clock TTL, the
/// same trick [`crate::websearch::engine::EngineHealth`]'s zero-cooldown test
/// uses.
pub struct TtlSourceCache {
    entries: std::sync::Mutex<VecDeque<Entry>>,
    ttl: std::time::Duration,
    cap: usize,
}

impl TtlSourceCache {
    /// Creates an empty cache with the given TTL and entry cap. Production
    /// callers pass [`crate::config::defaults::SEARCH_CACHE_TTL_S`] and
    /// [`crate::config::defaults::SEARCH_CACHE_MAX_ENTRIES`]; tests pass
    /// whatever the case needs.
    pub fn new(ttl: std::time::Duration, cap: usize) -> Self {
        Self {
            entries: std::sync::Mutex::new(VecDeque::new()),
            ttl,
            cap,
        }
    }
}

impl SourceCache for TtlSourceCache {
    fn entries(&self, scope: u64) -> Vec<CachedSearch> {
        let guard = self.entries.lock().unwrap();
        // Newest entries sit at the back (store appends), so iterate in reverse
        // to return them most-recent-first: the reuse gate prefers recent
        // sources when it must cap the union to fit the context budget.
        guard
            .iter()
            .rev()
            .filter(|e| e.scope == scope && e.fetched_at.elapsed() < self.ttl)
            .map(|e| e.search.clone())
            .collect()
    }

    fn store(&self, scope: u64, entry: CachedSearch) {
        let mut guard = self.entries.lock().unwrap();
        // A store under a new scope means the conversation the other entries
        // belonged to has ended (the epoch was bumped by a reset): drop them so
        // the bounded capacity is never spent on entries no read can return.
        guard.retain(|e| e.scope == scope);
        guard.push_back(Entry {
            scope,
            fetched_at: std::time::Instant::now(),
            search: entry,
        });
        // Evict oldest-first until the cap holds. `cap` is always >= 1 in
        // production (SEARCH_CACHE_MAX_ENTRIES), so the just-pushed entry
        // survives; a degenerate zero cap would drop it, which the entry-cap
        // test asserts is handled without panicking.
        while guard.len() > self.cap {
            guard.pop_front();
        }
    }
}

/// The process-wide [`TtlSourceCache`] shared by every turn, so a search stored
/// on one turn is visible to the next (the whole point of the cache).
/// Cross-conversation isolation is not a property of having one instance versus
/// many; it comes entirely from the `scope` check in [`TtlSourceCache::entries`]
/// (see module docs), so a single global instance is exactly as safe as one
/// instance per conversation would be, and mirrors
/// [`crate::websearch::engine::global_engine_health`]'s established pattern for
/// cross-turn pipeline memory. Coverage-excluded: a static constructor call;
/// the cache's behaviour is tested through instance methods on locally-built
/// caches above.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn global_search_cache() -> &'static TtlSourceCache {
    static GLOBAL: std::sync::LazyLock<TtlSourceCache> = std::sync::LazyLock::new(|| {
        TtlSourceCache::new(
            std::time::Duration::from_secs(crate::config::defaults::SEARCH_CACHE_TTL_S),
            crate::config::defaults::SEARCH_CACHE_MAX_ENTRIES,
        )
    });
    &GLOBAL
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(question: &str) -> CachedSearch {
        CachedSearch {
            standalone_question: question.into(),
            sources: vec![SourceBlock {
                index: 1,
                url: "https://a.example/".into(),
                title: "T".into(),
                text: "body".into(),
            }],
            route: SearchRoute::Web,
        }
    }

    fn cache(cap: usize) -> TtlSourceCache {
        TtlSourceCache::new(std::time::Duration::from_secs(600), cap)
    }

    #[test]
    fn empty_cache_returns_no_entries() {
        assert!(cache(4).entries(1).is_empty());
    }

    #[test]
    fn hit_within_ttl_and_same_scope() {
        let cache = cache(4);
        cache.store(1, search("what's the latest stable rust version"));
        let got = cache.entries(1);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].standalone_question,
            "what's the latest stable rust version"
        );
        assert_eq!(got[0].sources.len(), 1);
        assert_eq!(got[0].sources[0].url, "https://a.example/");
        assert_eq!(got[0].route, SearchRoute::Web);
    }

    #[test]
    fn expired_ttl_returns_no_entries() {
        // A zero TTL means `fetched_at.elapsed()` (always >= 0) is never
        // strictly less than the TTL, so the entry reads as expired
        // immediately without a real sleep.
        let cache = TtlSourceCache::new(std::time::Duration::ZERO, 4);
        cache.store(1, search("q"));
        assert!(cache.entries(1).is_empty());
    }

    #[test]
    fn different_scope_returns_no_entries() {
        // A fresh conversation (a different epoch) must never see the previous
        // conversation's cached sources.
        let cache = cache(4);
        cache.store(1, search("q"));
        assert!(cache.entries(2).is_empty());
    }

    #[test]
    fn store_keeps_multiple_same_scope_entries_most_recent_first() {
        // Within one conversation, successive searches accumulate; a reuse gate
        // sees all of them, newest first.
        let cache = cache(4);
        cache.store(1, search("first"));
        cache.store(1, search("second"));
        cache.store(1, search("third"));
        let got = cache.entries(1);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].standalone_question, "third");
        assert_eq!(got[1].standalone_question, "second");
        assert_eq!(got[2].standalone_question, "first");
    }

    #[test]
    fn store_evicts_oldest_entry_beyond_the_cap() {
        // Cap of 2: the third store drops the oldest ("first"), leaving the two
        // most recent, still newest-first.
        let cache = cache(2);
        cache.store(1, search("first"));
        cache.store(1, search("second"));
        cache.store(1, search("third"));
        let got = cache.entries(1);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].standalone_question, "third");
        assert_eq!(got[1].standalone_question, "second");
    }

    #[test]
    fn store_under_a_new_scope_drops_every_other_scope_entry() {
        // A scope change (conversation reset) orphans the prior scope's entries
        // rather than letting them occupy the bounded capacity forever.
        let cache = cache(4);
        cache.store(1, search("first"));
        cache.store(1, search("second"));
        cache.store(2, search("new-conversation"));
        assert!(cache.entries(1).is_empty());
        let got = cache.entries(2);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].standalone_question, "new-conversation");
    }

    #[test]
    fn zero_cap_never_panics_and_holds_nothing() {
        // A degenerate zero cap evicts the just-pushed entry: the read is empty
        // and nothing panics. Production always passes a cap of at least one.
        let cache = cache(0);
        cache.store(1, search("q"));
        assert!(cache.entries(1).is_empty());
    }
}
