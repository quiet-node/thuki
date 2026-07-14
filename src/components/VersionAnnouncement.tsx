/**
 * Reusable one-shot version / feature announcement panel (design D footer).
 *
 * Presentational only: host owns when to show, persistence of dismiss, and
 * Settings deep-links. Content (title, body, CTAs, optional learn URL) is
 * injected per announcement so the same shell works for every release.
 *
 * Visual: flat body for a parent footer slot (border-top + dim fill). Never
 * gates compose.
 */

import { invoke } from '@tauri-apps/api/core';

/** One CTA in the announcement action row. */
export interface VersionAnnouncementAction {
  /** Button label shown to the user. */
  label: string;
  /** Click handler (host wires persist / deep-link / dismiss). */
  onClick: () => void;
  /**
   * primary = outlined primary CTA (e.g. Acknowledge).
   * secondary = ghost text button (e.g. Turn on/off in Settings).
   */
  variant: 'primary' | 'secondary';
  /** Optional data-testid for tests. */
  testId?: string;
}

/** Optional in-body learn link opened via open_url (http/https only). */
export interface VersionAnnouncementLearnLink {
  /** Visible label, often including ↗. */
  label: string;
  /** Absolute https URL. */
  url: string;
  /** Optional data-testid for the learn control. */
  testId?: string;
}

export interface VersionAnnouncementProps {
  /** Headline (e.g. feature or version pitch). */
  title: string;
  /** Single paragraph body; soft-wraps, no hard newlines required. */
  body: string;
  /** Optional learn link appended inline after the body. */
  learn?: VersionAnnouncementLearnLink;
  /** Ordered action buttons under the body. */
  actions: VersionAnnouncementAction[];
  /** Root data-testid; default version-announcement. */
  testId?: string;
}

/**
 * Opens a learn URL through the Tauri open_url command (scheme-gated).
 *
 * @param url Absolute http(s) URL to open in the default browser.
 */
function openLearnUrl(url: string): void {
  void invoke('open_url', { url });
}

/**
 * Flat announcement panel for ask-bar (or similar) footer slots.
 *
 * @param title Announcement headline.
 * @param body One-paragraph body copy.
 * @param learn Optional inline learn link.
 * @param actions Primary/secondary action row.
 * @param testId Root test id.
 */
export function VersionAnnouncement({
  title,
  body,
  learn,
  actions,
  testId = 'version-announcement',
}: VersionAnnouncementProps) {
  return (
    <div data-testid={testId} role="region" aria-label={title}>
      <p className="text-[13px] font-medium text-white/90 leading-snug">
        {title}
      </p>
      <p className="mt-1 text-xs text-white/45 leading-relaxed">
        {body}
        {learn ? (
          <>
            {' '}
            <button
              type="button"
              data-testid={learn.testId ?? `${testId}-learn`}
              onClick={() => openLearnUrl(learn.url)}
              className="inline cursor-pointer border-0 bg-transparent p-0 m-0 text-xs text-primary/80 hover:text-primary underline-offset-2 hover:underline"
            >
              {learn.label}
            </button>
          </>
        ) : null}
      </p>
      {actions.length > 0 ? (
        <div className="mt-2.5 flex flex-wrap items-center gap-2">
          {actions.map((action) => (
            <button
              key={action.label}
              type="button"
              data-testid={action.testId}
              onClick={action.onClick}
              className={
                action.variant === 'primary'
                  ? 'cursor-pointer rounded-lg border border-primary/45 bg-transparent px-3 py-1.5 text-[11.5px] font-semibold text-primary transition-colors hover:bg-primary/10'
                  : 'cursor-pointer border-0 bg-transparent px-1 py-1.5 text-[11.5px] font-medium text-white/50 transition-colors hover:text-white/75'
              }
            >
              {action.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
