import { invoke } from '@tauri-apps/api/core';

/**
 * A capability strip message is either a plain string (passive
 * informational strip) or a three-part shape with an inline link
 * (`before` + `link` + `after`). The inline link form keeps the rest of
 * the strip text non-interactive so users only click when they want the
 * documented recovery path.
 */
export type CapabilityMismatchMessage =
  | string
  | {
      /** Copy rendered before the inline link. */
      before: string;
      /** The inline anchor: link text + URL opened on click. */
      link: { text: string; url: string };
      /** Copy rendered after the inline link. */
      after: string;
    };

/** Props for the {@link CapabilityMismatchStrip} component. */
export interface CapabilityMismatchStripProps {
  /**
   * Human-readable reason rendered as the strip body. The strip renders
   * only when this is non-empty; pass either a plain string or a
   * `{ before, link, after }` shape to embed an inline link.
   */
  message: CapabilityMismatchMessage;
}

/**
 * Inline informational strip that surfaces a capability mismatch between
 * the user's compose state (image attached, `/screen` queued) and the
 * active model, or between the conversation history and the active model.
 *
 * Two variants:
 * - **Plain text**: passive; no action button, no link. Recovery happens
 *   through the existing model picker chip in WindowControls.
 * - **Inline link**: a small clickable anchor sits inside the message
 *   (e.g. "Use an [OCR-supported command ↗], or switch..."). Clicking
 *   the anchor opens the documented recovery URL in the user's default
 *   browser via the Tauri `open_url` command. The rest of the strip
 *   remains non-interactive so it does not feel like a giant button.
 *
 * The host is responsible for rendering the strip only when there is a
 * real conflict (use `getCapabilityConflict` to compute the message).
 * The strip itself does not animate; the host can wrap it in
 * AnimatePresence if a fade-in / fade-out is desired.
 */
export function CapabilityMismatchStrip({
  message,
}: CapabilityMismatchStripProps) {
  const baseClass =
    'mx-4 mt-2 mb-0 flex items-center gap-2.5 px-3 py-2 rounded-lg border text-xs';
  const baseStyle = {
    background: 'rgba(230, 156, 5, 0.10)',
    borderColor: 'rgba(230, 156, 5, 0.30)',
    color: 'var(--color-text-primary, #f0f0f2)',
  } as const;

  const dot = (
    <span
      aria-hidden="true"
      className="shrink-0 w-2 h-2 rounded-full"
      style={{
        background: 'rgb(230, 156, 5)',
        boxShadow: '0 0 6px rgba(230, 156, 5, 0.6)',
      }}
    />
  );

  const body =
    typeof message === 'string' ? (
      message
    ) : (
      <>
        {message.before}
        <button
          type="button"
          data-testid="capability-mismatch-strip-link"
          aria-label={`Open documentation: ${message.link.url}`}
          onClick={() => {
            void invoke('open_url', { url: message.link.url });
          }}
          className="cursor-pointer underline decoration-dotted underline-offset-2 decoration-[rgba(230,156,5,0.55)] text-[color:rgb(230,156,5)] hover:text-[color:rgb(245,176,30)] transition-colors"
        >
          {message.link.text} ↗
        </button>
        {message.after}
      </>
    );

  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="capability-mismatch-strip"
      className={baseClass}
      style={baseStyle}
    >
      {dot}
      <span className="flex-1 leading-snug">{body}</span>
    </div>
  );
}
