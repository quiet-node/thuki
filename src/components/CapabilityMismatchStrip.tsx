import { invoke } from '@tauri-apps/api/core';

/**
 * A capability strip message is either a plain string or a `{ text, url }`
 * pair where the whole strip becomes clickable and opens `url` in the user's
 * default browser via the Tauri `open_url` command. Use the linked form for
 * conflicts that have a public-doc explanation (e.g. OCR-supported commands
 * as the recovery path when a non-vision model is active).
 */
export type CapabilityMismatchMessage =
  | string
  | {
      /** Body copy rendered inside the strip. */
      text: string;
      /** Public URL opened on click. */
      url: string;
    };

/** Props for the {@link CapabilityMismatchStrip} component. */
export interface CapabilityMismatchStripProps {
  /**
   * Human-readable reason rendered as the strip body. The strip renders
   * only when this is non-empty; pass either a plain string or a
   * `{ text, url }` pair to make the strip clickable.
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
 * - **Linked**: the whole strip is a button that opens `url` in the
 *   user's default browser via the Tauri `open_url` command. Used when
 *   there is a documented alternative recovery path (e.g. OCR-supported
 *   commands).
 *
 * The host is responsible for rendering the strip only when there is a
 * real conflict (use `getCapabilityConflict` to compute the message).
 * The strip itself does not animate; the host can wrap it in
 * AnimatePresence if a fade-in / fade-out is desired.
 */
export function CapabilityMismatchStrip({
  message,
}: CapabilityMismatchStripProps) {
  const isLinked = typeof message !== 'string';
  const text = isLinked ? message.text : message;
  const url = isLinked ? message.url : null;

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

  if (url !== null) {
    return (
      <button
        type="button"
        role="status"
        aria-live="polite"
        data-testid="capability-mismatch-strip"
        aria-label={`Open documentation: ${url}`}
        onClick={() => {
          void invoke('open_url', { url });
        }}
        className={`${baseClass} w-[calc(100%-2rem)] cursor-pointer text-left transition-colors hover:bg-[rgba(230,156,5,0.16)]`}
        style={baseStyle}
      >
        {dot}
        <span className="flex-1 leading-snug underline decoration-dotted underline-offset-2 decoration-[rgba(230,156,5,0.45)]">
          {text}
        </span>
      </button>
    );
  }

  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="capability-mismatch-strip"
      className={baseClass}
      style={baseStyle}
    >
      {dot}
      <span className="flex-1 leading-snug">{text}</span>
    </div>
  );
}
