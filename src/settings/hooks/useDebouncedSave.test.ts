import { renderHook, act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import { useDebouncedSave } from './useDebouncedSave';
import type { ConfigError, RawAppConfig } from '../types';

// Cast `invoke` to the mocked vi.fn so tests can stub return values.
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const SAMPLE_CONFIG: RawAppConfig = {
  inference: {
    ollama_url: 'http://127.0.0.1:11434',
    keep_warm_inactivity_minutes: 0,
    num_ctx: 16384,
  },
  prompt: { system: '' },
  window: {
    overlay_width: 600,
    max_chat_height: 648,
    max_images: 3,
  },
  quote: {
    max_display_lines: 4,
    max_display_chars: 300,
    max_context_length: 4096,
  },
  search: {
    searxng_url: 'http://127.0.0.1:25017',
    reader_url: 'http://127.0.0.1:25018',
    max_iterations: 3,
    top_k_urls: 10,
    searxng_max_results: 10,
    search_timeout_s: 20,
    reader_per_url_timeout_s: 10,
    reader_batch_timeout_s: 30,
    judge_timeout_s: 30,
    router_timeout_s: 45,
  },
  debug: {
    trace_enabled: false,
  },
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(SAMPLE_CONFIG);
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useDebouncedSave', () => {
  it('does not save the seed value on mount (idempotent under StrictMode)', () => {
    renderHook(() =>
      useDebouncedSave('window', 'overlay_width', 600, { onSaved: vi.fn() }),
    );
    act(() => {
      vi.runAllTimers();
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('schedules a save when the value changes and fires after the delay', async () => {
    const onSaved = vi.fn();
    const { rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, {
          onSaved,
          delayMs: 100,
        }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });

    expect(invokeMock).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(100);
      // Two microtask ticks: one for the timer's awaited invoke promise,
      // one for the post-await `onSaved` callback.
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(invokeMock).toHaveBeenCalledWith('set_config_field', {
      section: 'window',
      key: 'overlay_width',
      value: 700,
    });
    expect(onSaved).toHaveBeenCalledWith(SAMPLE_CONFIG);
  });

  it('does not re-save the same value twice in a row', async () => {
    const { rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 50 }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });
    await act(async () => {
      vi.advanceTimersByTime(50);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);

    // Second render with the same value: lastSavedRef === value, no save.
    rerender({ v: 700 });
    await act(async () => {
      vi.advanceTimersByTime(50);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('fires save again when value changes after a previous save', async () => {
    const { rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 50 }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });
    await act(async () => {
      vi.advanceTimersByTime(50);
      await Promise.resolve();
    });
    rerender({ v: 800 });
    await act(async () => {
      vi.advanceTimersByTime(50);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledTimes(2);
  });

  it('coalesces rapid changes within the debounce window into a single save', async () => {
    const { rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 100 }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });
    rerender({ v: 750 });
    rerender({ v: 800 });
    await act(async () => {
      vi.advanceTimersByTime(100);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock.mock.calls[0][1]).toEqual({
      section: 'window',
      key: 'overlay_width',
      value: 800,
    });
  });

  it('exposes the typed error when set_config_field rejects', async () => {
    const failure: ConfigError = {
      kind: 'type_mismatch',
      section: 'window',
      key: 'overlay_width',
      message: 'expected integer',
    };
    invokeMock.mockRejectedValueOnce(failure);

    const { result, rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 10 }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });
    await act(async () => {
      vi.advanceTimersByTime(10);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(result.current.error).toEqual(failure);
  });

  it('flushNow forces an immediate save and returns the resolved config', async () => {
    const { result, rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 10_000 }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 999 });
    let next: RawAppConfig | null = null;
    await act(async () => {
      next = await result.current.flushNow();
    });
    expect(next).toEqual(SAMPLE_CONFIG);
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('flushNow with no pending change still calls save', async () => {
    const { result } = renderHook(() =>
      useDebouncedSave('window', 'overlay_width', 600, { delayMs: 10_000 }),
    );
    await act(async () => {
      await result.current.flushNow();
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('flushNow returns null on save failure', async () => {
    invokeMock.mockRejectedValueOnce({
      kind: 'io_error',
      path: '/x',
    } as ConfigError);
    const { result } = renderHook(() =>
      useDebouncedSave('window', 'overlay_width', 600),
    );
    let next: RawAppConfig | null | undefined;
    await act(async () => {
      next = await result.current.flushNow();
    });
    expect(next).toBeNull();
  });

  it('resetTo updates the baseline so subsequent identical changes are no-ops', async () => {
    const { result, rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 10 }),
      { initialProps: { v: 600 } },
    );
    act(() => {
      result.current.resetTo(800);
    });

    // Re-render with the new baseline; the hook should NOT save.
    rerender({ v: 800 });
    await act(async () => {
      vi.advanceTimersByTime(50);
      await Promise.resolve();
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('resetTo cancels a pending debounce timer before it fires', async () => {
    const { result, rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 100 }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });
    // Timer is pending; resetTo must clear it.
    act(() => {
      result.current.resetTo(900);
    });
    await act(async () => {
      vi.advanceTimersByTime(500);
      await Promise.resolve();
    });
    // The pending change was cancelled — no save fires.
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('flushNow with a pending timer cancels it before invoking save', async () => {
    const { result, rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 10_000 }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });
    await act(async () => {
      await result.current.flushNow();
    });
    // Only one save: the flush, not a deferred timer fire.
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('resetTo invalidates an in-flight save (epoch bump)', async () => {
    const onSaved = vi.fn();
    let resolveInvoke: ((value: RawAppConfig) => void) | undefined;
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<RawAppConfig>((resolve) => {
          resolveInvoke = resolve;
        }),
    );

    const { result, rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, {
          onSaved,
          delayMs: 10,
        }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });
    await act(async () => {
      vi.advanceTimersByTime(10);
      await Promise.resolve();
    });

    // Save is in-flight. Resync arrives with a different baseline.
    act(() => {
      result.current.resetTo(900);
    });

    // Resolve the stale invoke. onSaved must NOT fire (epoch mismatch).
    await act(async () => {
      resolveInvoke?.(SAMPLE_CONFIG);
      await Promise.resolve();
    });

    expect(onSaved).not.toHaveBeenCalled();
  });

  it('clears any pending timer on unmount and flushes the latest change', async () => {
    const onSaved = vi.fn();
    const { rerender, unmount } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, {
          onSaved,
          delayMs: 10_000,
        }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });
    expect(invokeMock).not.toHaveBeenCalled();

    await act(async () => {
      unmount();
      await Promise.resolve();
    });

    // Unmount cleanup flushes the pending change so the user's last edit
    // is not dropped on tab switch.
    expect(invokeMock).toHaveBeenCalledWith('set_config_field', {
      section: 'window',
      key: 'overlay_width',
      value: 700,
    });
  });

  it('unmount with no pending change does not save', async () => {
    const { unmount } = renderHook(() =>
      useDebouncedSave('window', 'overlay_width', 600),
    );
    await act(async () => {
      unmount();
      await Promise.resolve();
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('treats string[] arrays as equal element-wise (no spurious saves on stable list)', async () => {
    const { rerender } = renderHook(
      ({ v }) => useDebouncedSave('inference', 'available', v, { delayMs: 10 }),
      { initialProps: { v: ['gemma:2b'] } },
    );
    // Pass a NEW array reference with the same contents.
    rerender({ v: ['gemma:2b'] });
    await act(async () => {
      vi.advanceTimersByTime(50);
      await Promise.resolve();
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('saves when an array element changes', async () => {
    const { rerender } = renderHook(
      ({ v }) => useDebouncedSave('inference', 'available', v, { delayMs: 10 }),
      { initialProps: { v: ['gemma:2b'] } },
    );
    rerender({ v: ['gemma:2b', 'qwen3:8b'] });
    await act(async () => {
      vi.advanceTimersByTime(10);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('saves when an array contains a different scalar at the same index', async () => {
    const { rerender } = renderHook(
      ({ v }) => useDebouncedSave('inference', 'available', v, { delayMs: 10 }),
      { initialProps: { v: ['a', 'b'] } },
    );
    rerender({ v: ['a', 'c'] });
    await act(async () => {
      vi.advanceTimersByTime(10);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('discards a stale rejection that arrives after resetTo (epoch mismatch in catch)', async () => {
    let rejectInvoke: ((reason: unknown) => void) | undefined;
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<RawAppConfig>((_, reject) => {
          rejectInvoke = reject;
        }),
    );

    const { result, rerender } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 10 }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });
    await act(async () => {
      vi.advanceTimersByTime(10);
      await Promise.resolve();
    });

    // Resync arrives mid-await; epoch bumps.
    act(() => {
      result.current.resetTo(900);
    });

    // Reject the stale invoke. The error must NOT surface because the
    // epoch check rejects the post-await branch.
    await act(async () => {
      rejectInvoke?.({ kind: 'io_error', path: '/x' });
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.error).toBeNull();
  });

  it('does not setError after unmount when an invoke rejects later', async () => {
    let rejectInvoke: ((reason: unknown) => void) | undefined;
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<RawAppConfig>((_, reject) => {
          rejectInvoke = reject;
        }),
    );

    const { rerender, unmount } = renderHook(
      ({ v }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 10 }),
      { initialProps: { v: 600 } },
    );
    rerender({ v: 700 });
    await act(async () => {
      vi.advanceTimersByTime(10);
      await Promise.resolve();
    });

    unmount();
    // Reject after unmount. The catch branch must skip setError because
    // isMountedRef.current is false.
    await act(async () => {
      rejectInvoke?.({ kind: 'io_error', path: '/x' });
      await Promise.resolve();
    });
    // Nothing to assert beyond "did not throw and did not warn".
  });

  it('resetTo after unmount does not call setError on the dead hook', async () => {
    const { result, unmount } = renderHook(() =>
      useDebouncedSave('window', 'overlay_width', 600, { delayMs: 10 }),
    );
    const { resetTo } = result.current;
    unmount();
    // Calling resetTo post-unmount must not crash; the isMountedRef guard
    // skips the setError call.
    expect(() => resetTo(900)).not.toThrow();
  });

  it('non-equal scalar vs array is also treated as a change', async () => {
    // Coverage: areEqual fallthrough when one side is an array, the other
    // is not (Object.is fails AND only one is Array). The hook is generic
    // enough to accept the union via `unknown`.
    type V = number | number[];
    const { rerender } = renderHook(
      ({ v }: { v: V }) =>
        useDebouncedSave('window', 'overlay_width', v, { delayMs: 10 }),
      { initialProps: { v: 1 as V } },
    );
    rerender({ v: [1] });
    await act(async () => {
      vi.advanceTimersByTime(10);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });
});
