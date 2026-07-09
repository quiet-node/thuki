/**
 * Download-state machine for a single starter model download (onboarding).
 *
 * Drives the onboarding download UI (StarterPicker + DownloadProgress) through
 * one discriminated-union state, fed by the `download_*` Tauri channel and,
 * optionally, the `engine:status` Tauri event. Per-event state transitions live
 * in the shared {@link reduceDownloadEvent} reducer so this single-download hook
 * and the multi-download Settings registry ({@link useDownloads}) never diverge.
 *
 * Engine handoff: by default `AllDone` transitions straight to `ready`, because
 * after a Settings-context download nobody starts the engine until the first
 * chat, so waiting on `engine:status` would hang forever. A consumer that does
 * prime the engine right after the download (onboarding) passes
 * `awaitEngine: true`; then `AllDone` parks in `installing` and the
 * `engine:status` listener advances `installing -> warming_up -> ready` (or
 * `failed` with kind `engine`).
 *
 * The backend emits `AllDone` only after the install is recorded; a finalize
 * failure (the manifest write failed) emits `Failed` instead. `Failed` is
 * terminal from any state. Terminal means no *event* moves the machine out of
 * it; the user can still leave through `reset`.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  type DownloadAccumulator,
  type DownloadProgressInfo,
  type DownloadUiState,
  initialAccumulator,
  reduceDownloadEvent,
  startingAccumulator,
} from './downloadReducer';
import { downloadKey } from './downloadKey';
import type {
  DownloadEvent,
  EngineStatus,
  StarterTier,
} from '../types/starter';

// Re-export the shared download vocabulary so existing consumers keep importing
// it from this hook; the definitions now live in `downloadReducer`.
export {
  computeEtaSeconds,
  computeSpeedBytesPerSec,
  downloadFailureMessage,
  isDownloadInFlight,
} from './downloadReducer';
export type {
  DownloadProgressInfo,
  DownloadUiFailKind,
  DownloadUiState,
} from './downloadReducer';

export interface UseDownloadModel {
  state: DownloadUiState;
  progress: DownloadProgressInfo | null;
  etaSeconds: number | null;
  /**
   * Cumulative bytes downloaded across every file of the current run
   * (weights + vision companion), or null when idle. The two files are one
   * continuous figure: this never resets between them.
   */
  combinedBytes: number | null;
  /** Rolling download rate in bytes per second, or null until measurable. */
  speedBytesPerSec: number | null;
  /** idle -> confirming. No backend call; shows the confirm card. */
  beginConfirm: (tier: StarterTier) => void;
  /** confirming -> idle. */
  cancelConfirm: () => void;
  /**
   * confirming -> downloading; invokes `download_starter` with a channel.
   * `userInitiated` (default true) threads through to the command's safe-mode
   * gate (issue #296): every real button click leaves it true; the one
   * non-click caller (the app-root auto-resume-on-launch effect) passes
   * false so a post-crash safe-mode session refuses the auto-restart.
   */
  start: (tier: StarterTier, userInitiated?: boolean) => Promise<void>;
  /**
   * idle -> downloading for a pasted-repo model; invokes `download_repo_model`
   * with a channel. Same event stream, terminal states, and `userInitiated`
   * contract as `start`.
   */
  startRepo: (
    repo: string,
    file: string,
    userInitiated?: boolean,
  ) => Promise<void>;
  /**
   * idle -> downloading for a Staff Picks catalog entry, keyed by its stable
   * `id`; invokes `download_staff_pick` with a channel. Same event stream,
   * terminal states, and `userInitiated` contract as `start`; `retry` replays
   * it (always as a user action), and a resume is just calling it again (the
   * backend resumes the partial via Range).
   */
  startById: (id: string, userInitiated?: boolean) => Promise<void>;
  /**
   * Invokes `cancel_model_download` for the run this hook last started. The
   * state flips back to idle when the backend's Cancelled event lands; the
   * partial is KEPT, so the caller refreshes options to surface resume_pending.
   */
  cancel: () => Promise<void>;
  /**
   * failed -> downloading (also the safe-mode `rejected_safe_mode` card's
   * Resume). A checksum failure already deleted the partial on the backend,
   * so retrying is just starting the same download (starter tier, staff pick,
   * or pasted repo, whichever ran last) again. Always replays as
   * `userInitiated: true`: clicking Retry is itself the user action, even
   * when the run it replays was originally started non-user-initiated (the
   * launch-time auto-resume).
   */
  retry: () => Promise<void>;
  /** resume_pending -> downloading; the backend resumes via Range. */
  resume: (tier: StarterTier, userInitiated?: boolean) => Promise<void>;
  /** resume_pending -> idle; invokes `discard_partial_download`. */
  discard: (sha256: string) => Promise<void>;
  /** Caller sets this when starter options show partial_bytes. */
  enterResumePending: () => void;
  /**
   * failed -> idle and ready -> idle; no-op in every other phase. A user
   * action, not an event transition, so the terminal-Failed contract is
   * intact: no backend event ever leaves `failed`, but the user may step
   * back to the picker to choose a different model.
   */
  reset: () => void;
}

export interface UseDownloadModelOptions {
  /**
   * When true, `AllDone` parks in `installing` and `engine:status` drives
   * the warming_up/ready/failed handoff. Leave false (the default) unless
   * the consumer starts the engine immediately after the download.
   */
  awaitEngine?: boolean;
}

export function useDownloadModel(
  options?: UseDownloadModelOptions,
): UseDownloadModel {
  const awaitEngine = options?.awaitEngine === true;

  const [acc, setAcc] = useState<DownloadAccumulator>(initialAccumulator);
  /** Download key of the run in flight, so `cancel` targets the right slot. */
  const currentKeyRef = useRef('');
  /**
   * Replays the most recent start (tier / repo / id) for `retry`, taking the
   * `userInitiated` flag to send: `retry()` always passes true (see its doc),
   * independent of whatever flag the original call used.
   */
  const lastStartRef = useRef<
    ((userInitiated: boolean) => Promise<void>) | null
  >(null);

  useEffect(() => {
    if (!awaitEngine) return;
    const unlistenPromise = listen<EngineStatus>('engine:status', (event) => {
      const status = event.payload;
      setAcc((prev) => {
        if (
          prev.state.phase !== 'installing' &&
          prev.state.phase !== 'warming_up'
        ) {
          return prev;
        }
        if (status.state === 'starting') {
          return { ...prev, state: { phase: 'warming_up' } };
        }
        if (status.state === 'loaded') {
          return { ...prev, state: { phase: 'ready' } };
        }
        if (status.state === 'failed') {
          return {
            ...prev,
            state: {
              phase: 'failed',
              kind: 'engine',
              message: status.error ?? 'the engine could not start',
            },
          };
        }
        return prev;
      });
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [awaitEngine]);

  const beginConfirm = useCallback((tier: StarterTier) => {
    setAcc((prev) => ({ ...prev, state: { phase: 'confirming', tier } }));
  }, []);

  const cancelConfirm = useCallback(() => {
    setAcc(initialAccumulator());
  }, []);

  /** Shared start path: resets the accumulator, wires the event channel, and
   * invokes the given download command with its download key and the
   * safe-mode `userInitiated` flag (issue #296). */
  const run = useCallback(
    async (
      command: string,
      args: Record<string, unknown>,
      key: string,
      userInitiated: boolean,
    ) => {
      currentKeyRef.current = key;
      setAcc(startingAccumulator());
      const channel = new Channel<DownloadEvent>();
      channel.onmessage = (event) =>
        setAcc((prev) => reduceDownloadEvent(prev, event, awaitEngine));
      try {
        await invoke(command, {
          ...args,
          key,
          userInitiated,
          onEvent: channel,
        });
      } catch (err) {
        setAcc((prev) => ({
          ...prev,
          state: { phase: 'failed', kind: 'other', message: String(err) },
        }));
      }
    },
    [awaitEngine],
  );

  const start = useCallback(
    async (tier: StarterTier, userInitiated = true) => {
      const replay = (ui: boolean) =>
        run(
          'download_starter',
          { tier },
          downloadKey({ kind: 'tier', tier }),
          ui,
        );
      lastStartRef.current = replay;
      await replay(userInitiated);
    },
    [run],
  );

  const startRepo = useCallback(
    async (repo: string, file: string, userInitiated = true) => {
      const replay = (ui: boolean) =>
        run(
          'download_repo_model',
          { repo, file },
          downloadKey({ kind: 'repo', repo, file }),
          ui,
        );
      lastStartRef.current = replay;
      await replay(userInitiated);
    },
    [run],
  );

  const startById = useCallback(
    async (id: string, userInitiated = true) => {
      const replay = (ui: boolean) =>
        run(
          'download_staff_pick',
          { id },
          downloadKey({ kind: 'staff', id }),
          ui,
        );
      lastStartRef.current = replay;
      await replay(userInitiated);
    },
    [run],
  );

  const cancel = useCallback(async () => {
    await invoke('cancel_model_download', { key: currentKeyRef.current });
  }, []);

  const retry = useCallback(async () => {
    const replay = lastStartRef.current;
    if (replay === null) return;
    // Clicking Retry is itself the user action, regardless of whether the run
    // it replays was originally started non-user-initiated (the launch-time
    // auto-resume): always send true, never the stale original flag.
    await replay(true);
  }, []);

  const discard = useCallback(async (sha256: string) => {
    try {
      await invoke('discard_partial_download', { sha256 });
    } catch (err) {
      setAcc((prev) => ({
        ...prev,
        state: { phase: 'failed', kind: 'other', message: String(err) },
      }));
      return;
    }
    setAcc((prev) => ({ ...prev, state: { phase: 'idle' } }));
  }, []);

  const enterResumePending = useCallback(() => {
    setAcc((prev) => ({ ...prev, state: { phase: 'resume_pending' } }));
  }, []);

  const reset = useCallback(() => {
    setAcc((prev) =>
      prev.state.phase === 'failed' || prev.state.phase === 'ready'
        ? initialAccumulator()
        : {
            // Stale byte counts from the run that just ended; the next start
            // reseeds them. Callers only invoke reset from the terminal cards.
            ...prev,
            progress: null,
            etaSeconds: null,
            speedBytesPerSec: null,
            combinedBytes: null,
            completedBytes: 0,
            currentFileTotal: 0,
          },
    );
  }, []);

  return {
    state: acc.state,
    progress: acc.progress,
    etaSeconds: acc.etaSeconds,
    combinedBytes: acc.combinedBytes,
    speedBytesPerSec: acc.speedBytesPerSec,
    beginConfirm,
    cancelConfirm,
    start,
    startRepo,
    startById,
    cancel,
    retry,
    resume: start,
    discard,
    enterResumePending,
    reset,
  };
}
