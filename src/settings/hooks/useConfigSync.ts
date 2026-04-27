/**
 * Loads the resolved `RawAppConfig` on mount and re-syncs whenever the
 * Settings window gains focus (file may have changed externally).
 *
 * Replaces the file-watcher subsystem the eng review collapsed: the
 * `tauri://focus` event covers 99% of "I hand-edited the file" cases
 * because users naturally bounce focus to see results, and the explicit
 * "↻ Refresh from disk" button in About covers the 1%.
 *
 * Returns the current config plus a reload function the About-tab button
 * binds to. `null` while the initial fetch is in flight; render gating
 * is the SettingsWindow's responsibility.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

import type { RawAppConfig } from '../types';

export interface ConfigSyncHandle {
  config: RawAppConfig | null;
  /** Replaces local state with what Rust currently considers canonical. */
  reload: () => Promise<void>;
  /** Replaces local state without an IPC round-trip. Used after a save. */
  setConfig: (next: RawAppConfig) => void;
}

export function useConfigSync(): ConfigSyncHandle {
  const [config, setConfig] = useState<RawAppConfig | null>(null);

  const reload = useCallback(async () => {
    try {
      const next = await invoke<RawAppConfig>('reload_config_from_disk');
      setConfig(next);
    } catch {
      // Reload failure is non-fatal; the previous in-memory snapshot is
      // still valid. We surface nothing to the user because they did not
      // explicitly request the reload (focus-event triggered).
    }
  }, []);

  // Initial mount + focus listener.
  useEffect(() => {
    let mounted = true;
    void invoke<RawAppConfig>('get_config').then((next) => {
      if (mounted) setConfig(next);
    });

    const window = getCurrentWindow();
    let unlisten: (() => void) | null = null;
    void window
      .onFocusChanged(({ payload: focused }) => {
        if (focused) void reload();
      })
      .then((stop) => {
        unlisten = stop;
        if (!mounted) stop();
      });

    return () => {
      mounted = false;
      unlisten?.();
    };
  }, [reload]);

  return { config, reload, setConfig };
}
