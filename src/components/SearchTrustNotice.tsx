/**
 * First-use, non-blocking card explaining that auto-search sends queries
 * from the device to search services. Shown above the first search progress
 * block while Auto search is on and the user has not acknowledged yet.
 */

import { invoke } from '@tauri-apps/api/core';

/**
 * Public blog post URL for how Thuki search works.
 * Null until the post title/slug are chosen and the page is live.
 * When set, the notice shows "How search works" and opens this URL.
 */
export const SEARCH_DISCLOSURE_URL: string | null = null;

export const SEARCH_TRUST_NOTICE_TITLE =
  'Thuki searches the web for current info';

export const SEARCH_TRUST_NOTICE_BODY =
  'Questions that need fresh data are searched automatically. Queries go directly from your device to search services like DuckDuckGo and Wikipedia. Thuki has no servers and never sees them.';

export interface SearchTrustNoticeProps {
  /** Persist acknowledgement and hide the card. */
  onAcknowledge: () => void;
  /** Open Settings → Behavior with Auto search highlighted. */
  onOpenSettings: () => void;
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
        {SEARCH_TRUST_NOTICE_BODY}
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
        {SEARCH_DISCLOSURE_URL ? (
          <button
            type="button"
            data-testid="search-trust-notice-how"
            onClick={() =>
              void invoke('open_url', { url: SEARCH_DISCLOSURE_URL })
            }
            className="rounded-md px-2.5 py-1 text-xs font-medium text-white/55 hover:text-white/80 transition-colors"
          >
            How search works
          </button>
        ) : null}
      </div>
    </div>
  );
}
