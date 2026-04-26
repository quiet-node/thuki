import { useEffect } from 'react';

/** Props for the {@link Toast} component. */
export interface ToastProps {
  /** Body text shown in the toast. The toast renders only when truthy. */
  message: string | null;
  /** Called when the auto-dismiss timer fires or the user closes the toast. */
  onDismiss: () => void;
  /** Auto-dismiss delay in ms. Defaults to 3000. */
  durationMs?: number;
}

const DEFAULT_DURATION_MS = 3000;

/**
 * Bottom-anchored transient toast used by the submit-time capability
 * gate. Renders nothing when `message` is null. Schedules a single
 * auto-dismiss timer per non-null `message` and clears it on unmount or
 * before the next message replaces it.
 *
 * Positioning is `absolute` against the nearest positioned ancestor; the
 * caller wraps it in a `relative` container to anchor.
 */
export function Toast({
  message,
  onDismiss,
  durationMs = DEFAULT_DURATION_MS,
}: ToastProps) {
  useEffect(() => {
    if (!message) return;
    const timer = setTimeout(onDismiss, durationMs);
    return () => clearTimeout(timer);
  }, [message, onDismiss, durationMs]);

  if (!message) return null;

  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="toast"
      className="absolute left-1/2 -translate-x-1/2 bottom-16 z-10 flex items-center gap-2 px-3.5 py-2.5 rounded-lg text-xs whitespace-nowrap"
      style={{
        background: 'rgba(20, 14, 10, 0.96)',
        border: '1px solid rgba(230, 156, 5, 0.30)',
        color: 'var(--color-text-primary, #f0f0f2)',
        boxShadow: '0 8px 24px -6px rgba(0, 0, 0, 0.6)',
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
      <span>{message}</span>
    </div>
  );
}
