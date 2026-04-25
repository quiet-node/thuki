/**
 * Single-field auto-save wrapper.
 *
 * Co-locates `useDebouncedSave` with one form row: the wrapper owns the
 * local field state, fires a debounced `set_config_field` invoke on every
 * change after the seed render, and surfaces validation errors inline.
 *
 * On a successful save it lifts the loader-corrected value back via
 * `onSaved` so the parent's `RawAppConfig` snapshot stays in lock-step
 * with what is actually on disk (e.g. when the loader clamps an
 * out-of-bounds value or applies the cross-field `reader_batch_timeout`
 * correction).
 */

import { useRef, useState, type ReactNode } from 'react';

import { SettingRow } from './index';
import { useDebouncedSave } from '../hooks/useDebouncedSave';
import type { RawAppConfig } from '../types';

type Primitive = string | number | boolean | string[];

interface SaveFieldProps<TValue extends Primitive> {
  section: string;
  fieldKey: string;
  label: string;
  helper?: ReactNode;
  vertical?: boolean;
  /** Value snapshot from the parent's resolved config. */
  initialValue: TValue;
  /** Bumps when the parent reloaded from disk; resets the local form value. */
  resyncToken: number;
  /** Lifts loader-corrected values back to the parent. */
  onSaved: (next: RawAppConfig) => void;
  /** Render-prop for the actual control. */
  render: (
    value: TValue,
    setValue: (next: TValue) => void,
    errored: boolean,
  ) => ReactNode;
}

export function SaveField<TValue extends Primitive>({
  section,
  fieldKey,
  label,
  helper,
  vertical,
  initialValue,
  resyncToken,
  onSaved,
  render,
}: SaveFieldProps<TValue>) {
  const [value, setValue] = useState<TValue>(initialValue);

  const { error, resetTo } = useDebouncedSave(section, fieldKey, value, {
    onSaved,
  });

  // External reload (focus event, reset, save returning corrected value):
  // re-seed local state without scheduling a save. The token-based effect
  // detects parent-driven resyncs without triggering on the local
  // `setValue` round-trip.
  const lastTokenRef = useRef(resyncToken);
  if (lastTokenRef.current !== resyncToken) {
    lastTokenRef.current = resyncToken;
    // useState initializer + a render-phase setter is the React-recommended
    // way to react to a derived prop without an effect.
    setValue(initialValue);
    resetTo(initialValue);
  }

  return (
    <SettingRow label={label} helper={helper} vertical={vertical} error={error}>
      {render(value, setValue, error !== null)}
    </SettingRow>
  );
}
