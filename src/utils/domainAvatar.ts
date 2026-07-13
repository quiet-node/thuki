/**
 * Deterministic letter-avatar coloring for web-search sources.
 *
 * Shared by the citation chips in `ChatBubble` and the source rows in
 * `SearchProgressBlock` so a given domain always renders the same color in
 * both surfaces. Letter avatars only: no favicon network fetches.
 */

/**
 * Extracts a bare hostname from a URL for source rows. Strips a leading
 * `www.` prefix; falls back to the raw input if URL parsing fails.
 */
export function domainOf(url: string): string {
  try {
    const host = new URL(url).hostname;
    return host.startsWith('www.') ? host.slice(4) : host;
  } catch {
    return url;
  }
}

/**
 * Deterministic 0–359 hue from a domain string so each source keeps a stable
 * letter-avatar color across re-renders without network favicon fetches.
 */
export function domainHue(domain: string): number {
  let h = 0;
  for (let i = 0; i < domain.length; i++) {
    h = (h * 31 + domain.charCodeAt(i)) >>> 0;
  }
  return h % 360;
}

/**
 * Hand-picked palette of light, summery, slightly-cool gradient pairs for
 * letter avatars. Each entry is a two-stop linear-gradient suitable as the
 * `background` of a small circular badge. The domain hash selects one pair
 * deterministically so a given source always renders the same color.
 *
 * Picked to keep the palette pleasant and varied without clashing: no neon,
 * no muddy shades, all readable under white/90 letter text.
 */
export const AVATAR_PALETTE: readonly string[] = [
  'linear-gradient(135deg, #ffb8a1, #ff8c77)', // peach coral
  'linear-gradient(135deg, #ffc3d5, #ff9cbd)', // cotton candy pink
  'linear-gradient(135deg, #a8d8ff, #7cb8ff)', // sky blue
  'linear-gradient(135deg, #a8e6cf, #7ecfb0)', // mint
  'linear-gradient(135deg, #c7b8ff, #a896ff)', // lavender
  'linear-gradient(135deg, #ffd3a5, #ffa978)', // sunset
  'linear-gradient(135deg, #9ee6d7, #6fc9b5)', // seafoam
  'linear-gradient(135deg, #fff0a5, #ffd96b)', // lemon sorbet
  'linear-gradient(135deg, #b8e0ff, #85b9ff)', // periwinkle
  'linear-gradient(135deg, #ffb6e1, #ff8cc8)', // bubblegum
  'linear-gradient(135deg, #c4eaa8, #9bd076)', // kiwi
  'linear-gradient(135deg, #ffc8a8, #ff9e78)', // papaya
] as const;

/**
 * Returns a CSS gradient background for a letter avatar keyed by domain.
 * Picks one of the hand-curated palette entries by the domain hash for
 * consistent but varied coloring.
 */
export function avatarColor(domain: string): string {
  return AVATAR_PALETTE[domainHue(domain) % AVATAR_PALETTE.length];
}
