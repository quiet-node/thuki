/**
 * Live snapshot of the built-in engine's lifecycle, sourced from the same
 * global Tauri events the Settings Providers pane uses for its status line
 * (`engine:status`, `warmup:builtin-warming` / `warmup:builtin-warmed`, all
 * emitted app-wide so any window can subscribe). Ollama has no equivalent
 * signal for either, since Thuki does not manage its process, so `warming`
 * never flips true and `engineState` stays at its initial value while
 * Ollama is the active provider.
 *
 * Meant to be mounted once near the app root (the main window's React tree
 * stays alive for the app's lifetime, even while hidden), so the value is
 * already current by the time a chat turn needs it. A component that mounts
 * only in chat mode would risk missing a warming state that started before
 * it mounted.
 */

import { useEffect, useState } from 'react';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { EngineStatus } from '../types/starter';

export interface EngineWarmupStatus {
  /** True while the built-in engine is priming the system-prompt prefix. */
  warming: boolean;
  /** The built-in engine's last known lifecycle state. */
  engineState: EngineStatus['state'];
}

export function useEngineWarmupStatus(): EngineWarmupStatus {
  const [warming, setWarming] = useState(false);
  const [engineState, setEngineState] =
    useState<EngineStatus['state']>('stopped');

  useEffect(() => {
    let cancelled = false;
    const unlisteners: UnlistenFn[] = [];

    const subscribe = <T>(event: string, handler: (payload: T) => void) => {
      void listen<T>(event, (e) => handler(e.payload))
        .then((stop) => {
          if (cancelled) {
            stop();
            return;
          }
          unlisteners.push(stop);
        })
        .catch(() => {
          // Event bridge unavailable (test env / Tauri not ready).
        });
    };

    subscribe('warmup:builtin-warming', () => setWarming(true));
    subscribe('warmup:builtin-warmed', () => setWarming(false));
    subscribe<EngineStatus>('engine:status', (status) =>
      setEngineState(status.state),
    );

    return () => {
      cancelled = true;
      unlisteners.forEach((stop) => stop());
    };
  }, []);

  return { warming, engineState };
}
