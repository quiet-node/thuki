/**
 * Ambient model-download indicator for the ask bar and the onboarding intro.
 *
 * A sibling of {@link CapabilityMismatchStrip}: same compact strip shape and
 * margins, so it slots into the same spot above the input. It carries the
 * background download's state once the user has left the picker, the only
 * place the second file (vision companion) is ever surfaced as one figure.
 */
import type React from 'react';

/** The strip's three states, mirroring the download machine's terminal arc. */
export type DownloadStripStatus =
  | { kind: 'downloading'; percent: number; etaSeconds: number | null }
  | { kind: 'ready' }
  | { kind: 'failed'; message: string; onRetry: () => void };

const baseClass =
  'mx-4 mt-2 mb-0 flex items-center gap-2.5 px-3 py-2 rounded-lg border text-xs';

/** Seconds rendered as a compact countdown: "45s", "5m", "2h 1m". */
function formatEta(etaSeconds: number): string {
  if (etaSeconds < 60) return `${etaSeconds}s`;
  if (etaSeconds < 3600) return `${Math.floor(etaSeconds / 60)}m`;
  const hours = Math.floor(etaSeconds / 3600);
  const minutes = Math.floor((etaSeconds % 3600) / 60);
  return `${hours}h ${minutes}m`;
}

function Dot({ color }: { color: string }) {
  return (
    <span
      aria-hidden="true"
      className="shrink-0 w-2 h-2 rounded-full"
      style={{ background: color, boxShadow: `0 0 6px ${color}` }}
    />
  );
}

function Shell({
  color,
  tint,
  children,
}: {
  color: string;
  tint: string;
  children: React.ReactNode;
}) {
  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="download-status-strip"
      className={baseClass}
      style={{
        background: `${tint}1a`,
        borderColor: `${tint}4d`,
        color: 'var(--color-text-primary, #f0f0f2)',
      }}
    >
      <Dot color={color} />
      {children}
    </div>
  );
}

export function DownloadStatusStrip({
  status,
}: {
  status: DownloadStripStatus;
}) {
  if (status.kind === 'ready') {
    return (
      <Shell color="rgb(95,207,134)" tint="#5fcf86">
        <span className="flex-1 leading-snug">Model ready</span>
      </Shell>
    );
  }

  if (status.kind === 'failed') {
    return (
      <Shell color="rgb(239,68,68)" tint="#ef4444">
        <span className="flex-1 leading-snug">{status.message}</span>
        <button
          type="button"
          aria-label="Retry download"
          onClick={status.onRetry}
          className="shrink-0 font-bold cursor-pointer"
          style={{
            color: '#ff8d5c',
            background: 'transparent',
            border: 'none',
          }}
        >
          Retry
        </button>
      </Shell>
    );
  }

  const trailing =
    status.etaSeconds !== null
      ? `${status.percent}% · ${formatEta(status.etaSeconds)} left`
      : `${status.percent}%`;
  return (
    <Shell color="rgb(255,141,92)" tint="#ff8d5c">
      <span className="leading-snug">Setting up your model</span>
      <span
        aria-hidden="true"
        className="flex-1 h-[3px] rounded-full overflow-hidden"
        style={{ background: 'rgba(255,255,255,0.08)', maxWidth: 140 }}
      >
        <span
          className="block h-full rounded-full"
          style={{
            width: `${status.percent}%`,
            background: 'linear-gradient(90deg,#ffa06f,#d45a1e)',
          }}
        />
      </span>
      <span className="shrink-0">{trailing}</span>
    </Shell>
  );
}
