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
    });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.activeModel).toBe('gemma4:e2b');
    expect(result.current.availableModels).toEqual([
      'gemma4:e2b',
      'qwen2.5:7b',
    ]);
  });

  it('persists a new active model and updates local state', async () => {
    invoke
      .mockResolvedValueOnce({
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
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

  it('clears available models when backend fetch fails', async () => {
    invoke.mockRejectedValueOnce(new Error('backend offline'));

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.activeModel).toBe('');
  });

  it('falls back to empty state when payload shape is invalid', async () => {
    invoke.mockResolvedValueOnce({ active: 42, all: 'not-an-array' });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.activeModel).toBe('');
  });

  it('re-fetches models when refreshModels is called', async () => {
    invoke
      .mockResolvedValueOnce({ active: 'gemma4:e2b', all: ['gemma4:e2b'] })
      .mockResolvedValueOnce({
        active: 'qwen2.5:7b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
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
    expect(result.current.activeModel).toBe('');
  });

  it('rejects non-object payloads from the backend', async () => {
    invoke.mockResolvedValueOnce('gemma4:e2b');

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.activeModel).toBe('');
  });

  it('rejects payloads whose `all` array contains non-string entries', async () => {
    invoke.mockResolvedValueOnce({ active: 'gemma4:e2b', all: ['ok', 7] });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.activeModel).toBe('');
  });

  it('surfaces backend errors and leaves active model unchanged on rejection', async () => {
    invoke
      .mockResolvedValueOnce({
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
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
      })
      .mockResolvedValueOnce({ active: 42, all: 'not-an-array' });

    const { result } = renderHook(() => useModelSelection());
    await act(async () => {});
    expect(result.current.activeModel).toBe('gemma4:e2b');

    await act(async () => {
      await result.current.refreshModels();
    });

    expect(result.current.activeModel).toBe('');
    expect(result.current.availableModels).toEqual([]);
  });

  it('drops a stale setActiveModel resolution when a newer call supersedes it', async () => {
    invoke.mockResolvedValueOnce({
      active: 'A',
      all: ['A', 'B', 'C'],
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
      resolveLate({ active: 'A', all: ['A'] });
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
