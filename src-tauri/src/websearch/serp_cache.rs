//! Process-lifetime, in-memory result cache for the built-in web search: one
//! side holds per-engine SERP result lists, the other holds extracted page
//! bodies. A repeat within a short window is served from memory instead of
//! re-scraping, which cuts latency AND starves the keyless engines'
//! volume-triggered rate limits (a burst of identical requests is exactly what
//! earns a multi-hour DuckDuckGo IP block, observed live). The closest OSS peer
//! (SearXNG) ships no such cache; this is a deliberate addition, not a port.
//!
//! ## Security: in-memory ONLY, never disk
//!
//! Thuki is privacy-first. User search queries and fetched page content are
//! sensitive and MUST never be persisted. This cache lives entirely in process
//! memory: process exit wipes it, and nothing here ever touches the filesystem.
//! This is a deliberate rejection of a SQLite-backed design; do not add one.
//!
//! ## Structure
//!
//! Both sides are the same shape (TTL + insertion-order eviction + a hard entry
//! cap), so they share one generic [`BoundedTtlMap`] rather than two hand-rolled
//! copies (DRY: the map logic is written and tested once). Each side gets its
//! OWN [`BoundedTtlMap`], and each [`BoundedTtlMap`] owns its OWN `Mutex`, so a
//! SERP write never blocks a page read and vice versa. This is the "one mutex
//! over a struct" option the design allows: the struct behind the mutex bundles
//! the map with a monotonic sequence counter used for deterministic eviction.
//!
//! ## Lock discipline
//!
//! Every method locks only for the duration of the map operation and never holds
//! the lock across an `await` or any I/O, mirroring the documented pattern of
//! [`crate::websearch::engine::EngineHealth`]. The cache methods do no I/O at
//! all; the network work happens in the coverage-excluded async glue of
//! [`crate::websearch::engine::web_search`] and
//! [`crate::websearch::fetch::fetch_pages`], with cache reads before the work
//! and cache writes after it.
//!
//! ## Eviction: insertion-order FIFO, not full LRU
//!
//! At the cap, the oldest-INSERTED entry is evicted (a monotonic per-map
//! sequence counter orders entries deterministically; a wall-clock `Instant`
//! alone can tie under rapid inserts and make eviction nondeterministic). Full
//! LRU would additionally bump recency on every read, which at these small caps
//! buys little (recency-of-insert closely tracks recency-of-use over a short
//! session) at the cost of write-bookkeeping on the read path. Expired entries
//! are pruned lazily on read AND on insert, so a read frees a capped slot and an
//! insert never counts stale entries toward the cap.

use crate::config::defaults::{
    PAGE_CACHE_MAX_ENTRIES, PAGE_CACHE_TTL_S, SERP_CACHE_MAX_ENTRIES, SERP_CACHE_TTL_S,
};
use crate::websearch::engine::SearchHit;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One cached value plus the metadata the TTL check and eviction ordering need:
/// when it was inserted (for expiry) and a monotonic sequence number (for
/// deterministic oldest-first eviction).
struct Entry<V> {
    value: V,
    inserted_at: Instant,
    seq: u64,
}

/// The mutable state behind a [`BoundedTtlMap`]'s single mutex: the entry map and
/// the next sequence number to hand out. Bundling the counter with the map means
/// it is incremented under the same lock that guards the map, so eviction
/// ordering can never race.
struct Inner<K, V> {
    map: HashMap<K, Entry<V>>,
    next_seq: u64,
}

/// A generic bounded, TTL-expiring map shared by both cache sides. Each instance
/// owns its own mutex, so the two caches never contend on one lock.
struct BoundedTtlMap<K, V> {
    inner: Mutex<Inner<K, V>>,
    ttl: Duration,
    cap: usize,
}

impl<K: std::hash::Hash + Eq + Clone, V: Clone> BoundedTtlMap<K, V> {
    /// Creates an empty map with the given TTL and hard entry cap.
    fn new(ttl: Duration, cap: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                next_seq: 0,
            }),
            ttl,
            cap,
        }
    }

    /// Returns a clone of the value for `key` when present and unexpired. An
    /// expired entry is removed before returning `None`, so a stale entry frees
    /// its capped slot on the next read rather than lingering until an insert.
    fn get(&self, key: &K) -> Option<V> {
        let mut inner = self.inner.lock().unwrap();
        let expired = match inner.map.get(key) {
            Some(entry) => {
                if entry.inserted_at.elapsed() < self.ttl {
                    return Some(entry.value.clone());
                }
                true
            }
            None => false,
        };
        if expired {
            inner.map.remove(key);
        }
        None
    }

    /// Inserts (or overwrites) `key`. Expired entries are pruned first, then, if
    /// this is a new key that would exceed the cap, the oldest-inserted entry is
    /// evicted so the map stays within [`Self::cap`]. Overwriting an existing key
    /// never evicts, since it does not grow the map.
    fn insert(&self, key: K, value: V) {
        let mut inner = self.inner.lock().unwrap();
        let ttl = self.ttl;
        inner
            .map
            .retain(|_, entry| entry.inserted_at.elapsed() < ttl);
        if !inner.map.contains_key(&key) && inner.map.len() >= self.cap {
            if let Some(oldest) = inner
                .map
                .iter()
                .min_by_key(|(_, entry)| entry.seq)
                .map(|(k, _)| k.clone())
            {
                inner.map.remove(&oldest);
            }
        }
        let seq = inner.next_seq;
        inner.next_seq += 1;
        inner.map.insert(
            key,
            Entry {
                value,
                inserted_at: Instant::now(),
                seq,
            },
        );
    }
}

/// Identity of a cached SERP list. A list is only reusable for the exact engine,
/// query, and freshness flag it was fetched under: a different freshness flag
/// biases the request toward recent results, so it is a genuinely different
/// query and must be a distinct cache entry.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SerpKey {
    engine: &'static str,
    query: String,
    freshness: bool,
}

/// The built-in web search's in-memory result cache: per-engine SERP lists on
/// one side, extracted page bodies on the other. See the module docs for the
/// security constraint (in-memory only) and eviction policy.
pub struct WebCache {
    serp: BoundedTtlMap<SerpKey, Vec<SearchHit>>,
    pages: BoundedTtlMap<String, String>,
}

impl WebCache {
    /// Creates an empty cache with explicit TTLs and caps. Production uses
    /// [`global_web_cache`]; tests pass short TTLs and tiny caps to exercise
    /// expiry and eviction deterministically.
    pub fn new(serp_ttl: Duration, page_ttl: Duration, serp_cap: usize, page_cap: usize) -> Self {
        Self {
            serp: BoundedTtlMap::new(serp_ttl, serp_cap),
            pages: BoundedTtlMap::new(page_ttl, page_cap),
        }
    }

    /// Returns the cached SERP list for `engine`/`query`/`freshness` when present
    /// and unexpired.
    pub fn serp_get(
        &self,
        engine: &'static str,
        query: &str,
        freshness: bool,
    ) -> Option<Vec<SearchHit>> {
        self.serp.get(&SerpKey {
            engine,
            query: query.to_string(),
            freshness,
        })
    }

    /// Caches an engine's parsed SERP list under `engine`/`query`/`freshness`.
    /// Only successfully-parsed (Ok) lists are stored by the caller; blocked and
    /// empty outcomes are never cached (a block must not be replayed as truth).
    pub fn serp_put(
        &self,
        engine: &'static str,
        query: &str,
        freshness: bool,
        hits: Vec<SearchHit>,
    ) {
        self.serp.insert(
            SerpKey {
                engine,
                query: query.to_string(),
                freshness,
            },
            hits,
        );
    }

    /// Returns the cached extracted page text for `url` when present and
    /// unexpired. Keyed on the SERP hit URL (the URL as it appears in
    /// [`crate::websearch::fetch::FetchedPage`]), not the post-redirect final
    /// URL, so the pre-fetch lookup and the post-extract store agree on the key.
    pub fn page_get(&self, url: &str) -> Option<String> {
        self.pages.get(&url.to_string())
    }

    /// Caches the extracted text for `url`. The caller stores only real extracted
    /// article text, never a SERP-snippet fallback or a failed fetch.
    pub fn page_put(&self, url: &str, text: String) {
        self.pages.insert(url.to_string(), text);
    }
}

/// The process-wide [`WebCache`] shared by every turn, so a scrape on one turn is
/// reusable on the next (the whole point). A single global instance is safe: the
/// cache holds no per-conversation data that could leak across conversations (a
/// SERP list and a page body are public web content, not user text), unlike
/// [`crate::websearch::cache::global_search_cache`], which scopes by conversation
/// epoch precisely because it holds the user's own resolved question. Mirrors
/// [`crate::websearch::engine::global_engine_health`]'s cross-turn pattern.
/// Coverage-excluded: a static constructor call; the cache's behaviour is tested
/// through instance methods on locally-built caches below.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn global_web_cache() -> &'static WebCache {
    static GLOBAL: std::sync::LazyLock<WebCache> = std::sync::LazyLock::new(|| {
        WebCache::new(
            Duration::from_secs(SERP_CACHE_TTL_S),
            Duration::from_secs(PAGE_CACHE_TTL_S),
            SERP_CACHE_MAX_ENTRIES,
            PAGE_CACHE_MAX_ENTRIES,
        )
    });
    &GLOBAL
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(url: &str) -> SearchHit {
        SearchHit {
            title: "T".into(),
            url: url.into(),
            snippet: "s".into(),
        }
    }

    /// A cache with generous TTLs and caps for the hit/miss/eviction cases that
    /// do not care about expiry.
    fn cache() -> WebCache {
        WebCache::new(
            Duration::from_secs(600),
            Duration::from_secs(600),
            SERP_CACHE_MAX_ENTRIES,
            PAGE_CACHE_MAX_ENTRIES,
        )
    }

    // ── SERP side ─────────────────────────────────────────────────────────────

    #[test]
    fn serp_miss_on_empty_cache() {
        assert!(cache().serp_get("duckduckgo", "q", false).is_none());
    }

    #[test]
    fn serp_hit_within_ttl() {
        let c = cache();
        c.serp_put("duckduckgo", "q", false, vec![hit("https://a/")]);
        let got = c.serp_get("duckduckgo", "q", false).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].url, "https://a/");
    }

    #[test]
    fn serp_miss_after_ttl() {
        // A zero TTL means `elapsed()` (always >= 0) is never strictly less than
        // the TTL, so the entry reads as expired immediately without a real
        // sleep (the same trick `websearch::cache` uses).
        let c = WebCache::new(Duration::ZERO, Duration::from_secs(600), 8, 8);
        c.serp_put("duckduckgo", "q", false, vec![hit("https://a/")]);
        assert!(c.serp_get("duckduckgo", "q", false).is_none());
    }

    #[test]
    fn serp_key_distinguishes_engine_query_and_freshness() {
        let c = cache();
        c.serp_put("duckduckgo", "q", false, vec![hit("https://ddg/")]);
        // Same query+freshness, different engine: miss.
        assert!(c.serp_get("mojeek", "q", false).is_none());
        // Same engine+freshness, different query: miss.
        assert!(c.serp_get("duckduckgo", "other", false).is_none());
        // Same engine+query, different freshness flag: miss (a genuinely
        // different, recency-biased request).
        assert!(c.serp_get("duckduckgo", "q", true).is_none());
        // Exact triple: hit.
        assert!(c.serp_get("duckduckgo", "q", false).is_some());
    }

    #[test]
    fn serp_cap_evicts_oldest_inserted() {
        let c = WebCache::new(Duration::from_secs(600), Duration::from_secs(600), 2, 8);
        c.serp_put("duckduckgo", "q1", false, vec![hit("https://1/")]);
        c.serp_put("duckduckgo", "q2", false, vec![hit("https://2/")]);
        // Third insert is over cap: the oldest (q1) is evicted, q2 and q3 remain.
        c.serp_put("duckduckgo", "q3", false, vec![hit("https://3/")]);
        assert!(c.serp_get("duckduckgo", "q1", false).is_none());
        assert!(c.serp_get("duckduckgo", "q2", false).is_some());
        assert!(c.serp_get("duckduckgo", "q3", false).is_some());
    }

    #[test]
    fn serp_overwrite_at_cap_does_not_evict() {
        // Re-putting an existing key when full must not grow the map or evict a
        // different entry: it overwrites in place.
        let c = WebCache::new(Duration::from_secs(600), Duration::from_secs(600), 2, 8);
        c.serp_put("duckduckgo", "q1", false, vec![hit("https://1/")]);
        c.serp_put("duckduckgo", "q2", false, vec![hit("https://2/")]);
        c.serp_put("duckduckgo", "q1", false, vec![hit("https://1b/")]);
        // Both original keys still present; q1 now carries the new value.
        assert_eq!(
            c.serp_get("duckduckgo", "q1", false).unwrap()[0].url,
            "https://1b/"
        );
        assert!(c.serp_get("duckduckgo", "q2", false).is_some());
    }

    #[test]
    fn serp_expired_entry_pruned_on_insert() {
        // Zero TTL: every entry is expired the instant it lands. Inserting a
        // second key runs the on-insert prune, which drops the first (expired)
        // key before the new one is stored, so the map never accumulates stale
        // entries toward the cap. Only the just-inserted key remains present as a
        // map slot (itself already expired, so it reads back as a miss).
        let c = WebCache::new(Duration::ZERO, Duration::from_secs(600), 8, 8);
        c.serp_put("duckduckgo", "q1", false, vec![hit("https://1/")]);
        c.serp_put("duckduckgo", "q2", false, vec![hit("https://2/")]);
        assert!(c.serp_get("duckduckgo", "q1", false).is_none());
        assert!(c.serp_get("duckduckgo", "q2", false).is_none());
    }

    // ── Page side ─────────────────────────────────────────────────────────────

    #[test]
    fn page_miss_on_empty_cache() {
        assert!(cache().page_get("https://a/").is_none());
    }

    #[test]
    fn page_hit_within_ttl() {
        let c = cache();
        c.page_put("https://a/", "body".into());
        assert_eq!(c.page_get("https://a/").unwrap(), "body");
    }

    #[test]
    fn page_miss_after_ttl() {
        let c = WebCache::new(Duration::from_secs(600), Duration::ZERO, 8, 8);
        c.page_put("https://a/", "body".into());
        assert!(c.page_get("https://a/").is_none());
    }

    #[test]
    fn page_cap_evicts_oldest_inserted() {
        let c = WebCache::new(Duration::from_secs(600), Duration::from_secs(600), 8, 2);
        c.page_put("https://1/", "b1".into());
        c.page_put("https://2/", "b2".into());
        c.page_put("https://3/", "b3".into());
        assert!(c.page_get("https://1/").is_none());
        assert_eq!(c.page_get("https://2/").unwrap(), "b2");
        assert_eq!(c.page_get("https://3/").unwrap(), "b3");
    }

    #[test]
    fn page_expired_entry_pruned_on_insert() {
        let c = WebCache::new(Duration::from_secs(600), Duration::ZERO, 8, 8);
        c.page_put("https://1/", "b1".into());
        c.page_put("https://2/", "b2".into());
        assert!(c.page_get("https://1/").is_none());
        assert!(c.page_get("https://2/").is_none());
    }
}
