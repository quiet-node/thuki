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
  useRef,
  useState,
  type ReactNode,
} from 'react';

import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { useConfigSync } from './hooks/useConfigSync';
import { useSettingsAutoResize } from './hooks/useSettingsAutoResize';
import { ModelTab } from './tabs/ModelTab';
import { SearchTab } from './tabs/SearchTab';
import { DisplayTab } from './tabs/DisplayTab';
import { AboutTab } from './tabs/AboutTab';
import { SavedPill } from './components';
import { WindowControls } from '../components/WindowControls';
import styles from '../styles/settings.module.css';
import type { CorruptMarker, RawAppConfig, SettingsTabId } from './types';

const TABS: ReadonlyArray<{
  id: SettingsTabId;
  label: string;
  icon: ReactNode;
}> = [
  {
    id: 'general',
    label: 'AI',
    // Brain — visual cue that this tab is for the AI itself.
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
        <path d="M9.5 2a3 3 0 0 0-3 3v.5a2.5 2.5 0 0 0-2 4 3 3 0 0 0 .5 5 2.5 2.5 0 0 0 1.5 4.5 3 3 0 0 0 5.5-1.5V5a3 3 0 0 0-2.5-3z" />
        <path d="M14.5 2a3 3 0 0 1 3 3v.5a2.5 2.5 0 0 1 2 4 3 3 0 0 1-.5 5 2.5 2.5 0 0 1-1.5 4.5 3 3 0 0 1-5.5-1.5V5a3 3 0 0 1 2.5-3z" />
      </svg>
    ),
  },
  {
    id: 'search',
    label: 'Web',
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
        <line x1="2" y1="12" x2="22" y2="12" />
        <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
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
 *   window padding-top (8) + WindowControls strip (~28) + tab bar (~70)
 *   + body padding top+bottom (18 + 24 = 42).
 * Empirically measured against the rendered Settings window. If any of
 * the chrome surfaces change height, update this constant rather than
 * trying to read `offsetHeight` at runtime — the auto-resize hook fires
 * before paint settles, so dynamic measurement of chrome would miss.
 */
const CHROME_HEIGHT = 148;
/** Recovery banner height when the corrupt-config marker is shown. */
const BANNER_HEIGHT = 56;

export function SettingsWindow() {
  const { config, reload, setConfig } = useConfigSync();
  const [activeTab, setActiveTab] = useState<SettingsTabId>('general');
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
        void getCurrentWindow().hide();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, []);

  const handleHide = useCallback(() => {
    void getCurrentWindow().hide();
  }, []);

  /**
   * Native window drag from any non-interactive surface — mirrors the
   * chat overlay's `handleDragStart` in App.tsx. Walks up the DOM from
   * the click target and bails if it hits a form control or button so
   * those keep working; otherwise calls `startDragging()`. We do this
   * via JS instead of `data-tauri-drag-region` because the attribute
   * only initiates drag from the element it's set on (and form
   * children inside the body block the attribute from working there).
   */
  const handleDragStart = useCallback((e: React.MouseEvent) => {
    const el = e.target as HTMLElement | null;
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
    let current = el;
    while (current) {
      if (INTERACTIVE_TAGS.has(current.tagName.toUpperCase())) return;
      current = current.parentElement;
    }
    e.preventDefault();
    void getCurrentWindow().startDragging();
  }, []);

  if (!config) return null;

  return (
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

      <div
        role="tablist"
        aria-label="Settings sections"
        className={styles.tabBar}
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
              className={`${styles.tab} ${active ? styles.tabActive : ''}`}
              onClick={() => setActiveTab(tab.id)}
              onKeyDown={(e) => {
                if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
                  e.preventDefault();
                  const idx = TABS.findIndex((t) => t.id === activeTab);
                  const next =
                    e.key === 'ArrowRight'
                      ? TABS[(idx + 1) % TABS.length]
                      : TABS[(idx - 1 + TABS.length) % TABS.length];
                  setActiveTab(next.id);
                }
              }}
            >
              <span className={styles.tabIcon} aria-hidden>
                {tab.icon}
              </span>
              <span className={styles.tabLabel}>{tab.label}</span>
            </button>
          );
        })}
      </div>

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
            />
          ) : null}
          {activeTab === 'search' ? (
            <SearchTab
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

      <SavedPill visible={savedVisible} />
    </div>
  );
}

function baseName(path: string): string {
  const idx = path.lastIndexOf('/');
  return idx >= 0 ? path.slice(idx + 1) : path;
}
