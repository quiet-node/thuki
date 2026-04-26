/** Props for the {@link CapabilityMismatchStrip} component. */
export interface CapabilityMismatchStripProps {
  /**
   * Human-readable reason rendered as the strip body. The strip renders
   * only when this is a non-empty string.
   */
  message: string;
}

/**
 * Inline informational strip that surfaces a capability mismatch between
 * the user's compose state (image attached, `/screen` queued) and the
 * active model. Passive: the strip carries no action button. Recovery
 * happens through the existing model picker chip in WindowControls so
 * the picker remains the single source of truth for switching models.
 *
 * The host is responsible for rendering the strip only when there is a
 * real conflict (use `getCapabilityConflict` to compute the message).
 * The strip itself does not animate; the host can wrap it in
 * AnimatePresence if a fade-in / fade-out is desired.
 */
export function CapabilityMismatchStrip({
  message,
}: CapabilityMismatchStripProps) {
  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="capability-mismatch-strip"
      className="mx-4 mt-2 mb-0 flex items-center gap-2.5 px-3 py-2 rounded-lg border text-xs"
      style={{
        background: 'rgba(230, 156, 5, 0.10)',
        borderColor: 'rgba(230, 156, 5, 0.30)',
        color: 'var(--color-text-primary, #f0f0f2)',
      }}
    >
      <span
        aria-hidden="true"
        className="shrink-0 w-2 h-2 rounded-full"
        style={{
          background: 'rgb(230, 156, 5)',
          boxShadow: '0 0 6px rgba(230, 156, 5, 0.6)',
        }}
      />
      <span className="flex-1 leading-snug">{message}</span>
    </div>
  );
}
