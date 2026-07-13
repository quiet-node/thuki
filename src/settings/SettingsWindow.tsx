/**
 * Top-level component for the Settings NSWindow.
 *
 * Owns the tab navigation, corrupt-recovery banner, the cross-tab Saved
 * pill, and the document-level Cmd+, re-focus listener (the one place a
 * keyboard accelerator can fire on the Settings window itself; tray-menu
 * accelerator is handled OS-side).
 *
 * Render gating: until the initial `get_config` resolves, the window
 * renders `null` rather than a flash skeleton (per the eng-review
 * Performance finding P1).
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';

import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';

import { DownloadsProvider } from '../contexts/DownloadsContext';
import { useConfigSync } from './hooks/useConfigSync';
import { useSettingsAutoResize } from './hooks/useSettingsAutoResize';
import { ModelTab } from './tabs/ModelTab';
import type { ModelsSubview } from './tabs/models/ModelsSegmented';
import { BehaviorTab } from './tabs/BehaviorTab';
import { DisplayTab } from './tabs/DisplayTab';
import { AboutTab } from './tabs/AboutTab';
import { SavedPill } from './components';
import { WindowControls } from '../components/WindowControls';
import { UpdateBanner } from '../components/UpdateBanner';
import { useUpdater } from '../hooks/useUpdater';
import { blurOnProgrammaticFocus } from '../utils/blurOnProgrammaticFocus';
import styles from '../styles/settings.module.css';
import type { CorruptMarker, RawAppConfig, SettingsTabId } from './types';

const TABS: ReadonlyArray<{
  id: SettingsTabId;
  label: string;
  icon: ReactNode;
}> = [
  {
    id: 'general',
    label: 'Models',
    // Grid — the model library / management surface.
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <rect x="3" y="3" width="7" height="7" rx="1.5" />
        <rect x="14" y="3" width="7" height="7" rx="1.5" />
        <rect x="3" y="14" width="7" height="7" rx="1.5" />
        <rect x="14" y="14" width="7" height="7" rx="1.5" />
      </svg>
    ),
  },
  {
    id: 'behavior',
    label: 'Behavior',
    // Sliders — settings that change how Thuki acts (text replacement, etc.).
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <line x1="21" y1="4" x2="14" y2="4" />
        <line x1="10" y1="4" x2="3" y2="4" />
        <line x1="21" y1="12" x2="12" y2="12" />
        <line x1="8" y1="12" x2="3" y2="12" />
        <line x1="21" y1="20" x2="16" y2="20" />
        <line x1="12" y1="20" x2="3" y2="20" />
        <line x1="14" y1="2" x2="14" y2="6" />
        <line x1="8" y1="10" x2="8" y2="14" />
        <line x1="16" y1="18" x2="16" y2="22" />
      </svg>
    ),
  },
  {
    id: 'display',
    label: 'Display',
    // Monitor with stand — appearance + presentation knobs.
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <rect x="2" y="3" width="20" height="14" rx="2" />
        <line x1="8" y1="21" x2="16" y2="21" />
        <line x1="12" y1="17" x2="12" y2="21" />
      </svg>
    ),
  },
  {
    id: 'about',
    label: 'About',
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <circle cx="12" cy="12" r="10" />
        <line x1="12" y1="16" x2="12" y2="12" />
        <line x1="12" y1="8" x2="12.01" y2="8" />
      </svg>
    ),
  },
];

const SAVED_PILL_DURATION_MS = 1500;

/**
 * Static chrome offset from inner content to total window height:
 *   window padding-top (8) + WindowControls strip (~28)
 *   + body padding top+bottom (18 + 24 = 42).
 * The section nav now lives in a left sidebar beside the content, so it no
 * longer adds vertical chrome (the old top tab bar did). The sidebar's own
 * height is seated by the hook's MIN_HEIGHT floor instead.
 * Empirically measured against the rendered Settings window. If any of
 * the chrome surfaces change height, update this constant rather than
 * trying to read `offsetHeight` at runtime — the auto-resize hook fires
 * before paint settles, so dynamic measurement of chrome would miss.
 */
const CHROME_HEIGHT = 78;
/** Recovery banner height when the corrupt-config marker is shown. */
const BANNER_HEIGHT = 56;

export function SettingsWindow() {
  const { config, reload, setConfig } = useConfigSync();
  const updater = useUpdater();
  const settingsSnoozed = useMemo(
    () => (updater.state.settings_snoozed_until ?? 0) * 1000 > Date.now(),
    [updater.state.settings_snoozed_until],
  );
  const [activeTab, setActiveTab] = useState<SettingsTabId>('general');
  // One-shot deep-link target for the Models tab's sub-view, set when the
  // overlay picker asks Settings to open straight on the Discover download
  // browser. Cleared by `ModelTab` once applied.
  const [pendingModelsView, setPendingModelsView] =
    useState<ModelsSubview | null>(null);
  const clearPendingModelsView = useCallback(
    () => setPendingModelsView(null),
    [],
  );
  const [savedVisible, setSavedVisible] = useState(false);
  const [marker, setMarker] = useState<CorruptMarker | null>(null);
  const [markerDismissed, setMarkerDismissed] = useState(false);

  // resyncToken bumps whenever a save lands so all SaveField rows re-seed
  // their local state from the new resolved config without scheduling
  // their own saves.
  const [resyncToken, setResyncToken] = useState(0);

  const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // State-backed ref so the auto-resize hook re-runs its effect when the
  // wrapper element actually mounts (it is gated behind `if (!config)
  // return null` and so does not exist on the first render).
  const [contentEl, setContentEl] = useState<HTMLDivElement | null>(null);

  const bannerVisible = Boolean(marker && !markerDismissed);
  const bodyShouldScroll = useSettingsAutoResize(
    contentEl,
    CHROME_HEIGHT + (bannerVisible ? BANNER_HEIGHT : 0),
    activeTab,
  );

  const handleSaved = useCallback(
    (next: RawAppConfig) => {
      setConfig(next);
      setResyncToken((prev) => prev + 1);
      setSavedVisible(true);
      if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
      savedTimerRef.current = setTimeout(() => {
        setSavedVisible(false);
        savedTimerRef.current = null;
      }, SAVED_PILL_DURATION_MS);
    },
    [setConfig],
  );

  useEffect(
    () => () => {
      if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
    },
    [],
  );

  // Consume the corrupt-recovery marker on mount.
  useEffect(() => {
    void invoke<CorruptMarker | null>('get_corrupt_marker').then((m) => {
      if (m) setMarker(m);
    });
  }, []);

  // The overlay model picker's "Settings" link opens this window straight on
  // the Models tab's Discover download browser, not the default Providers view.
  useEffect(() => {
    const unlistenPromise = listen('thuki://settings-show-discover', () => {
      setActiveTab('general');
      setPendingModelsView('discover');
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  // The ask-bar "Ollama isn't running" strip's "switch to Built-in" link opens
  // this window on the Models tab's Providers pane so the user can flip the
  // active provider back to the built-in engine. Forces the sub-view even if
  // the window was last left on Library/Discover.
  useEffect(() => {
    const unlistenPromise = listen('thuki://settings-show-providers', () => {
      setActiveTab('general');
      setPendingModelsView('providers');
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  // Keyboard shortcuts scoped to the Settings window.
  // Cmd+,: re-focus/re-raise (mac convention for "already open").
  // Cmd+W: hide the window (mac convention for closing a panel).
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.metaKey && e.key === ',') {
        e.preventDefault();
        void getCurrentWindow().setFocus();
      }
      if (e.metaKey && e.key === 'w') {
        e.preventDefault();
        void invoke('hide_settings_window');
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, []);

  // Route the close through the backend (not getCurrentWindow().hide()) so the
  // Rust side clears its Settings-open flag and drops the macOS Dock icon. A
  // raw hide() never reaches Rust, leaving the Dock icon stuck on.
  const handleHide = useCallback(() => {
    void invoke('hide_settings_window');
  }, []);

  /**
   * Native window drag from non-interactive, non-text surfaces. Walks
   * up the DOM and bails on:
   *   1. Interactive tags (form controls, buttons, links, SVGs) so
   *      clicks on them still register as clicks, not drags.
   *   2. Text-bearing leaves — any element that directly contains a
   *      non-empty text node. This lets users click-drag to highlight
   *      labels, values, and descriptions inside the body, then Cmd+C
   *      to copy. Without this check the whole window would slide
   *      under the cursor and the selection would never start.
   *
   * We do this via JS instead of `data-tauri-drag-region` because the
   * attribute only initiates drag from the element it's set on, and
   * form children inside the body block it from working at the root.
   *
   * Only the primary mouse button initiates a drag; secondary/middle
   * clicks pass through so context menus and middle-click behaviors
   * are unaffected.
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
    ]);
    let current: HTMLElement | null = el;
    while (current) {
      if (INTERACTIVE_TAGS.has(current.tagName.toUpperCase())) return;
      current = current.parentElement;
    }

    // Bail if the click landed directly on a text node. Layout
    // wrappers (DIV/SECTION) without their own text still drag.
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

  if (!config) return null;

  // The Settings window is its own webview root (see `main.tsx`), so it hosts
  // its own download registry: the Discover panes read their downloads from it,
  // and hosting it here (above the section nav and the Models segmented control)
  // keeps every in-flight download alive across each in-window tab switch. It is
  // independent of the main overlay's onboarding provider; the backend's keyed
  // slots are the real cross-window coordinator.
  return (
    <DownloadsProvider>
      <div className={styles.window} onMouseDown={handleDragStart}>
        <WindowControls onClose={handleHide} />

        {marker && !markerDismissed ? (
          <div className={styles.banner} role="alert">
            <span className={styles.bannerIcon} aria-hidden>
              ⚠
            </span>
            <span className={styles.bannerText}>
              Your previous <code>config.toml</code> had a syntax error and was
              saved as <code>{baseName(marker.path)}</code>. Defaults are now
              active.
            </span>
            <span className={styles.bannerActions}>
              <button
                type="button"
                className={`${styles.button} ${styles.buttonGhost}`}
                onClick={() =>
                  void invoke('open_url', {
                    url: `file://${encodeURI(marker.path).replace(/'/g, '%27')}`,
                  })
                }
              >
                Reveal
              </button>
              <button
                type="button"
                className={`${styles.button} ${styles.buttonGhost}`}
                onClick={() => setMarkerDismissed(true)}
              >
                Dismiss
              </button>
            </span>
          </div>
        ) : null}

        {updater.state.update && !settingsSnoozed ? (
          <UpdateBanner
            version={updater.state.update.version}
            notesUrl={updater.state.update.notes_url}
            onInstall={() => void updater.openWindow()}
            onLater={() => void updater.snoozeSettings(24)}
          />
        ) : null}

        <div className={styles.stage}>
          <div className={styles.side}>
            <div className={styles.sideGroup}>Settings</div>
            <div
              role="tablist"
              aria-label="Settings sections"
              aria-orientation="vertical"
              className={styles.sideTabs}
            >
              {TABS.map((tab) => {
                const active = tab.id === activeTab;
                return (
                  <button
                    key={tab.id}
                    type="button"
                    role="tab"
                    aria-selected={active}
                    aria-controls={`panel-${tab.id}`}
                    tabIndex={active ? 0 : -1}
                    className={`${styles.sideItem} ${active ? styles.sideItemActive : ''}`}
                    onClick={() => setActiveTab(tab.id)}
                    onFocus={blurOnProgrammaticFocus}
                    onKeyDown={(e) => {
                      const isNext =
                        e.key === 'ArrowDown' || e.key === 'ArrowRight';
                      const isPrev =
                        e.key === 'ArrowUp' || e.key === 'ArrowLeft';
                      if (isNext || isPrev) {
                        e.preventDefault();
                        const idx = TABS.findIndex((t) => t.id === activeTab);
                        const next = isNext
                          ? TABS[(idx + 1) % TABS.length]
                          : TABS[(idx - 1 + TABS.length) % TABS.length];
                        setActiveTab(next.id);
                      }
                    }}
                  >
                    <span className={styles.sideItemIcon} aria-hidden>
                      {tab.icon}
                    </span>
                    <span className={styles.sideItemLabel}>{tab.label}</span>
                  </button>
                );
              })}
            </div>
            <div className={styles.sideSpacer} />
          </div>

          <div className={styles.main}>
            <div
              className={`${styles.body} ${bodyShouldScroll ? styles.bodyScrollable : ''}`}
              id={`panel-${activeTab}`}
              role="tabpanel"
            >
              <div ref={setContentEl}>
                {activeTab === 'general' ? (
                  <ModelTab
                    config={config}
                    resyncToken={resyncToken}
                    onSaved={handleSaved}
                    pendingView={pendingModelsView}
                    onPendingViewConsumed={clearPendingModelsView}
                  />
                ) : null}
                {activeTab === 'behavior' ? (
                  <BehaviorTab
                    config={config}
                    resyncToken={resyncToken}
                    onSaved={handleSaved}
                  />
                ) : null}
                {activeTab === 'display' ? (
                  <DisplayTab
                    config={config}
                    resyncToken={resyncToken}
                    onSaved={handleSaved}
                  />
                ) : null}
                {activeTab === 'about' ? (
                  <AboutTab onSaved={handleSaved} onReload={reload} />
                ) : null}
              </div>
            </div>
          </div>
        </div>

        <SavedPill visible={savedVisible} />
      </div>
    </DownloadsProvider>
  );
}

function baseName(path: string): string {
  const idx = path.lastIndexOf('/');
  return idx >= 0 ? path.slice(idx + 1) : path;
}
