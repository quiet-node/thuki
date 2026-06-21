/**
 * Unit tests for {@link useHfSearch}.
 *
 * The hook debounces the query, serializes overlapping fetches with a
 * monotonic token, drops post-unmount resolutions, and guards the IPC
 * payload at runtime. The backend returns a {@link HfSearchPage}
 * (`{ rows, has_more }`); `canLoadMore` follows `has_more`, not the row count,
 * so the backend's chat-model allowlist cannot end pagination early. The tests
 * drive the debounce with fake timers and control resolution order with
 * externally-settled promises so the stale-token path is exercised
 * deterministically.
 */

import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import {
  useHfSearch,
  HF_SEARCH_DEBOUNCE_MS,
  HF_PAGE_SIZE,
  clearHfSearchCache,
} from './useHfSearch';
import type { HfModelSummary, HfSearchPage } from '../../../types/hf';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

/** Wraps rows in the page envelope the backend returns. */
function page(rows: HfModelSummary[], hasMore = false): HfSearchPage {
  return { rows, has_more: hasMore };
}

const POPULAR: HfModelSummary[] = [
  {
    id: 'google/gemma-popular-GGUF',
    downloads: 1_000_000,
    gated: false,
    vision: false,
    thinking: false,
  },
];

const GEMMA: HfModelSummary[] = [
  {
    id: 'google/gemma-4-12b-it-GGUF',
    downloads: 1_200_000,
    gated: false,
    vision: true,
    thinking: false,
  },
  {
    id: 'unsloth/gemma-4-27b-it-GGUF',
    downloads: 410_000,
    gated: false,
    vision: false,
    thinking: false,
  },
];

beforeEach(() => {
  invokeMock.mockReset();
  clearHfSearchCache();
});

afterEach(() => {
  vi.useRealTimers();
});

/** Externally-settled promise so a test can control when invoke resolves. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('useHfSearch', () => {
  it('fetches the popular browse list on mount with an empty query', async () => {
    invokeMock.mockResolvedValue(page(POPULAR));
    const { result } = renderHook(() => useHfSearch());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', {
      query: '',
      limit: HF_PAGE_SIZE,
    });
    expect(result.current.results).toEqual(POPULAR);
    expect(result.current.query).toBe('');
  });

  it('sets the query immediately but debounces the fetch', async () => {
    vi.useFakeTimers();
    invokeMock.mockResolvedValue(page(POPULAR));
    const { result } = renderHook(() => useHfSearch());
    // Drain the mount fetch.
    await act(async () => {
      await Promise.resolve();
    });
    invokeMock.mockClear();
    invokeMock.mockResolvedValue(page(GEMMA));

    act(() => result.current.setQuery('gemma'));
    // Query is visible immediately; no fetch has fired yet.
    expect(result.current.query).toBe('gemma');
    expect(invokeMock).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', {
      query: 'gemma',
      limit: HF_PAGE_SIZE,
    });
    expect(result.current.results).toEqual(GEMMA);
  });

  it('coalesces rapid input into a single fetch', async () => {
    vi.useFakeTimers();
    invokeMock.mockResolvedValue(page(POPULAR));
    const { result } = renderHook(() => useHfSearch());
    await act(async () => {
      await Promise.resolve();
    });
    invokeMock.mockClear();
    invokeMock.mockResolvedValue(page(GEMMA));

    act(() => {
      result.current.setQuery('g');
      result.current.setQuery('ge');
      result.current.setQuery('gem');
    });
    // Each keystroke restarts the timer; nothing has fired between them.
    act(() => vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS - 1));
    expect(invokeMock).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(1);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', {
      query: 'gem',
      limit: HF_PAGE_SIZE,
    });
  });

  it('drops a stale response that resolves after a newer one', async () => {
    vi.useFakeTimers();
    const first = deferred<HfSearchPage>();
    const second = deferred<HfSearchPage>();
    // Mount fetch resolves immediately so the two we care about are #2 and #3.
    invokeMock.mockResolvedValueOnce(page(POPULAR));
    invokeMock.mockReturnValueOnce(first.promise);
    invokeMock.mockReturnValueOnce(second.promise);
    const { result } = renderHook(() => useHfSearch());
    // Fire and drain the debounced mount fetch so it consumes POPULAR; the
    // two requests we care about are the next two (first, second).
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });

    act(() => result.current.setQuery('a'));
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
    });
    act(() => result.current.setQuery('ab'));
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
    });

    // Resolve the NEWER request first, then the older one.
    await act(async () => {
      second.resolve(page(GEMMA));
      await Promise.resolve();
    });
    expect(result.current.results).toEqual(GEMMA);
    await act(async () => {
      first.resolve(page(POPULAR));
      await Promise.resolve();
    });
    // The stale (older) response must not overwrite the newer result.
    expect(result.current.results).toEqual(GEMMA);
  });

  it('drops a resolution that lands after unmount', async () => {
    const pending = deferred<HfSearchPage>();
    invokeMock.mockReturnValue(pending.promise);
    const { result, unmount } = renderHook(() => useHfSearch());
    expect(result.current.loading).toBe(true);
    unmount();
    // Resolving after unmount must not throw or update state.
    await act(async () => {
      pending.resolve(page(POPULAR));
      await Promise.resolve();
    });
    // No assertion on state (unmounted); the test passes if nothing throws.
  });

  it('treats a non-object payload as an empty result', async () => {
    invokeMock.mockResolvedValue('nope');
    const { result } = renderHook(() => useHfSearch());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.results).toEqual([]);
    expect(result.current.canLoadMore).toBe(false);
  });

  it('treats a payload with a non-boolean has_more as an empty result', async () => {
    invokeMock.mockResolvedValue({ rows: POPULAR, has_more: 'yes' });
    const { result } = renderHook(() => useHfSearch());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.results).toEqual([]);
  });

  it('treats a payload whose rows are not an array as an empty result', async () => {
    invokeMock.mockResolvedValue({ rows: 'nope', has_more: false });
    const { result } = renderHook(() => useHfSearch());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.results).toEqual([]);
  });

  it('rejects rows with any malformed field', async () => {
    // Each row trips a different guard branch: bad id, downloads, gated,
    // vision, thinking, a non-object row, and a null row.
    const malformed = [
      { id: 5, downloads: 1, gated: false, vision: false, thinking: false },
      {
        id: 'a/b',
        downloads: 'x',
        gated: false,
        vision: false,
        thinking: false,
      },
      { id: 'a/b', downloads: 1, gated: 'x', vision: false, thinking: false },
      { id: 'a/b', downloads: 1, gated: false, vision: 'x', thinking: false },
      { id: 'a/b', downloads: 1, gated: false, vision: false, thinking: 'x' },
      'not-an-object',
      null,
    ];
    for (const bad of malformed) {
      clearHfSearchCache();
      invokeMock.mockReset();
      invokeMock.mockResolvedValue({
        rows: [
          {
            id: 'ok/repo',
            downloads: 1,
            gated: false,
            vision: false,
            thinking: false,
          },
          bad,
        ],
        has_more: false,
      });
      const { result, unmount } = renderHook(() => useHfSearch());
      await waitFor(() => expect(result.current.loading).toBe(false));
      expect(result.current.results).toEqual([]);
      unmount();
    }
  });

  it('drops a stale rejection that lands after a newer success', async () => {
    vi.useFakeTimers();
    const first = deferred<HfSearchPage>();
    const second = deferred<HfSearchPage>();
    invokeMock.mockResolvedValueOnce(page(POPULAR));
    invokeMock.mockReturnValueOnce(first.promise);
    invokeMock.mockReturnValueOnce(second.promise);
    const { result } = renderHook(() => useHfSearch());
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });

    act(() => result.current.setQuery('a'));
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
    });
    act(() => result.current.setQuery('ab'));
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
    });

    // Newer request succeeds first; the older one then rejects.
    await act(async () => {
      second.resolve(page(GEMMA));
      await Promise.resolve();
    });
    expect(result.current.results).toEqual(GEMMA);
    await act(async () => {
      first.reject(new Error('stale failure'));
      await Promise.resolve();
    });
    // The stale rejection must not clear the newer result.
    expect(result.current.results).toEqual(GEMMA);
  });

  it('falls back to an empty result when the fetch rejects', async () => {
    invokeMock.mockRejectedValue(new Error('network down'));
    const { result } = renderHook(() => useHfSearch());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.results).toEqual([]);
  });

  it('passes a non-empty query verbatim', async () => {
    vi.useFakeTimers();
    invokeMock.mockResolvedValue(page(POPULAR));
    const { result } = renderHook(() => useHfSearch());
    await act(async () => {
      await Promise.resolve();
    });
    invokeMock.mockClear();
    invokeMock.mockResolvedValue(page(GEMMA));

    act(() => result.current.setQuery('llama'));
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', {
      query: 'llama',
      limit: HF_PAGE_SIZE,
    });
  });

  it('serves a repeated query from cache without re-fetching', async () => {
    invokeMock.mockResolvedValue(page(POPULAR));
    const first = renderHook(() => useHfSearch());
    await waitFor(() => expect(first.result.current.loading).toBe(false));
    expect(invokeMock).toHaveBeenCalledTimes(1);
    first.unmount();

    // A fresh mount (a Discover tab revisit) seeds from cache: results are
    // present immediately, there is no loading flash, and no new call fires.
    invokeMock.mockClear();
    const second = renderHook(() => useHfSearch());
    expect(second.result.current.loading).toBe(false);
    expect(second.result.current.results).toEqual(POPULAR);
    await act(async () => {
      await Promise.resolve();
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('follows has_more for Load more, regardless of the row count', async () => {
    // A short page that still reports more is offered Load more: the count is a
    // poor signal once the backend drops non-chat rows, so has_more is the truth.
    invokeMock.mockResolvedValue(page(POPULAR, true));
    const { result } = renderHook(() => useHfSearch());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.results).toEqual(POPULAR);
    expect(result.current.canLoadMore).toBe(true);
  });

  it('does not offer Load more when the Hub reports no more', async () => {
    invokeMock.mockResolvedValue(page(POPULAR, false));
    const { result } = renderHook(() => useHfSearch());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.canLoadMore).toBe(false);
  });

  it('Load more requests the next page and clears canLoadMore when it runs dry', async () => {
    vi.useFakeTimers();
    const full = (n: number): HfModelSummary[] =>
      Array.from({ length: n }, (_, i) => ({
        id: `org/repo-${i}-GGUF`,
        downloads: n - i,
        gated: false,
        vision: false,
        thinking: false,
      }));
    invokeMock.mockResolvedValueOnce(page(full(HF_PAGE_SIZE), true));
    const { result } = renderHook(() => useHfSearch());
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    expect(result.current.results).toHaveLength(HF_PAGE_SIZE);
    expect(result.current.canLoadMore).toBe(true);

    invokeMock.mockClear();
    // Page 2 reports the Hub is exhausted: Load more disappears.
    invokeMock.mockResolvedValueOnce(page(full(HF_PAGE_SIZE + 15), false));
    act(() => result.current.loadMore());
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', {
      query: '',
      limit: HF_PAGE_SIZE * 2,
    });
    expect(result.current.results).toHaveLength(HF_PAGE_SIZE + 15);
    expect(result.current.canLoadMore).toBe(false);
  });
});
