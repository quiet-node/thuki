//! Multi-turn source cache: the sources of the most recent successful search,
//! reused when the classifier judges a follow-up answerable from what was
//! just fetched (a `cached` decision, see
//! [`crate::websearch::prepass::SearchDecision`]).
//!
//! This is a chat follow-up cache, not a general results cache: at most one
//! entry is ever held, replaced whole by every new successful search
//! regardless of tier (weather, sports, news, wiki, or the scraped engines).
//! Injected like the pipeline's other effectful dependencies
//! ([`crate::websearch::prepass::PrePass`], [`crate::net::transport::HttpTransport`],
//! [`crate::websearch::rank::Scorer`]) so the orchestrator's hit/miss/expiry
//! branch is unit-tested without a live wall clock.
//!
//! ## Scoping
//!
//! The caller supplies an opaque `scope` key on every read and write (in
//! production, the backend's conversation epoch — see
//! `crate::commands::ConversationHistory`, which increments its epoch on
//! every reset: "New conversation", loading a different conversation from
//! history, or clearing the current one). An entry is only ever returned for
//! the exact `scope` it was stored under, so a new conversation, or a reset
//! of the current one, can never reuse another conversation's sources: the
//! epoch that scoped the entry no longer matches the epoch of any turn that
//! follows a reset.

use crate::websearch::assemble::SourceBlock;

/// The sources and standalone question of one successful search, cached for a
/// follow-up turn to reuse without re-retrieving.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedSearch {
    pub standalone_question: String,
    pub sources: Vec<SourceBlock>,
}

/// Cross-turn, single-entry source cache. See the module docs for the scoping
/// and last-search-only contract.
pub trait SourceCache: Send + Sync {
    /// Returns the cached entry if one exists for `scope` and has not expired.
    fn get(&self, scope: u64) -> Option<CachedSearch>;
    /// Stores `entry` for `scope`, replacing any previous entry (regardless of
    /// its own scope: this cache never holds more than one entry).
    fn store(&self, scope: u64, entry: CachedSearch);
}

/// The single held entry, with the scope key and fetch time an expiry check
/// needs.
struct Slot {
    scope: u64,
    fetched_at: std::time::Instant,
    entry: CachedSearch,
}

/// The production [`SourceCache`]: one mutex-guarded slot with a TTL supplied
/// at construction, so tests can exercise expiry deterministically (a
/// `Duration::ZERO` TTL expires immediately) rather than sleeping a real
/// wall-clock TTL, the same trick
/// [`crate::websearch::engine::EngineHealth`]'s zero-cooldown test uses.
pub struct TtlSourceCache {
    slot: std::sync::Mutex<Option<Slot>>,
    ttl: std::time::Duration,
}

impl TtlSourceCache {
    /// Creates an empty cache with the given TTL. Production callers pass
    /// [`crate::config::defaults::SEARCH_CACHE_TTL_S`]; tests pass whatever
    /// TTL the case needs.
    pub fn new(ttl: std::time::Duration) -> Self {
        Self {
            slot: std::sync::Mutex::new(None),
            ttl,
        }
    }
}

impl SourceCache for TtlSourceCache {
    fn get(&self, scope: u64) -> Option<CachedSearch> {
        let guard = self.slot.lock().unwrap();
        match guard.as_ref() {
            Some(slot) if slot.scope == scope && slot.fetched_at.elapsed() < self.ttl => {
                Some(slot.entry.clone())
            }
            _ => None,
        }
    }

    fn store(&self, scope: u64, entry: CachedSearch) {
        *self.slot.lock().unwrap() = Some(Slot {
            scope,
            fetched_at: std::time::Instant::now(),
            entry,
        });
    }
}

/// The process-wide [`TtlSourceCache`] shared by every turn, so a search
/// stored on one turn is visible to the next (the whole point of the cache).
/// Cross-conversation isolation is not a property of having one instance
/// versus many; it comes entirely from the `scope` check in
/// [`TtlSourceCache::get`] (see module docs), so a single global instance is
/// exactly as safe as one instance per conversation would be, and mirrors
/// [`crate::websearch::engine::global_engine_health`]'s established pattern
/// for cross-turn pipeline memory. Coverage-excluded: a static constructor
/// call; the cache's behaviour is tested through instance methods on
/// locally-built caches above.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn global_search_cache() -> &'static TtlSourceCache {
    static GLOBAL: std::sync::LazyLock<TtlSourceCache> = std::sync::LazyLock::new(|| {
        TtlSourceCache::new(std::time::Duration::from_secs(
            crate::config::defaults::SEARCH_CACHE_TTL_S,
        ))
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
        }
    }

    #[test]
    fn empty_cache_misses() {
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600));
        assert!(cache.get(1).is_none());
    }

    #[test]
    fn hit_within_ttl_and_same_scope() {
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600));
        cache.store(1, search("what's the latest stable rust version"));
        let got = cache.get(1).unwrap();
        assert_eq!(
            got.standalone_question,
            "what's the latest stable rust version"
        );
        assert_eq!(got.sources.len(), 1);
        assert_eq!(got.sources[0].url, "https://a.example/");
    }

    #[test]
    fn expired_ttl_misses() {
        // A zero TTL means `fetched_at.elapsed()` (always >= 0) is never
        // strictly less than the TTL, so the entry reads as expired
        // immediately without a real sleep.
        let cache = TtlSourceCache::new(std::time::Duration::ZERO);
        cache.store(1, search("q"));
        assert!(cache.get(1).is_none());
    }

    #[test]
    fn different_scope_misses() {
        // A fresh conversation (a different epoch) must never see the
        // previous conversation's cached sources.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600));
        cache.store(1, search("q"));
        assert!(cache.get(2).is_none());
    }

    #[test]
    fn store_replaces_previous_entry_regardless_of_scope() {
        // Last-search-only: never more than one entry, even across scopes.
        let cache = TtlSourceCache::new(std::time::Duration::from_secs(600));
        cache.store(1, search("first"));
        cache.store(2, search("second"));
        assert!(cache.get(1).is_none());
        assert_eq!(cache.get(2).unwrap().standalone_question, "second");
    }
}
