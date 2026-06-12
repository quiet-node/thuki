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
    case 'downloading_mmproj':
      return (
        <Card>
          <Headline>
            {state.phase === 'downloading_mmproj'
              ? 'Downloading vision companion'
              : 'Downloading model'}
          </Headline>
          <ProgressBar
            percent={
              progress && progress.totalBytes > 0
                ? Math.floor((progress.bytes / progress.totalBytes) * 100)
                : 0
            }
          />
          {progress ? (
            <Detail>
              {gb(progress.bytes)} GB of {gb(progress.totalBytes)} GB
            </Detail>
          ) : null}
          {etaSeconds !== null ? (
            <Detail>About {formatEta(etaSeconds)} left</Detail>
          ) : null}
          <ButtonRow>
            <FlowButton label="Cancel" onClick={onCancel} />
          </ButtonRow>
        </Card>
      );
    case 'verifying':
      return (
        <Card>
          <Headline>Verifying download</Headline>
          <ProgressBar indeterminate />
        </Card>
      );
    case 'installing':
      return (
        <Card>
          <Headline>Installing</Headline>
          <ProgressBar indeterminate />
        </Card>
      );
    case 'warming_up':
      return (
        <Card>
          <Headline>Starting the engine</Headline>
          <ProgressBar indeterminate />
        </Card>
      );
    case 'ready':
      return (
        <Card>
          <Headline>
            <span
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 6,
                color: '#22c55e',
              }}
            >
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
            </span>
          </Headline>
        </Card>
      );
    case 'failed':
      return (
        <Card>
          <Headline>{failureHeadline(state.kind, state.message)}</Headline>
          {state.kind === 'http' ? <Detail>{state.message}</Detail> : null}
          <ButtonRow>
            <FlowButton label="Retry" primary onClick={onRetry} />
            {onChooseAnother ? (
              <FlowButton
                label="Choose a different model"
                onClick={onChooseAnother}
              />
            ) : null}
          </ButtonRow>
        </Card>
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

interface ProgressBarProps {
  percent?: number;
  indeterminate?: boolean;
}

function ProgressBar({ percent = 0, indeterminate = false }: ProgressBarProps) {
  return (
    <div>
      {!indeterminate ? (
        <div
          style={{
            textAlign: 'right',
            fontSize: 10.5,
            color: 'rgba(255,255,255,0.45)',
            marginBottom: 3,
          }}
        >
          {percent}%
        </div>
      ) : null}
      <div
        data-progress-bar
        data-indeterminate={indeterminate}
        style={{
          position: 'relative',
          height: 5,
          borderRadius: 999,
          background: 'rgba(255,255,255,0.06)',
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            position: 'absolute',
            top: 0,
            left: 0,
            bottom: 0,
            width: indeterminate ? '40%' : `${percent}%`,
            borderRadius: 999,
            background: 'linear-gradient(135deg, #ff8d5c 0%, #d45a1e 100%)',
            opacity: indeterminate ? 0.6 : 1,
          }}
        />
      </div>
    </div>
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
