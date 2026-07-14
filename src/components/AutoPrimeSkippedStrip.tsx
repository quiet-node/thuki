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
 * advances to stage 2, which keeps the fit warning and adds the consequence
 * as muted text underneath, with "Acknowledge" as the deliberate second-click
 * force-load. There is no dismiss affordance: the strip resolves by switching
 * model or loading anyway, and clears on its own when a load or download
 * supersedes it.
 *
 * Visual: amber status dot, primary body copy, muted consequence on confirm,
 * SearchTrustNotice-style action row (outlined primary + ghost secondary).
 */
import { useState } from 'react';
import { INSUFFICIENT_MEMORY_CONSEQUENCE } from './ErrorCard';

/** Warning amber, matching `ErrorCard.tsx`'s `barColors.InsufficientMemory`. */
const AMBER = '#f59e0b';

/** Bytes per gigabyte, matching `ErrorCard.tsx`'s divisor. */
const BYTES_PER_GB = 1024 ** 3;

/**
 * Formats a byte count as a one-decimal GB string, matching `ErrorCard.tsx`.
 */
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
  /**
   * The memory gate's ceiling fraction (`MODEL_FIT_CEILING_FRACTION` on the
   * backend), e.g. `0.8` for 80%. Rendered in the stage-1 message so the "may
   * not fit" verdict states the actual headroom rule instead of leaving the
   * 80%-of-available gate invisible in the copy.
   */
  ceilingFraction: number;
  /** Opens the model picker so the user can pick a different model. */
  onSwitchModel: () => void;
  /**
   * Force-loads the oversized model past the memory gate. Called only on the
   * stage-2 "Acknowledge" click, so this fires as a deliberate, confirmed
   * action, never on the first click.
   */
  onLoadAnyway: () => void;
}

/** Outlined primary CTA, matching SearchTrustNotice "Got it". */
const PRIMARY_BTN_CLASS =
  'cursor-pointer rounded-lg border border-primary/45 bg-transparent px-3 py-1.5 text-[11.5px] font-semibold text-primary transition-colors hover:bg-primary/10 w-fit';

/** Ghost secondary CTA, matching SearchTrustNotice "Turn off in Settings". */
const GHOST_BTN_CLASS =
  'cursor-pointer border-0 bg-transparent px-1 py-1.5 text-[11.5px] font-medium text-white/50 transition-colors hover:text-white/75 w-fit';

/**
 * Renders the two-stage ambient memory warning. Stage 1 shows the model name
 * and need-vs-available GB figures; stage 2 keeps that line and adds the
 * consequence as muted text, with Acknowledge as the deliberate force-load.
 */
export function AutoPrimeSkippedStrip({
  modelName,
  requiredBytes,
  availableBytes,
  ceilingFraction,
  onSwitchModel,
  onLoadAnyway,
}: AutoPrimeSkippedStripProps) {
  // why: the stage lives inside the strip, not the host, so the confirm is a
  // pure presentation detail. The first "Load anyway" click only flips this;
  // the actual force-load fires on the stage-2 click. Keeping it internal also
  // means the strip resets to stage 1 whenever the host remounts it (a fresh
  // skip event), so a stale confirm never carries over to a new warning.
  const [confirming, setConfirming] = useState(false);

  // Keep fit line always; ceilingFraction is this branch's 80% headroom copy.
  const fitMessage = `${modelName} may not fit in memory (~${formatGb(requiredBytes)} GB needed, ~${formatGb(availableBytes)} GB available, over the ${Math.round(ceilingFraction * 100)}% safe limit)`;

  // Stage 1: Switch model = primary (safe path). Load anyway = ghost.
  // Stage 2: Acknowledge = primary (deliberate force). Switch model = ghost.
  const primaryLabel = confirming ? 'Acknowledge' : 'Switch model';
  const secondaryLabel = confirming ? 'Switch model' : 'Load anyway';

  /**
   * Handles the primary button: Switch model in stage 1, force-load in stage 2.
   */
  function onPrimaryClick(): void {
    if (confirming) {
      onLoadAnyway();
    } else {
      onSwitchModel();
    }
  }

  /**
   * Handles the secondary button: advance to confirm in stage 1, or Switch
   * model in stage 2.
   */
  function onSecondaryClick(): void {
    if (confirming) {
      onSwitchModel();
    } else {
      setConfirming(true);
    }
  }

  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="auto-prime-skipped-strip"
      className="mx-4 mt-2 mb-0 px-0.5"
    >
      <div className="flex items-start gap-2.5">
        <span
          aria-hidden="true"
          data-testid="auto-prime-skipped-dot"
          className="mt-1 shrink-0 w-2 h-2 rounded-full"
          style={{ background: AMBER, boxShadow: `0 0 6px ${AMBER}` }}
        />
        <div className="min-w-0 flex-1">
          <p className="text-xs text-text-primary leading-relaxed">
            {fitMessage}
          </p>
          {confirming ? (
            <p
              data-testid="auto-prime-skipped-consequence"
              className="mt-1 text-xs text-white/45 leading-relaxed"
            >
              {INSUFFICIENT_MEMORY_CONSEQUENCE}
            </p>
          ) : null}
        </div>
      </div>
      <div className="mt-2.5 flex flex-wrap items-center gap-2">
        <button
          type="button"
          aria-label={primaryLabel}
          onClick={onPrimaryClick}
          className={PRIMARY_BTN_CLASS}
        >
          {primaryLabel}
        </button>
        <button
          type="button"
          aria-label={secondaryLabel}
          onClick={onSecondaryClick}
          className={GHOST_BTN_CLASS}
        >
          {secondaryLabel}
        </button>
      </div>
    </div>
  );
}
