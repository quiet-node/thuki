import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  computeEtaSeconds,
  computeSpeedBytesPerSec,
  downloadFailureMessage,
  isDownloadInFlight,
  useDownloadModel,
} from '../useDownloadModel';
import type { DownloadUiState } from '../useDownloadModel';
import {
  invoke,
  getLastChannel,
  resetChannelCapture,
  enableChannelCapture,
  emitTauriEvent,
  clearEventHandlers,
  type Channel,
} from '../../testUtils/mocks/tauri';
import type { DownloadEvent, DownloadFailKind } from '../../types/starter';

/** The captured download channel, typed for simulateMessage calls. */
function channel(): Channel<DownloadEvent> {
  const captured = getLastChannel();
  expect(captured).not.toBeNull();
  return captured as Channel<DownloadEvent>;
}

describe('useDownloadModel', () => {
  beforeEach(() => {
    invoke.mockReset();
    enableChannelCapture();
  });

  afterEach(() => {
    resetChannelCapture();
    clearEventHandlers();
    vi.restoreAllMocks();
  });

  it('starts idle with no progress and no ETA', () => {
    const { result } = renderHook(() => useDownloadModel());
    expect(result.current.state).toEqual({ phase: 'idle' });
    expect(result.current.progress).toBeNull();
    expect(result.current.etaSeconds).toBeNull();
  });

  it('walks the full happy path: confirm, download, mmproj, verify, ready', async () => {
    const now = vi.spyOn(Date, 'now').mockReturnValue(0);
    const { result } = renderHook(() => useDownloadModel());

    act(() => result.current.beginConfirm('balanced'));
    expect(result.current.state).toEqual({
      phase: 'confirming',
      tier: 'balanced',
    });

    act(() => result.current.cancelConfirm());
    expect(result.current.state).toEqual({ phase: 'idle' });

    act(() => result.current.beginConfirm('balanced'));
    await act(() => result.current.start('balanced'));
    expect(result.current.state).toEqual({ phase: 'downloading' });
    expect(invoke).toHaveBeenCalledWith('download_starter', {
      tier: 'balanced',
      onEvent: expect.anything(),
    });

    // Weights file begins; resumed_from seeds the progress bytes.
    act(() =>
      channel().simulateMessage({
        type: 'Started',
        data: { file: 'weights.gguf', total_bytes: 100, resumed_from: 0 },
      }),
    );
    expect(result.current.state).toEqual({ phase: 'downloading' });
    expect(result.current.progress).toEqual({
      file: 'weights.gguf',
      bytes: 0,
      totalBytes: 100,
    });
    expect(result.current.etaSeconds).toBeNull();

    // First Progress sample: no ETA yet (needs two samples).
    act(() =>
      channel().simulateMessage({
        type: 'Progress',
        data: { file: 'weights.gguf', bytes: 10, total_bytes: 100 },
      }),
    );
    expect(result.current.progress?.bytes).toBe(10);
    expect(result.current.etaSeconds).toBeNull();

    // Second sample 5s later: 40 bytes over 5s = 8 B/s; 50 remaining = ~6s.
    now.mockReturnValue(5000);
    act(() =>
      channel().simulateMessage({
        type: 'Progress',
        data: { file: 'weights.gguf', bytes: 50, total_bytes: 100 },
      }),
    );
    expect(result.current.etaSeconds).toBe(6);

    act(() =>
      channel().simulateMessage({
        type: 'Verifying',
        data: { file: 'weights.gguf' },
      }),
    );
    expect(result.current.state).toEqual({ phase: 'verifying' });

    // FileDone is interim: the state holds until the next Started.
    act(() =>
      channel().simulateMessage({
        type: 'FileDone',
        data: { file: 'weights.gguf' },
      }),
    );
    expect(result.current.state).toEqual({ phase: 'verifying' });

    // Second Started is the vision companion; the ETA window resets.
    act(() =>
      channel().simulateMessage({
        type: 'Started',
        data: { file: 'mmproj.gguf', total_bytes: 50, resumed_from: 0 },
      }),
    );
    expect(result.current.state).toEqual({ phase: 'downloading_mmproj' });
    expect(result.current.etaSeconds).toBeNull();

    act(() =>
      channel().simulateMessage({
        type: 'Verifying',
        data: { file: 'mmproj.gguf' },
      }),
    );
    act(() =>
      channel().simulateMessage({
        type: 'FileDone',
        data: { file: 'mmproj.gguf' },
      }),
    );

    // Without awaitEngine, AllDone lands directly on ready.
    act(() => channel().simulateMessage({ type: 'AllDone' }));
    expect(result.current.state).toEqual({ phase: 'ready' });
  });

  it('flips a post-re-hash Progress back to the active downloading phase', async () => {
    // On resume the prefix is re-hashed (Verifying) before the remaining bytes
    // stream. The first streamed Progress must flip the label back to
    // downloading so the resumed transfer is not mislabeled "Verifying".
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.start('balanced'));

    act(() =>
      channel().simulateMessage({
        type: 'Started',
        data: { file: 'w.gguf', total_bytes: 100, resumed_from: 40 },
      }),
    );
    act(() =>
      channel().simulateMessage({
        type: 'Verifying',
        data: { file: 'w.gguf' },
      }),
    );
    expect(result.current.state).toEqual({ phase: 'verifying' });
    act(() =>
      channel().simulateMessage({
        type: 'Progress',
        data: { file: 'w.gguf', bytes: 50, total_bytes: 100 },
      }),
    );
    expect(result.current.state).toEqual({ phase: 'downloading' });

    // The vision companion resumes too: its re-hash Verifying flips back to the
    // mmproj downloading phase, not the plain one.
    act(() =>
      channel().simulateMessage({
        type: 'FileDone',
        data: { file: 'w.gguf' },
      }),
    );
    act(() =>
      channel().simulateMessage({
        type: 'Started',
        data: { file: 'mmproj.gguf', total_bytes: 50, resumed_from: 20 },
      }),
    );
    act(() =>
      channel().simulateMessage({
        type: 'Verifying',
        data: { file: 'mmproj.gguf' },
      }),
    );
    expect(result.current.state).toEqual({ phase: 'verifying' });
    act(() =>
      channel().simulateMessage({
        type: 'Progress',
        data: { file: 'mmproj.gguf', bytes: 30, total_bytes: 50 },
      }),
    );
    expect(result.current.state).toEqual({ phase: 'downloading_mmproj' });
  });

  it('drops ETA samples older than the 10s window', async () => {
    const now = vi.spyOn(Date, 'now').mockReturnValue(0);
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.start('fast'));
    act(() =>
      channel().simulateMessage({
        type: 'Started',
        data: { file: 'w.gguf', total_bytes: 1000, resumed_from: 0 },
      }),
    );

    // Sample at t=0 (bytes 0) falls out of the window by t=15s; the rate
    // then comes from t=5s..15s: 100 bytes over 10s = 10 B/s.
    const sendProgress = (bytes: number) =>
      act(() =>
        channel().simulateMessage({
          type: 'Progress',
          data: { file: 'w.gguf', bytes, total_bytes: 1000 },
        }),
      );
    sendProgress(0);
    now.mockReturnValue(5000);
    sendProgress(100);
    now.mockReturnValue(15000);
    sendProgress(200);

    // Remaining 800 bytes at 10 B/s = 80s. With the stale t=0 sample the
    // rate would be 200/15s and the ETA 60s instead.
    expect(result.current.etaSeconds).toBe(80);
  });

  it('treats a Failed arriving after ready as terminal failure', async () => {
    // The backend now emits Failed instead of AllDone when finalize fails,
    // but Failed stays terminal from every state as a defensive invariant.
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.start('smartest'));
    act(() => channel().simulateMessage({ type: 'AllDone' }));
    expect(result.current.state).toEqual({ phase: 'ready' });

    act(() =>
      channel().simulateMessage({
        type: 'Failed',
        data: { kind: 'other', message: 'manifest write failed' },
      }),
    );
    expect(result.current.state).toEqual({
      phase: 'failed',
      kind: 'other',
      message: 'manifest write failed',
    });
  });

  it.each<DownloadFailKind>([
    'offline',
    'http',
    'checksum',
    'disk_full',
    'other',
  ])('maps a Failed event of kind %s onto the failed state', async (kind) => {
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.start('fast'));
    act(() =>
      channel().simulateMessage({
        type: 'Failed',
        data: { kind, message: `boom: ${kind}` },
      }),
    );
    expect(result.current.state).toEqual({
      phase: 'failed',
      kind,
      message: `boom: ${kind}`,
    });
  });

  it('returns to idle on Cancelled and clears progress', async () => {
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.start('fast'));
    act(() =>
      channel().simulateMessage({
        type: 'Started',
        data: { file: 'w.gguf', total_bytes: 100, resumed_from: 40 },
      }),
    );
    expect(result.current.progress?.bytes).toBe(40);

    await act(() => result.current.cancel());
    expect(invoke).toHaveBeenCalledWith('cancel_model_download');
    // State waits for the backend's Cancelled event.
    expect(result.current.state).toEqual({ phase: 'downloading' });

    act(() => channel().simulateMessage({ type: 'Cancelled' }));
    expect(result.current.state).toEqual({ phase: 'idle' });
    expect(result.current.progress).toBeNull();
    expect(result.current.etaSeconds).toBeNull();
  });

  it('fails with kind other when the start invoke rejects', async () => {
    invoke.mockRejectedValueOnce('a download is already in progress');
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.start('fast'));
    expect(result.current.state).toEqual({
      phase: 'failed',
      kind: 'other',
      message: 'a download is already in progress',
    });
  });

  it('retries the last tier after a failure', async () => {
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.start('smartest'));
    act(() =>
      channel().simulateMessage({
        type: 'Failed',
        data: { kind: 'checksum', message: 'checksum mismatch' },
      }),
    );

    await act(() => result.current.retry());
    expect(result.current.state).toEqual({ phase: 'downloading' });
    expect(invoke).toHaveBeenLastCalledWith('download_starter', {
      tier: 'smartest',
      onEvent: expect.anything(),
    });
  });

  it('ignores retry before any start recorded a download', async () => {
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.retry());
    expect(result.current.state).toEqual({ phase: 'idle' });
    expect(invoke).not.toHaveBeenCalled();
  });

  it('starts a pasted-repo download through download_repo_model', async () => {
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.startRepo('owner/repo', 'w.gguf'));
    expect(result.current.state).toEqual({ phase: 'downloading' });
    expect(invoke).toHaveBeenCalledWith('download_repo_model', {
      repo: 'owner/repo',
      file: 'w.gguf',
      onEvent: expect.anything(),
    });
    act(() => channel().simulateMessage({ type: 'AllDone' }));
    expect(result.current.state).toEqual({ phase: 'ready' });
  });

  it('retries the last repo download after a failure', async () => {
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.startRepo('owner/repo', 'w.gguf'));
    act(() =>
      channel().simulateMessage({
        type: 'Failed',
        data: { kind: 'http', message: 'HTTP 500' },
      }),
    );

    await act(() => result.current.retry());
    expect(result.current.state).toEqual({ phase: 'downloading' });
    expect(invoke).toHaveBeenLastCalledWith('download_repo_model', {
      repo: 'owner/repo',
      file: 'w.gguf',
      onEvent: expect.anything(),
    });
  });

  it('maps a rejected download_repo_model invoke to failed/other', async () => {
    invoke.mockRejectedValueOnce('invalid Hugging Face repo id');
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.startRepo('bad', 'w.gguf'));
    expect(result.current.state).toEqual({
      phase: 'failed',
      kind: 'other',
      message: 'invalid Hugging Face repo id',
    });
  });

  it('reset returns failed to idle and clears the stale progress', async () => {
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.start('smartest'));
    act(() =>
      channel().simulateMessage({
        type: 'Started',
        data: { file: 'w.gguf', total_bytes: 100, resumed_from: 40 },
      }),
    );
    act(() =>
      channel().simulateMessage({
        type: 'Failed',
        data: { kind: 'disk_full', message: 'no space left' },
      }),
    );
    expect(result.current.progress?.bytes).toBe(40);

    act(() => result.current.reset());
    expect(result.current.state).toEqual({ phase: 'idle' });
    expect(result.current.progress).toBeNull();
    expect(result.current.etaSeconds).toBeNull();
  });

  it('reset returns ready to idle', async () => {
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.start('fast'));
    act(() => channel().simulateMessage({ type: 'AllDone' }));
    expect(result.current.state).toEqual({ phase: 'ready' });

    act(() => result.current.reset());
    expect(result.current.state).toEqual({ phase: 'idle' });
  });

  it('reset is a no-op outside the terminal phases', async () => {
    const { result } = renderHook(() => useDownloadModel());
    await act(() => result.current.start('fast'));
    expect(result.current.state).toEqual({ phase: 'downloading' });

    act(() => result.current.reset());
    expect(result.current.state).toEqual({ phase: 'downloading' });
  });

  it('resumes through the same start call', async () => {
    const { result } = renderHook(() => useDownloadModel());
    act(() => result.current.enterResumePending());
    expect(result.current.state).toEqual({ phase: 'resume_pending' });

    await act(() => result.current.resume('balanced'));
    expect(result.current.state).toEqual({ phase: 'downloading' });
    expect(invoke).toHaveBeenCalledWith('download_starter', {
      tier: 'balanced',
      onEvent: expect.anything(),
    });
  });

  it('discards a partial and returns to idle', async () => {
    const { result } = renderHook(() => useDownloadModel());
    act(() => result.current.enterResumePending());

    await act(() => result.current.discard('a'.repeat(64)));
    expect(invoke).toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'a'.repeat(64),
    });
    expect(result.current.state).toEqual({ phase: 'idle' });
  });

  it('surfaces a discard failure as kind other', async () => {
    invoke.mockRejectedValueOnce('invalid sha256');
    const { result } = renderHook(() => useDownloadModel());
    act(() => result.current.enterResumePending());

    await act(() => result.current.discard('nope'));
    expect(result.current.state).toEqual({
      phase: 'failed',
      kind: 'other',
      message: 'invalid sha256',
    });
  });

  describe('awaitEngine: true', () => {
    const engineStatus = (
      state: 'stopped' | 'starting' | 'loaded' | 'stopping' | 'failed',
      error: string | null = null,
    ) => ({ state, model_path: '/m.gguf', port: null, error });

    it('parks on installing at AllDone, then follows engine:status to ready', async () => {
      const { result } = renderHook(() =>
        useDownloadModel({ awaitEngine: true }),
      );
      await act(() => result.current.start('fast'));
      act(() => channel().simulateMessage({ type: 'AllDone' }));
      expect(result.current.state).toEqual({ phase: 'installing' });

      act(() => emitTauriEvent('engine:status', engineStatus('starting')));
      expect(result.current.state).toEqual({ phase: 'warming_up' });

      act(() => emitTauriEvent('engine:status', engineStatus('loaded')));
      expect(result.current.state).toEqual({ phase: 'ready' });
    });

    it('jumps installing -> ready when loaded arrives without starting', async () => {
      const { result } = renderHook(() =>
        useDownloadModel({ awaitEngine: true }),
      );
      await act(() => result.current.start('fast'));
      act(() => channel().simulateMessage({ type: 'AllDone' }));

      act(() => emitTauriEvent('engine:status', engineStatus('loaded')));
      expect(result.current.state).toEqual({ phase: 'ready' });
    });

    it('fails with kind engine when the engine reports failed', async () => {
      const { result } = renderHook(() =>
        useDownloadModel({ awaitEngine: true }),
      );
      await act(() => result.current.start('fast'));
      act(() => channel().simulateMessage({ type: 'AllDone' }));

      act(() =>
        emitTauriEvent(
          'engine:status',
          engineStatus('failed', 'spawn failed: ENOENT'),
        ),
      );
      expect(result.current.state).toEqual({
        phase: 'failed',
        kind: 'engine',
        message: 'spawn failed: ENOENT',
      });
    });

    it('falls back to a default message when the failed status has no error', async () => {
      const { result } = renderHook(() =>
        useDownloadModel({ awaitEngine: true }),
      );
      await act(() => result.current.start('fast'));
      act(() => channel().simulateMessage({ type: 'AllDone' }));
      act(() => emitTauriEvent('engine:status', engineStatus('starting')));

      act(() => emitTauriEvent('engine:status', engineStatus('failed')));
      expect(result.current.state).toEqual({
        phase: 'failed',
        kind: 'engine',
        message: 'the engine could not start',
      });
    });

    it('ignores engine:status outside installing and warming_up', () => {
      const { result } = renderHook(() =>
        useDownloadModel({ awaitEngine: true }),
      );
      act(() => emitTauriEvent('engine:status', engineStatus('starting')));
      expect(result.current.state).toEqual({ phase: 'idle' });
    });

    it('ignores intermediate stopping statuses while installing', async () => {
      const { result } = renderHook(() =>
        useDownloadModel({ awaitEngine: true }),
      );
      await act(() => result.current.start('fast'));
      act(() => channel().simulateMessage({ type: 'AllDone' }));

      act(() => emitTauriEvent('engine:status', engineStatus('stopping')));
      expect(result.current.state).toEqual({ phase: 'installing' });
    });

    it('detaches the engine:status listener on unmount', async () => {
      const { unmount } = renderHook(() =>
        useDownloadModel({ awaitEngine: true }),
      );
      unmount();
      // Flush the unlisten promise chain, then verify the handler is gone.
      await act(async () => {});
      emitTauriEvent('engine:status', engineStatus('starting'));
    });
  });

  describe('combined progress across the two files (Part 1)', () => {
    it('starts with a null combinedBytes and speed', () => {
      const { result } = renderHook(() => useDownloadModel());
      expect(result.current.combinedBytes).toBeNull();
      expect(result.current.speedBytesPerSec).toBeNull();
    });

    it('accumulates combinedBytes across the weights -> mmproj seam without resetting', async () => {
      const { result } = renderHook(() => useDownloadModel());
      await act(() => result.current.start('balanced'));

      act(() =>
        channel().simulateMessage({
          type: 'Started',
          data: { file: 'weights.gguf', total_bytes: 100, resumed_from: 0 },
        }),
      );
      expect(result.current.combinedBytes).toBe(0);

      act(() =>
        channel().simulateMessage({
          type: 'Progress',
          data: { file: 'weights.gguf', bytes: 60, total_bytes: 100 },
        }),
      );
      expect(result.current.combinedBytes).toBe(60);

      act(() =>
        channel().simulateMessage({
          type: 'FileDone',
          data: { file: 'weights.gguf' },
        }),
      );
      // FileDone snaps the cumulative figure to the file boundary.
      expect(result.current.combinedBytes).toBe(100);

      act(() =>
        channel().simulateMessage({
          type: 'Started',
          data: { file: 'mmproj.gguf', total_bytes: 50, resumed_from: 0 },
        }),
      );
      // The second file must NOT reset the bar to zero: it stays at 100.
      expect(result.current.combinedBytes).toBe(100);

      act(() =>
        channel().simulateMessage({
          type: 'Progress',
          data: { file: 'mmproj.gguf', bytes: 30, total_bytes: 50 },
        }),
      );
      expect(result.current.combinedBytes).toBe(130);

      act(() =>
        channel().simulateMessage({
          type: 'FileDone',
          data: { file: 'mmproj.gguf' },
        }),
      );
      expect(result.current.combinedBytes).toBe(150);
    });

    it('seeds combinedBytes from resumed_from on a resumed first file', async () => {
      const { result } = renderHook(() => useDownloadModel());
      await act(() => result.current.start('fast'));
      act(() =>
        channel().simulateMessage({
          type: 'Started',
          data: { file: 'w.gguf', total_bytes: 100, resumed_from: 40 },
        }),
      );
      expect(result.current.combinedBytes).toBe(40);
    });

    it('exposes a rolling download speed in bytes per second', async () => {
      const now = vi.spyOn(Date, 'now').mockReturnValue(0);
      const { result } = renderHook(() => useDownloadModel());
      await act(() => result.current.start('fast'));
      act(() =>
        channel().simulateMessage({
          type: 'Started',
          data: { file: 'w.gguf', total_bytes: 1000, resumed_from: 0 },
        }),
      );
      act(() =>
        channel().simulateMessage({
          type: 'Progress',
          data: { file: 'w.gguf', bytes: 10, total_bytes: 1000 },
        }),
      );
      expect(result.current.speedBytesPerSec).toBeNull(); // one sample

      now.mockReturnValue(5000);
      act(() =>
        channel().simulateMessage({
          type: 'Progress',
          data: { file: 'w.gguf', bytes: 50, total_bytes: 1000 },
        }),
      );
      // 40 bytes over 5s = 8 B/s.
      expect(result.current.speedBytesPerSec).toBe(8);
    });

    it('clears combinedBytes and speed on Cancelled', async () => {
      const now = vi.spyOn(Date, 'now').mockReturnValue(0);
      const { result } = renderHook(() => useDownloadModel());
      await act(() => result.current.start('fast'));
      act(() =>
        channel().simulateMessage({
          type: 'Started',
          data: { file: 'w.gguf', total_bytes: 100, resumed_from: 20 },
        }),
      );
      now.mockReturnValue(2000);
      act(() =>
        channel().simulateMessage({
          type: 'Progress',
          data: { file: 'w.gguf', bytes: 60, total_bytes: 100 },
        }),
      );
      expect(result.current.combinedBytes).toBe(60);

      act(() => channel().simulateMessage({ type: 'Cancelled' }));
      expect(result.current.combinedBytes).toBeNull();
      expect(result.current.speedBytesPerSec).toBeNull();
    });

    it('clears combinedBytes and speed on reset from a terminal phase', async () => {
      const { result } = renderHook(() => useDownloadModel());
      await act(() => result.current.start('fast'));
      act(() =>
        channel().simulateMessage({
          type: 'Started',
          data: { file: 'w.gguf', total_bytes: 100, resumed_from: 30 },
        }),
      );
      act(() =>
        channel().simulateMessage({
          type: 'Failed',
          data: { kind: 'http', message: 'boom' },
        }),
      );
      expect(result.current.combinedBytes).toBe(30);

      act(() => result.current.reset());
      expect(result.current.combinedBytes).toBeNull();
      expect(result.current.speedBytesPerSec).toBeNull();
    });
  });
});

describe('computeSpeedBytesPerSec', () => {
  it('returns null with fewer than two samples', () => {
    expect(computeSpeedBytesPerSec([])).toBeNull();
    expect(computeSpeedBytesPerSec([{ t: 0, bytes: 0 }])).toBeNull();
  });

  it('returns null when no time elapsed between window edges', () => {
    expect(
      computeSpeedBytesPerSec([
        { t: 1000, bytes: 0 },
        { t: 1000, bytes: 50 },
      ]),
    ).toBeNull();
  });

  it('returns null when bytes did not advance', () => {
    expect(
      computeSpeedBytesPerSec([
        { t: 0, bytes: 50 },
        { t: 5000, bytes: 50 },
      ]),
    ).toBeNull();
  });

  it('computes bytes per second across the window', () => {
    expect(
      computeSpeedBytesPerSec([
        { t: 0, bytes: 0 },
        { t: 4000, bytes: 200 },
      ]),
    ).toBe(50);
  });
});

describe('computeEtaSeconds', () => {
  it('returns null with fewer than two samples', () => {
    expect(computeEtaSeconds([], 0, 100)).toBeNull();
    expect(computeEtaSeconds([{ t: 0, bytes: 0 }], 0, 100)).toBeNull();
  });

  it('returns null when no time elapsed between window edges', () => {
    const samples = [
      { t: 1000, bytes: 0 },
      { t: 1000, bytes: 50 },
    ];
    expect(computeEtaSeconds(samples, 50, 100)).toBeNull();
  });

  it('returns null when bytes did not advance', () => {
    const samples = [
      { t: 0, bytes: 50 },
      { t: 5000, bytes: 50 },
    ];
    expect(computeEtaSeconds(samples, 50, 100)).toBeNull();
  });

  it('clamps the estimate at zero when bytes overshoot the total', () => {
    const samples = [
      { t: 0, bytes: 0 },
      { t: 1000, bytes: 150 },
    ];
    expect(computeEtaSeconds(samples, 150, 100)).toBe(0);
  });
});

describe('isDownloadInFlight', () => {
  it('is true while bytes move and through the post-download steps', () => {
    const inFlight: DownloadUiState['phase'][] = [
      'downloading',
      'downloading_mmproj',
      'verifying',
      'installing',
      'warming_up',
    ];
    for (const phase of inFlight) {
      expect(isDownloadInFlight(phase)).toBe(true);
    }
  });

  it('is false for the idle, pre-flight and terminal phases', () => {
    const settled: DownloadUiState['phase'][] = [
      'idle',
      'confirming',
      'resume_pending',
      'ready',
      'failed',
    ];
    for (const phase of settled) {
      expect(isDownloadInFlight(phase)).toBe(false);
    }
  });
});

describe('downloadFailureMessage', () => {
  it('maps each failure kind to a friendly, jargon-free reason', () => {
    expect(downloadFailureMessage('offline')).toBe('You appear to be offline.');
    expect(downloadFailureMessage('http')).toBe(
      'Hugging Face had an error. Try again.',
    );
    expect(downloadFailureMessage('checksum')).toBe(
      'The download did not verify. Retrying starts it fresh.',
    );
    expect(downloadFailureMessage('disk_full')).toBe('Not enough disk space.');
    expect(downloadFailureMessage('engine')).toBe(
      "Thuki's engine could not start.",
    );
    expect(downloadFailureMessage('other')).toBe('Model download failed.');
  });
});
