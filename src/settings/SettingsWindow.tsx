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
import { GeneralTab } from './tabs/GeneralTab';
import { SearchTab } from './tabs/SearchTab';
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
    label: 'General',
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
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
      </svg>
    ),
  },
  {
    id: 'search',
    label: 'Search',
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
        <circle cx="11" cy="11" r="8" />
        <line x1="21" y1="21" x2="16.65" y2="16.65" />
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

  // Cmd+, on the Settings window itself: re-focus / re-raise (mac convention
  // for "already open"). Listener only fires while Settings is the focused
  // window, which is the only context where this shortcut should do
  // anything per design doc P5.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.metaKey && e.key === ',') {
        e.preventDefault();
        void getCurrentWindow().setFocus();
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

      <div className={styles.body} id={`panel-${activeTab}`} role="tabpanel">
        {activeTab === 'general' ? (
          <GeneralTab
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
        {activeTab === 'about' ? (
          <AboutTab onSaved={handleSaved} onReload={reload} />
        ) : null}
      </div>

      <SavedPill visible={savedVisible} />
    </div>
  );
}

function baseName(path: string): string {
  const idx = path.lastIndexOf('/');
  return idx >= 0 ? path.slice(idx + 1) : path;
}
