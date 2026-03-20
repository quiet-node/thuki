import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';

/**
 * Entry point for the React application.
 *
 * Mounts the root App component into the DOM container with ID 'root'.
 */
ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
