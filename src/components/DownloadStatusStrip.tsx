/**
 * Ambient model-download indicator for the ask bar and the onboarding intro.
 *
 * A borderless status line, not a floating chip: a thin progress edge rides
 * the top, and a single row below it carries the label, the live figures, and
 * the inline controls. It blends into whatever surface sits behind it (the ask
 * bar, or the intro overlay's own surface), so it reads as part of the bar
 * rather than a separate box. It is the only place the background download is
 * surfaced once the user has left the picker.
 */
import { useEffect, useState, type ReactNode } from 'react';
import { AnimatePresence, motion } from 'framer-motion';

/** The strip's states, mirroring the download machine plus a paused hop. */
export type DownloadStripStatus =
  | {
      kind: 'downloading';
      /** Display name of the model being downloaded, e.g. "Qwen3.5 9B". */
      modelName: string;
      percent: number;
      etaSeconds: number | null;
      onPause: () => void;
    }
  | {
      kind: 'paused';
      percent: number;
      onResume: () => void;
    }
  | { kind: 'pausing'; percent: number }
  | { kind: 'verifying'; percent: number }
  | { kind: 'ready'; modelName: string }
  | { kind: 'failed'; message: string; onRetry: () => void };

/**
 * How long each half of the downloading label shows before crossfading to the
 * other. Slow on purpose: the strip is ambient, so the swap should be a calm
 * background rhythm, not something that pulls the eye.
 */
const LABEL_ROTATE_MS = 12000;
/**
 * The reassurance half of the alternating label (ask bar only): closing Thuki
 * keeps the download going, but quitting stops it. Short and succinct.
 */
const BACKGROUND_HINT = "Safe to close, just don't quit";

const ORANGE = 'rgb(255,141,92)';
const ORANGE_FILL = 'linear-gradient(90deg,#ffa06f,#d45a1e)';
const MUTED = 'rgba(255,255,255,0.4)';
const MUTED_FILL = 'rgba(255,255,255,0.28)';
const GREEN = 'rgb(95,207,134)';
const GREEN_FILL = '#5fcf86';
const RED = 'rgb(239,68,68)';
const RED_FILL = '#ef4444';
/** Brand-orange used for the primary inline action (Resume / Retry). */
const ACTION = '#ff8d5c';

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

function Action({
  label,
  ariaLabel,
  color,
  onClick,
}: {
  label: string;
  ariaLabel: string;
  color: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={ariaLabel}
      onClick={onClick}
      className="shrink-0 font-bold cursor-pointer"
      style={{ color, background: 'transparent', border: 'none' }}
    >
      {label}
    </button>
  );
}

/**
 * Borderless shell: a top progress edge filled to `percent` plus the row. No
 * box or tint of its own, so it inherits the surface behind it.
 */
function Shell({
  color,
  fill,
  percent,
  children,
}: {
  color: string;
  fill: string;
  percent: number;
  children: ReactNode;
}) {
  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="download-status-strip"
      className="mx-4 mt-2 mb-0"
      style={{ color: 'var(--color-text-primary, #f0f0f2)' }}
    >
      <span
        aria-hidden="true"
        className="block h-[2px] rounded-full overflow-hidden"
        style={{ background: 'rgba(255,255,255,0.08)' }}
      >
        <span
          className="block h-full rounded-full"
          style={{ width: `${percent}%`, background: fill }}
        />
      </span>
      <div className="flex items-center gap-2.5 pt-1.5 text-xs">
        <Dot color={color} />
        {children}
      </div>
    </div>
  );
}

export function DownloadStatusStrip({
  status,
  alternate = false,
}: {
  status: DownloadStripStatus;
  /**
   * When true, the downloading label alternates with the "safe to close" hint.
   * Used on the ask bar; the intro shows just the model name (the hint would
   * read oddly on a full setup screen the user is looking at).
   */
  alternate?: boolean;
}) {
  if (status.kind === 'ready') {
    return (
      <Shell color={GREEN} fill={GREEN_FILL} percent={100}>
        <span className="flex-1 leading-snug">
          {status.modelName} ready. Send your first message!
        </span>
      </Shell>
    );
  }

  if (status.kind === 'failed') {
    return (
      <Shell color={RED} fill={RED_FILL} percent={100}>
        <span className="flex-1 leading-snug">{status.message}</span>
        <Action
          label="Retry"
          ariaLabel="Retry download"
          color={ACTION}
          onClick={status.onRetry}
        />
      </Shell>
    );
  }

  if (status.kind === 'pausing') {
    return (
      <Shell color={MUTED} fill={MUTED_FILL} percent={status.percent}>
        <span className="flex-1 leading-snug">Pausing…</span>
      </Shell>
    );
  }

  if (status.kind === 'verifying') {
    // The integrity re-hash on resume (and the brief end-of-download verify):
    // an active working step, so it keeps the orange treatment but offers no
    // controls of its own. The re-hash of a multi-GB partial is a slow read, so
    // the sub-line reassures the user it is working rather than hung.
    return (
      <Shell color={ORANGE} fill={ORANGE_FILL} percent={status.percent}>
        <span className="flex-1 flex flex-col leading-snug">
          <span>Verifying…</span>
          <span style={{ color: MUTED }} className="text-[11px]">
            This can take a minute for large models
          </span>
        </span>
      </Shell>
    );
  }

  if (status.kind === 'paused') {
    // Resume only here. Discard belongs to the picker, where a Download button
    // can re-trigger; in the ambient strip a discard would strand the user with
    // no way back to start a download.
    return (
      <Shell color={MUTED} fill={MUTED_FILL} percent={status.percent}>
        <span className="flex-1 leading-snug">Paused · {status.percent}%</span>
        <Action
          label="Resume"
          ariaLabel="Resume download"
          color={ACTION}
          onClick={status.onResume}
        />
      </Shell>
    );
  }

  return <DownloadingRow status={status} alternate={alternate} />;
}

/**
 * The byte-moving downloading row. On the ask bar (`alternate`) its label
 * crossfades between the model name and the "safe to close" reassurance so both
 * fit the single line; on the intro it stays the model name. The percent, ETA,
 * and Pause stay fixed.
 */
function DownloadingRow({
  status,
  alternate,
}: {
  status: Extract<DownloadStripStatus, { kind: 'downloading' }>;
  alternate: boolean;
}) {
  const [showHint, setShowHint] = useState(false);
  useEffect(() => {
    if (!alternate) return;
    const id = setInterval(() => setShowHint((s) => !s), LABEL_ROTATE_MS);
    return () => clearInterval(id);
  }, [alternate]);

  const label =
    alternate && showHint ? BACKGROUND_HINT : `Downloading ${status.modelName}`;
  const trailing =
    status.etaSeconds !== null
      ? `${status.percent}% · ${formatEta(status.etaSeconds)} left`
      : `${status.percent}%`;
  return (
    <Shell color={ORANGE} fill={ORANGE_FILL} percent={status.percent}>
      {/* Crossfade between the two labels so the swap is a soft dissolve, not
          a hard cut. mode="wait" fades the old out before the new fades in. */}
      <AnimatePresence mode="wait">
        <motion.span
          key={label}
          className="leading-snug"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.45 }}
        >
          {label}
        </motion.span>
      </AnimatePresence>
      <span className="flex-1" />
      <span className="shrink-0">{trailing}</span>
      <Action
        label="Pause"
        ariaLabel="Pause download"
        color="rgba(255,255,255,0.55)"
        onClick={status.onPause}
      />
    </Shell>
  );
}
