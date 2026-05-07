import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface UpdaterState {
  last_check_at_unix: number | null;
  update: { version: string; notes_url: string | null } | null;
  settings_snoozed_until: number | null;
  chat_snoozed_until: number | null;
}

const EMPTY: UpdaterState = {
  last_check_at_unix: null,
  update: null,
  settings_snoozed_until: null,
  chat_snoozed_until: null,
};

export function useUpdater() {
  const [state, setState] = useState<UpdaterState>(EMPTY);

  const refresh = useCallback(async () => {
    const next = await invoke<UpdaterState>('get_updater_state');
    if (next) setState(next);
  }, []);

  useEffect(() => {
    void refresh();
    const unlistenPromise = listen<UpdaterState>(
      'update-available',
      (event) => {
        setState(event.payload);
      },
    );
    return () => {
      void unlistenPromise.then((fn) => fn());
    };
  }, [refresh]);

  const checkNow = useCallback(async () => {
    const next = await invoke<UpdaterState>('check_for_update');
    if (next) setState(next);
  }, []);

  const install = useCallback(async () => {
    await invoke('install_update');
  }, []);

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

  return { state, checkNow, install, snoozeChat, snoozeSettings };
}
