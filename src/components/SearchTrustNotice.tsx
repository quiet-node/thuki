/**
 * First-use, non-blocking card for Auto search (v0.16.0). Mounted on the ask
 * bar above the logo/input row when Auto search is on and the notice is not
 * acknowledged. Stays until Got it (or auto_search is off). Never gates compose.
 *
 * Visual: elevated warm panel (surface-elevated + warm border, soft shadow).
 * No version kicker; body still mentions v0.16.0 in copy.
 */

import { invoke } from '@tauri-apps/api/core';

/**
 * Public blog URL for the in-body "See how Auto search works" link.
 * Blog index placeholder until the dedicated post slug lands (issue #320).
 */
export const SEARCH_DISCLOSURE_URL = 'https://thuki.app/blog';

/** Notice title: v0.16.0 Auto search intro. */
export const SEARCH_TRUST_NOTICE_TITLE =
  'Thuki can now search the web, automatically!';

/**
 * Body copy before the disclosure link. One paragraph with the local-off line.
 */
export const SEARCH_TRUST_NOTICE_BODY =
  'Since v0.16.0, when a question needs fresh facts, Thuki smartly searches the web for them. Turn Auto search off to stay fully local and use /search only when you want a look-up.';

/** Label for the in-body disclosure control (↗ marks external open). */
export const SEARCH_TRUST_NOTICE_LEARN_LABEL = 'See how Auto search works ↗';

export interface SearchTrustNoticeProps {
  /** Persist acknowledgement and hide the card. */
  onAcknowledge: () => void;
  /** Open Settings → Behavior with Auto search highlighted. */
  onOpenSettings: () => void;
}

/**
 * Opens the Auto search disclosure URL via Tauri open_url (http/https only).
 */
function openDisclosure(): void {
  void invoke('open_url', { url: SEARCH_DISCLOSURE_URL });
}

/**
 * Compact elevated panel matching design variant C without a version kicker.
 * Never blocks compose; search proceeds underneath.
 */
export function SearchTrustNotice({
  onAcknowledge,
  onOpenSettings,
}: SearchTrustNoticeProps) {
  return (
    <div
      data-testid="search-trust-notice"
      role="region"
      aria-label={SEARCH_TRUST_NOTICE_TITLE}
      className="mb-2 rounded-xl border border-surface-border bg-surface-elevated px-3.5 py-3 shadow-[0_2px_8px_-2px_rgba(0,0,0,0.4)]"
    >
      <p className="text-sm font-medium text-white/90 leading-snug">
        {SEARCH_TRUST_NOTICE_TITLE}
      </p>
      <p className="mt-1.5 text-xs text-white/45 leading-relaxed">
        {SEARCH_TRUST_NOTICE_BODY}{' '}
        <button
          type="button"
          data-testid="search-trust-notice-how"
          onClick={openDisclosure}
          className="inline cursor-pointer border-0 bg-transparent p-0 m-0 text-xs text-primary/80 hover:text-primary underline-offset-2 hover:underline"
        >
          {SEARCH_TRUST_NOTICE_LEARN_LABEL}
        </button>
      </p>
      <div className="mt-2.5 flex flex-wrap items-center gap-2">
        <button
          type="button"
          data-testid="search-trust-notice-got-it"
          onClick={onAcknowledge}
          className="cursor-pointer rounded-lg border border-primary/45 bg-transparent px-3 py-1.5 text-[11.5px] font-semibold text-primary transition-colors hover:bg-primary/10"
        >
          Got it
        </button>
        <button
          type="button"
          data-testid="search-trust-notice-settings"
          onClick={onOpenSettings}
          className="cursor-pointer border-0 bg-transparent px-1 py-1.5 text-[11.5px] font-medium text-white/50 transition-colors hover:text-white/75"
        >
          Turn off in Settings
        </button>
      </div>
    </div>
  );
}
