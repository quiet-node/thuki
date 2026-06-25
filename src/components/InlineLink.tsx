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
 * Styling is inline (not Tailwind classes) on purpose: the Settings window is a
 * separate webview without the main app's Tailwind layer, so a class-based link
 * would render unstyled there. Inline styles keep one consistent link look in
 * every window. Pass `style` only to tweak per-surface details (e.g. weight);
 * the colour and underline stay constant for consistency.
 */
const LINK_STYLE: React.CSSProperties = {
  padding: 0,
  border: 'none',
  background: 'transparent',
  fontFamily: 'inherit',
  fontSize: 'inherit',
  fontWeight: 'inherit',
  fontStyle: 'inherit',
  color: '#ffb892',
  textDecoration: 'underline',
  textDecorationColor: 'rgba(255, 141, 92, 0.5)',
  textUnderlineOffset: 2,
  cursor: 'pointer',
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
  /** Per-surface style overrides merged last (colour/underline stay constant). */
  style?: React.CSSProperties;
}

export function InlineLink({
  url,
  children,
  ariaLabel,
  style,
}: InlineLinkProps) {
  return (
    <button
      type="button"
      onClick={() => void invoke('open_url', { url })}
      title={url}
      aria-label={ariaLabel}
      style={style ? { ...LINK_STYLE, ...style } : LINK_STYLE}
    >
      {children}
    </button>
  );
}
