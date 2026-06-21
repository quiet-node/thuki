/**
 * Search hook for the Discover pane's Hugging Face GGUF browser.
 *
 * Mirrors the request-serialization discipline of `useModelSelection`: a
 * monotonic token drops stale/out-of-order responses, a mounted flag drops
 * post-unmount resolutions, and a runtime guard validates the IPC payload
 * before it is trusted. On top of that, the query input is debounced so a
 * burst of keystrokes makes one backend call, not one per keystroke.
 *
 * The backend command `search_hf_models` returns the most-downloaded GGUF
 * repos for a blank query (a "browse popular" list), so the hook fetches once
 * on mount with an empty query and again on every debounced query change.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { HfModelSummary, HfSearchPage } from '../../../types/hf';

/** Debounce window before a query change triggers a backend fetch. */
export const HF_SEARCH_DEBOUNCE_MS = 300;

/** How many more results each "Load more" press requests. Mirrors the backend
 * page step (`HF_SEARCH_LIMIT`); the backend clamps the total to its own max. */
export const HF_PAGE_SIZE = 30;

/** Max search length the input accepts. Mirrors the backend's
 * `MAX_HF_SEARCH_QUERY_LEN` byte cap; capping the field keeps an over-long
 * paste from reaching the backend, which rejects it and silently empties the
 * results. A character cap, exact for the ASCII model names searched here. */
export const HF_SEARCH_QUERY_MAX_LEN = 200;

/**
 * Session-scoped cache of search pages, keyed by `query::limit`. Switching to
 * another tab unmounts the Discover pane, so without this every return trip
 * would re-hit the Hub and flash "Searching…"; serving an already-seen query
 * from cache makes the tab feel instant and avoids the redundant call. Lives
 * for the app session (cleared on reload), since Hub rankings barely move on
 * that timescale.
 */
const searchCache = new Map<string, HfSearchPage>();

function cacheKey(query: string, limit: number): string {
  return `${query}::${limit}`;
}

/**
 * Clears the session search cache. Exposed for tests, which need a clean cache
 * between cases; production never evicts (the cache is bounded by the small set
 * of queries a user types in one session).
 */
export function clearHfSearchCache(): void {
  searchCache.clear();
}

/**
 * Runtime guard for a single search row. The Rust backend is trusted, but this
 * keeps the hook robust against shape drift (schema changes, legacy builds,
 * mocks) without pulling in a schema library.
 */
function isHfModelSummary(item: unknown): item is HfModelSummary {
  if (typeof item !== 'object' || item === null) return false;
  const candidate = item as {
    id?: unknown;
    downloads?: unknown;
    gated?: unknown;
    vision?: unknown;
    thinking?: unknown;
  };
  return (
    typeof candidate.id === 'string' &&
    typeof candidate.downloads === 'number' &&
    typeof candidate.gated === 'boolean' &&
    typeof candidate.vision === 'boolean' &&
    typeof candidate.thinking === 'boolean'
  );
}

/**
 * Runtime guard for the IPC boundary: a `{ rows, has_more }` page whose rows are
 * all well-formed. A malformed payload is treated as a transport failure and
 * collapses to an empty page.
 */
function isHfSearchPage(value: unknown): value is HfSearchPage {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as { rows?: unknown; has_more?: unknown };
  return (
    typeof candidate.has_more === 'boolean' &&
    Array.isArray(candidate.rows) &&
    candidate.rows.every(isHfModelSummary)
  );
}

/** Shape returned by {@link useHfSearch}. */
export interface UseHfSearchResult {
  /** The current query text, updated synchronously on every keystroke. */
  query: string;
  /** Set the query. Updates immediately; the backend fetch is debounced.
   * A new query resets pagination back to the first page. */
  setQuery: (q: string) => void;
  /** The most recent (validated) search results, or `[]` on any failure. */
  results: HfModelSummary[];
  /** True while a debounced fetch is in flight. */
  loading: boolean;
  /** Request the next page (one more {@link HF_PAGE_SIZE} of results). */
  loadMore: () => void;
  /** True when the last response filled the requested page, so more may exist. */
  canLoadMore: boolean;
}

/**
 * React hook that drives the Discover pane's repo search. Fetches the popular
 * browse list on mount, then re-fetches on each debounced query change.
 *
 * Request serialization: every fetch increments a monotonic token.
 * Resolutions that belong to a stale token are dropped so rapid out-of-order
 * responses cannot overwrite newer state. Resolutions that fire after unmount
 * are also dropped.
 */
export function useHfSearch(): UseHfSearchResult {
  const [queryText, setQueryText] = useState('');
  const [limit, setLimit] = useState(HF_PAGE_SIZE);
  // Seed straight from the cache so a remount (tab switch) paints the last
  // results with no loading flash; a cold first run still starts in `loading`.
  const [results, setResults] = useState<HfModelSummary[]>(
    () => searchCache.get(cacheKey('', HF_PAGE_SIZE))?.rows ?? [],
  );
  const [hasMore, setHasMore] = useState(
    () => searchCache.get(cacheKey('', HF_PAGE_SIZE))?.has_more ?? false,
  );
  const [loading, setLoading] = useState(
    () => !searchCache.has(cacheKey('', HF_PAGE_SIZE)),
  );

  const mountedRef = useRef(true);
  const latestTokenRef = useRef(0);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const isLatest = useCallback((token: number): boolean => {
    return mountedRef.current && token === latestTokenRef.current;
  }, []);

  // A new query starts over at the first page; growing `limit` mid-query is
  // what "Load more" does.
  const setQuery = useCallback((q: string) => {
    setQueryText(q);
    setLimit(HF_PAGE_SIZE);
  }, []);

  const loadMore = useCallback(() => {
    setLimit((current) => current + HF_PAGE_SIZE);
  }, []);

  const runSearch = useCallback(
    async (q: string, lim: number): Promise<void> => {
      const key = cacheKey(q, lim);
      latestTokenRef.current += 1;
      const token = latestTokenRef.current;
      // Cache hit: serve immediately, no network, no spinner. This lives here
      // (a callback) rather than in the effect body so it is not a synchronous
      // setState in an effect.
      const cached = searchCache.get(key);
      if (cached) {
        setResults(cached.rows);
        setHasMore(cached.has_more);
        setLoading(false);
        return;
      }
      setLoading(true);
      try {
        const payload = await invoke<unknown>('search_hf_models', {
          query: q,
          limit: lim,
        });
        if (!isLatest(token)) return;
        if (isHfSearchPage(payload)) {
          searchCache.set(key, payload);
          setResults(payload.rows);
          setHasMore(payload.has_more);
        } else {
          setResults([]);
          setHasMore(false);
        }
      } catch {
        if (!isLatest(token)) return;
        setResults([]);
        setHasMore(false);
      } finally {
        if (isLatest(token)) setLoading(false);
      }
    },
    [isLatest],
  );

  // Debounced fetch: a query change schedules a fetch, and any further change
  // within the window cancels and reschedules it, so a burst of keystrokes
  // makes a single call. The empty-query mount fetch and "Load more" (a
  // `limit` bump) ride the same path.
  useEffect(() => {
    // A cache hit serves instantly (runSearch short-circuits to the cache); a
    // miss is debounced so a burst of keystrokes makes one network call.
    if (searchCache.has(cacheKey(queryText, limit))) {
      void runSearch(queryText, limit);
      return;
    }
    const timer = window.setTimeout(() => {
      void runSearch(queryText, limit);
    }, HF_SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [queryText, limit, runSearch]);

  // The Hub reported a full page, so it may hold more rows. Driven by the page's
  // `has_more` flag, not `results.length`: the backend drops non-chat rows, so a
  // short page can still have more behind it.
  const canLoadMore = !loading && hasMore;

  return {
    query: queryText,
    setQuery,
    results,
    loading,
    loadMore,
    canLoadMore,
  };
}
