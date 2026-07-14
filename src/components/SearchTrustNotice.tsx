/**
 * First-use, non-blocking card for Auto search (v0.16.0). Mounted on the ask
 * bar **below** the logo/input row (design D footer) until acknowledged,
 * whether Auto search is on or off. Never gates compose.
 *
 * Visual: flat content inside the ask-bar footer slot (parent supplies the
 * hairline border + dim fill). No nested elevated card so the bar does not
 * look double-boxed.
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
  /**
   * Current Auto search setting. Drives Settings CTA: "Turn off" when on,
   * "Turn on" when off. Deep-link never flips the toggle itself.
   */
  autoSearchOn?: boolean;
}

/**
 * Opens the Auto search disclosure URL via Tauri open_url (http/https only).
 */
function openDisclosure(): void {
  void invoke('open_url', { url: SEARCH_DISCLOSURE_URL });
}

/**
 * Flat footer notice body (design D). Parent AskBarView owns the border-top
 * slot chrome. Never blocks compose.
 *
 * @param onAcknowledge Got-it / Acknowledge persistence path.
 * @param onOpenSettings Settings deep-link (no silent auto_search flip).
 * @param autoSearchOn Current toggle; defaults true (product default).
 */
export function SearchTrustNotice({
  onAcknowledge,
  onOpenSettings,
  autoSearchOn = true,
}: SearchTrustNoticeProps) {
  const settingsCta = autoSearchOn
    ? 'Turn off in Settings'
    : 'Turn on in Settings';

  return (
    <div
      data-testid="search-trust-notice"
      role="region"
      aria-label={SEARCH_TRUST_NOTICE_TITLE}
    >
      <p className="text-[13px] font-medium text-white/90 leading-snug">
        {SEARCH_TRUST_NOTICE_TITLE}
      </p>
      <p className="mt-1 text-xs text-white/45 leading-relaxed">
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
          Acknowledge
        </button>
        <button
          type="button"
          data-testid="search-trust-notice-settings"
          onClick={onOpenSettings}
          className="cursor-pointer border-0 bg-transparent px-1 py-1.5 text-[11.5px] font-medium text-white/50 transition-colors hover:text-white/75"
        >
          {settingsCta}
        </button>
      </div>
    </div>
  );
}
