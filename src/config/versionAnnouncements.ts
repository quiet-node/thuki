/**
 * Per-release announcement content for {@link VersionAnnouncement}.
 *
 * Add a new export for each versioned spotlight. Hosts wire show/hide +
 * persistence; this module holds copy and learn URLs only.
 */

/** Public blog post for Auto search / built-in web search disclosure. */
export const AUTO_SEARCH_PUBLIC_BLOG_POST_URL =
  'https://thuki.app/blog/thuki-built-in-web-search';

/**
 * v0.16.0 Auto search spotlight shown on the ask bar until the user
 * acknowledges (`behavior.search_notice_acknowledged`).
 */
export const V016_AUTO_SEARCH_ANNOUNCEMENT = {
  title: 'Thuki can now search the web, automatically!',
  body: 'Since v0.16.0, when a question needs fresh facts, Thuki smartly searches the web for them. Turn Auto search off to stay fully local and use /search only when you want a look-up.',
  learn: {
    label: 'See how Auto search works ↗',
    url: AUTO_SEARCH_PUBLIC_BLOG_POST_URL,
  },
} as const;

/**
 * Settings CTA label for the v0.16 Auto search announcement.
 *
 * @param autoSearchOn Current `behavior.auto_search` value.
 * @returns Turn off when on; Turn on when off.
 */
export function v016AutoSearchSettingsCta(autoSearchOn: boolean): string {
  return autoSearchOn ? 'Turn off in Settings' : 'Turn on in Settings';
}

/**
 * One-shot chat-header notice after the first auto-saved conversation.
 * Shown until `behavior.auto_save_notice_acknowledged` is true.
 */
export const AUTO_SAVE_NOTICE_ANNOUNCEMENT = {
  title: 'Chats are now auto-saved',
  body: 'Turn auto-save off if you prefer to bookmark only what matters.',
} as const;

/**
 * Settings CTA label for the auto-save chat notice tip.
 *
 * @returns Fixed deep-link copy (opens Settings; does not toggle auto-save).
 */
export function autoSaveNoticeSettingsCta(): string {
  return 'Turn off in Settings';
}
