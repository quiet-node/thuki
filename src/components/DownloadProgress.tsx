/**
 * Presentational download flow card: one render per useDownloadModel state.
 *
 * The component owns the per-state copy (including the exact failure
 * strings) and emits plain callbacks; the state machine itself lives in
 * useDownloadModel so onboarding and Settings share both halves.
 */

import type React from 'react';
import type {
  DownloadProgressInfo,
  DownloadUiState,
} from '../hooks/useDownloadModel';

/** Disk headroom (GB) below which the confirm card warns. Warn, never block. */
const LOW_DISK_HEADROOM_GB = 2;

export interface ConfirmInfo {
  /** Total download size in decimal GB (weights + vision companion). */
  sizeGb: number;
  /** Free disk space in decimal GB; null hides the disk line entirely. */
  freeDiskGb: number | null;
  /** RAM-fit caution passed through from the picker; null hides it. */
  ramWarning: string | null;
}

export interface DownloadProgressProps {
  state: DownloadUiState;
  progress: DownloadProgressInfo | null;
  etaSeconds: number | null;
  /** Cumulative bytes across weights + companion: the unified numerator. */
  combinedBytes?: number | null;
  /** Full on-disk total (weights + companion): the unified denominator. */
  grandTotalBytes?: number | null;
  /** Rolling download rate in bytes per second; drives the unified ETA. */
  speedBytesPerSec?: number | null;
  confirmInfo?: ConfirmInfo;
  onConfirm: () => void;
  onCancelConfirm: () => void;
  onCancel: () => void;
  onRetry: () => void;
  /**
   * Renders a "Choose a different model" button on the failed card. Hosts
   * wire it to the hook's `reset` so a user stuck on a terminal failure
   * (disk full, checksum) can get back to the picker instead of being
   * limited to retrying the same download.
   */
  onChooseAnother?: () => void;
}

/** Seconds rendered as a compact countdown: "45s", "5m", "2h 1m". */
function formatEta(etaSeconds: number): string {
  if (etaSeconds < 60) return `${etaSeconds}s`;
  if (etaSeconds < 3600) return `${Math.floor(etaSeconds / 60)}m`;
  const hours = Math.floor(etaSeconds / 3600);
  const minutes = Math.floor((etaSeconds % 3600) / 60);
  return `${hours}h ${minutes}m`;
}

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "8.2"). */
function gb(bytes: number): string {
  return (bytes / 1e9).toFixed(1);
}

/** Inputs for the single download figures line. */
export interface DownloadLineInput {
  /** Per-file byte counts; the fallback when no grand total is known. */
  progress: DownloadProgressInfo | null;
  /** Rolling ETA seconds for the per-file fallback path. */
  etaSeconds: number | null;
  /** Cumulative bytes across weights + companion: the unified numerator. */
  combinedBytes: number | null;
  /** Full on-disk total (weights + companion): the unified denominator. */
  grandTotalBytes: number | null;
  /** Rolling rate; drives the unified ETA when present. */
  speedBytesPerSec: number | null;
}

/** Percent plus a "x / y GB · ~eta" string, or null figures before any bytes. */
export interface DownloadLine {
  percent: number;
  figures: string | null;
}

/**
 * One continuous progress reading. Prefers the unified weights + companion
 * figure, so a vision download is a single bar to 100% that never resets
 * between the two files; falls back to the current file's own byte counts for
 * single-file repo downloads where no grand total is known up front.
 */
export function downloadLine({
  progress,
  etaSeconds,
  combinedBytes,
  grandTotalBytes,
  speedBytesPerSec,
}: DownloadLineInput): DownloadLine {
  let bytes: number;
  let total: number;
  let eta: number | null;
  if (
    grandTotalBytes !== null &&
    grandTotalBytes > 0 &&
    combinedBytes !== null
  ) {
    bytes = combinedBytes;
    total = grandTotalBytes;
    eta =
      speedBytesPerSec !== null
        ? Math.max(0, Math.round((total - bytes) / speedBytesPerSec))
        : etaSeconds;
  } else if (progress !== null && progress.totalBytes > 0) {
    bytes = progress.bytes;
    total = progress.totalBytes;
    eta = etaSeconds;
  } else {
    return { percent: 0, figures: null };
  }
  const percent = Math.min(100, Math.floor((bytes / total) * 100));
  const figures =
    `${gb(bytes)} / ${gb(total)} GB` +
    (eta !== null ? ` · ~${formatEta(eta)}` : '');
  return { percent, figures };
}

/** Failure headline per kind. Exact copy; consumed verbatim by tests. */
function failureHeadline(kind: string, message: string): string {
  switch (kind) {
    case 'offline':
      return 'You appear to be offline.';
    case 'http': {
      const status = /\b(\d{3})\b/.exec(message);
      return status
        ? `Hugging Face returned an error (status ${status[1]}).`
        : 'Hugging Face returned an error.';
    }
    case 'checksum':
      return "Download didn't verify. Retrying re-downloads it.";
    case 'disk_full':
      return 'Not enough disk space. Free up space and retry.';
    case 'engine':
      return "Thuki's engine could not start.";
    default:
      return message;
  }
}

export function DownloadProgress({
  state,
  progress,
  etaSeconds,
  combinedBytes = null,
  grandTotalBytes = null,
  speedBytesPerSec = null,
  confirmInfo,
  onConfirm,
  onCancelConfirm,
  onCancel,
  onRetry,
  onChooseAnother,
}: DownloadProgressProps) {
  switch (state.phase) {
    case 'confirming':
      return (
        <Card>
          {confirmInfo ? (
            <>
              <Headline>{confirmInfo.sizeGb.toFixed(1)} GB download.</Headline>
              {confirmInfo.freeDiskGb !== null ? (
                <Detail>
                  {confirmInfo.freeDiskGb.toFixed(1)} GB free on this disk.
                </Detail>
              ) : null}
              {confirmInfo.freeDiskGb !== null &&
              confirmInfo.freeDiskGb <
                confirmInfo.sizeGb + LOW_DISK_HEADROOM_GB ? (
                <Detail warn>
                  Low on disk space. The download may not fit.
                </Detail>
              ) : null}
              {confirmInfo.ramWarning !== null ? (
                <Detail warn>{confirmInfo.ramWarning}</Detail>
              ) : null}
            </>
          ) : null}
          <ButtonRow>
            <FlowButton label="Download" primary onClick={onConfirm} />
            <FlowButton label="Cancel" onClick={onCancelConfirm} />
          </ButtonRow>
        </Card>
      );
    case 'downloading':
    case 'downloading_mmproj': {
      const { percent, figures } = downloadLine({
        progress,
        etaSeconds,
        combinedBytes,
        grandTotalBytes,
        speedBytesPerSec,
      });
      return (
        <Hairline edge={<Edge percent={percent} tone="accent" />}>
          <span data-testid="download-figures" style={FIGURES_STYLE}>
            <strong style={{ color: '#f0f0f2', fontWeight: 700 }}>
              {percent}%
            </strong>
            {figures !== null ? ` · ${figures}` : ''}
            {state.phase === 'downloading_mmproj' ? ' · finishing vision' : ''}
          </span>
          <span style={{ flex: 1 }} />
          <CancelX onClick={onCancel} />
        </Hairline>
      );
    }
    case 'verifying':
      return (
        <Hairline edge={<Edge indeterminate tone="accent" />}>
          <StatusText>Verifying download</StatusText>
        </Hairline>
      );
    case 'installing':
      return (
        <Hairline edge={<Edge indeterminate tone="accent" />}>
          <StatusText>Installing</StatusText>
        </Hairline>
      );
    case 'warming_up':
      return (
        <Hairline edge={<Edge indeterminate tone="accent" />}>
          <StatusText>Starting the engine</StatusText>
        </Hairline>
      );
    case 'ready':
      return (
        <Hairline edge={<Edge percent={100} tone="green" />}>
          <StatusText ready>
            <svg width="12" height="12" viewBox="0 0 16 16" fill="none">
              <path
                d="M3 8.5l3.2 3.2L13 5"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
            Ready
          </StatusText>
        </Hairline>
      );
    case 'failed':
      return (
        <Hairline edge={<Edge percent={100} tone="red" />}>
          <span style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <span style={{ fontSize: 12.5, fontWeight: 600, color: '#ff7a6e' }}>
              {failureHeadline(state.kind, state.message)}
            </span>
            {state.kind === 'http' ? (
              <span style={FIGURES_STYLE}>{state.message}</span>
            ) : null}
          </span>
          <span style={{ flex: 1 }} />
          <GhostButton label="Retry" tone="accent" onClick={onRetry} />
          {onChooseAnother ? (
            <GhostButton
              label="Choose a different model"
              tone="muted"
              onClick={onChooseAnother}
            />
          ) : null}
        </Hairline>
      );
    default:
      // idle and resume_pending have no progress UI; the picker owns them.
      return null;
  }
}

function Card({ children }: { children: React.ReactNode }) {
  return (
    <div
      data-download-progress
      style={{
        padding: '12px 14px',
        borderRadius: 14,
        border: '1px solid rgba(255,255,255,0.06)',
        background: 'rgba(255,255,255,0.03)',
        display: 'flex',
        flexDirection: 'column',
        gap: 6,
      }}
    >
      {children}
    </div>
  );
}

function Headline({ children }: { children: React.ReactNode }) {
  return (
    <p
      style={{
        fontSize: 13,
        fontWeight: 600,
        color: '#f0f0f2',
        letterSpacing: '-0.1px',
        lineHeight: 1.4,
        margin: 0,
      }}
    >
      {children}
    </p>
  );
}

function Detail({
  children,
  warn = false,
}: {
  children: React.ReactNode;
  warn?: boolean;
}) {
  return (
    <p
      style={{
        fontSize: 11,
        color: warn ? '#ff8d5c' : 'rgba(255,255,255,0.45)',
        lineHeight: 1.5,
        margin: 0,
      }}
    >
      {children}
    </p>
  );
}

/** Subtitle figures line: muted, tabular so the digits do not jitter. */
const FIGURES_STYLE: React.CSSProperties = {
  fontSize: 11.5,
  color: 'rgba(236,234,231,0.54)',
  fontVariantNumeric: 'tabular-nums',
  lineHeight: 1.4,
};

/**
 * Inline shell for every active state: one quiet line with a 2px accent edge
 * pinned to the bottom of the row (the hairline). No box of its own, so the
 * download reads as part of the model row rather than a nested card.
 */
function Hairline({
  children,
  edge,
}: {
  children: React.ReactNode;
  edge: React.ReactNode;
}) {
  return (
    <div
      data-download-progress
      style={{
        position: 'relative',
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '8px 2px 12px',
        minHeight: 30,
      }}
    >
      {children}
      {edge}
    </div>
  );
}

/**
 * The 2px progress edge. Determinate fills to `percent`; indeterminate shows a
 * fixed segment. `tone` is the warm accent while working and green at ready.
 */
function Edge({
  percent = 0,
  indeterminate = false,
  tone,
}: {
  percent?: number;
  indeterminate?: boolean;
  tone: 'accent' | 'green' | 'red';
}) {
  const fill =
    tone === 'green'
      ? '#5fcf86'
      : tone === 'red'
        ? '#ef6b6b'
        : 'linear-gradient(90deg, #ffa06f, #d45a1e)';
  return (
    <span
      data-progress-bar
      data-indeterminate={indeterminate}
      style={{
        position: 'absolute',
        left: 0,
        right: 0,
        bottom: 0,
        height: 2,
        borderRadius: 999,
        background: 'rgba(255,255,255,0.08)',
        overflow: 'hidden',
      }}
    >
      <span
        style={{
          position: 'absolute',
          left: 0,
          top: 0,
          bottom: 0,
          width: indeterminate ? '40%' : `${percent}%`,
          borderRadius: 999,
          background: fill,
        }}
      />
    </span>
  );
}

/** A borderless text button for the inline hairline actions (Retry, etc.). */
function GhostButton({
  label,
  tone,
  onClick,
}: {
  label: string;
  tone: 'accent' | 'muted';
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        background: 'transparent',
        border: 'none',
        fontFamily: 'inherit',
        fontSize: 11.5,
        fontWeight: 700,
        cursor: 'pointer',
        whiteSpace: 'nowrap',
        padding: '2px 4px',
        color: tone === 'accent' ? '#ff8d5c' : 'rgba(236,234,231,0.54)',
      }}
    >
      {label}
    </button>
  );
}

/** The single status line for the post-download steps (and the ready check). */
function StatusText({
  children,
  ready = false,
}: {
  children: React.ReactNode;
  ready?: boolean;
}) {
  return (
    <p
      style={{
        margin: 0,
        fontSize: 12.5,
        fontWeight: 600,
        color: ready ? '#5fcf86' : '#f0f0f2',
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        lineHeight: 1.4,
      }}
    >
      {children}
    </p>
  );
}

/** The inline cancel control: a quiet × that warms on hover via the theme. */
function CancelX({ onClick }: { onClick: () => void }) {
  return (
    <button
      aria-label="Cancel"
      onClick={onClick}
      style={{
        background: 'transparent',
        border: 'none',
        color: 'rgba(236,234,231,0.34)',
        fontSize: 15,
        lineHeight: 1,
        cursor: 'pointer',
        fontFamily: 'inherit',
        padding: '2px 6px',
      }}
    >
      ✕
    </button>
  );
}

interface FlowButtonProps {
  label: string;
  onClick: () => void;
  primary?: boolean;
}

function FlowButton({ label, onClick, primary = false }: FlowButtonProps) {
  return (
    <button
      onClick={onClick}
      style={{
        padding: '6px 12px',
        borderRadius: 8,
        background: primary
          ? 'linear-gradient(135deg, #ff8d5c 0%, #d45a1e 100%)'
          : 'rgba(255,255,255,0.04)',
        border: primary ? 'none' : '1px solid rgba(255,255,255,0.1)',
        color: primary ? 'white' : 'rgba(255,255,255,0.7)',
        fontSize: 11.5,
        fontWeight: 600,
        fontFamily: 'inherit',
        cursor: 'pointer',
      }}
    >
      {label}
    </button>
  );
}

function ButtonRow({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>{children}</div>
  );
}
