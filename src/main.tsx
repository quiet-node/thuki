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

const root = ReactDOM.createRoot(
  document.getElementById('root') as HTMLElement,
);
const label = getCurrentWindow().label;

if (label === 'settings') {
  root.render(
    <React.StrictMode>
      <SettingsWindow />
    </React.StrictMode>,
  );
} else {
  root.render(
    <React.StrictMode>
      <ConfigProvider>
        <App />
      </ConfigProvider>
    </React.StrictMode>,
  );
}
