import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ModelPickerState } from '../types/model';

/**
 * Runtime guard for the IPC boundary. The Rust backend is trusted, but this
 * keeps the hook robust against shape drift (schema changes, legacy builds,
 * mocks) without pulling in a schema library.
 */
function isModelPickerState(value: unknown): value is ModelPickerState {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as { active?: unknown; all?: unknown };
  return (
    typeof candidate.active === 'string' &&
    Array.isArray(candidate.all) &&
    candidate.all.every((entry) => typeof entry === 'string')
  );
}

/**
 * Shape returned by {@link useModelSelection}.
 */
export interface UseModelSelectionResult {
  /** The currently active Ollama model name. Empty string until loaded. */
  activeModel: string;
  /** All locally installed Ollama model names available for selection. */
  availableModels: string[];
  /**
   * Re-fetch the model picker state from the backend. Clears both
   * `activeModel` and `availableModels` when the backend returns a malformed
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
  const [activeModel, setActiveModelState] = useState('');
  const [availableModels, setAvailableModels] = useState<string[]>([]);

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
        setActiveModelState('');
        setAvailableModels([]);
        return;
      }
      setActiveModelState(state.active);
      setAvailableModels(state.all);
    } catch {
      if (!isLatest(token)) return;
      setActiveModelState('');
      setAvailableModels([]);
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

  return { activeModel, availableModels, refreshModels, setActiveModel };
}
