import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ModelCapabilities, ModelCapabilitiesMap } from '../types/model';

/**
 * Runtime guard that the IPC payload is a `{ [name]: Capabilities }` map.
 * Mirrors the defensive shape check in `useModelSelection` so the hook
 * stays robust against a backend / mock that returns the wrong shape.
 */
function isCapabilities(value: unknown): value is ModelCapabilities {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.vision === 'boolean' &&
    typeof candidate.thinking === 'boolean'
  );
}

function isCapabilitiesMap(value: unknown): value is ModelCapabilitiesMap {
  if (typeof value !== 'object' || value === null) return false;
  return Object.values(value).every(isCapabilities);
}

/** Shape returned by {@link useModelCapabilities}. */
export interface UseModelCapabilitiesResult {
  /**
   * Map of model slug to its capability flags. Empty until the first
   * fetch resolves or if the backend rejects.
   */
  capabilities: ModelCapabilitiesMap;
  /**
   * Re-fetches the capabilities map. Callers are the single trigger:
   * the hook fetches once on mount and never auto-retries.
   */
  refresh: () => Promise<void>;
}

/**
 * React hook that pulls the per-model capability map from the Rust
 * `get_model_capabilities` Tauri command. Used by the picker to render
 * capability labels and by the submit gate to refuse messages whose
 * attached content does not match the active model's capabilities.
 *
 * The same monotonic-token pattern as `useModelSelection` keeps rapid
 * out-of-order responses from overwriting newer state and drops
 * resolutions that fire after unmount.
 */
export function useModelCapabilities(): UseModelCapabilitiesResult {
  const [capabilities, setCapabilities] = useState<ModelCapabilitiesMap>({});
  const mountedRef = useRef(true);
  const latestTokenRef = useRef(0);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const isLatest = useCallback(
    (token: number): boolean =>
      mountedRef.current && token === latestTokenRef.current,
    [],
  );

  const refresh = useCallback(async (): Promise<void> => {
    latestTokenRef.current += 1;
    const token = latestTokenRef.current;
    try {
      const payload = await invoke<unknown>('get_model_capabilities');
      if (!isLatest(token)) return;
      if (!isCapabilitiesMap(payload)) {
        setCapabilities({});
        return;
      }
      setCapabilities(payload);
    } catch {
      if (!isLatest(token)) return;
      setCapabilities({});
    }
  }, [isLatest]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { capabilities, refresh };
}
