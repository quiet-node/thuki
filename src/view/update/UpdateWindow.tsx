/**
 * Top-level component for the "What's New" update NSWindow.
 *
 * Mounted by `rootForLabel` when the Tauri window label is `update`. Shows
 * the available version's release notes (rendered markdown from the updater
 * manifest, with a GitHub-link fallback when the manifest omits notes) and
 * four explicit actions so an install never starts on a single stray click:
 *
 *   - Skip This Version  : never nag for this exact version again
 *   - Remind Me Later     : snooze both surfaces for 24h
 *   - Install & Quit      : download + swap the bundle, then exit
 *   - Install & Restart   : download + swap + relaunch
 *
 * Visual direction: "Editorial / Luxury" (approved design). Centered, airy,
 * the real Thuki mascot in a soft radial vignette, a light-weight display
 * title, typeset release notes, and a strictly single-line action row.
 *
 * Single source of truth: every color comes from the `@theme` tokens in
 * App.css (Tailwind `*-primary` / `*-text-*` / `*-surface-*` utilities) and
 * the app font from `font-sans` (Nunito). No hardcoded palette values here.
 * Release notes render through MarkdownRenderer inside `.update-notes`,
 * which App.css scopes to a refined editorial type scale (still Nunito,
 * inherited from the global `.markdown-body` rule) instead of chat prose.
 *
 * The window is an NSPanel (see `init_update_panel` in lib.rs); closing it
 * hides rather than destroys (CloseRequested intercept), so reopening is
 * cheap and React state is preserved.
 */

import { useCallback } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { useUpdater } from '../../hooks/useUpdater';
import { MarkdownRenderer } from '../../components/MarkdownRenderer';
import { WindowControls } from '../../components/WindowControls';

/**
 * Extracts a human-readable `YYYY-MM-DD` from the manifest date. The
 * backend forwards `OffsetDateTime`'s Display string, whose exact shape is
 * not guaranteed to parse via `new Date`, so we pull the leading ISO date
 * defensively and render nothing if it is absent.
 */
function formatReleaseDate(date: string | null): string | null {
  if (!date) return null;
  const match = /^\d{4}-\d{2}-\d{2}/.exec(date.trim());
  return match ? match[0] : null;
}

export function UpdateWindow() {
  const updater = useUpdater();
  const update = updater.state.update;

  const close = useCallback(() => {
    void getCurrentWindow().hide();
  }, []);

  /**
   * Native window drag from non-interactive surfaces. Mirrors
   * SettingsWindow: bail on interactive tags and text-bearing leaves so
   * users can still click buttons and select the release notes.
   */
  const handleDragStart = useCallback((e: React.MouseEvent) => {
    if (e.button !== 0) return;
    const el = e.target as HTMLElement;

    const INTERACTIVE_TAGS = new Set([
      'TEXTAREA',
      'INPUT',
      'BUTTON',
      'A',
      'SELECT',
      'PATH',
      'SVG',
      'IMG',
      'LABEL',
      'CODE',
      'PRE',
    ]);
    let current: HTMLElement | null = el;
    while (current) {
      if (INTERACTIVE_TAGS.has(current.tagName.toUpperCase())) return;
      current = current.parentElement;
    }

    for (const node of Array.from(el.childNodes)) {
      if (
        node.nodeType === Node.TEXT_NODE &&
        node.textContent &&
        node.textContent.trim().length > 0
      ) {
        return;
      }
    }

    e.preventDefault();
    void getCurrentWindow().startDragging();
  }, []);

  /**
   * Runs an update action, then closes the window. A rejected action is
   * logged (never silently swallowed) and still closes the window so it
   * cannot hang with no feedback: the chat footer and Settings banner
   * persist as the retry surface, and a failed install simply leaves the
   * app on its current version.
   */
  const runAction = useCallback(
    (action: Promise<unknown>) => {
      action
        .catch((err: unknown) => {
          console.error('update window action failed', err);
        })
        .finally(close);
    },
    [close],
  );

  const handleSkip = useCallback(() => {
    runAction(updater.skip());
  }, [updater, runAction]);

  const handleLater = useCallback(() => {
    // "Later" should quiet every surface (chat footer + settings banner),
    // not just the one the window happened to be opened from.
    runAction(
      Promise.all([updater.snoozeChat(24), updater.snoozeSettings(24)]),
    );
  }, [updater, runAction]);

  const handleInstallQuit = useCallback(() => {
    runAction(updater.installAndQuit());
  }, [updater, runAction]);

  const handleInstallRestart = useCallback(() => {
    runAction(updater.install());
  }, [updater, runAction]);

  const releaseDate = update ? formatReleaseDate(update.date) : null;

  return (
    <div
      className="font-sans flex h-screen w-screen flex-col overflow-hidden rounded-2xl bg-surface-base text-text-primary"
      onMouseDown={handleDragStart}
    >
      <WindowControls onClose={close} />

      {update ? (
        <>
          <header className="px-12 pt-7 pb-7 text-center">
            <div
              className="mx-auto mb-5 flex h-[78px] w-[78px] items-center justify-center rounded-full"
              style={{
                background:
                  'radial-gradient(circle, color-mix(in srgb, var(--color-primary) 10%, transparent) 0%, transparent 70%)',
              }}
            >
              <img
                src="/thuki-logo.png"
                alt="Thuki"
                className="h-[62px] w-[62px] object-contain"
              />
            </div>
            <div className="text-[11px] font-bold tracking-[3px] uppercase text-text-secondary">
              Update Available
            </div>
            <h1 className="mt-3 text-[32px] font-light tracking-[-0.6px] text-text-primary">
              {'Thuki '}
              <span className="font-bold">{update.version}</span>
            </h1>
            {releaseDate ? (
              <div className="mt-3 text-[12.5px] tracking-[0.2px] text-text-secondary">
                {`Released ${releaseDate}`}
              </div>
            ) : null}
          </header>

          <div className="mx-12 h-px bg-surface-border" />

          <div
            className="update-notes min-h-0 flex-1 overflow-y-auto px-14 pt-6 pb-7"
            data-testid="update-notes"
          >
            <MarkdownRenderer
              content={
                update.body && update.body.trim().length > 0
                  ? update.body
                  : update.notes_url
                    ? `Release notes for this version aren't bundled in the update manifest. [View them on GitHub](${update.notes_url}).`
                    : 'No release notes are available for this version.'
              }
            />
          </div>

          <footer className="flex flex-nowrap items-center gap-[5px] border-t border-surface-border px-6 pt-[18px] pb-[22px]">
            <button
              type="button"
              onClick={handleSkip}
              className="shrink-0 rounded-md px-[9px] py-[9px] text-[12px] whitespace-nowrap text-text-secondary transition-colors duration-150 hover:text-text-primary cursor-pointer"
            >
              Skip This Version
            </button>
            <span className="ml-auto" />
            <button
              type="button"
              onClick={handleLater}
              className="shrink-0 rounded-md px-[11px] py-[9px] text-[12.5px] whitespace-nowrap text-text-secondary transition-colors duration-150 hover:text-text-primary cursor-pointer"
            >
              Remind Me Later
            </button>
            <button
              type="button"
              onClick={handleInstallQuit}
              className="shrink-0 rounded-[9px] border border-surface-border px-[15px] py-[9px] text-[12.5px] whitespace-nowrap text-text-primary transition-colors duration-150 hover:bg-white/[0.04] cursor-pointer"
            >
              Install &amp; Quit
            </button>
            <button
              type="button"
              onClick={handleInstallRestart}
              className="ml-[5px] shrink-0 rounded-[9px] bg-primary px-[16px] py-[9px] text-[12.5px] font-bold whitespace-nowrap text-neutral transition-[filter] duration-150 hover:brightness-105 cursor-pointer"
            >
              Install &amp; Restart
            </button>
          </footer>
        </>
      ) : (
        <div
          className="flex flex-1 items-center justify-center px-6 text-[13px] text-text-secondary"
          data-testid="update-empty"
        >
          Thuki is up to date.
        </div>
      )}
    </div>
  );
}
