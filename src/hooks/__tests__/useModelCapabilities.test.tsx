import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
import { useModelCapabilities } from '../useModelCapabilities';
import { invoke } from '../../testUtils/mocks/tauri';

const FULL = {
  vision: true,
  thinking: true,
};

const TEXT_ONLY = {
  vision: false,
  thinking: false,
};

describe('useModelCapabilities', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('loads the capability map from the backend', async () => {
    invoke.mockResolvedValueOnce({
      'llama3.2-vision': FULL,
      llama3: TEXT_ONLY,
    });
    const { result } = renderHook(() => useModelCapabilities());
    await act(async () => {});
    expect(result.current.capabilities).toEqual({
      'llama3.2-vision': FULL,
      llama3: TEXT_ONLY,
    });
  });

  it('clears state on backend reject', async () => {
    invoke.mockRejectedValueOnce(new Error('backend offline'));
    const { result } = renderHook(() => useModelCapabilities());
    await act(async () => {});
    expect(result.current.capabilities).toEqual({});
  });

  it('clears state when payload is not an object', async () => {
    invoke.mockResolvedValueOnce('not-a-map');
    const { result } = renderHook(() => useModelCapabilities());
    await act(async () => {});
    expect(result.current.capabilities).toEqual({});
  });

  it('clears state when payload is null', async () => {
    invoke.mockResolvedValueOnce(null);
    const { result } = renderHook(() => useModelCapabilities());
    await act(async () => {});
    expect(result.current.capabilities).toEqual({});
  });

  it('clears state when an entry has the wrong shape', async () => {
    invoke.mockResolvedValueOnce({
      llama3: {
        vision: 'yes',
        thinking: false,
      },
    });
    const { result } = renderHook(() => useModelCapabilities());
    await act(async () => {});
    expect(result.current.capabilities).toEqual({});
  });

  it('clears state when an entry is null', async () => {
    invoke.mockResolvedValueOnce({ llama3: null });
    const { result } = renderHook(() => useModelCapabilities());
    await act(async () => {});
    expect(result.current.capabilities).toEqual({});
  });

  it('refresh re-fetches the map', async () => {
    invoke
      .mockResolvedValueOnce({ a: TEXT_ONLY })
      .mockResolvedValueOnce({ a: TEXT_ONLY, b: FULL });
    const { result } = renderHook(() => useModelCapabilities());
    await act(async () => {});
    expect(result.current.capabilities).toEqual({ a: TEXT_ONLY });
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.capabilities).toEqual({ a: TEXT_ONLY, b: FULL });
  });

  it('drops a stale resolution from a superseded fetch', async () => {
    // Symmetric to the rejection case: a hanging first fetch eventually
    // resolves AFTER a second refresh has already bumped the token. The
    // late resolution must short-circuit at the isLatest check so it does
    // not overwrite the newer state.
    let resolveFirst: (val: Record<string, typeof FULL>) => void = () => {};
    const firstPromise = new Promise<Record<string, typeof FULL>>((resolve) => {
      resolveFirst = resolve;
    });
    invoke.mockReturnValueOnce(firstPromise).mockResolvedValueOnce({ b: FULL });

    const { result } = renderHook(() => useModelCapabilities());
    await act(async () => {});
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.capabilities).toEqual({ b: FULL });
    await act(async () => {
      resolveFirst({ stale: TEXT_ONLY });
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(result.current.capabilities).toEqual({ b: FULL });
  });

  it('drops a stale rejection from a superseded fetch', async () => {
    // First mount-call hangs and is later rejected. A second refresh in
    // the meantime resolves successfully and bumps the token. The first
    // call's late rejection must be ignored so the resolved state from
    // the second call is preserved.
    let rejectFirst: (err: Error) => void = () => {};
    const firstPromise = new Promise((_, reject) => {
      rejectFirst = reject;
    });
    invoke.mockReturnValueOnce(firstPromise).mockResolvedValueOnce({ b: FULL });

    const { result } = renderHook(() => useModelCapabilities());
    await act(async () => {});
    // Kick off a second refresh that supersedes the first.
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.capabilities).toEqual({ b: FULL });
    // Now reject the first hanging call. Its catch must short-circuit
    // because the token is stale and so leave state untouched.
    await act(async () => {
      rejectFirst(new Error('late'));
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(result.current.capabilities).toEqual({ b: FULL });
  });
});
