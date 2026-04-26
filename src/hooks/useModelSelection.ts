import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ModelPickerState } from '../types/model';

/**
 * Runtime guard for the IPC boundary. The Rust backend is trusted, but this
 * keeps the hook robust against shape drift (schema changes, legacy builds,
 * mocks) without pulling in a schema library. Accepts `null` for `active`
 * because Ollama's `/api/tags` is the single source of truth: the backend
 * returns null when nothing is installed and nothing is persisted.
 */
function isModelPickerState(value: unknown): value is ModelPickerState {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as {
    active?: unknown;
    all?: unknown;
    ollamaReachable?: unknown;
  };
  const activeOk =
    candidate.active === null || typeof candidate.active === 'string';
  return (
    activeOk &&
    Array.isArray(candidate.all) &&
    candidate.all.every((entry) => typeof entry === 'string') &&
    typeof candidate.ollamaReachable === 'boolean'
  );
}

/**
 * Shape returned by {@link useModelSelection}.
 */
export interface UseModelSelectionResult {
  /**
   * The currently active Ollama model name, or `null` when none is selected
   * (either nothing is installed or the picker has not resolved yet).
   * Consumers must treat `null` as "block the action and surface the picker",
   * never as a trigger to invent a default.
   */
  activeModel: string | null;
  /** All locally installed Ollama model names available for selection. */
  availableModels: string[];
  /**
   * Whether the most recent backend call reached the local Ollama daemon.
   * `true` is the optimistic default before the first fetch resolves so the
   * UI does not flash an "Ollama is down" strip during cold start. Set to
   * `false` only when the backend explicitly reports unreachability or the
   * IPC call itself rejects, so the strip can route the user to "start
   * Ollama" instead of "pull a model".
   */
  ollamaReachable: boolean;
  /**
   * Re-fetch the model picker state from the backend. Sets `activeModel` to
   * `null` and clears `availableModels` when the backend returns a malformed
   * payload or the call rejects. Callers are the single trigger: this hook
   * does not auto-retry.
   */
  refreshModels: () => Promise<void>;
  /**
   * Persist a new active model through the backend and sync local state
   * after the backend acknowledges the change. Rejects with the backend
   * error string so callers can surface the failure and trigger a refresh
   * to resync the UI.
   */
  setActiveModel: (model: string) => Promise<void>;
}

/**
 * React hook that manages the active Ollama model selection. Loads the
 * current model + the installed model list from the Rust backend on mount,
 * and exposes imperative helpers for refresh and selection.
 *
 * Request serialization: every refresh and selection increments a monotonic
 * token. Resolutions that belong to a stale token are dropped so rapid
 * out-of-order responses cannot overwrite newer state. Resolutions that fire
 * after unmount are also dropped to avoid React 18 StrictMode warnings.
 */
export function useModelSelection(): UseModelSelectionResult {
  // The state setter is intentionally renamed because `setActiveModel` is the
  // public async callback returned by this hook.
  // eslint-disable-next-line @eslint-react/use-state
  const [activeModel, setActiveModelState] = useState<string | null>(null);
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  // Optimistic default: assume reachable until the first fetch tells us
  // otherwise. This prevents a cold-start flash of the "Ollama is down"
  // strip while the IPC call is in flight.
  const [ollamaReachable, setOllamaReachable] = useState<boolean>(true);

  const mountedRef = useRef(true);
  const latestTokenRef = useRef(0);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const isLatest = useCallback((token: number): boolean => {
    return mountedRef.current && token === latestTokenRef.current;
  }, []);

  const refreshModels = useCallback(async (): Promise<void> => {
    latestTokenRef.current += 1;
    const token = latestTokenRef.current;
    try {
      const state = await invoke<unknown>('get_model_picker_state');
      if (!isLatest(token)) return;
      if (!isModelPickerState(state)) {
        // Treat malformed payloads as a transport failure: we cannot trust
        // any field, so fall back to the no-model state and assume Ollama
        // is unreachable so the strip nudges the user toward starting it.
        setActiveModelState(null);
        setAvailableModels([]);
        setOllamaReachable(false);
        return;
      }
      setActiveModelState(state.active);
      setAvailableModels(state.all);
      setOllamaReachable(state.ollamaReachable);
    } catch {
      if (!isLatest(token)) return;
      setActiveModelState(null);
      setAvailableModels([]);
      setOllamaReachable(false);
    }
  }, [isLatest]);

  useEffect(() => {
    void refreshModels();
  }, [refreshModels]);

  const setActiveModel = useCallback(
    async (model: string): Promise<void> => {
      latestTokenRef.current += 1;
      const token = latestTokenRef.current;
      try {
        await invoke('set_active_model', { model });
      } catch (err) {
        if (isLatest(token)) {
          throw err;
        }
        return;
      }
      if (isLatest(token)) {
        setActiveModelState(model);
      }
    },
    [isLatest],
  );

  return {
    activeModel,
    availableModels,
    ollamaReachable,
    refreshModels,
    setActiveModel,
  };
}
