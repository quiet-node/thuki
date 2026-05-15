/**
 * Top-level component for the "What's New" update NSWindow.
 *
 * Mounted by `rootForLabel` when the Tauri window label is `update`. Shows
 * the available version's release notes (rendered markdown from the updater
 * manifest, with a GitHub-link fallback when the manifest omits notes) and
 * four explicit actions so an install never starts on a single stray click:
 *
 *   - Skip This Version  → never nag for this exact version again
 *   - Remind Me Later     → snooze both surfaces for 24h
 *   - Install & Quit      → download + swap the bundle, then exit
 *   - Install & Restart   → download + swap + relaunch
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

/** Hoisted "gift" header glyph: a release/unwrap visual cue. */
const GIFT_ICON = (
  <svg
    width="22"
    height="22"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.8"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <polyline points="20 12 20 22 4 22 4 12" />
    <rect x="2" y="7" width="20" height="5" />
    <line x1="12" y1="22" x2="12" y2="7" />
    <path d="M12 7H7.5a2.5 2.5 0 0 1 0-5C11 2 12 7 12 7z" />
    <path d="M12 7h4.5a2.5 2.5 0 0 0 0-5C13 2 12 7 12 7z" />
  </svg>
);

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

  const handleSkip = useCallback(() => {
    void updater.skip().then(close);
  }, [updater, close]);

  const handleLater = useCallback(() => {
    // "Later" should quiet every surface (chat footer + settings banner),
    // not just the one the window happened to be opened from.
    void Promise.all([updater.snoozeChat(24), updater.snoozeSettings(24)]).then(
      close,
    );
  }, [updater, close]);

  const handleInstallQuit = useCallback(() => {
    void updater.installAndQuit();
  }, [updater]);

  const handleInstallRestart = useCallback(() => {
    void updater.install();
  }, [updater]);

  return (
    <div
      className="flex h-screen w-screen flex-col overflow-hidden rounded-xl bg-[#0d0d0f] text-text-primary"
      onMouseDown={handleDragStart}
    >
      <WindowControls onClose={close} />

      {update ? (
        <>
          <header className="flex items-center gap-3 px-6 pt-4 pb-3">
            <span className="text-primary" aria-hidden>
              {GIFT_ICON}
            </span>
            <div className="min-w-0">
              <h1 className="truncate text-[15px] font-semibold text-text-primary">
                {`Thuki ${update.version} is ready`}
              </h1>
              {formatReleaseDate(update.date) ? (
                <p className="text-[11px] text-text-secondary">
                  {`Released ${formatReleaseDate(update.date)}`}
                </p>
              ) : null}
            </div>
          </header>

          <div className="h-px shrink-0 bg-surface-border" />

          <div
            className="min-h-0 flex-1 overflow-y-auto px-6 py-4 text-[13px] leading-relaxed"
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

          <div className="h-px shrink-0 bg-surface-border" />

          <footer className="flex shrink-0 items-center gap-2 px-6 py-3">
            <button
              type="button"
              onClick={handleSkip}
              className="rounded-md px-2.5 py-1.5 text-[12px] text-text-secondary transition-colors duration-150 hover:text-text-primary hover:bg-white/5 cursor-pointer"
            >
              Skip This Version
            </button>
            <button
              type="button"
              onClick={handleLater}
              className="rounded-md px-2.5 py-1.5 text-[12px] text-text-secondary transition-colors duration-150 hover:text-text-primary hover:bg-white/5 cursor-pointer"
            >
              Remind Me Later
            </button>
            <div className="ml-auto flex items-center gap-2">
              <button
                type="button"
                onClick={handleInstallQuit}
                className="rounded-md border border-surface-border px-3 py-1.5 text-[12px] text-text-primary transition-colors duration-150 hover:bg-white/5 cursor-pointer"
              >
                Install &amp; Quit
              </button>
              <button
                type="button"
                onClick={handleInstallRestart}
                className="rounded-md bg-primary px-3 py-1.5 text-[12px] font-medium text-black transition-opacity duration-150 hover:opacity-90 cursor-pointer"
              >
                Install &amp; Restart
              </button>
            </div>
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
