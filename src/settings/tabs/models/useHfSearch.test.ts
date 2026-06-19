/**
 * Unit tests for {@link useHfSearch}.
 *
 * The hook debounces the query, serializes overlapping fetches with a
 * monotonic token, drops post-unmount resolutions, and guards the IPC
 * payload at runtime. The tests drive the debounce with fake timers and
 * control resolution order with externally-settled promises so the
 * stale-token path is exercised deterministically.
 */

import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import { useHfSearch, HF_SEARCH_DEBOUNCE_MS } from './useHfSearch';
import type { HfModelSummary } from '../../../types/hf';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const POPULAR: HfModelSummary[] = [
  { id: 'google/gemma-popular-GGUF', downloads: 1_000_000, gated: false },
];

const GEMMA: HfModelSummary[] = [
  { id: 'google/gemma-4-12b-it-GGUF', downloads: 1_200_000, gated: false },
  { id: 'unsloth/gemma-4-27b-it-GGUF', downloads: 410_000, gated: false },
];

beforeEach(() => {
  invokeMock.mockReset();
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
    invokeMock.mockResolvedValue(POPULAR);
    const { result } = renderHook(() => useHfSearch());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', { query: '' });
    expect(result.current.results).toEqual(POPULAR);
    expect(result.current.query).toBe('');
  });

  it('sets the query immediately but debounces the fetch', async () => {
    vi.useFakeTimers();
    invokeMock.mockResolvedValue(POPULAR);
    const { result } = renderHook(() => useHfSearch());
    // Drain the mount fetch.
    await act(async () => {
      await Promise.resolve();
    });
    invokeMock.mockClear();
    invokeMock.mockResolvedValue(GEMMA);

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
    });
    expect(result.current.results).toEqual(GEMMA);
  });

  it('coalesces rapid input into a single fetch', async () => {
    vi.useFakeTimers();
    invokeMock.mockResolvedValue(POPULAR);
    const { result } = renderHook(() => useHfSearch());
    await act(async () => {
      await Promise.resolve();
    });
    invokeMock.mockClear();
    invokeMock.mockResolvedValue(GEMMA);

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
    });
  });

  it('drops a stale response that resolves after a newer one', async () => {
    vi.useFakeTimers();
    const first = deferred<HfModelSummary[]>();
    const second = deferred<HfModelSummary[]>();
    // Mount fetch resolves immediately so the two we care about are #2 and #3.
    invokeMock.mockResolvedValueOnce(POPULAR);
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
      second.resolve(GEMMA);
      await Promise.resolve();
    });
    expect(result.current.results).toEqual(GEMMA);
    await act(async () => {
      first.resolve(POPULAR);
      await Promise.resolve();
    });
    // The stale (older) response must not overwrite the newer result.
    expect(result.current.results).toEqual(GEMMA);
  });

  it('drops a resolution that lands after unmount', async () => {
    const pending = deferred<HfModelSummary[]>();
    invokeMock.mockReturnValue(pending.promise);
    const { result, unmount } = renderHook(() => useHfSearch());
    expect(result.current.loading).toBe(true);
    unmount();
    // Resolving after unmount must not throw or update state.
    await act(async () => {
      pending.resolve(POPULAR);
      await Promise.resolve();
    });
    // No assertion on state (unmounted); the test passes if nothing throws.
  });

  it('treats a malformed payload as an empty result', async () => {
    invokeMock.mockResolvedValue({ not: 'an array' });
    const { result } = renderHook(() => useHfSearch());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.results).toEqual([]);
  });

  it('treats an array with a malformed item as an empty result', async () => {
    invokeMock.mockResolvedValue([
      { id: 'ok/repo', downloads: 1, gated: false },
      { id: 5 },
    ]);
    const { result } = renderHook(() => useHfSearch());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.results).toEqual([]);
  });

  it('treats an array containing a null item as an empty result', async () => {
    invokeMock.mockResolvedValue([null]);
    const { result } = renderHook(() => useHfSearch());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.results).toEqual([]);
  });

  it('drops a stale rejection that lands after a newer success', async () => {
    vi.useFakeTimers();
    const first = deferred<HfModelSummary[]>();
    const second = deferred<HfModelSummary[]>();
    invokeMock.mockResolvedValueOnce(POPULAR);
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
      second.resolve(GEMMA);
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
    invokeMock.mockResolvedValue(POPULAR);
    const { result } = renderHook(() => useHfSearch());
    await act(async () => {
      await Promise.resolve();
    });
    invokeMock.mockClear();
    invokeMock.mockResolvedValue(GEMMA);

    act(() => result.current.setQuery('llama'));
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', {
      query: 'llama',
    });
  });
});
