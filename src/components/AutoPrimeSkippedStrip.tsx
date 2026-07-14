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
 * oversized model into memory and swaps the button roles so "Acknowledge"
 * becomes the deliberate, second-click confirmation. Only that stage-2 click
 * force-loads the model past the gate. There is no dismiss affordance: the
 * strip resolves by switching model or loading anyway, and clears on its own
 * when a load or download supersedes it.
 *
 * Visual: matches SearchTrustNotice footer actions (outlined primary + ghost
 * secondary, stacked under body copy). No top accent bar.
 */
import { useState } from 'react';
import { INSUFFICIENT_MEMORY_CONSEQUENCE } from './ErrorCard';

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
 * and need-vs-available GB figures; stage 2 (after the first "Load anyway"
 * click) replaces that message with the consequence copy and swaps the button
 * roles so a second, deliberate "Acknowledge" click confirms the force-load.
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

  const message = confirming
    ? INSUFFICIENT_MEMORY_CONSEQUENCE
    : `${modelName} may not fit in memory (~${formatGb(requiredBytes)} GB needed, ~${formatGb(availableBytes)} GB available, over the ${Math.round(ceilingFraction * 100)}% safe limit)`;

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
      <p className="text-xs text-white/45 leading-relaxed">{message}</p>
      <div className="mt-2.5 flex flex-col items-start gap-1">
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
