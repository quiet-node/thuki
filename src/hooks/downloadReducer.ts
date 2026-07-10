/**
 * Pure state for a single model download, plus the reducer that advances it on
 * each backend `DownloadEvent`.
 *
 * This is the one source of truth for "what a download channel's events mean".
 * The single-download onboarding hook ({@link useDownloadModel}) and the
 * multi-download Settings registry ({@link useDownloads}) both drive their state
 * through {@link reduceDownloadEvent}, so the two never diverge. The reducer is
 * pure (no React, no refs, no I/O): the byte accumulators that the old hook kept
 * in refs live on the accumulator here, so a registry can hold one per download.
 *
 * The post-download engine handoff (`installing -> warming_up -> ready`, driven
 * by the `engine:status` event when `awaitEngine` is set) is NOT modeled here:
 * it is a separate event stream owned by the onboarding hook. This reducer only
 * interprets the per-download `DownloadEvent` channel.
 */

import type {
  DownloadEvent,
  DownloadFailKind,
  StarterTier,
} from '../types/starter';

/** Failure kinds the UI can show: the backend's plus the engine handoff's. */
export type DownloadUiFailKind = DownloadFailKind | 'engine';

/** The download UI state machine's discriminated union. */
export type DownloadUiState =
  | { phase: 'idle' }
  | { phase: 'confirming'; tier: StarterTier }
  | { phase: 'queued' }
  | { phase: 'downloading' }
  | { phase: 'downloading_mmproj' }
  | { phase: 'verifying' }
  | { phase: 'installing' }
  | { phase: 'warming_up' }
  | { phase: 'ready' }
  | { phase: 'resume_pending' }
  | { phase: 'rejected_safe_mode' }
  | { phase: 'failed'; kind: DownloadUiFailKind; message: string };

/** Last reported byte counts for the file currently downloading. */
export interface DownloadProgressInfo {
  file: string;
  bytes: number;
  totalBytes: number;
}

/** One ETA sample: a Progress event's byte count and arrival time. */
export interface EtaSample {
  t: number;
  bytes: number;
}

/** Rolling-rate window: only Progress samples this recent feed the ETA. */
const ETA_WINDOW_MS = 10_000;

/**
 * Everything needed to render one download and to fold the next event in. The
 * fields below `speedBytesPerSec` are internal accumulators (the old hook's
 * refs); consumers read the render fields and pass the whole accumulator back
 * into {@link reduceDownloadEvent}.
 */
export interface DownloadAccumulator {
  state: DownloadUiState;
  progress: DownloadProgressInfo | null;
  etaSeconds: number | null;
  /**
   * Cumulative bytes downloaded across every file of the current run (weights +
   * vision companion), or null when idle. One continuous figure: never resets
   * between the two files.
   */
  combinedBytes: number | null;
  /** Rolling download rate in bytes per second, or null until measurable. */
  speedBytesPerSec: number | null;
  /** Recent Progress samples inside the rolling ETA window. */
  samples: EtaSample[];
  /** How many `Started` events have arrived (1 = weights, 2 = mmproj). */
  startedCount: number;
  /** Bytes from files that have already fully completed this run. */
  completedBytes: number;
  /** Declared total of the file currently downloading. */
  currentFileTotal: number;
}

/** A fresh accumulator parked at `idle` with empty counters. */
export function initialAccumulator(): DownloadAccumulator {
  return {
    state: { phase: 'idle' },
    progress: null,
    etaSeconds: null,
    combinedBytes: null,
    speedBytesPerSec: null,
    samples: [],
    startedCount: 0,
    completedBytes: 0,
    currentFileTotal: 0,
  };
}

/** An accumulator reset to the start of a fresh run (phase `downloading`). */
export function startingAccumulator(): DownloadAccumulator {
  return { ...initialAccumulator(), state: { phase: 'downloading' } };
}

/**
 * True while a download is active but not yet terminal: bytes still moving
 * (`downloading`/`downloading_mmproj`) or the post-download verify/install/warm
 * steps running. False for idle, the pre-flight confirm/resume states, and the
 * terminal `ready`/`failed`.
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
 * A short, jargon-free reason for a failed download, by kind, so the UI tells
 * the user what actually went wrong instead of a generic message.
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

/**
 * Bytes per second from the rolling sample window, or `null` while the rate is
 * not yet measurable (fewer than two samples, zero elapsed time, or no forward
 * progress between the window's edges).
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
 * Remaining seconds from the rolling sample window, or `null` while the rate is
 * not yet measurable (fewer than two samples, zero elapsed time, or no forward
 * progress between the window's edges).
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

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "8.2"). */
function gb(bytes: number): string {
  return (bytes / 1024 ** 3).toFixed(1);
}

/** Appends a sample and drops any that have aged out of the rolling window. */
function pushSample(
  samples: EtaSample[],
  sample: EtaSample,
  now: number,
): EtaSample[] {
  const next = [...samples, sample];
  let start = 0;
  while (start < next.length && now - next[start].t > ETA_WINDOW_MS) {
    start += 1;
  }
  return start > 0 ? next.slice(start) : next;
}

/**
 * Folds one backend `DownloadEvent` into the accumulator, returning a new
 * accumulator (the input is never mutated). `awaitEngine` decides the terminal
 * step: when set, `AllDone` parks in `installing` for the `engine:status`
 * handoff; otherwise it goes straight to `ready`.
 */
export function reduceDownloadEvent(
  acc: DownloadAccumulator,
  event: DownloadEvent,
  awaitEngine: boolean,
): DownloadAccumulator {
  switch (event.type) {
    case 'Queued':
      return { ...acc, state: { phase: 'queued' } };
    case 'InsufficientDisk':
      // Reuses the `failed`/`disk_full` phase (same red/Retry UI path a
      // Failed{kind:'disk_full'} event already drives) instead of a dedicated
      // phase, with a formatted detail line `failureHeadline` renders as the
      // second line.
      return {
        ...acc,
        state: {
          phase: 'failed',
          kind: 'disk_full',
          message: `Needs ~${gb(event.data.required_bytes)} GB, ~${gb(event.data.available_bytes)} GB free on disk.`,
        },
      };
    case 'RejectedSafeMode':
      return { ...acc, state: { phase: 'rejected_safe_mode' } };
    case 'Started': {
      const startedCount = acc.startedCount + 1;
      return {
        ...acc,
        startedCount,
        samples: [],
        etaSeconds: null,
        speedBytesPerSec: null,
        currentFileTotal: event.data.total_bytes,
        progress: {
          file: event.data.file,
          bytes: event.data.resumed_from,
          totalBytes: event.data.total_bytes,
        },
        combinedBytes: acc.completedBytes + event.data.resumed_from,
        // The second Started is always the mmproj companion: specs are ordered
        // weights first, mmproj second.
        state:
          startedCount >= 2
            ? { phase: 'downloading_mmproj' }
            : { phase: 'downloading' },
      };
    }
    case 'Progress': {
      const now = Date.now();
      const samples = pushSample(
        acc.samples,
        { t: now, bytes: event.data.bytes },
        now,
      );
      // A resume re-hash labels itself `verifying` before the remaining bytes
      // stream; the first streamed Progress returns the label to the active
      // downloading phase so the transfer is not mislabeled. Any other phase is
      // left untouched.
      const state: DownloadUiState =
        acc.state.phase === 'verifying'
          ? acc.startedCount >= 2
            ? { phase: 'downloading_mmproj' }
            : { phase: 'downloading' }
          : acc.state;
      return {
        ...acc,
        samples,
        state,
        progress: {
          file: event.data.file,
          bytes: event.data.bytes,
          totalBytes: event.data.total_bytes,
        },
        etaSeconds: computeEtaSeconds(
          samples,
          event.data.bytes,
          event.data.total_bytes,
        ),
        speedBytesPerSec: computeSpeedBytesPerSec(samples),
        combinedBytes: acc.completedBytes + event.data.bytes,
      };
    }
    case 'Verifying':
      return { ...acc, state: { phase: 'verifying' } };
    case 'FileDone': {
      // Fold this file's bytes into the completed total and snap the cumulative
      // figure to the boundary so the bar never dips. The next Started (mmproj)
      // or AllDone moves the state.
      const completedBytes = acc.completedBytes + acc.currentFileTotal;
      return {
        ...acc,
        completedBytes,
        currentFileTotal: 0,
        combinedBytes: completedBytes,
      };
    }
    case 'AllDone':
      return {
        ...acc,
        state: awaitEngine ? { phase: 'installing' } : { phase: 'ready' },
      };
    case 'Cancelled':
      return initialAccumulator();
    case 'Failed':
      // Terminal from ANY state, including verifying (finalize failure: the
      // manifest write failed, so AllDone never arrives).
      return {
        ...acc,
        state: {
          phase: 'failed',
          kind: event.data.kind,
          message: event.data.message,
        },
      };
  }
}
