import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { ConfigProvider } from './contexts/ConfigContext';

/**
 * Entry point for the React application.
 *
 * Mounts the root App component into the DOM container with ID 'root'.
 * ConfigProvider blocks child render until the backend `get_config` resolves,
 * so every component can read `useConfig()` synchronously.
 */
ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <ConfigProvider>
      <App />
    </ConfigProvider>
  </React.StrictMode>,
);
