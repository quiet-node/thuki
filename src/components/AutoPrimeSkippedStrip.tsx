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
 * Visual: amber status dot, primary body copy, muted consequence on confirm
 * (height+opacity expand matching ask-bar strips), SearchTrustNotice-style
 * action row (outlined primary + ghost secondary).
 */
import { useState } from 'react';
import { AnimatePresence, motion, useReducedMotion } from 'framer-motion';
import {
  INSUFFICIENT_MEMORY_CONSEQUENCE,
  MEMORY_FREEZE_NOTE,
  MemoryCriticalChip,
} from './ErrorCard';

/**
 * Shared ease for height expands elsewhere in the ask bar (command suggestion).
 * Soft overshoot-free curve for premium feel without bounce.
 */
const EXPAND_EASE: [number, number, number, number] = [0.16, 1, 0.3, 1];

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
  /**
   * Whether a per-model "remember" could suppress this warning (backend
   * `!is_freeze_band`, from the skip event's `can_remember`). When true, stage 2
   * offers the "Always allow this model" split action; when false (freeze band)
   * only the single "Acknowledge" force is shown, because the backend never
   * honors a remember for such a load. The frontend keeps no freeze-floor
   * number of its own.
   */
  canRemember: boolean;
  /** Opens the model picker so the user can pick a different model. */
  onSwitchModel: () => void;
  /**
   * Force-loads the oversized model past the memory gate. Called only on the
   * stage-2 "Acknowledge" click, so this fires as a deliberate, confirmed
   * action, never on the first click. `remember` carries the stage-2 opt-in
   * checkbox value so the host can persist the per-model override alongside the
   * force-load; always `false` when the checkbox was hidden (freeze band).
   */
  onLoadAnyway: (remember: boolean) => void;
}

/** Outlined primary CTA, matching SearchTrustNotice "Got it". Used for "Switch
 *  model" (stage 1), "Acknowledge" (freeze force), and "Load once" (mild). */
const PRIMARY_BTN_CLASS =
  'cursor-pointer rounded-lg border border-primary/45 bg-transparent px-3 py-1.5 text-[11.5px] font-semibold text-primary transition-colors hover:bg-primary/10 w-fit';

/** Emphasized "Always allow this model" CTA: the outlined primary plus a soft
 *  primary fill, marking it as the remember choice among the split actions. */
const ALWAYS_ALLOW_BTN_CLASS =
  'cursor-pointer rounded-lg border border-primary/45 bg-primary/[0.14] px-3 py-1.5 text-[11.5px] font-semibold text-primary transition-colors hover:bg-primary/20 w-fit';

/** Ghost secondary CTA, matching SearchTrustNotice "Turn off in Settings". */
const GHOST_BTN_CLASS =
  'cursor-pointer border-0 bg-transparent px-1 py-1.5 text-[11.5px] font-medium text-white/50 transition-colors hover:text-white/75 w-fit';

/**
 * Renders the ambient memory warning.
 *
 * BOTH bands gate the force behind two clicks, and the dangerous one is never
 * the cheaper click. The first "Load anyway" only advances the stage; the load
 * fires on the second.
 *
 * The FREEZE band (`canRemember` false) leads with the severity chip, the
 * free-vs-needed title, and the blunt note, then confirms with a single
 * "Acknowledge". The second click is not there to inform (the chip already
 * did that): it is there so a stray click on a strip that appears unprompted
 * cannot wire memory the machine does not have and lock up the Mac. No
 * remember is offered, because the backend refuses to honor one at this ratio.
 *
 * The MILD band shows the fit figures, then stage 2 adds the consequence copy
 * and splits the force into "Load once" / "Always allow this model", with
 * "Switch model" as the ghost escape.
 */
export function AutoPrimeSkippedStrip({
  modelName,
  requiredBytes,
  availableBytes,
  ceilingFraction,
  canRemember,
  onSwitchModel,
  onLoadAnyway,
}: AutoPrimeSkippedStripProps) {
  // why: the stage lives inside the strip, not the host, so the confirm is a
  // pure presentation detail. The first "Load anyway" click only flips this;
  // the actual force-load fires on the stage-2 click. Keeping it internal also
  // means the strip resets to stage 1 whenever the host remounts it (a fresh
  // skip event), so a stale confirm never carries over to a new warning. Both
  // bands read it, so neither can be force-loaded on a single click.
  const [confirming, setConfirming] = useState(false);
  const reduceMotion = useReducedMotion();

  // Keep fit line always; ceilingFraction is this branch's 80% headroom copy.
  const fitMessage = `${modelName} may not fit in memory (~${formatGb(requiredBytes)} GB needed, ~${formatGb(availableBytes)} GB available, over the ${Math.round(ceilingFraction * 100)}% safe limit)`;

  if (!canRemember) {
    return (
      <div
        role="status"
        aria-live="polite"
        data-testid="auto-prime-skipped-strip"
        className="px-3.5 py-2.5"
      >
        {/* No status dot in this band: the red severity tag IS the indicator,
            so rendering the amber dot as well would read as two signals. */}
        <div className="flex items-start gap-2.5">
          <div className="min-w-0 flex-1">
            <MemoryCriticalChip />
            <p className="mt-1.5 text-xs text-text-primary leading-relaxed">
              {`Only ~${formatGb(availableBytes)} GB free. ${modelName} needs ~${formatGb(requiredBytes)} GB.`}
            </p>
            <p
              data-testid="memory-freeze-note"
              className="mt-1 text-xs text-white/45 leading-relaxed"
            >
              {MEMORY_FREEZE_NOTE}
            </p>
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              {!confirming ? (
                // Stage 1: "Load anyway" only advances, so the riskiest load in
                // the app is never one click away.
                <button
                  type="button"
                  aria-label="Load anyway"
                  onClick={() => setConfirming(true)}
                  className={PRIMARY_BTN_CLASS}
                >
                  Load anyway
                </button>
              ) : (
                // Stage 2: the deliberate force. Same "Load once" wording as
                // the mild band, and for the same reason (a one-time force
                // that persists nothing), but without the "Always allow this
                // model" half, which the backend refuses to honor here.
                <button
                  type="button"
                  aria-label="Load once"
                  onClick={() => onLoadAnyway(false)}
                  className={PRIMARY_BTN_CLASS}
                >
                  Load once
                </button>
              )}
              <button
                type="button"
                aria-label="Switch model"
                onClick={onSwitchModel}
                className={GHOST_BTN_CLASS}
              >
                Switch model
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="auto-prime-skipped-strip"
      className="px-3.5 py-2.5"
    >
      <div className="flex items-start gap-2.5">
        <span
          aria-hidden="true"
          data-testid="auto-prime-skipped-dot"
          className="mt-1 shrink-0 w-2 h-2 rounded-full"
          style={{ background: AMBER, boxShadow: `0 0 6px ${AMBER}` }}
        />
        {/* Copy + actions share this column so CTAs line up with the text,
            not under the status dot. */}
        <div className="min-w-0 flex-1">
          <p className="text-xs text-text-primary leading-relaxed">
            {fitMessage}
          </p>
          <AnimatePresence initial={false}>
            {confirming ? (
              <motion.div
                key="auto-prime-consequence"
                initial={
                  reduceMotion ? false : { height: 0, opacity: 0, y: -4 }
                }
                animate={{ height: 'auto', opacity: 1, y: 0 }}
                exit={
                  reduceMotion ? undefined : { height: 0, opacity: 0, y: -2 }
                }
                transition={{
                  height: { duration: 0.24, ease: EXPAND_EASE },
                  opacity: { duration: 0.2, ease: 'easeOut' },
                  y: { duration: 0.22, ease: EXPAND_EASE },
                }}
                style={{ overflow: 'hidden' }}
              >
                <p
                  data-testid="auto-prime-skipped-consequence"
                  className="mt-1 text-xs text-white/45 leading-relaxed"
                >
                  {INSUFFICIENT_MEMORY_CONSEQUENCE}
                </p>
              </motion.div>
            ) : null}
          </AnimatePresence>
          <div className="mt-2.5 flex flex-wrap items-center gap-2">
            {!confirming ? (
              // Stage 1: Switch model (safe, primary) or Load anyway, which only
              // advances to the consequence-shown stage 2, never loads yet.
              <>
                <button
                  type="button"
                  aria-label="Switch model"
                  onClick={onSwitchModel}
                  className={PRIMARY_BTN_CLASS}
                >
                  Switch model
                </button>
                <button
                  type="button"
                  aria-label="Load anyway"
                  onClick={() => setConfirming(true)}
                  className={GHOST_BTN_CLASS}
                >
                  Load anyway
                </button>
              </>
            ) : (
              // Stage 2, mild band (the only band that reaches here): split the
              // force into "Load once" and the emphasized "Always allow this
              // model" (persists the per-model override), with Switch model
              // demoted to the ghost escape. Only offered here, after the
              // consequence copy has been shown.
              <>
                <button
                  type="button"
                  aria-label="Load once"
                  onClick={() => onLoadAnyway(false)}
                  className={PRIMARY_BTN_CLASS}
                >
                  Load once
                </button>
                <button
                  type="button"
                  aria-label="Always allow this model"
                  onClick={() => onLoadAnyway(true)}
                  className={ALWAYS_ALLOW_BTN_CLASS}
                >
                  Always allow this model
                </button>
                <button
                  type="button"
                  aria-label="Switch model"
                  onClick={onSwitchModel}
                  className={GHOST_BTN_CLASS}
                >
                  Switch model
                </button>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
