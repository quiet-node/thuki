import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import type { ReactNode } from 'react';
import {
  DownloadsProvider,
  useDownloads,
  applyRemoteEvent,
  seedRemoteSnapshot,
} from '../DownloadsContext';
import {
  invoke,
  listen,
  enableChannelCapture,
  enableChannelCaptureWithResponses,
  emitTauriEvent,
  getLastChannel,
  resetChannelCapture,
  type Channel,
} from '../../testUtils/mocks/tauri';
import { downloadKey } from '../../hooks/downloadKey';
import { startingAccumulator } from '../../hooks/downloadReducer';
import type { ActiveDownload, DownloadEvent } from '../../types/starter';

/** The cross-window progress event the backend broadcasts; mirrors
 * `models::DOWNLOAD_PROGRESS_EVENT`. */
const DOWNLOAD_PROGRESS_EVENT = 'thuki://download-progress';
/** A vision model's two blobs (weights + mmproj). */
const WEIGHTS_SHA = 'a'.repeat(64);
const MMPROJ_SHA = 'b'.repeat(64);

/** The captured download channel, typed for simulateMessage calls. */
function channel(): Channel<DownloadEvent> {
  return getLastChannel() as Channel<DownloadEvent>;
}

function wrapper({ children }: { children: ReactNode }) {
  return <DownloadsProvider>{children}</DownloadsProvider>;
}

const STAFF_KEY = downloadKey({ kind: 'staff', id: 'gemma-4-12b' });
const REPO_KEY = downloadKey({
  kind: 'repo',
  repo: 'org/repo',
  file: 'w.gguf',
});
const REPO2_KEY = downloadKey({
  kind: 'repo',
  repo: 'org/repo2',
  file: 'w2.gguf',
});

describe('DownloadsContext', () => {
  beforeEach(() => {
    invoke.mockReset();
    enableChannelCapture();
  });

  afterEach(() => {
    resetChannelCapture();
    vi.restoreAllMocks();
  });

  it('throws when useDownloads is called outside a provider', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    expect(() => renderHook(() => useDownloads())).toThrow(
      'useDownloads must be used within a DownloadsProvider',
    );
    spy.mockRestore();
  });

  it('has no downloads when idle', () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    expect(result.current.get(STAFF_KEY)).toBeUndefined();
    expect(result.current.hasRepoDownload('org/repo')).toBe(false);
  });

  it('starts a Staff Picks download keyed by its id', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });

    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
    });

    expect(result.current.get(STAFF_KEY)?.state).toEqual({
      phase: 'downloading',
    });
    expect(invoke).toHaveBeenCalledWith('download_staff_pick', {
      id: 'gemma-4-12b',
      key: STAFF_KEY,
      userInitiated: true,
      onEvent: expect.anything(),
    });
  });

  it('advances a download through its channel events to ready', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
    });

    act(() =>
      channel().simulateMessage({
        type: 'Started',
        data: { file: 'w.gguf', total_bytes: 100, resumed_from: 0 },
      }),
    );
    act(() =>
      channel().simulateMessage({
        type: 'Progress',
        data: { file: 'w.gguf', bytes: 60, total_bytes: 100 },
      }),
    );
    expect(result.current.get(STAFF_KEY)?.combinedBytes).toBe(60);

    act(() => channel().simulateMessage({ type: 'AllDone' }));
    expect(result.current.get(STAFF_KEY)?.state).toEqual({ phase: 'ready' });
  });

  it('prunes an entry when its download is cancelled', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
    });
    expect(result.current.get(STAFF_KEY)).toBeDefined();

    act(() => channel().simulateMessage({ type: 'Cancelled' }));
    expect(result.current.get(STAFF_KEY)).toBeUndefined();
  });

  it('marks a download failed when the start invoke rejects', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'download_staff_pick') throw 'could not resolve model';
    });
    const { result } = renderHook(() => useDownloads(), { wrapper });

    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
      await Promise.resolve();
    });

    expect(result.current.get(STAFF_KEY)?.state).toEqual({
      phase: 'failed',
      kind: 'other',
      message: 'could not resolve model',
    });
  });

  it('drops the entry (no failure card) when the start is rejected as already in progress', async () => {
    // The backend by-sha guard rejects a start whose blob is already
    // downloading under another key (the same model running cross-window).
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'download_staff_pick')
        throw 'a download for this file is already in progress';
    });
    const { result } = renderHook(() => useDownloads(), { wrapper });

    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
      await Promise.resolve();
    });

    // No spurious failure: the optimistic entry is dropped so the row can fall
    // back to the live cross-window view.
    expect(result.current.get(STAFF_KEY)).toBeUndefined();
  });

  it('ignores a re-entrant start while the same key is already downloading', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
    });
    // A second click before the row hides its button must not fire a second
    // backend download (which claim_download would reject as a spurious flash).
    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
    });
    expect(
      invoke.mock.calls.filter((c) => c[0] === 'download_staff_pick'),
    ).toHaveLength(1);
    expect(result.current.get(STAFF_KEY)?.state).toEqual({
      phase: 'downloading',
    });
  });

  it('cancel targets the keyed download', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      result.current.cancel(STAFF_KEY);
    });
    expect(invoke).toHaveBeenCalledWith('cancel_model_download', {
      key: STAFF_KEY,
    });
  });

  it('retry replays the failed download, clear forgets it', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
    });
    act(() =>
      channel().simulateMessage({
        type: 'Failed',
        data: { kind: 'http', message: 'HTTP 500' },
      }),
    );

    await act(async () => {
      result.current.retry(STAFF_KEY);
    });
    expect(
      invoke.mock.calls.filter((c) => c[0] === 'download_staff_pick'),
    ).toHaveLength(2);

    // A retry with no entry for the key is a no-op (nothing to replay).
    invoke.mockClear();
    await act(async () => {
      result.current.retry('staff:does-not-exist');
    });
    expect(invoke).not.toHaveBeenCalled();

    act(() =>
      channel().simulateMessage({
        type: 'Failed',
        data: { kind: 'http', message: 'again' },
      }),
    );
    act(() => {
      result.current.clear(STAFF_KEY);
    });
    expect(result.current.get(STAFF_KEY)).toBeUndefined();
    // Clearing a key with no entry is a harmless no-op.
    act(() => {
      result.current.clear('staff:does-not-exist');
    });
  });

  it('discard removes a kept partial by sha', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      await result.current.discard('a'.repeat(64));
    });
    expect(invoke).toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'a'.repeat(64),
    });
  });

  it('tracks repo downloads for the re-expand check', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      result.current.startRepoDownload('org/repo', 'w.gguf');
    });
    expect(result.current.get(REPO_KEY)?.state).toEqual({
      phase: 'downloading',
    });
    expect(result.current.hasRepoDownload('org/repo')).toBe(true);
    expect(result.current.hasRepoDownload('other/repo')).toBe(false);
    expect(invoke).toHaveBeenCalledWith('download_repo_model', {
      repo: 'org/repo',
      file: 'w.gguf',
      key: REPO_KEY,
      userInitiated: true,
      onEvent: expect.anything(),
    });
  });

  it('reports zero counts for a repo with no live downloads', () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    expect(result.current.repoDownloadSummary('org/repo')).toEqual({
      downloading: 0,
      verifying: 0,
      failed: 0,
    });
  });

  it("counts a repo's live downloads by state, mmproj as downloading, excluding ready and other repos", async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });

    // a.gguf: plain downloading (default phase on start).
    await act(async () => {
      result.current.startRepoDownload('org/repo', 'a.gguf');
    });

    // b.gguf: a second Started flips it to downloading_mmproj (still downloading).
    await act(async () => {
      result.current.startRepoDownload('org/repo', 'b.gguf');
    });
    const chB = channel();
    act(() =>
      chB.simulateMessage({
        type: 'Started',
        data: { file: 'b.gguf', total_bytes: 100, resumed_from: 0 },
      }),
    );
    act(() =>
      chB.simulateMessage({
        type: 'Started',
        data: { file: 'b.mmproj', total_bytes: 50, resumed_from: 0 },
      }),
    );

    // c.gguf: verifying.
    await act(async () => {
      result.current.startRepoDownload('org/repo', 'c.gguf');
    });
    act(() =>
      channel().simulateMessage({
        type: 'Verifying',
        data: { file: 'c.gguf' },
      }),
    );

    // d.gguf: failed.
    await act(async () => {
      result.current.startRepoDownload('org/repo', 'd.gguf');
    });
    act(() =>
      channel().simulateMessage({
        type: 'Failed',
        data: { kind: 'http', message: 'HTTP 500' },
      }),
    );

    // e.gguf: ready is terminal-success and must not appear as a live pill.
    await act(async () => {
      result.current.startRepoDownload('org/repo', 'e.gguf');
    });
    act(() => channel().simulateMessage({ type: 'AllDone' }));

    // A different repo's download must not leak into org/repo's counts.
    await act(async () => {
      result.current.startRepoDownload('other/repo', 'z.gguf');
    });

    expect(result.current.repoDownloadSummary('org/repo')).toEqual({
      downloading: 2,
      verifying: 1,
      failed: 1,
    });
    expect(result.current.repoDownloadSummary('other/repo')).toEqual({
      downloading: 1,
      verifying: 0,
      failed: 0,
    });
  });

  it('excludes Staff Picks downloads from a repo summary', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
    });
    expect(result.current.repoDownloadSummary('org/repo')).toEqual({
      downloading: 0,
      verifying: 0,
      failed: 0,
    });
  });

  it('ignores a late channel event after its entry is cleared', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
    });
    const late = channel();
    act(() => {
      result.current.clear(STAFF_KEY);
    });
    // The download task may still emit; with no entry the event is dropped.
    act(() =>
      late.simulateMessage({
        type: 'Progress',
        data: { file: 'w.gguf', bytes: 10, total_bytes: 100 },
      }),
    );
    expect(result.current.get(STAFF_KEY)).toBeUndefined();
  });

  it('queuePosition gives each simultaneously-queued entry its own distinct FIFO position', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
    });
    const staffChannel = channel();
    await act(async () => {
      result.current.startRepoDownload('org/repo', 'w.gguf');
    });
    const repoChannel = channel();
    await act(async () => {
      result.current.startRepoDownload('org/repo2', 'w2.gguf');
    });
    const repo2Channel = channel();

    // None is queued yet (all still `downloading`), so none has a position.
    expect(result.current.queuePosition(STAFF_KEY)).toBeUndefined();
    expect(result.current.queuedTotal).toBe(0);

    act(() => staffChannel.simulateMessage({ type: 'Queued' }));
    act(() => repoChannel.simulateMessage({ type: 'Queued' }));
    act(() => repo2Channel.simulateMessage({ type: 'Queued' }));

    // Each of the 3 simultaneously-queued downloads reads its own 1-indexed
    // start-order position, not a repeated sibling count (the old bug: all 3
    // showed the same "#2 in queue").
    expect(result.current.queuePosition(STAFF_KEY)).toBe(1);
    expect(result.current.queuePosition(REPO_KEY)).toBe(2);
    expect(result.current.queuePosition(REPO2_KEY)).toBe(3);
    // A key that owns no queued entry has no position at all.
    expect(
      result.current.queuePosition('staff:does-not-exist'),
    ).toBeUndefined();
    // Derived from the same source as queuePosition, so the two never disagree.
    expect(result.current.queuedTotal).toBe(3);
  });
});

// ─── Cross-window download sync ──────────────────────────────────────────────

describe('applyRemoteEvent', () => {
  const PROGRESS: ActiveDownload = {
    key: 'tier:balanced',
    shas: [WEIGHTS_SHA, MMPROJ_SHA],
    event: {
      type: 'Progress',
      data: { file: 'w.gguf', bytes: 50, total_bytes: 100 },
    },
  };

  it('skips a key this window already owns locally (no double-fold)', () => {
    const prev = new Map();
    const next = applyRemoteEvent(prev, new Set(['tier:balanced']), PROGRESS);
    expect(next).toBe(prev);
    expect(next.size).toBe(0);
  });

  it('seeds a starting accumulator when the event is null', () => {
    const next = applyRemoteEvent(new Map(), new Set(), {
      key: 'tier:balanced',
      shas: [WEIGHTS_SHA],
      event: null,
    });
    expect(next.get('tier:balanced')?.acc.state).toEqual({
      phase: 'downloading',
    });
    expect(next.get('tier:balanced')?.shas).toEqual([WEIGHTS_SHA]);
  });

  it('folds a progress event onto the existing accumulator', () => {
    const next = applyRemoteEvent(new Map(), new Set(), PROGRESS);
    const entry = next.get('tier:balanced');
    expect(entry?.acc.state).toEqual({ phase: 'downloading' });
    expect(entry?.acc.combinedBytes).toBe(50);
  });

  it('keeps a successful AllDone as ready (for the install effect)', () => {
    const next = applyRemoteEvent(new Map(), new Set(), {
      key: 'tier:balanced',
      shas: [WEIGHTS_SHA],
      event: { type: 'AllDone' },
    });
    expect(next.get('tier:balanced')?.acc.state).toEqual({ phase: 'ready' });
  });

  it('drops the entry on Cancelled (idle) and on Failed', () => {
    const seeded = new Map([
      ['tier:balanced', { shas: [WEIGHTS_SHA], acc: startingAccumulator() }],
    ]);
    const cancelled = applyRemoteEvent(seeded, new Set(), {
      key: 'tier:balanced',
      shas: [WEIGHTS_SHA],
      event: { type: 'Cancelled' },
    });
    expect(cancelled.has('tier:balanced')).toBe(false);

    const failed = applyRemoteEvent(seeded, new Set(), {
      key: 'tier:balanced',
      shas: [WEIGHTS_SHA],
      event: { type: 'Failed', data: { kind: 'http', message: 'boom' } },
    });
    expect(failed.has('tier:balanced')).toBe(false);
  });
});

describe('seedRemoteSnapshot', () => {
  it('seeds a key that is absent', () => {
    const next = seedRemoteSnapshot(new Map(), new Set(), {
      key: 'tier:balanced',
      shas: [WEIGHTS_SHA],
      event: null,
    });
    expect(next.get('tier:balanced')?.acc.state).toEqual({
      phase: 'downloading',
    });
  });

  it('does not clobber an entry already present (a fresher live event won)', () => {
    const live = new Map([
      ['tier:balanced', { shas: [WEIGHTS_SHA], acc: startingAccumulator() }],
    ]);
    const next = seedRemoteSnapshot(live, new Set(), {
      key: 'tier:balanced',
      shas: [WEIGHTS_SHA],
      event: {
        type: 'Progress',
        data: { file: 'w.gguf', bytes: 99, total_bytes: 100 },
      },
    });
    expect(next).toBe(live);
    // The fresher live accumulator is untouched: bytes not overwritten by the
    // older snapshot.
    expect(next.get('tier:balanced')?.acc.combinedBytes).toBeNull();
  });
});

describe('DownloadsContext cross-window', () => {
  function wrapper({ children }: { children: ReactNode }) {
    return <DownloadsProvider>{children}</DownloadsProvider>;
  }

  beforeEach(() => {
    invoke.mockReset();
    enableChannelCapture();
  });

  afterEach(() => {
    resetChannelCapture();
    vi.restoreAllMocks();
  });

  it('hydrates remote downloads from get_active_downloads on mount', async () => {
    enableChannelCaptureWithResponses({
      get_active_downloads: [
        {
          key: 'tier:balanced',
          shas: [WEIGHTS_SHA, MMPROJ_SHA],
          event: {
            type: 'Progress',
            data: { file: 'w.gguf', bytes: 40, total_bytes: 100 },
          },
        },
      ] satisfies ActiveDownload[],
    });

    const { result } = renderHook(() => useDownloads(), { wrapper });
    // Let the mount effect's get_active_downloads promise resolve and apply.
    await act(async () => {
      await Promise.resolve();
    });

    const active = result.current.getActiveDownload(WEIGHTS_SHA);
    expect(active?.key).toBe('tier:balanced');
    expect(active?.view.state).toEqual({ phase: 'downloading' });
    expect(active?.view.combinedBytes).toBe(40);
    // A sha no in-flight download writes has no match.
    expect(result.current.getActiveDownload('c'.repeat(64))).toBeUndefined();
  });

  it('tracks a cross-window download live via the global event', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      await Promise.resolve();
    });

    act(() =>
      emitTauriEvent(DOWNLOAD_PROGRESS_EVENT, {
        key: 'tier:balanced',
        shas: [WEIGHTS_SHA],
        event: {
          type: 'Progress',
          data: { file: 'w.gguf', bytes: 70, total_bytes: 100 },
        },
      } satisfies ActiveDownload),
    );

    const active = result.current.getActiveDownload(WEIGHTS_SHA);
    expect(active?.key).toBe('tier:balanced');
    expect(active?.view.combinedBytes).toBe(70);

    // A terminal Cancelled drops it; the row reverts to its normal controls.
    act(() =>
      emitTauriEvent(DOWNLOAD_PROGRESS_EVENT, {
        key: 'tier:balanced',
        shas: [WEIGHTS_SHA],
        event: { type: 'Cancelled' },
      } satisfies ActiveDownload),
    );
    expect(result.current.getActiveDownload(WEIGHTS_SHA)).toBeUndefined();
  });

  it('does not mirror this window’s own download into the remote registry', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
    });
    const ownKey = downloadKey({ kind: 'staff', id: 'gemma-4-12b' });

    // The backend broadcasts the same event globally; this window must ignore it
    // for its own key (the channel already drives it; a second fold would
    // phantom-count).
    act(() =>
      emitTauriEvent(DOWNLOAD_PROGRESS_EVENT, {
        key: ownKey,
        shas: [WEIGHTS_SHA],
        event: {
          type: 'Progress',
          data: { file: 'w.gguf', bytes: 90, total_bytes: 100 },
        },
      } satisfies ActiveDownload),
    );
    expect(result.current.getActiveDownload(WEIGHTS_SHA)).toBeUndefined();
    expect(result.current.get(ownKey)?.state).toEqual({ phase: 'downloading' });
  });

  it('tolerates a get_active_downloads rejection (not under Tauri)', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_active_downloads') throw new Error('no bridge');
    });
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      await Promise.resolve();
    });
    // The rejected snapshot leaves the registry empty; no crash.
    expect(result.current.getActiveDownload(WEIGHTS_SHA)).toBeUndefined();
  });

  it('tolerates an unavailable event bridge (listen rejects)', async () => {
    vi.mocked(listen).mockRejectedValueOnce(new Error('no bridge'));
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      await Promise.resolve();
    });
    // Cross-window live progress is simply absent; local downloads still work.
    expect(result.current.getActiveDownload(WEIGHTS_SHA)).toBeUndefined();
  });

  it('unsubscribes when unmounted before the listen subscription resolves', async () => {
    const { result, unmount } = renderHook(() => useDownloads(), { wrapper });
    // Unmount synchronously, before the listen promise resolves: the resolved
    // handler must immediately unsubscribe rather than leak.
    unmount();
    await act(async () => {
      await Promise.resolve();
    });
    // A late broadcast after teardown reaches no handler and has no effect.
    act(() =>
      emitTauriEvent(DOWNLOAD_PROGRESS_EVENT, {
        key: 'tier:balanced',
        shas: [WEIGHTS_SHA],
        event: {
          type: 'Progress',
          data: { file: 'w.gguf', bytes: 10, total_bytes: 100 },
        },
      } satisfies ActiveDownload),
    );
    expect(result.current.getActiveDownload(WEIGHTS_SHA)).toBeUndefined();
  });

  it('clear drops a remote entry by its real key', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      await Promise.resolve();
    });
    act(() =>
      emitTauriEvent(DOWNLOAD_PROGRESS_EVENT, {
        key: 'tier:balanced',
        shas: [WEIGHTS_SHA],
        event: { type: 'AllDone' },
      } satisfies ActiveDownload),
    );
    expect(result.current.getActiveDownload(WEIGHTS_SHA)?.view.state).toEqual({
      phase: 'ready',
    });

    act(() => {
      result.current.clear('tier:balanced');
    });
    expect(result.current.getActiveDownload(WEIGHTS_SHA)).toBeUndefined();
  });

  it('queuePosition numbers a remote (other-window) queued download, skipping a non-queued remote entry', async () => {
    const { result } = renderHook(() => useDownloads(), { wrapper });
    await act(async () => {
      await Promise.resolve();
    });
    // A remote download that is actively downloading (not queued) must not
    // consume a queue slot or be reported as having a position.
    act(() =>
      emitTauriEvent(DOWNLOAD_PROGRESS_EVENT, {
        key: 'tier:other',
        shas: [MMPROJ_SHA],
        event: {
          type: 'Progress',
          data: { file: 'w.gguf', bytes: 1, total_bytes: 100 },
        },
      } satisfies ActiveDownload),
    );
    act(() =>
      emitTauriEvent(DOWNLOAD_PROGRESS_EVENT, {
        key: 'tier:balanced',
        shas: [WEIGHTS_SHA],
        event: { type: 'Queued' },
      } satisfies ActiveDownload),
    );
    expect(result.current.queuePosition('tier:balanced')).toBe(1);
    expect(result.current.queuePosition('tier:other')).toBeUndefined();
    expect(
      result.current.queuePosition('staff:does-not-exist'),
    ).toBeUndefined();
    // The downloading remote entry does not consume a queue slot; the total
    // counts only the one actually-queued entry.
    expect(result.current.queuedTotal).toBe(1);
  });
});
