import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import type { ReactNode } from 'react';
import { DownloadProvider, useDownloadCtx } from '../DownloadContext';
import {
  invoke,
  enableChannelCapture,
  resetChannelCapture,
  clearEventHandlers,
} from '../../testUtils/mocks/tauri';
import type { StarterOption } from '../../types/starter';

function option(
  overrides: Partial<StarterOption['starter']> = {},
): StarterOption {
  return {
    starter: {
      tier: 'balanced',
      display_name: 'Balanced',
      repo: 'acme/balanced',
      revision: 'rev',
      file_name: 'weights.gguf',
      sha256: 'sha',
      size_bytes: 8_000_000_000,
      quant: 'Q4_K_M',
      vision: true,
      thinking: false,
      mmproj_file: 'mmproj.gguf',
      mmproj_sha256: 'mmsha',
      mmproj_bytes: 2_000_000_000,
      est_runtime_gb: 10,
      license_note: 'MIT',
      origin: 'Acme',
      origin_repo: 'acme/origin',
      ...overrides,
    },
    fit: 'fits',
    installed: false,
    partial_bytes: null,
  };
}

function wrapper({ children }: { children: ReactNode }) {
  return <DownloadProvider>{children}</DownloadProvider>;
}

describe('DownloadContext', () => {
  beforeEach(() => {
    invoke.mockReset();
    enableChannelCapture();
  });

  afterEach(() => {
    resetChannelCapture();
    clearEventHandlers();
    vi.restoreAllMocks();
  });

  it('throws when useDownloadCtx is called outside a provider', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    expect(() => renderHook(() => useDownloadCtx())).toThrow(
      'useDownloadCtx must be used within a DownloadProvider',
    );
    spy.mockRestore();
  });

  it('exposes the idle download machine with no active download', () => {
    const { result } = renderHook(() => useDownloadCtx(), { wrapper });
    expect(result.current.state).toEqual({ phase: 'idle' });
    expect(result.current.combinedBytes).toBeNull();
    expect(result.current.downloadingTier).toBeNull();
    expect(result.current.resumeSeedBytes).toBeNull();
    expect(result.current.activeOption).toBeNull();
    expect(result.current.grandTotalBytes).toBeNull();
  });

  it('beginDownload records the tier, option, grand total and starts the machine', async () => {
    const { result } = renderHook(() => useDownloadCtx(), { wrapper });
    const opt = option();

    await act(async () => {
      result.current.beginDownload('balanced', opt);
    });

    expect(result.current.downloadingTier).toBe('balanced');
    expect(result.current.activeOption).toBe(opt);
    expect(result.current.resumeSeedBytes).toBeNull();
    // Grand total is weights + vision companion summed.
    expect(result.current.grandTotalBytes).toBe(10_000_000_000);
    expect(result.current.state).toEqual({ phase: 'downloading' });
    expect(invoke).toHaveBeenCalledWith('download_starter', {
      tier: 'balanced',
      onEvent: expect.anything(),
    });
  });

  it('resumeDownload floors the bar at the partial bytes and restarts the machine', async () => {
    const { result } = renderHook(() => useDownloadCtx(), { wrapper });
    const opt = option({
      tier: 'fast',
      size_bytes: 4_000_000_000,
      mmproj_bytes: 0,
    });

    await act(async () => {
      result.current.resumeDownload('fast', opt, 3_000_000_000);
    });

    expect(result.current.downloadingTier).toBe('fast');
    expect(result.current.activeOption).toBe(opt);
    expect(result.current.resumeSeedBytes).toBe(3_000_000_000);
    expect(result.current.grandTotalBytes).toBe(4_000_000_000);
    expect(result.current.state).toEqual({ phase: 'downloading' });
    expect(invoke).toHaveBeenCalledWith('download_starter', {
      tier: 'fast',
      onEvent: expect.anything(),
    });
  });
});
