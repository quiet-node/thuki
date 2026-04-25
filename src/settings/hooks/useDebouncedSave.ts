/**
 * Per-field debounced auto-save hook for the Settings panel.
 *
 * Contract:
 * 1. On the FIRST render after mount, the hook does nothing (no save fired
 *    for the seed value the form was hydrated with).
 * 2. On every subsequent change to `value`, the hook schedules a single
 *    `set_config_field` invoke after `delayMs` of idle time. Subsequent
 *    changes within that window cancel the pending invoke and reschedule;
 *    only the latest value is sent.
 * 3. On unmount, any pending timer is cleared. The most-recent value is
 *    NOT flushed: tabs that need flush-before-switch (see SettingsWindow)
 *    call `flushNow()` from the returned handle.
 *
 * The hook intentionally does NOT swallow errors: callers display them
 * via the inline error pill on the failing row.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import type { ConfigError, RawAppConfig } from '../types';

export interface DebouncedSaveHandle<TValue> {
  /** True between the last user change and the next successful save. */
  pending: boolean;
  /** Most-recent error from `set_config_field`, or null on success/idle. */
  error: ConfigError | null;
  /** Forces an immediate save and resolves with the new resolved AppConfig. */
  flushNow: () => Promise<RawAppConfig | null>;
  /**
   * Replaces the in-flight value without scheduling a save. Used by the
   * focus-event reload path so external file changes do not race with a
   * pending GUI save.
   */
  resetTo: (next: TValue) => void;
}

/**
 * Schedules a debounced `set_config_field` invoke whenever `value` changes
 * after mount. The hook is value-agnostic; it serializes whatever JSON
 * primitive the caller hands in.
 *
 * The `onSaved` callback fires after each successful invoke with the
 * resolved `RawAppConfig` so callers can replace local form state with
 * the loader-corrected value (handles clamp + cross-field correction).
 */
export function useDebouncedSave<TValue>(
  section: string,
  key: string,
  value: TValue,
  options: {
    delayMs?: number;
    onSaved?: (next: RawAppConfig) => void;
  } = {},
): DebouncedSaveHandle<TValue> {
  const { delayMs = 250, onSaved } = options;
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<ConfigError | null>(null);

  // Latest value lives in a ref so flushNow() can access it without making
  // the hook's identity churn between renders.
  const valueRef = useRef(value);
  valueRef.current = value;

  // Skip the first effect run after mount so the seed value never fires
  // a save (which would be a no-op write back to disk).
  const isInitialMountRef = useRef(true);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Stable refs for callbacks so flushNow() / resetTo() remain identity-stable.
  const sectionRef = useRef(section);
  const keyRef = useRef(key);
  const onSavedRef = useRef(onSaved);
  sectionRef.current = section;
  keyRef.current = key;
  onSavedRef.current = onSaved;

  const performSave = useCallback(async (): Promise<RawAppConfig | null> => {
    setPending(true);
    try {
      const next = await invoke<RawAppConfig>('set_config_field', {
        section: sectionRef.current,
        key: keyRef.current,
        value: valueRef.current,
      });
      setError(null);
      onSavedRef.current?.(next);
      return next;
    } catch (e) {
      // Tauri serializes Result::Err variants as the inner type, which for
      // ConfigError is the tagged-enum object. Anything else is a Tauri
      // bridge failure surfaced as a string.
      setError(e as ConfigError);
      return null;
    } finally {
      setPending(false);
    }
  }, []);

  useEffect(() => {
    if (isInitialMountRef.current) {
      isInitialMountRef.current = false;
      return;
    }
    if (timerRef.current) clearTimeout(timerRef.current);
    setPending(true);
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      void performSave();
    }, delayMs);
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [value, delayMs, performSave]);

  const flushNow = useCallback(async (): Promise<RawAppConfig | null> => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    return performSave();
  }, [performSave]);

  const resetTo = useCallback((next: TValue) => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    valueRef.current = next;
    isInitialMountRef.current = true;
    setPending(false);
    setError(null);
  }, []);

  return { pending, error, flushNow, resetTo };
}
