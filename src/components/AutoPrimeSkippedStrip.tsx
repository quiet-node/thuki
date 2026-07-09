/**
 * Ambient ask-bar warning shown when the built-in engine's pre-load memory
 * gate skipped auto-priming the active model because it likely will not fit
 * in available memory (issue #296). Without this, the skip was silent (only
 * an `eprintln!` on the backend) and the user's first surprise was the
 * per-message `InsufficientMemory` chat error card. This strip surfaces the
 * same figures proactively, before any message is sent.
 *
 * Two-stage confirm: stage 1 states the fit warning and offers "Switch model"
 * (primary) or "Load anyway"; clicking "Load anyway" does NOT load yet, it
 * advances to stage 2, which spells out the consequence of forcing an
 * oversized model into memory and swaps the button roles so "Load anyway"
 * becomes the deliberate, second-click confirmation. Only that stage-2 click
 * force-loads the model past the gate. There is no dismiss affordance: the
 * strip resolves by switching model or loading anyway, and clears on its own
 * when a load or download supersedes it.
 *
 * Mirrors `DownloadStatusStrip`'s borderless Shell pattern (thin accent
 * edge, dot, inline row, no box of its own) and reuses the warning amber
 * already established for `InsufficientMemory` (`ErrorCard.tsx`'s
 * `barColors.InsufficientMemory`) rather than inventing a new color.
 */
import { useState } from 'react';
import { INSUFFICIENT_MEMORY_CONSEQUENCE } from './ErrorCard';

/** Warning amber, matching `ErrorCard.tsx`'s `barColors.InsufficientMemory`. */
const AMBER = '#f59e0b';
/** Muted secondary-action color, matching `DownloadStatusStrip`'s `MUTED`. */
const MUTED = 'rgba(255,255,255,0.4)';
/** Primary-action color, matching `DownloadStatusStrip`'s `ACTION`. */
const ACTION = '#ff8d5c';

/** Bytes per gigabyte, matching `ErrorCard.tsx`'s divisor. */
const BYTES_PER_GB = 1024 ** 3;

/** Formats a byte count as a one-decimal GB string, matching `ErrorCard.tsx`. */
function formatGb(bytes: number): string {
  return (bytes / BYTES_PER_GB).toFixed(1);
}

/** Props for {@link AutoPrimeSkippedStrip}. */
export interface AutoPrimeSkippedStripProps {
  /** Display name of the model the memory gate skipped. */
  modelName: string;
  /** Estimated memory the model needs, in bytes. */
  requiredBytes: number;
  /** Memory estimated available at skip time, in bytes. */
  availableBytes: number;
  /** Opens the model picker so the user can pick a different model. */
  onSwitchModel: () => void;
  /**
   * Force-loads the oversized model past the memory gate. Called only on the
   * stage-2 "Load anyway" click, so this fires as a deliberate, confirmed
   * action, never on the first click.
   */
  onLoadAnyway: () => void;
}

/**
 * Shared style for both action buttons in both stages: a plain text button
 * with no background, border, or box. Prominence comes purely from the color
 * ({@link ACTION} vs {@link MUTED}) and position, never from a filled or
 * bordered treatment, so the primary and secondary read as the same control
 * with only color and order distinguishing them across the two stages.
 */
const BUTTON_CLASS = 'shrink-0 font-bold cursor-pointer';
const BUTTON_STYLE_BASE = {
  background: 'transparent',
  border: 'none',
} as const;

/**
 * Renders the two-stage ambient memory warning. Stage 1 shows the model name
 * and need-vs-available GB figures; stage 2 (after the first "Load anyway"
 * click) replaces that message with the consequence copy and swaps the button
 * roles so a second, deliberate "Load anyway" click confirms the force-load.
 */
export function AutoPrimeSkippedStrip({
  modelName,
  requiredBytes,
  availableBytes,
  onSwitchModel,
  onLoadAnyway,
}: AutoPrimeSkippedStripProps) {
  // why: the stage lives inside the strip, not the host, so the confirm is a
  // pure interaction detail. The first "Load anyway" click only flips this;
  // the actual force-load fires on the stage-2 click. Keeping it internal also
  // means the strip resets to stage 1 whenever the host remounts it (a fresh
  // skip event), so a stale confirm never carries over to a new warning.
  const [confirming, setConfirming] = useState(false);

  const message = confirming
    ? INSUFFICIENT_MEMORY_CONSEQUENCE
    : `${modelName} may not fit in memory (~${formatGb(requiredBytes)} GB needed, ~${formatGb(availableBytes)} GB available)`;

  // The "Load anyway" button: advances to the confirm stage on the first
  // click, force-loads on the second. Primary (ACTION color, rendered first)
  // only in stage 2, where forcing is the deliberate choice being confirmed.
  const loadAnyway = (
    <button
      type="button"
      aria-label="Load anyway"
      onClick={confirming ? onLoadAnyway : () => setConfirming(true)}
      className={BUTTON_CLASS}
      style={{ ...BUTTON_STYLE_BASE, color: confirming ? ACTION : MUTED }}
    >
      Load anyway
    </button>
  );

  // The "Switch model" button: primary (ACTION color, rendered first) in
  // stage 1 where picking a smaller model is the safe recommendation; muted in
  // stage 2 where "Load anyway" takes over as the confirmed action.
  const switchModel = (
    <button
      type="button"
      aria-label="Switch model"
      onClick={onSwitchModel}
      className={BUTTON_CLASS}
      style={{ ...BUTTON_STYLE_BASE, color: confirming ? MUTED : ACTION }}
    >
      Switch model
    </button>
  );

  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="auto-prime-skipped-strip"
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
          style={{ width: '100%', background: AMBER }}
        />
      </span>
      <div className="flex items-center gap-2.5 pt-1.5 text-xs">
        <span
          aria-hidden="true"
          className="shrink-0 w-2 h-2 rounded-full"
          style={{ background: AMBER, boxShadow: `0 0 6px ${AMBER}` }}
        />
        <span className="flex-1 leading-snug">{message}</span>
        {/* Role swap by stage: stage 1 leads with "Switch model" (the safe
            recommendation); stage 2 leads with "Load anyway" (the confirmed
            force). Order and color are the only things that change. */}
        {confirming ? (
          <>
            {loadAnyway}
            {switchModel}
          </>
        ) : (
          <>
            {switchModel}
            {loadAnyway}
          </>
        )}
      </div>
    </div>
  );
}
