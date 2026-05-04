/**
 * Single-field auto-save wrapper.
 *
 * Co-locates `useDebouncedSave` with one form row: the wrapper owns the
 * local field state, fires a debounced `set_config_field` invoke when
 * the value drifts away from the last persisted value, and surfaces
 * validation errors inline.
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
  /** Long-form description shown in the `?` tooltip next to the label. */
  helper?: string;
  vertical?: boolean;
  /** Tooltip placement for the `?` info button. Default `'bottom'`; use `'top'` near the bottom of the window to avoid clipping. */
  tooltipPlacement?: 'top' | 'bottom';
  /** When true, aligns the control to the far right of its container. */
  rightAlign?: boolean;
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
  tooltipPlacement,
  rightAlign,
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
  // re-seed local state without scheduling a save. The token-based check
  // detects parent-driven resyncs without triggering on the local
  // `setValue` round-trip.
  const lastTokenRef = useRef(resyncToken);
  if (lastTokenRef.current !== resyncToken) {
    lastTokenRef.current = resyncToken;
    setValue(initialValue);
    resetTo(initialValue);
  }

  return (
    <SettingRow
      label={label}
      helper={helper}
      vertical={vertical}
      tooltipPlacement={tooltipPlacement}
      rightAlign={rightAlign}
      error={error}
    >
      {render(value, setValue, error !== null)}
    </SettingRow>
  );
}
