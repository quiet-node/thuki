/**
 * Live snapshot of the built-in engine's prefill-priming state, sourced from
 * the same global Tauri events the Settings Providers pane uses for its
 * "Warming up…" status line (`warmup:builtin-warming` / `warmup:builtin-warmed`,
 * emitted app-wide so any window can subscribe). Ollama has no equivalent
 * signal — Thuki does not manage its process — so `warming` never flips true
 * while Ollama is the active provider.
 *
 * Meant to be mounted once near the app root (the main window's React tree
 * stays alive for the app's lifetime, even while hidden), so the value is
 * already current by the time a chat turn needs it. A component that mounts
 * only in chat mode would risk missing a warming state that started before
 * it mounted.
 */

import { useEffect, useState } from 'react';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface EngineWarmupStatus {
  /** True while the built-in engine is priming the system-prompt prefix. */
  warming: boolean;
}

export function useEngineWarmupStatus(): EngineWarmupStatus {
  const [warming, setWarming] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: UnlistenFn[] = [];

    const subscribe = (event: string, handler: () => void) => {
      void listen(event, handler)
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

    return () => {
      cancelled = true;
      unlisteners.forEach((stop) => stop());
    };
  }, []);

  return { warming };
}
