import { Fragment } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type {
  CapabilityConflictMessage,
  StripLink,
} from '../utils/capabilityConflicts';

/**
 * The strip's message type. Re-exported under the component's local name so
 * call sites that import it from here keep working; the canonical definition
 * lives next to the logic that produces it in `capabilityConflicts`.
 */
export type CapabilityMismatchMessage = CapabilityConflictMessage;

/** Shared Tailwind class for an inline strip link (dotted amber underline). */
const LINK_CLASS =
  'cursor-pointer underline decoration-dotted underline-offset-2 decoration-[rgba(230,156,5,0.55)] text-[color:rgb(230,156,5)] hover:text-[color:rgb(245,176,30)] transition-colors';

/**
 * Renders one {@link StripLink} as an inline button. A `url` link opens the
 * page in the browser and shows the ↗ external-link glyph; a `nav` link runs
 * the in-app action (open Settings → Providers) and omits ↗ since it stays in
 * Thuki.
 */
function StripLinkButton({ link }: { link: StripLink }) {
  return (
    <button
      type="button"
      data-testid="capability-mismatch-strip-link"
      aria-label={
        'url' in link
          ? `Open ${link.url}`
          : 'Switch to the Built-in provider in Settings'
      }
      onClick={() => {
        if ('url' in link) {
          void invoke('open_url', { url: link.url });
        } else {
          void invoke('open_settings_to_providers');
        }
      }}
      className={LINK_CLASS}
    >
      {link.text}
      {'url' in link ? ' ↗' : ''}
    </button>
  );
}

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
    ) : 'segments' in message ? (
      message.segments.map((segment) =>
        typeof segment === 'string' ? (
          <Fragment key={segment}>{segment}</Fragment>
        ) : (
          <StripLinkButton key={segment.text} link={segment} />
        ),
      )
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
          className={LINK_CLASS}
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
