import { useCallback, useEffect, useState } from 'react';
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
   * Re-fetch the model picker state from the backend. On failure the
   * available models list is cleared to avoid showing stale entries.
   */
  refreshModels: () => Promise<void>;
  /**
   * Persist a new active model through the backend and sync local state
   * after the backend acknowledges the change.
   */
  setActiveModel: (model: string) => Promise<void>;
}

/**
 * React hook that manages the active Ollama model selection. Loads the
 * current model + the installed model list from the Rust backend on mount,
 * and exposes imperative helpers for refresh and selection.
 *
 * Callers are expected to invoke `refreshModels` when they need to pick up
 * external state changes (e.g. after a model install completes). The hook
 * does not poll or auto-refresh.
 */
export function useModelSelection(): UseModelSelectionResult {
  // The state setter is intentionally renamed because `setActiveModel` is the
  // public async callback returned by this hook.
  // eslint-disable-next-line @eslint-react/use-state
  const [activeModel, setActiveModelState] = useState('');
  const [availableModels, setAvailableModels] = useState<string[]>([]);

  const refreshModels = useCallback(async (): Promise<void> => {
    try {
      const state = await invoke<ModelPickerState>('get_model_picker_state');
      if (!isModelPickerState(state)) {
        setAvailableModels([]);
        return;
      }
      setActiveModelState(state.active);
      setAvailableModels(state.all);
    } catch {
      setAvailableModels([]);
    }
  }, []);

  useEffect(() => {
    void refreshModels();
  }, [refreshModels]);

  const setActiveModel = useCallback(async (model: string): Promise<void> => {
    await invoke('set_active_model', { model });
    setActiveModelState(model);
  }, []);

  return { activeModel, availableModels, refreshModels, setActiveModel };
}
