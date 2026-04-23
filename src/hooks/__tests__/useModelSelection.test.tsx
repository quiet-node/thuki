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
  });
});
