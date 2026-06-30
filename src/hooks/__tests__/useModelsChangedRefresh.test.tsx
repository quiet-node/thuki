import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  listen,
  emitTauriEvent,
  clearEventHandlers,
} from '../../testUtils/mocks/tauri';
import {
  useModelsChangedRefresh,
  MODELS_CHANGED_EVENT,
} from '../useModelsChangedRefresh';

describe('useModelsChangedRefresh', () => {
  beforeEach(() => {
    listen.mockClear();
    clearEventHandlers();
  });

  it('runs refresh when the models-changed event fires', async () => {
    const refresh = vi.fn();
    renderHook(() => useModelsChangedRefresh(refresh));
    await act(async () => {});
    expect(refresh).not.toHaveBeenCalled();

    await act(async () => {
      emitTauriEvent(MODELS_CHANGED_EVENT, null);
    });
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it('stops running refresh after unmount', async () => {
    const refresh = vi.fn();
    const { unmount } = renderHook(() => useModelsChangedRefresh(refresh));
    await act(async () => {});

    unmount();
    await act(async () => {
      emitTauriEvent(MODELS_CHANGED_EVENT, null);
    });
    expect(refresh).not.toHaveBeenCalled();
  });

  it('survives a listen rejection without crashing', async () => {
    listen.mockRejectedValueOnce(new Error('event bridge missing'));
    const refresh = vi.fn();
    renderHook(() => useModelsChangedRefresh(refresh));
    await act(async () => {});
    expect(refresh).not.toHaveBeenCalled();
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

    const { unmount } = renderHook(() => useModelsChangedRefresh(vi.fn()));
    unmount();

    await act(async () => {
      resolveListen(unlistenSpy);
    });

    expect(unlistenSpy).toHaveBeenCalledTimes(1);
  });
});
