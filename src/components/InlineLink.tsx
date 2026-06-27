import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/**
 * Shared inline external link.
 *
 * Renders link text that sits inside a sentence: a warm, underlined accent run
 * that opens its destination in the user's browser via the `open_url` command
 * (never an anchor navigating the webview). The underline makes it obvious the
 * run is clickable, and the native `title` surfaces the destination URL on
 * hover so the user knows where it leads before clicking.
 *
 * The `subtle` variant drops the accent colour and underline so the run reads
 * as plain heading text (used for model titles in the Settings model panes,
 * where a row of coloured, underlined names is visually noisy). It restores the
 * accent colour and underline on hover so the run still reveals itself as a
 * link before the click, and the hover `title` surfaces the destination URL.
 *
 * Styling is inline (not Tailwind classes) on purpose: the Settings window is a
 * separate webview without the main app's Tailwind layer, so a class-based link
 * would render unstyled there. Inline styles keep one consistent link look in
 * every window. Pass `style` only to tweak per-surface details (e.g. weight);
 * the variant decides colour and underline.
 */
const LINK_BASE_STYLE: React.CSSProperties = {
  padding: 0,
  border: 'none',
  background: 'transparent',
  fontFamily: 'inherit',
  fontSize: 'inherit',
  fontWeight: 'inherit',
  fontStyle: 'inherit',
  cursor: 'pointer',
};

const ACCENT_STYLE: React.CSSProperties = {
  color: '#ffb892',
  textDecoration: 'underline',
  textDecorationColor: 'rgba(255, 141, 92, 0.5)',
  textUnderlineOffset: 2,
};

// Plain heading text: `--t1` is the Settings webview's primary text token, so
// the name matches the surrounding card copy instead of reading as a link.
const SUBTLE_STYLE: React.CSSProperties = {
  color: 'var(--t1)',
  textDecoration: 'none',
};

interface InlineLinkProps {
  /** Destination opened in the user's browser via the `open_url` command. */
  url: string;
  /** The inline link content (text, and any trailing arrow glyph). */
  children: React.ReactNode;
  /**
   * Accessible name. Optional: when omitted the button's text content names it.
   * Pass a fuller label where the visible text alone is ambiguous out of context
   * (e.g. a bare version string or a short "Origin" value).
   */
  ariaLabel?: string;
  /** Render as plain heading text (no accent colour or underline). */
  subtle?: boolean;
  /** Per-surface style overrides merged last (e.g. weight, alignment). */
  style?: React.CSSProperties;
}

export function InlineLink({
  url,
  children,
  ariaLabel,
  subtle,
  style,
}: InlineLinkProps) {
  const [hovered, setHovered] = useState(false);
  // The subtle variant only sheds its link look at rest; on hover it returns to
  // the accent treatment so the run still reads as a link before the click.
  const variantStyle = subtle && !hovered ? SUBTLE_STYLE : ACCENT_STYLE;
  return (
    <button
      type="button"
      onClick={() => void invoke('open_url', { url })}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      title={url}
      aria-label={ariaLabel}
      style={{ ...LINK_BASE_STYLE, ...variantStyle, ...style }}
    >
      {children}
    </button>
  );
}
