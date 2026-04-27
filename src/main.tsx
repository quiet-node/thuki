import React from 'react';
import ReactDOM from 'react-dom/client';
import { getCurrentWindow } from '@tauri-apps/api/window';

import App from './App';
import { ConfigProvider } from './contexts/ConfigContext';
import { SettingsWindow } from './settings/SettingsWindow';

/**
 * Entry point for the React application.
 *
 * One bundle serves both Tauri windows defined in `tauri.conf.json`. The
 * window label decides which root to mount: the `main` overlay gets the
 * full app + ConfigProvider; the `settings` window gets the standalone
 * Settings tree (which manages its own config snapshot via
 * `useConfigSync`).
 *
 * Mounting per-label keeps the Settings window from paying the cost of
 * the chat surface and avoids accidental cross-window state coupling.
 */

/**
 * Pure label-dispatch helper. Pulled out of the module-init expression so
 * tests can exercise both branches without re-evaluating the entire
 * module (vitest caches dynamic imports aggressively).
 */
export function rootForLabel(label: string): React.ReactElement {
  if (label === 'settings') {
    return (
      <React.StrictMode>
        <SettingsWindow />
      </React.StrictMode>
    );
  }
  return (
    <React.StrictMode>
      <ConfigProvider>
        <App />
      </ConfigProvider>
    </React.StrictMode>
  );
}

/* v8 ignore start */
// Entry-point boilerplate: tested indirectly via `rootForLabel` above. The
// `#root` existence guard lets the test suite import this module without
// the React entry trying to mount into a missing container.
const rootEl = document.getElementById('root');
if (rootEl) {
  ReactDOM.createRoot(rootEl).render(rootForLabel(getCurrentWindow().label));
}
/* v8 ignore stop */
