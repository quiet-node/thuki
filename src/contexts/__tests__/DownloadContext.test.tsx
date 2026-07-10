import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import type { ReactNode } from 'react';
import { DownloadProvider, useDownloadCtx } from '../DownloadContext';
import { ConfigProviderForTest, DEFAULT_CONFIG } from '../ConfigContext';
import {
  invoke,
  enableChannelCapture,
  enableChannelCaptureWithResponses,
  getLastChannel,
  resetChannelCapture,
  clearEventHandlers,
  type Channel,
} from '../../testUtils/mocks/tauri';
import type { DownloadEvent, StarterOption } from '../../types/starter';

/** The captured download channel, typed for simulateMessage calls. */
function channel(): Channel<DownloadEvent> {
  return getLastChannel() as Channel<DownloadEvent>;
}

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

/** AppConfig whose active provider is the bundled built-in engine. */
const BUILTIN_CONFIG = {
  ...DEFAULT_CONFIG,
  inference: {
    ...DEFAULT_CONFIG.inference,
    activeProvider: 'builtin',
    activeProviderKind: 'builtin',
  },
};

/** Provider tree with the built-in engine active. */
function builtinWrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProviderForTest value={BUILTIN_CONFIG}>
      <DownloadProvider>{children}</DownloadProvider>
    </ConfigProviderForTest>
  );
}

/** Counts how many times `invoke` was called for a given command. */
function invokeCount(command: string): number {
  return invoke.mock.calls.filter((c) => c[0] === command).length;
}

/** Stub the launch probes: the persisted onboarding stage and the starters. */
function mockLaunch(stage: string, options: StarterOption[] = []) {
  invoke.mockImplementation((cmd) => {
    if (cmd === 'onboarding_stage') return Promise.resolve(stage);
    if (cmd === 'get_starter_options') return Promise.resolve(options);
    return Promise.resolve();
  });
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
      key: 'tier:balanced',
      userInitiated: true,
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
      key: 'tier:fast',
      userInitiated: true,
      onEvent: expect.anything(),
    });
  });

  it('pauseDownload remembers the bytes so far and cancels the run', async () => {
    const { result } = renderHook(() => useDownloadCtx(), { wrapper });
    const opt = option();

    await act(async () => {
      result.current.beginDownload('balanced', opt);
    });
    act(() =>
      channel().simulateMessage({
        type: 'Started',
        data: { file: 'weights.gguf', total_bytes: 100, resumed_from: 0 },
      }),
    );
    act(() =>
      channel().simulateMessage({
        type: 'Progress',
        data: { file: 'weights.gguf', bytes: 60, total_bytes: 100 },
      }),
    );

    await act(async () => {
      result.current.pauseDownload();
    });

    // Cancel fired and the bytes were captured. The pause is NOT committed
    // until the backend Cancelled lands (slot released) so a resume cannot
    // race; meanwhile `isPausing` is true for instant "Pausing…" feedback.
    expect(result.current.pausedBytes).toBe(60);
    expect(invoke).toHaveBeenCalledWith('cancel_model_download', {
      key: 'tier:balanced',
    });
    expect(result.current.isPaused).toBe(false);
    expect(result.current.isPausing).toBe(true);

    act(() => channel().simulateMessage({ type: 'Cancelled' }));
    expect(result.current.isPaused).toBe(true);
    expect(result.current.isPausing).toBe(false);

    // The paused state is reported to the backend so Cmd+Q warns while paused.
    expect(invoke).toHaveBeenCalledWith('set_download_paused', {
      paused: true,
    });
  });

  it('pauseDownload defaults to zero bytes before the first event arrives', async () => {
    const { result } = renderHook(() => useDownloadCtx(), { wrapper });

    await act(async () => {
      result.current.beginDownload('balanced', option());
    });
    await act(async () => {
      result.current.pauseDownload();
    });
    act(() => channel().simulateMessage({ type: 'Cancelled' }));

    expect(result.current.isPaused).toBe(true);
    expect(result.current.pausedBytes).toBe(0);
  });

  it('resumeFromPause restarts the download and clears the paused flag', async () => {
    const { result } = renderHook(() => useDownloadCtx(), { wrapper });
    const opt = option();

    await act(async () => {
      result.current.beginDownload('balanced', opt);
    });
    await act(async () => {
      result.current.pauseDownload();
    });
    act(() => channel().simulateMessage({ type: 'Cancelled' }));
    expect(result.current.isPaused).toBe(true);

    await act(async () => {
      result.current.resumeFromPause();
    });

    expect(result.current.isPaused).toBe(false);
    expect(result.current.downloadingTier).toBe('balanced');
    expect(
      invoke.mock.calls.filter((c) => c[0] === 'download_starter'),
    ).toHaveLength(2);
    // Resume is a real click: userInitiated stays true, unlike the
    // launch-time auto-resume. (Not toHaveBeenLastCalledWith: the pause's
    // set_download_paused effect can commit after this invoke.)
    expect(invoke).toHaveBeenCalledWith('download_starter', {
      tier: 'balanced',
      key: 'tier:balanced',
      userInitiated: true,
      onEvent: expect.anything(),
    });
  });

  it('shows Paused from the on-disk partial when paused from another window', async () => {
    // The cross-window case: this window started the download, another window
    // (Settings) paused it. The backend cancels this window's task, so Cancelled
    // lands here and the machine goes idle WITHOUT a local pauseDownload
    // (pauseRequested stays false). The strip must still read Paused, derived
    // from the partial that get_starter_options now reports for the active model.
    const opt = option({ tier: 'balanced' });
    enableChannelCaptureWithResponses({
      get_starter_options: [{ ...opt, partial_bytes: 4_000_000_000 }],
    });

    const { result } = renderHook(() => useDownloadCtx(), { wrapper });
    await act(async () => {}); // useStarterOptions mount fetch resolves

    await act(async () => {
      result.current.beginDownload('balanced', opt);
    });
    // Remote pause: the task is cancelled elsewhere; only the Cancelled event
    // reaches this window. No local pauseDownload was called.
    act(() => channel().simulateMessage({ type: 'Cancelled' }));

    expect(result.current.isPaused).toBe(true);
    expect(result.current.pausedBytes).toBe(4_000_000_000);
  });

  it('does not read Paused for an installed model even if a partial lingers', async () => {
    // Guard mirroring Settings (`!installed`): an installed model is never
    // shown as a resumable paused download, even in the unlikely event a partial
    // is still reported alongside it.
    const opt = option({ tier: 'balanced' });
    enableChannelCaptureWithResponses({
      get_starter_options: [
        { ...opt, partial_bytes: 4_000_000_000, installed: true },
      ],
    });

    const { result } = renderHook(() => useDownloadCtx(), { wrapper });
    await act(async () => {});

    await act(async () => {
      result.current.beginDownload('balanced', opt);
    });
    act(() => channel().simulateMessage({ type: 'Cancelled' }));

    expect(result.current.isPaused).toBe(false);
  });

  it('discardDownload deletes both partials and clears the ambient strip', async () => {
    const { result } = renderHook(() => useDownloadCtx(), { wrapper });
    const opt = option();

    await act(async () => {
      result.current.beginDownload('balanced', opt);
    });
    await act(async () => {
      result.current.pauseDownload();
    });
    act(() => channel().simulateMessage({ type: 'Cancelled' }));
    expect(result.current.isPaused).toBe(true);

    await act(async () => {
      result.current.discardDownload();
    });
    // Flush the discard IIFE so the second (mmproj) delete is recorded.
    await act(async () => {
      await Promise.resolve();
    });

    // Both blobs' partials are deleted from disk.
    expect(invoke).toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'sha',
    });
    expect(invoke).toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'mmsha',
    });
    // The strip is cleared: no paused state, no active option, idle machine.
    expect(result.current.isPaused).toBe(false);
    expect(result.current.activeOption).toBeNull();
    expect(result.current.downloadingTier).toBeNull();
    expect(result.current.grandTotalBytes).toBeNull();
  });

  it('discardDownload deletes only the weights partial for a text-only model', async () => {
    const { result } = renderHook(() => useDownloadCtx(), { wrapper });
    const opt = option({
      mmproj_file: null,
      mmproj_sha256: null,
      mmproj_bytes: 0,
    });

    await act(async () => {
      result.current.beginDownload('fast', opt);
    });
    await act(async () => {
      result.current.pauseDownload();
    });
    act(() => channel().simulateMessage({ type: 'Cancelled' }));

    await act(async () => {
      result.current.discardDownload();
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(invoke).toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'sha',
    });
    expect(invoke).not.toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'mmsha',
    });
    expect(result.current.activeOption).toBeNull();
  });

  describe('launch auto-resume', () => {
    /** Flush the multi-await auto-resume IIFE (stage, options, discards). */
    async function flushLaunch() {
      for (let i = 0; i < 6; i++) {
        await act(async () => {
          await Promise.resolve();
        });
      }
    }

    it('discards an interrupted partial and downloads fresh past the picker', async () => {
      const partial: StarterOption = {
        ...option({ tier: 'fast' }),
        partial_bytes: 3_000_000_000,
      };
      mockLaunch('intro', [partial]);

      const { result } = renderHook(() => useDownloadCtx(), {
        wrapper: builtinWrapper,
      });
      await flushLaunch();

      expect(invokeCount('onboarding_stage')).toBe(1);
      // The unreliable cold-resume is skipped: both blobs' partials are
      // discarded and a fresh download starts (no resume seed).
      expect(invoke).toHaveBeenCalledWith('discard_partial_download', {
        sha256: 'sha',
      });
      expect(invoke).toHaveBeenCalledWith('discard_partial_download', {
        sha256: 'mmsha',
      });
      expect(result.current.downloadingTier).toBe('fast');
      expect(result.current.resumeSeedBytes).toBeNull();
      expect(result.current.state).toEqual({ phase: 'downloading' });
      // Not a user click: the launch-time auto-resume sends
      // userInitiated: false so a safe-mode session refuses it (issue #296).
      expect(invoke).toHaveBeenCalledWith('download_starter', {
        tier: 'fast',
        key: 'tier:fast',
        userInitiated: false,
        onEvent: expect.anything(),
      });
    });

    it('discards only the weights partial for a text-only starter', async () => {
      const partial: StarterOption = {
        ...option({ mmproj_file: null, mmproj_sha256: null, mmproj_bytes: 0 }),
        partial_bytes: 3_000_000_000,
      };
      mockLaunch('intro', [partial]);

      renderHook(() => useDownloadCtx(), { wrapper: builtinWrapper });
      await flushLaunch();

      expect(invoke).toHaveBeenCalledWith('discard_partial_download', {
        sha256: 'sha',
      });
      expect(invoke).not.toHaveBeenCalledWith('discard_partial_download', {
        sha256: 'mmsha',
      });
      expect(invokeCount('download_starter')).toBe(1);
    });

    it('does not resume at the model_check picker (it owns the resume choice)', async () => {
      const partial: StarterOption = {
        ...option(),
        partial_bytes: 3_000_000_000,
      };
      mockLaunch('model_check', [partial]);

      const { result } = renderHook(() => useDownloadCtx(), {
        wrapper: builtinWrapper,
      });
      await act(async () => {});

      // Gated out at the stage check; the picker owns the partial, so no resume
      // download is started even though a partial exists on disk.
      expect(invokeCount('download_starter')).toBe(0);
      expect(result.current.state).toEqual({ phase: 'idle' });
    });

    it('does not resume when a model is already installed (complete stage)', async () => {
      const installed: StarterOption = { ...option(), installed: true };
      mockLaunch('complete', [installed]);

      const { result } = renderHook(() => useDownloadCtx(), {
        wrapper: builtinWrapper,
      });
      await act(async () => {});

      expect(invokeCount('onboarding_stage')).toBe(1);
      expect(result.current.state).toEqual({ phase: 'idle' });
      expect(invokeCount('download_starter')).toBe(0);
    });

    it('does not resume when no partial is on disk', async () => {
      mockLaunch('intro', [option()]);

      const { result } = renderHook(() => useDownloadCtx(), {
        wrapper: builtinWrapper,
      });
      await act(async () => {});

      expect(invokeCount('onboarding_stage')).toBe(1);
      expect(result.current.state).toEqual({ phase: 'idle' });
      expect(invokeCount('download_starter')).toBe(0);
    });

    it('does not probe anything when the active provider is not the built-in engine', async () => {
      const { result } = renderHook(() => useDownloadCtx(), { wrapper });
      await act(async () => {});

      // Auto-resume is gated before any probe: the onboarding stage is never
      // queried when the engine is not the active provider.
      expect(invokeCount('onboarding_stage')).toBe(0);
      expect(result.current.state).toEqual({ phase: 'idle' });
    });

    it('fires once: a later provider change does not re-trigger the launch probe', async () => {
      mockLaunch('intro', [{ ...option(), partial_bytes: 1_000 }]);

      let cfg = BUILTIN_CONFIG;
      function mutableWrapper({ children }: { children: ReactNode }) {
        return (
          <ConfigProviderForTest value={cfg}>
            <DownloadProvider>{children}</DownloadProvider>
          </ConfigProviderForTest>
        );
      }

      const { rerender } = renderHook(() => useDownloadCtx(), {
        wrapper: mutableWrapper,
      });
      await act(async () => {});
      expect(invokeCount('onboarding_stage')).toBe(1);

      // Flipping the active provider re-runs the effect; the fire-once ref
      // blocks a second probe.
      cfg = {
        ...BUILTIN_CONFIG,
        inference: {
          ...BUILTIN_CONFIG.inference,
          activeProviderKind: 'ollama',
        },
      };
      await act(async () => {
        rerender();
      });
      expect(invokeCount('onboarding_stage')).toBe(1);
    });
  });
});
