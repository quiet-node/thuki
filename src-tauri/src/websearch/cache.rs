//! Multi-turn page cache: the fetched pages of the recent successful searches
//! of a conversation, reused when the classifier judges a follow-up answerable
//! from what was already fetched (a `cached` decision, see
//! [`crate::websearch::prepass::SearchDecision`]) AND the reuse gate can ground
//! the new question in those pages (see [`crate::websearch::orchestrator`]).
//!
//! ## Why pages, not assembled blocks
//!
//! An entry stores the full cleaned page set a search fetched, NOT the source
//! blocks it assembled. Assembled blocks are chunks already BM25-selected
//! against the ORIGINAL question, so a horizontal follow-up (net worth, then
//! age) could never recover a sentence the first selection discarded. Caching
//! the pages instead lets the reuse gate re-run the exact fresh post-fetch
//! pipeline (`select_chunks` → `filter_evidence_chunks` → `assemble_context`)
//! against the NEW question: reuse becomes literally a fresh search minus the
//! network. Only the general scraped-engine tier fetches pages, so every stored
//! entry is produced by it and carries [`SearchRoute::Web`]; the keyless
//! verticals build source blocks directly and never store.
//!
//! This is a chat follow-up cache, not a general results cache: it holds a
//! small, bounded FIFO of entries per conversation
//! ([`crate::config::defaults::SEARCH_CACHE_MAX_ENTRIES`]), each entry the page
//! set of one successful engine-tier search. The oldest entry is evicted once
//! the cap is exceeded, so a follow-up can be grounded in the last few searches,
//! not only the most recent one. Injected like the pipeline's other effectful
//! dependencies ([`crate::websearch::prepass::PrePass`],
//! [`crate::net::transport::HttpTransport`], [`crate::websearch::rank::Scorer`])
//! so the orchestrator's reuse/escalate branch is unit-tested without a live
//! wall clock.
//!
//! ## Memory bound
//!
//! Each stored page's text is truncated to
//! [`crate::config::defaults::SEARCH_CACHE_PAGE_TEXT_MAX_BYTES`] (at a UTF-8 char
//! boundary), and once one entry's cumulative page text would exceed
//! [`crate::config::defaults::SEARCH_CACHE_ENTRY_TEXT_MAX_BYTES`] the remaining
//! pages are dropped from the tail (the first page is always retained, so an
//! entry is never empty). With the entry cap, this fixes the cache's retained
//! text memory at a hard upper bound regardless of how large a fetched page is.
//!
//! ## Scoping
//!
//! The caller supplies an opaque `scope` key on every read and write (in
//! production, the backend's conversation epoch, see
//! `crate::commands::ConversationHistory`, which increments its epoch on
//! every reset: "New conversation", loading a different conversation from
//! history, or clearing the current one). An entry is only ever returned for
//! the exact `scope` it was stored under, so a new conversation, or a reset of
//! the current one, can never reuse another conversation's pages: the epoch that
//! scoped the entries no longer matches the epoch of any turn that follows a
//! reset. A write under a new scope additionally drops every entry from any
//! other scope, so the cache never accumulates orphaned entries from
//! conversations that have already ended and never spends its bounded capacity
//! on entries no read can ever see.

use crate::config::defaults::{
    SEARCH_CACHE_ENTRY_TEXT_MAX_BYTES, SEARCH_CACHE_PAGE_TEXT_MAX_BYTES,
};
use crate::websearch::fetch::FetchedPage;
use crate::websearch::prepass::SearchRoute;
use std::collections::VecDeque;

/// The fetched pages and provenance of one successful engine-tier search, cached
/// for a follow-up turn to reuse without re-retrieving.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedSearch {
    /// The cleaned pages the search fetched, page text already truncated to the
    /// per-page byte cap (see the module "Memory bound" docs). The reuse gate
    /// re-chunks and re-ranks these against the follow-up question.
    pub pages: Vec<FetchedPage>,
    /// The retrieval route that produced this entry. Always [`SearchRoute::Web`]
    /// in production (only the scraped-engine tier fetches pages); kept as
    /// provenance so the reuse gate's volatile-route exclusion stays a correct,
    /// defense-in-depth filter (see `crate::websearch::orchestrator`).
    pub route: SearchRoute,
}

/// Cross-turn, bounded multi-entry page cache. See the module docs for the
/// scoping, TTL, entry-cap, and byte-cap contract.
pub trait SourceCache: Send + Sync {
    /// Returns every live entry (matching `scope` and unexpired) for reuse,
    /// most recent first. Empty when nothing is cached for `scope`.
    fn entries(&self, scope: u64) -> Vec<CachedSearch>;
    /// Stores `entry` under `scope`, enforcing the per-page and per-entry byte
    /// caps on its pages first. Drops every entry from any other scope (a scope
    /// change orphans prior conversations, see module docs), appends the new
    /// entry, then evicts the oldest entries beyond
    /// [`crate::config::defaults::SEARCH_CACHE_MAX_ENTRIES`].
    fn store(&self, scope: u64, entry: CachedSearch);
}

/// Truncates `text` to at most `max_bytes`, cutting at the largest UTF-8 char
/// boundary that does not exceed the cap so a multi-byte scalar is never split.
/// A no-op when `text` already fits.
fn truncate_text_to_bytes(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
}

/// Enforces the memory bound on one entry's pages: each page's text is truncated
/// to `page_cap` bytes, then pages are kept in order until the running total of
/// (capped) page text would exceed `entry_cap`, at which point the remaining
/// pages are dropped. The first page is always retained (an entry is never
/// emptied by the cap), so a single oversize page is stored per-page-capped
/// rather than dropped. `pub(crate)` and cap-parameterized so the deterministic
/// truncate/drop behaviour is unit-tested with small caps.
pub(crate) fn bound_pages(
    pages: Vec<FetchedPage>,
    page_cap: usize,
    entry_cap: usize,
) -> Vec<FetchedPage> {
    let mut out: Vec<FetchedPage> = Vec::new();
    let mut total: usize = 0;
    for mut page in pages {
        truncate_text_to_bytes(&mut page.text, page_cap);
        let cost = page.text.len();
        if !out.is_empty() && total + cost > entry_cap {
            break;
        }
        total += cost;
        out.push(page);
    }
    out
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
        // to return them most-recent-first: the reuse gate prefers recent pages
        // when deduping across entries and when capping the union.
        guard
            .iter()
            .rev()
            .filter(|e| e.scope == scope && e.fetched_at.elapsed() < self.ttl)
            .map(|e| e.search.clone())
            .collect()
    }

    fn store(&self, scope: u64, mut entry: CachedSearch) {
        // Enforce the byte caps at the storage boundary so no caller can grow
        // the cache's retained text past the fixed bound (see module docs).
        entry.pages = bound_pages(
            entry.pages,
            SEARCH_CACHE_PAGE_TEXT_MAX_BYTES,
            SEARCH_CACHE_ENTRY_TEXT_MAX_BYTES,
        );
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

    fn page(url: &str, text: &str) -> FetchedPage {
        FetchedPage {
            url: url.into(),
            title: "T".into(),
            text: text.into(),
            published: None,
        }
    }

    fn search(url: &str) -> CachedSearch {
        CachedSearch {
            pages: vec![page(url, "body")],
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
    fn hit_within_ttl_and_same_scope_returns_the_pages() {
        let cache = cache(4);
        cache.store(1, search("https://a.example/"));
        let got = cache.entries(1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].pages.len(), 1);
        assert_eq!(got[0].pages[0].url, "https://a.example/");
        assert_eq!(got[0].pages[0].text, "body");
        assert_eq!(got[0].route, SearchRoute::Web);
    }

    #[test]
    fn expired_ttl_returns_no_entries() {
        // A zero TTL means `fetched_at.elapsed()` (always >= 0) is never
        // strictly less than the TTL, so the entry reads as expired
        // immediately without a real sleep.
        let cache = TtlSourceCache::new(std::time::Duration::ZERO, 4);
        cache.store(1, search("https://a.example/"));
        assert!(cache.entries(1).is_empty());
    }

    #[test]
    fn different_scope_returns_no_entries() {
        // A fresh conversation (a different epoch) must never see the previous
        // conversation's cached pages.
        let cache = cache(4);
        cache.store(1, search("https://a.example/"));
        assert!(cache.entries(2).is_empty());
    }

    #[test]
    fn store_keeps_multiple_same_scope_entries_most_recent_first() {
        // Within one conversation, successive searches accumulate; a reuse gate
        // sees all of them, newest first.
        let cache = cache(4);
        cache.store(1, search("https://first.example/"));
        cache.store(1, search("https://second.example/"));
        cache.store(1, search("https://third.example/"));
        let got = cache.entries(1);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].pages[0].url, "https://third.example/");
        assert_eq!(got[1].pages[0].url, "https://second.example/");
        assert_eq!(got[2].pages[0].url, "https://first.example/");
    }

    #[test]
    fn store_evicts_oldest_entry_beyond_the_cap() {
        // Cap of 2: the third store drops the oldest ("first"), leaving the two
        // most recent, still newest-first.
        let cache = cache(2);
        cache.store(1, search("https://first.example/"));
        cache.store(1, search("https://second.example/"));
        cache.store(1, search("https://third.example/"));
        let got = cache.entries(1);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].pages[0].url, "https://third.example/");
        assert_eq!(got[1].pages[0].url, "https://second.example/");
    }

    #[test]
    fn store_under_a_new_scope_drops_every_other_scope_entry() {
        // A scope change (conversation reset) orphans the prior scope's entries
        // rather than letting them occupy the bounded capacity forever.
        let cache = cache(4);
        cache.store(1, search("https://first.example/"));
        cache.store(1, search("https://second.example/"));
        cache.store(2, search("https://new-conversation.example/"));
        assert!(cache.entries(1).is_empty());
        let got = cache.entries(2);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].pages[0].url, "https://new-conversation.example/");
    }

    #[test]
    fn zero_cap_never_panics_and_holds_nothing() {
        // A degenerate zero cap evicts the just-pushed entry: the read is empty
        // and nothing panics. Production always passes a cap of at least one.
        let cache = cache(0);
        cache.store(1, search("https://a.example/"));
        assert!(cache.entries(1).is_empty());
    }

    #[test]
    fn store_truncates_oversize_page_text_to_the_per_page_cap() {
        // The production caps are large, so exercise the store-boundary
        // enforcement by asserting a very long page body is truncated to at most
        // the per-page cap (at a char boundary), not stored whole.
        let cache = cache(4);
        let big = "x".repeat(SEARCH_CACHE_PAGE_TEXT_MAX_BYTES + 5_000);
        cache.store(
            1,
            CachedSearch {
                pages: vec![page("https://a.example/", &big)],
                route: SearchRoute::Web,
            },
        );
        let got = cache.entries(1);
        assert_eq!(got[0].pages[0].text.len(), SEARCH_CACHE_PAGE_TEXT_MAX_BYTES);
    }

    #[test]
    fn bound_pages_truncates_each_page_at_a_char_boundary() {
        // Per-page cap of 2 bytes, entry cap large: a 2-byte scalar straddling
        // the cap is dropped whole rather than split (cut backs up to a char
        // boundary). "aée" is bytes [a=1][é=2][e=1]; a 2-byte cap lands mid-é
        // (byte 2), so cut backs up to byte 1 and keeps "a" only.
        let bounded = bound_pages(vec![page("https://a/", "aée")], 2, 1_000);
        assert_eq!(bounded.len(), 1);
        assert_eq!(bounded[0].text, "a");
    }

    #[test]
    fn bound_pages_drops_trailing_pages_past_the_entry_cap() {
        // Per-page cap 10, entry cap 25: three 10-byte pages cumulate to 20 then
        // 30; the third would exceed 25, so it is dropped from the tail. The
        // first two are kept.
        let bounded = bound_pages(
            vec![
                page("https://a/", "0123456789"),
                page("https://b/", "0123456789"),
                page("https://c/", "0123456789"),
            ],
            10,
            25,
        );
        assert_eq!(bounded.len(), 2);
        assert_eq!(bounded[0].url, "https://a/");
        assert_eq!(bounded[1].url, "https://b/");
    }

    #[test]
    fn bound_pages_always_keeps_the_first_page_even_when_it_alone_exceeds_the_entry_cap() {
        // A degenerate config where one per-page-capped page is larger than the
        // whole entry cap: the first page is still retained (never an empty
        // entry), the rest dropped.
        let bounded = bound_pages(
            vec![page("https://a/", "0123456789"), page("https://b/", "x")],
            10,
            5,
        );
        assert_eq!(bounded.len(), 1);
        assert_eq!(bounded[0].url, "https://a/");
    }
}
