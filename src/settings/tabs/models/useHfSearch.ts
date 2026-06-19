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
import type { HfModelSummary } from '../../../types/hf';

/** Debounce window before a query change triggers a backend fetch. */
export const HF_SEARCH_DEBOUNCE_MS = 300;

/** How many more results each "Load more" press requests. Mirrors the backend
 * page step (`HF_SEARCH_LIMIT`); the backend clamps the total to its own max. */
export const HF_PAGE_SIZE = 30;

/**
 * Runtime guard for the IPC boundary. The Rust backend is trusted, but this
 * keeps the hook robust against shape drift (schema changes, legacy builds,
 * mocks) without pulling in a schema library. A malformed payload is treated
 * as a transport failure and collapses to an empty result.
 */
function isHfModelSummaryArray(value: unknown): value is HfModelSummary[] {
  return (
    Array.isArray(value) &&
    value.every((item) => {
      if (typeof item !== 'object' || item === null) return false;
      const candidate = item as {
        id?: unknown;
        downloads?: unknown;
        gated?: unknown;
      };
      return (
        typeof candidate.id === 'string' &&
        typeof candidate.downloads === 'number' &&
        typeof candidate.gated === 'boolean'
      );
    })
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
  const [query, setQueryState] = useState('');
  const [limit, setLimit] = useState(HF_PAGE_SIZE);
  const [results, setResults] = useState<HfModelSummary[]>([]);
  const [loading, setLoading] = useState(true);

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
    setQueryState(q);
    setLimit(HF_PAGE_SIZE);
  }, []);

  const loadMore = useCallback(() => {
    setLimit((current) => current + HF_PAGE_SIZE);
  }, []);

  const runSearch = useCallback(
    async (q: string, lim: number): Promise<void> => {
      latestTokenRef.current += 1;
      const token = latestTokenRef.current;
      setLoading(true);
      try {
        const payload = await invoke<unknown>('search_hf_models', {
          query: q,
          limit: lim,
        });
        if (!isLatest(token)) return;
        setResults(isHfModelSummaryArray(payload) ? payload : []);
      } catch {
        if (!isLatest(token)) return;
        setResults([]);
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
    const timer = window.setTimeout(() => {
      void runSearch(query, limit);
    }, HF_SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [query, limit, runSearch]);

  // The last response filled the page, so the Hub may hold more rows.
  const canLoadMore = !loading && results.length >= limit;

  return { query, setQuery, results, loading, loadMore, canLoadMore };
}
