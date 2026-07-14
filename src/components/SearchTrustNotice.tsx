/**
 * First-use, non-blocking card for Auto search (v0.16.0). Stays until Got it
 * (or auto_search is off); does not dismiss when the answer stream finishes.
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
 * Compact glass card matching chat chrome. Never blocks compose; search
 * proceeds underneath.
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
      className="mb-2 rounded-xl border border-white/10 bg-white/[0.04] px-3.5 py-3"
    >
      <p className="text-sm font-medium text-white/85 leading-snug">
        {SEARCH_TRUST_NOTICE_TITLE}
      </p>
      <p className="mt-1 text-xs text-white/45 leading-relaxed">
        {SEARCH_TRUST_NOTICE_BODY}{' '}
        <button
          type="button"
          data-testid="search-trust-notice-how"
          onClick={openDisclosure}
          className="inline p-0 m-0 border-0 bg-transparent text-xs text-white/55 hover:text-white/80 underline-offset-2 hover:underline cursor-pointer"
        >
          {SEARCH_TRUST_NOTICE_LEARN_LABEL}
        </button>
      </p>
      <div className="mt-2.5 flex flex-wrap items-center gap-2">
        <button
          type="button"
          data-testid="search-trust-notice-got-it"
          onClick={onAcknowledge}
          className="rounded-md bg-white/12 hover:bg-white/18 px-2.5 py-1 text-xs font-medium text-white/90 transition-colors"
        >
          Got it
        </button>
        <button
          type="button"
          data-testid="search-trust-notice-settings"
          onClick={onOpenSettings}
          className="rounded-md px-2.5 py-1 text-xs font-medium text-white/55 hover:text-white/80 transition-colors"
        >
          Turn off in Settings
        </button>
      </div>
    </div>
  );
}
