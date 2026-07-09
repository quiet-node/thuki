/**
 * Ambient ask-bar warning shown when the built-in engine's pre-load memory
 * gate skipped auto-priming the active model because it likely will not fit
 * in available memory (issue #296). Without this, the skip was silent (only
 * an `eprintln!` on the backend) and the user's first surprise was the
 * per-message `InsufficientMemory` chat error card. This strip surfaces the
 * same figures proactively, before any message is sent.
 *
 * Mirrors `DownloadStatusStrip`'s borderless Shell pattern (thin accent
 * edge, dot, inline row, no box of its own) and reuses the warning amber
 * already established for `InsufficientMemory` (`ErrorCard.tsx`'s
 * `barColors.InsufficientMemory`) rather than inventing a new color.
 */

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
  /** Dismisses the strip without switching models. */
  onDismiss: () => void;
}

/**
 * Renders the one-line ambient warning: model name, need-vs-available GB
 * figures, and "Switch model" / "Dismiss" actions.
 */
export function AutoPrimeSkippedStrip({
  modelName,
  requiredBytes,
  availableBytes,
  onSwitchModel,
  onDismiss,
}: AutoPrimeSkippedStripProps) {
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
        <span className="flex-1 leading-snug">
          {`${modelName} may not fit in memory (~${formatGb(requiredBytes)} GB needed, ~${formatGb(availableBytes)} GB available)`}
        </span>
        <button
          type="button"
          aria-label="Switch model"
          onClick={onSwitchModel}
          className="shrink-0 font-bold cursor-pointer"
          style={{ color: ACTION, background: 'transparent', border: 'none' }}
        >
          Switch model
        </button>
        <button
          type="button"
          aria-label="Dismiss memory warning"
          onClick={onDismiss}
          className="shrink-0 font-bold cursor-pointer"
          style={{ color: MUTED, background: 'transparent', border: 'none' }}
        >
          Dismiss
        </button>
      </div>
    </div>
  );
}
