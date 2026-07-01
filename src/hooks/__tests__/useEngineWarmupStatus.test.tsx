import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  listen,
  emitTauriEvent,
  clearEventHandlers,
} from '../../testUtils/mocks/tauri';
import { useEngineWarmupStatus } from '../useEngineWarmupStatus';

describe('useEngineWarmupStatus', () => {
  beforeEach(() => {
    listen.mockClear();
    clearEventHandlers();
  });

  it('starts not warming', async () => {
    const { result } = renderHook(() => useEngineWarmupStatus());
    await act(async () => {});
    expect(result.current.warming).toBe(false);
  });

  it('flips to warming on warmup:builtin-warming', async () => {
    const { result } = renderHook(() => useEngineWarmupStatus());
    await act(async () => {});

    await act(async () => {
      emitTauriEvent('warmup:builtin-warming', null);
    });
    expect(result.current.warming).toBe(true);
  });

  it('flips back on warmup:builtin-warmed', async () => {
    const { result } = renderHook(() => useEngineWarmupStatus());
    await act(async () => {});
    await act(async () => {
      emitTauriEvent('warmup:builtin-warming', null);
    });
    expect(result.current.warming).toBe(true);

    await act(async () => {
      emitTauriEvent('warmup:builtin-warmed', null);
    });
    expect(result.current.warming).toBe(false);
  });

  it('stops updating after unmount', async () => {
    const { result, unmount } = renderHook(() => useEngineWarmupStatus());
    await act(async () => {});
    unmount();

    await act(async () => {
      emitTauriEvent('warmup:builtin-warming', null);
    });
    expect(result.current.warming).toBe(false);
  });

  it('survives a listen rejection without crashing', async () => {
    listen.mockRejectedValueOnce(new Error('event bridge missing'));
    const { result } = renderHook(() => useEngineWarmupStatus());
    await act(async () => {});
    expect(result.current.warming).toBe(false);
  });

  it('drops a late-arriving subscription after unmount', async () => {
    let resolveListen!: (fn: () => void) => void;
    const unlistenSpy = vi.fn();
    listen.mockImplementationOnce(
      () =>
        new Promise<() => void>((resolve) => {
          resolveListen = resolve;
        }),
    );

    const { unmount } = renderHook(() => useEngineWarmupStatus());
    unmount();

    await act(async () => {
      resolveListen(unlistenSpy);
    });

    expect(unlistenSpy).toHaveBeenCalledTimes(1);
  });
});
