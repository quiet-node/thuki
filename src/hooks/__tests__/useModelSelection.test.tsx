import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
import { useModelSelection } from '../useModelSelection';
import { invoke } from '../../testUtils/mocks/tauri';

describe('useModelSelection', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('loads active and installed models from the backend', async () => {
    invoke.mockResolvedValueOnce({
      active: 'gemma4:e2b',
      all: ['gemma4:e2b', 'qwen2.5:7b'],
      ollamaReachable: true,
    });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.activeModel).toBe('gemma4:e2b');
    expect(result.current.availableModels).toEqual([
      'gemma4:e2b',
      'qwen2.5:7b',
    ]);
    expect(result.current.ollamaReachable).toBe(true);
  });

  it('starts with a null active model before the first refresh resolves', () => {
    invoke.mockImplementationOnce(() => new Promise<unknown>(() => {}));
    const { result } = renderHook(() => useModelSelection());
    expect(result.current.activeModel).toBeNull();
    expect(result.current.availableModels).toEqual([]);
    // Optimistic default avoids a cold-start flash of the unreachable strip
    // while the first picker fetch is in flight.
    expect(result.current.ollamaReachable).toBe(true);
  });

  it('accepts a null active payload from the backend (no model selected)', async () => {
    // Ollama is the single source of truth: when nothing is installed and
    // nothing is persisted, the backend returns active: null with the
    // reachable flag still true so the strip points at "pull a model"
    // instead of "start Ollama".
    invoke.mockResolvedValueOnce({
      active: null,
      all: [],
      ollamaReachable: true,
    });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.activeModel).toBeNull();
    expect(result.current.availableModels).toEqual([]);
    expect(result.current.ollamaReachable).toBe(true);
  });

  it('marks Ollama unreachable when the backend reports it cannot connect', async () => {
    // S1: backend collapses a transport failure into a structured payload
    // so the hook can surface ollamaReachable=false without parsing error
    // strings.
    invoke.mockResolvedValueOnce({
      active: null,
      all: [],
      ollamaReachable: false,
    });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.activeModel).toBeNull();
    expect(result.current.availableModels).toEqual([]);
    expect(result.current.ollamaReachable).toBe(false);
  });

  it('persists a new active model and updates local state', async () => {
    invoke
      .mockResolvedValueOnce({
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      })
      .mockResolvedValueOnce(undefined);

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    await act(async () => {
      await result.current.setActiveModel('qwen2.5:7b');
    });

    expect(invoke).toHaveBeenCalledWith('set_active_model', {
      model: 'qwen2.5:7b',
    });
    expect(result.current.activeModel).toBe('qwen2.5:7b');
  });

  it('clears available models and marks unreachable when backend fetch rejects', async () => {
    invoke.mockRejectedValueOnce(new Error('backend offline'));

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.activeModel).toBeNull();
    // A rejected IPC call is treated as unreachable: we cannot trust any
    // field, so route the user toward starting Ollama rather than pulling.
    expect(result.current.ollamaReachable).toBe(false);
  });

  it('falls back to empty state when payload shape is invalid', async () => {
    invoke.mockResolvedValueOnce({ active: 42, all: 'not-an-array' });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.activeModel).toBeNull();
    // A malformed payload is also treated as unreachable: the hook cannot
    // tell whether the daemon is healthy, so the safe default mirrors the
    // rejection branch.
    expect(result.current.ollamaReachable).toBe(false);
  });

  it('re-fetches models when refreshModels is called', async () => {
    invoke
      .mockResolvedValueOnce({
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      })
      .mockResolvedValueOnce({
        active: 'qwen2.5:7b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    await act(async () => {
      await result.current.refreshModels();
    });

    expect(result.current.activeModel).toBe('qwen2.5:7b');
    expect(result.current.availableModels).toEqual([
      'gemma4:e2b',
      'qwen2.5:7b',
    ]);
  });

  it('rejects null payloads from the backend', async () => {
    invoke.mockResolvedValueOnce(null);

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.activeModel).toBeNull();
  });

  it('rejects non-object payloads from the backend', async () => {
    invoke.mockResolvedValueOnce('gemma4:e2b');

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.activeModel).toBeNull();
  });

  it('rejects payloads whose `all` array contains non-string entries', async () => {
    invoke.mockResolvedValueOnce({
      active: 'gemma4:e2b',
      all: ['ok', 7],
      ollamaReachable: true,
    });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.activeModel).toBeNull();
  });

  it('rejects payloads with non-boolean ollamaReachable', async () => {
    // Defense-in-depth: the backend always emits a boolean, but the guard
    // keeps the hook robust against shape drift in legacy builds or
    // mocks.
    invoke.mockResolvedValueOnce({
      active: 'gemma4:e2b',
      all: ['gemma4:e2b'],
      ollamaReachable: 'yes',
    });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.activeModel).toBeNull();
    expect(result.current.ollamaReachable).toBe(false);
  });

  it('surfaces backend errors and leaves active model unchanged on rejection', async () => {
    invoke
      .mockResolvedValueOnce({
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      })
      .mockRejectedValueOnce(
        new Error('Model is not installed in Ollama: mystery'),
      );

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    await expect(
      act(async () => {
        await result.current.setActiveModel('mystery');
      }),
    ).rejects.toThrow('Model is not installed in Ollama: mystery');

    expect(result.current.activeModel).toBe('gemma4:e2b');
  });

  it('clears active model when a later refresh returns a malformed payload', async () => {
    invoke
      .mockResolvedValueOnce({
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      })
      .mockResolvedValueOnce({ active: 42, all: 'not-an-array' });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});
    expect(result.current.activeModel).toBe('gemma4:e2b');

    await act(async () => {
      await result.current.refreshModels();
    });

    expect(result.current.activeModel).toBeNull();
    expect(result.current.availableModels).toEqual([]);
  });

  it('drops a stale setActiveModel resolution when a newer call supersedes it', async () => {
    invoke.mockResolvedValueOnce({
      active: 'A',
      all: ['A', 'B', 'C'],
      ollamaReachable: true,
    });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    let resolveSlow!: () => void;
    invoke
      .mockImplementationOnce(
        () =>
          new Promise<void>((r) => {
            resolveSlow = () => r();
          }),
      )
      .mockResolvedValueOnce(undefined);

    let slowPromise: Promise<void>;
    await act(async () => {
      slowPromise = result.current.setActiveModel('B');
      await result.current.setActiveModel('C');
    });

    // "C" wins because it was the latest call; "B"'s pending promise must be
    // a silent no-op when it finally resolves.
    expect(result.current.activeModel).toBe('C');

    await act(async () => {
      resolveSlow();
      await slowPromise;
    });

    expect(result.current.activeModel).toBe('C');
  });

  it('drops a stale setActiveModel rejection when a newer call supersedes it', async () => {
    invoke.mockResolvedValueOnce({
      active: 'A',
      all: ['A', 'B', 'C'],
      ollamaReachable: true,
    });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    let rejectSlow!: (err: unknown) => void;
    invoke
      .mockImplementationOnce(
        () =>
          new Promise<void>((_resolve, reject) => {
            rejectSlow = reject;
          }),
      )
      .mockResolvedValueOnce(undefined);

    let slowPromise: Promise<void>;
    await act(async () => {
      slowPromise = result.current.setActiveModel('B');
      await result.current.setActiveModel('C');
    });

    expect(result.current.activeModel).toBe('C');

    // The stale rejection must not bubble up to callers or revert state.
    await act(async () => {
      rejectSlow(new Error('stale'));
      await slowPromise;
    });

    expect(result.current.activeModel).toBe('C');
  });

  it('drops a late refresh resolution after unmount', async () => {
    let resolveLate!: (value: unknown) => void;
    invoke.mockImplementationOnce(
      () =>
        new Promise<unknown>((resolve) => {
          resolveLate = resolve;
        }),
    );

    const { unmount } = renderHook(() => useModelSelection());
    unmount();

    // Resolving after unmount would setState on an unmounted component without
    // the mounted guard, producing a React warning / test failure.
    await act(async () => {
      resolveLate({ active: 'A', all: ['A'], ollamaReachable: true });
    });
  });

  it('drops a late refresh rejection after unmount', async () => {
    let rejectLate!: (err: unknown) => void;
    invoke.mockImplementationOnce(
      () =>
        new Promise<unknown>((_resolve, reject) => {
          rejectLate = reject;
        }),
    );

    const { unmount } = renderHook(() => useModelSelection());
    unmount();

    // Same shape as the late-resolve test but exercises the catch branch of
    // refreshModels so the post-unmount guard is covered in both arms.
    await act(async () => {
      rejectLate(new Error('late'));
    });
  });
});
