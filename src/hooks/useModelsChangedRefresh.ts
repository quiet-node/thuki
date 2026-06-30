/**
 * Cross-window refresh for the curated model lists.
 *
 * The Settings Discover panes and the onboarding picker each read the on-disk
 * model set (installed models + interrupted partials) from the backend. That
 * set can change in *another* window: discarding a paused download deletes its
 * partial from disk. Without a live signal, the other window keeps showing the
 * stale "Paused · N% / Resume / Discard" row until a remount. This hook
 * subscribes to the backend's models-changed broadcast and re-pulls so the row
 * drops in place.
 */

import { useEffect } from 'react';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

/**
 * Backend broadcast fired after the on-disk model set changes (currently: a
 * partial download was discarded). Mirrors the Rust-side `MODELS_CHANGED_EVENT`
 * in `models/mod.rs`.
 */
export const MODELS_CHANGED_EVENT = 'thuki://models-changed';

/**
 * Re-runs `refresh` whenever another window changes the on-disk model set.
 *
 * `refresh` should be a stable callback (e.g. a `useCallback`), since it is the
 * effect dependency. The subscription is torn down on unmount, including the
 * case where the unmount beats `listen`'s resolution (the `cancelled` guard). A
 * missing event bridge (test env / Tauri not ready) degrades silently: mount
 * fetches and explicit refreshes still work; only the live cross-window push is
 * lost.
 */
export function useModelsChangedRefresh(
  refresh: () => void | Promise<void>,
): void {
  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;
    void listen(MODELS_CHANGED_EVENT, () => {
      void refresh();
    })
      .then((stop) => {
        if (cancelled) {
          stop();
          return;
        }
        unlisten = stop;
      })
      .catch(() => {
        // Event bridge unavailable (test env / Tauri not ready).
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh]);
}
