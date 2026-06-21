/**
 * Loads the resolved `RawAppConfig` on mount and keeps it in sync via two
 * channels: the `thuki://config-updated` broadcast (fired after ANY in-app
 * config write, including a model change made from the overlay window) and
 * the Settings window's `tauri://focus` event (covers external hand-edits to
 * the file).
 *
 * The broadcast refresh reads the in-memory snapshot with `get_config`, never
 * `reload_config_from_disk`: that command re-emits `config-updated` (which
 * would loop) and re-runs residency side-effects the originating write already
 * performed. Focus still uses `reload` because a hand-edit only lands on disk.
 *
 * The explicit "↻ Refresh from disk" button in About covers the rare case
 * where neither fires.
 *
 * Returns the current config plus a reload function the About-tab button
 * binds to. `null` while the initial fetch is in flight; render gating
 * is the SettingsWindow's responsibility.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

import type { RawAppConfig } from '../types';

/**
 * Backend broadcast fired after the in-memory `AppConfig` is replaced. Mirrors
 * the Rust-side `CONFIG_UPDATED_EVENT` and the literal used in `ConfigContext`.
 */
const CONFIG_UPDATED_EVENT = 'thuki://config-updated';

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

  // Initial mount + focus listener + config-updated subscription.
  useEffect(() => {
    let mounted = true;

    // Read-only refresh from the in-memory snapshot. Used for the initial
    // hydrate and for every config-updated broadcast. Deliberately NOT
    // `reload()` (see the hook doc comment): that path loops and re-runs
    // residency side-effects.
    const refreshFromMemory = () => {
      void invoke<RawAppConfig>('get_config')
        .then((next) => {
          if (mounted) setConfig(next);
        })
        .catch(() => {
          // Non-fatal: keep the last good snapshot.
        });
    };

    refreshFromMemory();

    const window = getCurrentWindow();
    let unlistenFocus: (() => void) | null = null;
    void window
      .onFocusChanged(({ payload: focused }) => {
        if (focused) void reload();
      })
      .then((stop) => {
        unlistenFocus = stop;
        if (!mounted) stop();
      });

    let unlistenConfig: UnlistenFn | null = null;
    void listen(CONFIG_UPDATED_EVENT, () => {
      refreshFromMemory();
    })
      .then((stop) => {
        if (!mounted) {
          stop();
          return;
        }
        unlistenConfig = stop;
      })
      .catch(() => {
        // Event bridge unavailable (test env / Tauri not ready); focus and
        // the explicit Refresh button still resync.
      });

    return () => {
      mounted = false;
      unlistenFocus?.();
      unlistenConfig?.();
    };
  }, [reload]);

  return { config, reload, setConfig };
}
