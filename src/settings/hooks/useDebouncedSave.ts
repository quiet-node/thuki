/**
 * Per-field debounced auto-save hook for the Settings panel.
 *
 * Contract:
 * 1. The hook never saves a value that is equal to the last known
 *    persisted value. The seed value passed at first render counts as
 *    the initial "last saved" value, so no save fires for the hydrated
 *    seed (and React StrictMode's double-effect cannot trick the hook
 *    into firing one either, since both runs see value == lastSaved).
 * 2. On every change to `value` that differs from `lastSaved`, the hook
 *    schedules a single `set_config_field` invoke after `delayMs` of
 *    idle time. Subsequent changes within that window cancel the pending
 *    invoke and reschedule; only the latest value is sent.
 * 3. On a successful save, `lastSaved` is updated to the value that was
 *    sent so subsequent identical changes are no-ops.
 * 4. On unmount, any pending debounced change is flushed synchronously
 *    via `performSave`. This protects the user's last keystroke when the
 *    Settings tab is switched mid-debounce. The post-await callbacks
 *    are gated by an epoch counter so a stale invoke that resolves
 *    after `resetTo` (focus resync) cannot clobber the new baseline.
 *
 * The hook intentionally does NOT swallow errors: callers display them
 * via the inline error pill on the failing row.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import type { ConfigError, RawAppConfig } from '../types';

export interface DebouncedSaveHandle<TValue> {
  /** Most-recent error from `set_config_field`, or null on success/idle. */
  error: ConfigError | null;
  /** Forces an immediate save and resolves with the new resolved AppConfig. */
  flushNow: () => Promise<RawAppConfig | null>;
  /**
   * Replaces the in-flight value without scheduling a save and invalidates
   * any save that is already mid-await. Used by the resync path so external
   * file changes do not race with a pending GUI save and so the next
   * user-driven change is treated as a delta from the new baseline rather
   * than a delta from the original seed.
   */
  resetTo: (next: TValue) => void;
}

/**
 * Schedules a debounced `set_config_field` invoke whenever `value` differs
 * from the last known saved value. The hook is value-agnostic across the
 * JSON primitives that the Settings panel can write (string, number,
 * boolean, string[]); the equality check uses `Object.is` for scalars and
 * an element-wise `Object.is` walk for the only array field
 * (`model.available: string[]`).
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
  const [error, setError] = useState<ConfigError | null>(null);

  // Latest value lives in a ref so flushNow() can access it without making
  // the hook's identity churn between renders.
  const valueRef = useRef(value);
  valueRef.current = value;

  // The last value we successfully wrote (or the seed value before any
  // write). Comparing against this ref makes the save trigger idempotent
  // under React StrictMode's double-effect and prevents redundant
  // round-trips for no-op changes.
  const lastSavedRef = useRef<TValue>(value);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Epoch counter bumped on every `resetTo`. A `performSave` invocation
  // that started before the bump is considered stale on resolve and is
  // discarded so it cannot overwrite `lastSavedRef` with a pre-resync
  // value or fire `onSaved` with a config that no longer matches the
  // window's baseline.
  const epochRef = useRef(0);

  // Tracks mount lifetime so post-`await` `setError` calls do not fire on
  // an unmounted component (silences React's "set state on unmounted"
  // warning and avoids a tiny leak path).
  const isMountedRef = useRef(true);

  // Stable refs for callbacks so flushNow() / resetTo() remain identity-stable.
  const sectionRef = useRef(section);
  const keyRef = useRef(key);
  const onSavedRef = useRef(onSaved);
  sectionRef.current = section;
  keyRef.current = key;
  onSavedRef.current = onSaved;

  const performSave = useCallback(async (): Promise<RawAppConfig | null> => {
    const myEpoch = epochRef.current;
    const sentValue = valueRef.current;
    try {
      const next = await invoke<RawAppConfig>('set_config_field', {
        section: sectionRef.current,
        key: keyRef.current,
        value: sentValue,
      });
      if (epochRef.current !== myEpoch) return null;
      lastSavedRef.current = sentValue;
      if (isMountedRef.current) setError(null);
      onSavedRef.current?.(next);
      return next;
    } catch (e) {
      if (epochRef.current !== myEpoch) return null;
      // Tauri serializes Result::Err variants as the inner type, which for
      // ConfigError is the tagged-enum object. Anything else is a Tauri
      // bridge failure surfaced as a string.
      if (isMountedRef.current) setError(e as ConfigError);
      return null;
    }
  }, []);

  // Stable handle to performSave used by the unmount-flush cleanup so the
  // cleanup can run with empty deps without lint complaining.
  const performSaveRef = useRef(performSave);
  performSaveRef.current = performSave;

  useEffect(() => {
    if (areEqual(value, lastSavedRef.current)) return;
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      void performSave();
    }, delayMs);
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [value, delayMs, performSave]);

  // Mount-lifetime + final-flush effect. Runs cleanup only at unmount so
  // any debounced change still in the timer is forwarded to the backend
  // before the row disappears (e.g. when the user switches tabs mid-edit).
  useEffect(() => {
    return () => {
      isMountedRef.current = false;
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
        void performSaveRef.current();
      }
    };
  }, []);

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
    // Bump epoch so any in-flight performSave's post-await branch is
    // discarded instead of overwriting the new baseline.
    epochRef.current += 1;
    valueRef.current = next;
    lastSavedRef.current = next;
    if (isMountedRef.current) setError(null);
  }, []);

  return { error, flushNow, resetTo };
}

/**
 * Structural equality for the JSON primitives the Settings panel can
 * write: scalars compare via `Object.is`; arrays compare element-wise
 * with `Object.is` (sufficient because the only array field is the
 * model list, which is `string[]`). Any future array-of-object field
 * will need a deeper compare here; the current narrow shape is
 * intentional to keep the hot path cheap.
 */
function areEqual<T>(a: T, b: T): boolean {
  if (Object.is(a, b)) return true;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i += 1) {
      if (!Object.is(a[i], b[i])) return false;
    }
    return true;
  }
  return false;
}
