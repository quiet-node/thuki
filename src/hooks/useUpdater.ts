import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface AvailableUpdate {
  version: string;
  notes_url: string | null;
  /** Markdown release notes from the updater manifest, when present. */
  body: string | null;
  /** RFC3339 publish timestamp from the manifest, when present. */
  date: string | null;
}

export interface UpdaterState {
  last_check_at_unix: number | null;
  update: AvailableUpdate | null;
  settings_snoozed_until: number | null;
  chat_snoozed_until: number | null;
  /** Versions the user dismissed via "Skip This Version". */
  skipped_versions: string[];
}

const EMPTY: UpdaterState = {
  last_check_at_unix: null,
  update: null,
  settings_snoozed_until: null,
  chat_snoozed_until: null,
  skipped_versions: [],
};

function withFallbackNotes(s: UpdaterState): UpdaterState {
  if (!s.update || s.update.notes_url) return s;
  return {
    ...s,
    update: {
      ...s.update,
      notes_url: `https://github.com/quiet-node/thuki/releases/tag/v${s.update.version}`,
    },
  };
}

export function useUpdater() {
  const [state, setState] = useState<UpdaterState>(EMPTY);

  const refresh = useCallback(async () => {
    const next = await invoke<UpdaterState>('get_updater_state');
    if (next) setState(withFallbackNotes(next));
  }, []);

  useEffect(() => {
    void refresh();
    const unlistenPromise = listen<UpdaterState>(
      'update-available',
      (event) => {
        setState(withFallbackNotes(event.payload));
      },
    );
    return () => {
      void unlistenPromise.then((fn) => fn());
    };
  }, [refresh]);

  const checkNow = useCallback(async () => {
    const next = await invoke<UpdaterState>('check_for_update');
    if (next) setState(withFallbackNotes(next));
  }, []);

  const install = useCallback(async () => {
    await invoke('install_update');
  }, []);

  const installAndQuit = useCallback(async () => {
    await invoke('install_update_and_quit');
  }, []);

  const openWindow = useCallback(async () => {
    await invoke('open_update_window');
  }, []);

  const skip = useCallback(async () => {
    await invoke('skip_update_version');
    await refresh();
  }, [refresh]);

  const snoozeChat = useCallback(
    async (hours: number) => {
      await invoke('snooze_update_chat', { hours });
      await refresh();
    },
    [refresh],
  );

  const snoozeSettings = useCallback(
    async (hours: number) => {
      await invoke('snooze_update_settings', { hours });
      await refresh();
    },
    [refresh],
  );

  return {
    state,
    checkNow,
    install,
    installAndQuit,
    openWindow,
    skip,
    snoozeChat,
    snoozeSettings,
  };
}
