import { renderHook, act, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import {
  __emitFocus,
  __resetFocusListeners,
} from '../../testUtils/mocks/tauri-window';
import { useConfigSync } from './useConfigSync';
import type { RawAppConfig } from '../types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const CONFIG_A: RawAppConfig = {
  inference: { ollama_url: 'http://127.0.0.1:11434' },
  prompt: { system: '' },
  window: {
    overlay_width: 600,
    max_chat_height: 648,
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
};

const CONFIG_B: RawAppConfig = {
  ...CONFIG_A,
  inference: { ...CONFIG_A.inference, ollama_url: 'http://10.0.0.1:11434' },
};

beforeEach(() => {
  invokeMock.mockReset();
  __resetFocusListeners();
});

afterEach(() => {
  __resetFocusListeners();
});

describe('useConfigSync', () => {
  it('returns null until the initial get_config resolves', async () => {
    invokeMock.mockResolvedValue(CONFIG_A);
    const { result } = renderHook(() => useConfigSync());
    expect(result.current.config).toBeNull();
    await waitFor(() => expect(result.current.config).toEqual(CONFIG_A));
  });

  it('reload re-invokes reload_config_from_disk and replaces local state', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_config') return CONFIG_A;
      if (cmd === 'reload_config_from_disk') return CONFIG_B;
      return undefined;
    });

    const { result } = renderHook(() => useConfigSync());
    await waitFor(() => expect(result.current.config).toEqual(CONFIG_A));

    await act(async () => {
      await result.current.reload();
    });
    expect(result.current.config).toEqual(CONFIG_B);
  });

  it('setConfig replaces local state without an IPC call', async () => {
    invokeMock.mockResolvedValue(CONFIG_A);
    const { result } = renderHook(() => useConfigSync());
    await waitFor(() => expect(result.current.config).toEqual(CONFIG_A));

    invokeMock.mockClear();
    act(() => {
      result.current.setConfig(CONFIG_B);
    });
    expect(result.current.config).toEqual(CONFIG_B);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('reloads on focus event', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_config') return CONFIG_A;
      if (cmd === 'reload_config_from_disk') return CONFIG_B;
      return undefined;
    });

    const { result } = renderHook(() => useConfigSync());
    await waitFor(() => expect(result.current.config).toEqual(CONFIG_A));

    await act(async () => {
      __emitFocus(true);
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() => expect(result.current.config).toEqual(CONFIG_B));
  });

  it('does not reload on blur (focused: false)', async () => {
    invokeMock.mockResolvedValue(CONFIG_A);
    const { result } = renderHook(() => useConfigSync());
    await waitFor(() => expect(result.current.config).toEqual(CONFIG_A));

    invokeMock.mockClear();
    await act(async () => {
      __emitFocus(false);
      await Promise.resolve();
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('swallows reload errors and keeps the previous snapshot', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_config') return CONFIG_A;
      throw new Error('boom');
    });

    const { result } = renderHook(() => useConfigSync());
    await waitFor(() => expect(result.current.config).toEqual(CONFIG_A));

    await act(async () => {
      await result.current.reload();
    });
    expect(result.current.config).toEqual(CONFIG_A);
  });

  it('drops the late initial fetch when the hook unmounts first', async () => {
    let resolveGetConfig: ((value: RawAppConfig) => void) | undefined;
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<RawAppConfig>((resolve) => {
          resolveGetConfig = resolve;
        }),
    );
    const { result, unmount } = renderHook(() => useConfigSync());
    expect(result.current.config).toBeNull();

    unmount();
    await act(async () => {
      resolveGetConfig?.(CONFIG_A);
      await Promise.resolve();
    });
    // No assertion error; we are just exercising the `if (mounted)` guard.
  });

  it('cleans up the focus listener on unmount', async () => {
    invokeMock.mockResolvedValue(CONFIG_A);
    const { result, unmount } = renderHook(() => useConfigSync());
    await waitFor(() => expect(result.current.config).toEqual(CONFIG_A));

    unmount();
    invokeMock.mockClear();
    __emitFocus(true);
    // Listener was removed; no further reload invokes.
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
