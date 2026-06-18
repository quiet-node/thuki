/**
 * Download-state machine for starter model downloads.
 *
 * Drives the shared download UI (StarterPicker + DownloadProgress) through
 * one discriminated-union state, fed by the `download_starter` Tauri channel
 * and, optionally, the `engine:status` Tauri event.
 *
 * Engine handoff: by default `AllDone` transitions straight to `ready`,
 * because after a Settings-context download nobody starts the engine until
 * the first chat, so waiting on `engine:status` would hang forever. A
 * consumer that does prime the engine right after the download (onboarding)
 * passes `awaitEngine: true`; then `AllDone` parks in `installing` and the
 * `engine:status` listener advances `installing -> warming_up -> ready`
 * (or `failed` with kind `engine`).
 *
 * The backend emits `AllDone` only after the install is recorded; a finalize
 * failure (the manifest write failed) emits `Failed` instead of `AllDone`.
 * `Failed` is terminal from any state. Terminal means no *event* moves the
 * machine out of it; the user can still leave through `reset`, an explicit
 * action that returns the terminal `failed`/`ready` cards to the picker.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  DownloadEvent,
  DownloadFailKind,
  EngineStatus,
  StarterTier,
} from '../types/starter';

/** Failure kinds the UI can show: the backend's plus the engine handoff's. */
export type DownloadUiFailKind = DownloadFailKind | 'engine';

/** The download UI state machine's discriminated union. */
export type DownloadUiState =
  | { phase: 'idle' }
  | { phase: 'confirming'; tier: StarterTier }
  | { phase: 'downloading' }
  | { phase: 'downloading_mmproj' }
  | { phase: 'verifying' }
  | { phase: 'installing' }
  | { phase: 'warming_up' }
  | { phase: 'ready' }
  | { phase: 'resume_pending' }
  | { phase: 'failed'; kind: DownloadUiFailKind; message: string };

/**
 * True while a download is active but not yet terminal: bytes still moving
 * (`downloading`/`downloading_mmproj`) or the post-download verify/install/warm
 * steps running. False for idle, the pre-flight confirm/resume states, and the
 * terminal `ready`/`failed`. Shared by the picker's "Continue setup" line, the
 * ambient strip, and the submit soft-block so all three agree on "in flight".
 */
export function isDownloadInFlight(phase: DownloadUiState['phase']): boolean {
  return (
    phase === 'downloading' ||
    phase === 'downloading_mmproj' ||
    phase === 'verifying' ||
    phase === 'installing' ||
    phase === 'warming_up'
  );
}

/**
 * A short, jargon-free reason for a failed download, by kind, so the ambient
 * strip tells the user what actually went wrong instead of a generic message.
 */
export function downloadFailureMessage(kind: DownloadUiFailKind): string {
  switch (kind) {
    case 'offline':
      return 'You appear to be offline.';
    case 'http':
      return 'Hugging Face had an error. Try again.';
    case 'checksum':
      return 'The download did not verify. Retrying starts it fresh.';
    case 'disk_full':
      return 'Not enough disk space.';
    case 'engine':
      return "Thuki's engine could not start.";
    case 'other':
      return 'Model download failed.';
  }
}

/** Last reported byte counts for the file currently downloading. */
export interface DownloadProgressInfo {
  file: string;
  bytes: number;
  totalBytes: number;
}

/** One ETA sample: a Progress event's byte count and arrival time. */
interface EtaSample {
  t: number;
  bytes: number;
}

/** Rolling-rate window: only Progress samples this recent feed the ETA. */
const ETA_WINDOW_MS = 10_000;

/**
 * Bytes per second from the rolling sample window, or `null` while the rate
 * is not yet measurable (fewer than two samples, zero elapsed time, or no
 * forward progress between the window's edges).
 */
export function computeSpeedBytesPerSec(samples: EtaSample[]): number | null {
  if (samples.length < 2) return null;
  const first = samples[0];
  const last = samples[samples.length - 1];
  const elapsedSeconds = (last.t - first.t) / 1000;
  const deltaBytes = last.bytes - first.bytes;
  if (elapsedSeconds <= 0 || deltaBytes <= 0) return null;
  return deltaBytes / elapsedSeconds;
}

/**
 * Remaining seconds from the rolling sample window, or `null` while the
 * rate is not yet measurable (fewer than two samples, zero elapsed time,
 * or no forward progress between the window's edges).
 */
export function computeEtaSeconds(
  samples: EtaSample[],
  bytes: number,
  totalBytes: number,
): number | null {
  const bytesPerSecond = computeSpeedBytesPerSec(samples);
  if (bytesPerSecond === null) return null;
  return Math.max(0, Math.round((totalBytes - bytes) / bytesPerSecond));
}

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
  /** confirming -> downloading; invokes `download_starter` with a channel. */
  start: (tier: StarterTier) => Promise<void>;
  /**
   * idle -> downloading for a pasted-repo model; invokes `download_repo_model`
   * with a channel. Same event stream and terminal states as `start`.
   */
  startRepo: (repo: string, file: string) => Promise<void>;
  /**
   * Invokes `cancel_model_download`. The state flips back to idle when the
   * backend's Cancelled event lands; the partial is KEPT, so the caller
   * refreshes options to surface resume_pending.
   */
  cancel: () => Promise<void>;
  /**
   * failed -> downloading. A checksum failure already deleted the partial
   * on the backend, so retrying is just starting the same download (starter
   * tier or pasted repo, whichever ran last) again.
   */
  retry: () => Promise<void>;
  /** resume_pending -> downloading; the backend resumes via Range. */
  resume: (tier: StarterTier) => Promise<void>;
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

  const [state, setState] = useState<DownloadUiState>({ phase: 'idle' });
  const [progress, setProgress] = useState<DownloadProgressInfo | null>(null);
  const [etaSeconds, setEtaSeconds] = useState<number | null>(null);
  const [combinedBytes, setCombinedBytes] = useState<number | null>(null);
  const [speedBytesPerSec, setSpeedBytesPerSec] = useState<number | null>(null);

  const samplesRef = useRef<EtaSample[]>([]);
  const startedCountRef = useRef(0);
  /** Bytes from files that have already fully completed this run. */
  const completedBytesRef = useRef(0);
  /** Declared total of the file currently downloading. */
  const currentFileTotalRef = useRef(0);
  /** Replays the most recent start (tier or repo) for `retry`. */
  const lastStartRef = useRef<(() => Promise<void>) | null>(null);

  const handleEvent = useCallback(
    (event: DownloadEvent) => {
      switch (event.type) {
        case 'Started': {
          startedCountRef.current += 1;
          samplesRef.current = [];
          setEtaSeconds(null);
          setSpeedBytesPerSec(null);
          currentFileTotalRef.current = event.data.total_bytes;
          setProgress({
            file: event.data.file,
            bytes: event.data.resumed_from,
            totalBytes: event.data.total_bytes,
          });
          setCombinedBytes(completedBytesRef.current + event.data.resumed_from);
          // The second Started is always the mmproj companion: specs are
          // ordered weights first, mmproj second.
          setState(
            startedCountRef.current >= 2
              ? { phase: 'downloading_mmproj' }
              : { phase: 'downloading' },
          );
          break;
        }
        case 'Progress': {
          const now = Date.now();
          const samples = samplesRef.current;
          samples.push({ t: now, bytes: event.data.bytes });
          while (samples.length > 0 && now - samples[0].t > ETA_WINDOW_MS) {
            samples.shift();
          }
          setProgress({
            file: event.data.file,
            bytes: event.data.bytes,
            totalBytes: event.data.total_bytes,
          });
          setEtaSeconds(
            computeEtaSeconds(
              samples,
              event.data.bytes,
              event.data.total_bytes,
            ),
          );
          setSpeedBytesPerSec(computeSpeedBytesPerSec(samples));
          setCombinedBytes(completedBytesRef.current + event.data.bytes);
          // A resume re-hash labels itself `verifying` before the remaining
          // bytes stream; the first streamed Progress returns the label to the
          // active downloading phase so the transfer is not mislabeled. Any
          // other phase is left untouched (same reference → no re-render).
          setState((prev) =>
            prev.phase === 'verifying'
              ? startedCountRef.current >= 2
                ? { phase: 'downloading_mmproj' }
                : { phase: 'downloading' }
              : prev,
          );
          break;
        }
        case 'Verifying':
          setState({ phase: 'verifying' });
          break;
        case 'FileDone':
          // Fold this file's bytes into the completed total and snap the
          // cumulative figure to the boundary so the bar never dips. The next
          // Started (mmproj) or AllDone moves the state.
          completedBytesRef.current += currentFileTotalRef.current;
          currentFileTotalRef.current = 0;
          setCombinedBytes(completedBytesRef.current);
          break;
        case 'AllDone':
          setState(awaitEngine ? { phase: 'installing' } : { phase: 'ready' });
          break;
        case 'Cancelled':
          setProgress(null);
          setEtaSeconds(null);
          setSpeedBytesPerSec(null);
          setCombinedBytes(null);
          completedBytesRef.current = 0;
          currentFileTotalRef.current = 0;
          setState({ phase: 'idle' });
          break;
        case 'Failed':
          // Terminal from ANY state, including verifying (finalize failure:
          // the manifest write failed, so AllDone never arrives).
          setState({
            phase: 'failed',
            kind: event.data.kind,
            message: event.data.message,
          });
          break;
      }
    },
    [awaitEngine],
  );

  useEffect(() => {
    if (!awaitEngine) return;
    const unlistenPromise = listen<EngineStatus>('engine:status', (event) => {
      const status = event.payload;
      setState((prev) => {
        if (prev.phase !== 'installing' && prev.phase !== 'warming_up') {
          return prev;
        }
        if (status.state === 'starting') return { phase: 'warming_up' };
        if (status.state === 'loaded') return { phase: 'ready' };
        if (status.state === 'failed') {
          return {
            phase: 'failed',
            kind: 'engine',
            message: status.error ?? 'the engine could not start',
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
    setState({ phase: 'confirming', tier });
  }, []);

  const cancelConfirm = useCallback(() => {
    setState({ phase: 'idle' });
  }, []);

  /** Shared start path: resets per-run trackers, wires the event channel,
   * and invokes the given download command. */
  const run = useCallback(
    async (command: string, args: Record<string, unknown>) => {
      startedCountRef.current = 0;
      samplesRef.current = [];
      completedBytesRef.current = 0;
      currentFileTotalRef.current = 0;
      setProgress(null);
      setEtaSeconds(null);
      setSpeedBytesPerSec(null);
      setCombinedBytes(null);
      setState({ phase: 'downloading' });
      const channel = new Channel<DownloadEvent>();
      channel.onmessage = handleEvent;
      try {
        await invoke(command, { ...args, onEvent: channel });
      } catch (err) {
        setState({ phase: 'failed', kind: 'other', message: String(err) });
      }
    },
    [handleEvent],
  );

  const start = useCallback(
    async (tier: StarterTier) => {
      const replay = () => run('download_starter', { tier });
      lastStartRef.current = replay;
      await replay();
    },
    [run],
  );

  const startRepo = useCallback(
    async (repo: string, file: string) => {
      const replay = () => run('download_repo_model', { repo, file });
      lastStartRef.current = replay;
      await replay();
    },
    [run],
  );

  const cancel = useCallback(async () => {
    await invoke('cancel_model_download');
  }, []);

  const retry = useCallback(async () => {
    const replay = lastStartRef.current;
    if (replay === null) return;
    await replay();
  }, []);

  const discard = useCallback(async (sha256: string) => {
    try {
      await invoke('discard_partial_download', { sha256 });
    } catch (err) {
      setState({ phase: 'failed', kind: 'other', message: String(err) });
      return;
    }
    setState({ phase: 'idle' });
  }, []);

  const enterResumePending = useCallback(() => {
    setState({ phase: 'resume_pending' });
  }, []);

  const reset = useCallback(() => {
    setState((prev) =>
      prev.phase === 'failed' || prev.phase === 'ready'
        ? { phase: 'idle' }
        : prev,
    );
    // Stale byte counts from the run that just ended; the next start
    // reseeds them. Callers only invoke reset from the terminal cards.
    setProgress(null);
    setEtaSeconds(null);
    setSpeedBytesPerSec(null);
    setCombinedBytes(null);
    completedBytesRef.current = 0;
    currentFileTotalRef.current = 0;
  }, []);

  return {
    state,
    progress,
    etaSeconds,
    combinedBytes,
    speedBytesPerSec,
    beginConfirm,
    cancelConfirm,
    start,
    startRepo,
    cancel,
    retry,
    resume: start,
    discard,
    enterResumePending,
    reset,
  };
}
