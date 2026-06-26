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
import { invoke } from '@tauri-apps/api/core';

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
 * Whether the strip represents an in-flight first-model download: bytes still
 * moving, paused, or being verified. The model picker uses this to swap its
 * empty-state copy, since "download one in Settings" reads wrong while a
 * download is visibly underway right below the list. `ready` / `failed` /
 * absent are not in-flight.
 */
export function isDownloadActive(status: DownloadStripStatus | null): boolean {
  if (status === null) return false;
  return (
    status.kind === 'downloading' ||
    status.kind === 'paused' ||
    status.kind === 'pausing' ||
    status.kind === 'verifying'
  );
}

/**
 * How long each half of the downloading label shows before crossfading to the
 * other. Kept ambient (a calm background rhythm, not something that pulls the
 * eye), but short enough that the reassurance half appears promptly rather than
 * making the user wait to learn the download survives a close.
 */
const LABEL_ROTATE_MS = 5000;
/**
 * Control-key glyph rendered as a small keycap so it reads as a key rather than
 * a bare caret. Mirrors the intro tips' key chips.
 */
function ControlKeyCap() {
  return (
    <span
      style={{
        display: 'inline-block',
        padding: '0 4px',
        margin: '0 1px',
        background: 'rgba(255,255,255,0.08)',
        border: '1px solid rgba(255,255,255,0.16)',
        borderBottom: '2px solid rgba(255,255,255,0.1)',
        borderRadius: 4,
        fontSize: '0.85em',
        lineHeight: 1.3,
        verticalAlign: 'baseline',
      }}
    >
      ⌃
    </span>
  );
}

/**
 * The reassurance half of the alternating label (ask bar only): closing Thuki
 * keeps the download going, but quitting stops it. Double-tapping Control (the
 * toggle hotkey, shown as a keycap) closes the visible overlay, since there is
 * no window chrome to click.
 */
function BackgroundHint() {
  return (
    <>
      Safe to close (<ControlKeyCap /> ×2), just don&apos;t quit
    </>
  );
}

/**
 * The third ask-bar label: an invitation to line up more models while this one
 * downloads. Only the word "Settings" is the link, styled like the inline
 * embedded links elsewhere in the ask bar (brand-orange `text-primary`,
 * underlined, pointer); clicking opens Settings → Models → Discover (the same
 * `open_settings_window` deep-link the model picker uses).
 */
function BrowsePrompt() {
  return (
    <>
      Browse more models in{' '}
      <button
        type="button"
        aria-label="Browse more models in Settings"
        onClick={() => void invoke('open_settings_window')}
        className="cursor-pointer text-primary underline underline-offset-2 hover:opacity-80"
        style={{
          background: 'transparent',
          border: 'none',
          padding: 0,
          font: 'inherit',
        }}
      >
        Settings
      </button>
    </>
  );
}

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

/**
 * Where the strip is rendered. The surface drives the two pieces of copy that
 * must differ by context: on the ask bar the downloading label alternates with
 * the "safe to close" hint and the ready line invites the first message, since
 * the compose surface is right there; during onboarding the hint would read
 * oddly on a full setup screen and the user cannot send yet, so the ready line
 * points at the "Get Started" button that actually opens the ask bar.
 *
 * `onboarding-roadmap` is the optional roadmap/email step shown before the tips
 * card: it has no "Get Started" button, so its ready line confirms readiness
 * without pointing at one.
 */
type DownloadStripSurface = 'askbar' | 'onboarding' | 'onboarding-roadmap';

export function DownloadStatusStrip({
  status,
  surface,
}: {
  status: DownloadStripStatus;
  surface: DownloadStripSurface;
}) {
  if (status.kind === 'ready') {
    return (
      <Shell color={GREEN} fill={GREEN_FILL} percent={100}>
        <span className="flex-1 leading-snug">
          {status.modelName} ready.{' '}
          {surface === 'askbar'
            ? 'Send your first message!'
            : surface === 'onboarding'
              ? 'Hit Get Started to start chatting!'
              : "You're good to go!"}
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

  return <DownloadingRow status={status} alternate={surface === 'askbar'} />;
}

/**
 * The byte-moving downloading row. On the ask bar (`alternate`) its label
 * crossfades through three phases so each fits the single line: the model name,
 * the "safe to close" reassurance, and an invitation to browse more models in
 * Settings. On the intro it stays the model name. The percent, ETA, and Pause
 * stay fixed.
 */
function DownloadingRow({
  status,
  alternate,
}: {
  status: Extract<DownloadStripStatus, { kind: 'downloading' }>;
  alternate: boolean;
}) {
  const [phase, setPhase] = useState(0);
  useEffect(() => {
    if (!alternate) return;
    const id = setInterval(() => setPhase((p) => (p + 1) % 3), LABEL_ROTATE_MS);
    return () => clearInterval(id);
  }, [alternate]);

  // Off the ask bar the label never rotates: only the model name shows.
  const activePhase = alternate ? phase : 0;
  // Stable string key for the crossfade (two phases render JSX, not a string).
  const labelKey =
    activePhase === 1
      ? 'safe-to-close'
      : activePhase === 2
        ? 'browse-models'
        : `downloading:${status.modelName}`;
  const trailing =
    status.etaSeconds !== null
      ? `${status.percent}% · ${formatEta(status.etaSeconds)} left`
      : `${status.percent}%`;
  return (
    <Shell color={ORANGE} fill={ORANGE_FILL} percent={status.percent}>
      {/* Crossfade between the labels so each swap is a soft dissolve, not a
          hard cut. mode="wait" fades the old out before the new fades in. */}
      <AnimatePresence mode="wait">
        <motion.span
          key={labelKey}
          className="leading-snug"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.45 }}
        >
          {activePhase === 1 ? (
            <BackgroundHint />
          ) : activePhase === 2 ? (
            <BrowsePrompt />
          ) : (
            `Downloading ${status.modelName}`
          )}
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
