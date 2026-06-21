import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import type { ReactNode } from 'react';
import { DownloadsProvider, useDownloads } from '../DownloadsContext';
import {
  invoke,
  enableChannelCapture,
  getLastChannel,
  resetChannelCapture,
  type Channel,
} from '../../testUtils/mocks/tauri';
import { downloadKey } from '../../hooks/downloadKey';
import type { DownloadEvent } from '../../types/starter';

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
      if (cmd === 'download_staff_pick')
        throw 'a download is already in progress';
    });
    const { result } = renderHook(() => useDownloads(), { wrapper });

    await act(async () => {
      result.current.startStaffPick('gemma-4-12b');
      await Promise.resolve();
    });

    expect(result.current.get(STAFF_KEY)?.state).toEqual({
      phase: 'failed',
      kind: 'other',
      message: 'a download is already in progress',
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
      onEvent: expect.anything(),
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
});
