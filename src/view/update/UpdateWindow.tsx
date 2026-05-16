/**
 * Top-level component for the "What's New" update NSWindow.
 *
 * Mounted by `rootForLabel` when the Tauri window label is `update`. Shows
 * the available version's release notes (rendered markdown from the updater
 * manifest, with a GitHub-link fallback when the manifest omits notes) and
 * three explicit actions so an install never starts on a single stray click:
 *
 *   - Skip This Version : never nag for this exact version again
 *   - Remind Me Later    : snooze both surfaces for 24h
 *   - Install Update     : download + swap the bundle, then relaunch
 *
 * Visual direction: "Settings-panel parity" (approved design D). The window
 * mirrors the Settings window's visual system so the two read as the same
 * app: the radial-glow-over-surface-base chrome with its warm border and
 * `::before` hairline (`.update-window-shell` in App.css, lifted verbatim
 * from settings `.window`), a centered bare-mascot hero, the release notes
 * inside a `.updateHero`-style card, and the primary action styled like the
 * Settings "Check for updates" button (`.updateHeroBtn`).
 *
 * Single source of truth: brand colors come from the `@theme` tokens in
 * App.css; the few literal rgba values in the footer/shell are the exact
 * settings.module.css values (no Settings CSS module is importable here, so
 * they are reproduced as arbitrary Tailwind values and documented as such).
 *
 * The window is an NSPanel (see `init_update_panel` in lib.rs); closing it
 * hides rather than destroys (CloseRequested intercept), so reopening is
 * cheap and React state is preserved.
 */

import { useCallback, useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getVersion } from '@tauri-apps/api/app';

import { useUpdater } from '../../hooks/useUpdater';
import { MarkdownRenderer } from '../../components/MarkdownRenderer';
import { WindowControls } from '../../components/WindowControls';

export function UpdateWindow() {
  const updater = useUpdater();
  const update = updater.state.update;

  // The currently-installed version, for the "You have X." half of the
  // subline. Fetched async; until it resolves (or if it fails) the subline
  // simply omits that clause rather than blocking the window.
  const [currentVersion, setCurrentVersion] = useState<string | null>(null);
  useEffect(() => {
    void getVersion()
      .then(setCurrentVersion)
      .catch(() => {});
  }, []);

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

  const handleInstall = useCallback(() => {
    runAction(updater.install());
  }, [updater, runAction]);

  return (
    <div
      className="update-window-shell font-sans flex h-screen w-screen flex-col overflow-hidden rounded-[10px] border border-[rgba(255,141,92,0.12)] border-t-[rgba(255,141,92,0.2)] text-text-primary"
      onMouseDown={handleDragStart}
    >
      <WindowControls onClose={close} />

      {update ? (
        <>
          <header className="flex flex-col items-center px-12 pt-7 pb-6 text-center">
            <img
              src="/thuki-logo.png"
              alt="Thuki"
              className="h-[76px] w-[76px] object-contain"
            />
            <h1 className="mt-4 text-[18px] font-bold tracking-[-0.2px] text-text-primary">
              A new version of Thuki is available!
            </h1>
            <p className="mt-2 text-[13px] leading-[1.5] whitespace-nowrap text-text-secondary">
              {`Version ${update.version}`}
              {currentVersion ? ` · you have ${currentVersion}` : ''}
            </p>
          </header>

          <div className="mx-12 mb-[14px] text-[11px] font-semibold tracking-[0.18em] uppercase text-text-secondary">
            Release Notes
          </div>

          <div
            className="update-notes mx-12 min-h-0 flex-1 overflow-y-auto p-4 text-[13px] leading-[1.55]"
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

          <footer className="flex flex-nowrap items-center gap-2 border-t border-[rgba(255,255,255,0.045)] px-9 pt-[16px] pb-5">
            <button
              type="button"
              onClick={handleSkip}
              className="shrink-0 rounded-md px-2 py-[7px] text-[12.5px] whitespace-nowrap text-text-secondary transition-colors duration-150 hover:text-text-primary cursor-pointer"
            >
              Skip This Version
            </button>
            <span className="ml-auto" />
            <button
              type="button"
              onClick={handleLater}
              className="shrink-0 rounded-lg border border-[rgba(255,255,255,0.06)] px-[14px] py-[7px] text-[12.5px] font-medium whitespace-nowrap text-text-primary transition-colors duration-150 hover:bg-[rgba(255,255,255,0.03)] hover:border-[rgba(255,255,255,0.1)] cursor-pointer"
            >
              Remind Me Later
            </button>
            <button
              type="button"
              onClick={handleInstall}
              className="ml-2 shrink-0 rounded-lg border border-[rgba(255,141,92,0.3)] bg-[rgba(255,141,92,0.12)] px-[14px] py-[7px] text-[12px] font-medium whitespace-nowrap text-primary transition-colors duration-150 hover:bg-[rgba(255,141,92,0.18)] hover:border-[rgba(255,141,92,0.42)] cursor-pointer"
            >
              Install Update
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
