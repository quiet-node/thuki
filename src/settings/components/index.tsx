/**
 * Reusable form primitives for the Settings panel.
 *
 * Co-located in one file because each component is small and they all
 * share the same CSS module. Splitting them across N files would create
 * import noise without improving maintainability.
 */

import {
  type ChangeEvent,
  type ReactNode,
  useEffect,
  useId,
  useRef,
  useState,
} from 'react';

import styles from '../../styles/settings.module.css';
import { Tooltip } from '../../components/Tooltip';
import { describeConfigError } from '../types';
import type { ConfigError } from '../types';

// ─── Section + Row layout ────────────────────────────────────────────────

export function Section({
  heading,
  children,
}: {
  heading: string;
  children: ReactNode;
}) {
  return (
    <section className={styles.section}>
      <div className={styles.sectionHeading}>{heading}</div>
      {children}
    </section>
  );
}

export function SettingRow({
  label,
  helper,
  error,
  vertical = false,
  tooltipPlacement = 'bottom',
  rightAlign = false,
  children,
}: {
  label: string;
  /** Long-form description rendered in a `?` tooltip next to the label. */
  helper?: string;
  error?: ConfigError | null;
  vertical?: boolean;
  /** Tooltip placement for the `?` info button. Default `'bottom'`; use `'top'` near the bottom of the window to avoid clipping. */
  tooltipPlacement?: 'top' | 'bottom';
  /** When true, aligns the control to the far right of its container. */
  rightAlign?: boolean;
  children: ReactNode;
}) {
  const labelId = useId();
  return (
    <div
      className={vertical ? `${styles.row} ${styles.rowVertical}` : styles.row}
      role="group"
      aria-labelledby={labelId}
    >
      <div className={styles.rowLabelGroup}>
        <label id={labelId} className={styles.rowLabel}>
          {label}
        </label>
        {helper ? (
          <Tooltip label={helper} multiline placement={tooltipPlacement}>
            <button
              type="button"
              className={styles.infoBtn}
              aria-label={`About ${label}`}
            >
              ?
            </button>
          </Tooltip>
        ) : null}
      </div>
      <div
        className={
          rightAlign
            ? `${styles.rowControl} ${styles.rowControlRight}`
            : styles.rowControl
        }
      >
        {children}
        {error ? (
          <div className={styles.rowError} role="alert">
            {describeConfigError(error)}
          </div>
        ) : null}
      </div>
    </div>
  );
}

// ─── Inputs ──────────────────────────────────────────────────────────────

export function TextField({
  value,
  onChange,
  placeholder,
  errored,
  ariaLabel,
}: {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  errored?: boolean;
  ariaLabel?: string;
}) {
  return (
    <input
      type="text"
      className={`${styles.input} ${errored ? styles.inputError : ''}`}
      value={value}
      onChange={(e: ChangeEvent<HTMLInputElement>) => onChange(e.target.value)}
      placeholder={placeholder}
      aria-label={ariaLabel}
      spellCheck={false}
      autoComplete="off"
      autoCorrect="off"
      autoCapitalize="off"
    />
  );
}

export function Textarea({
  value,
  onChange,
  placeholder,
  maxLength,
  ariaLabel,
  rows = 4,
}: {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  maxLength?: number;
  ariaLabel?: string;
  rows?: number;
}) {
  return (
    <textarea
      className={styles.textarea}
      value={value}
      onChange={(e: ChangeEvent<HTMLTextAreaElement>) =>
        onChange(e.target.value)
      }
      placeholder={placeholder}
      maxLength={maxLength}
      aria-label={ariaLabel}
      rows={rows}
      spellCheck={false}
    />
  );
}

// ─── Slider (NumberSlider) ──────────────────────────────────────────────

export function NumberSlider({
  value,
  min,
  max,
  step = 1,
  unit,
  onChange,
  ariaLabel,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  onChange: (next: number) => void;
  ariaLabel?: string;
}) {
  // Track local value during a continuous drag so the displayed value
  // updates per pixel, but only fire onChange on commit (mouse-up / blur).
  // Otherwise every intermediate frame triggers a debounced save (which
  // collapses to one anyway, but the UI thread does a lot of useless work).
  const [local, setLocal] = useState(value);
  const draggingRef = useRef(false);
  useEffect(() => {
    // Sync external value into local state only when the user is not
    // actively dragging; otherwise the prop update would clobber the
    // in-progress drag position.
    if (!draggingRef.current) setLocal(value);
  }, [value]);

  return (
    <div className={styles.sliderRow}>
      <input
        type="range"
        className={styles.sliderInput}
        min={min}
        max={max}
        step={step}
        value={local}
        aria-label={ariaLabel}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={local}
        aria-valuetext={unit ? `${local} ${unit}` : `${local}`}
        onChange={(e: ChangeEvent<HTMLInputElement>) => {
          draggingRef.current = true;
          setLocal(Number(e.target.value));
        }}
        onMouseUp={() => {
          draggingRef.current = false;
          if (local !== value) onChange(local);
        }}
        onTouchEnd={() => {
          draggingRef.current = false;
          if (local !== value) onChange(local);
        }}
        onBlur={() => {
          draggingRef.current = false;
          if (local !== value) onChange(local);
        }}
        onKeyUp={() => {
          if (local !== value) onChange(local);
        }}
      />
      <div className={styles.valChip} aria-hidden>
        {unit ? `${local} ${unit}` : local}
      </div>
    </div>
  );
}

// ─── Stepper (NumberStepper) ────────────────────────────────────────────

export function NumberStepper({
  value,
  min,
  max,
  step = 1,
  onChange,
  ariaLabel,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (next: number) => void;
  ariaLabel?: string;
}) {
  // The buttons are disabled at the bounds (see `disabled` props below) so
  // these handlers cannot be invoked when the next value would breach them;
  // no runtime guard is needed.
  const decrement = () => onChange(value - step);
  const increment = () => onChange(value + step);
  return (
    <div
      className={styles.stepper}
      role="spinbutton"
      aria-label={ariaLabel}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
    >
      <button
        type="button"
        className={styles.stepperBtn}
        onClick={decrement}
        disabled={value - step < min}
        aria-label="Decrease"
      >
        −
      </button>
      <div className={styles.stepperValue}>{value}</div>
      <button
        type="button"
        className={styles.stepperBtn}
        onClick={increment}
        disabled={value + step > max}
        aria-label="Increase"
      >
        +
      </button>
    </div>
  );
}

// ─── Dropdown (single-select, controlled) ───────────────────────────────

export function Dropdown<T extends string>({
  value,
  options,
  onChange,
  ariaLabel,
}: {
  value: T;
  options: readonly T[];
  onChange: (next: T) => void;
  ariaLabel?: string;
}) {
  return (
    <select
      className={styles.dropdown}
      value={value}
      aria-label={ariaLabel}
      onChange={(e: ChangeEvent<HTMLSelectElement>) =>
        onChange(e.target.value as T)
      }
    >
      {options.map((opt) => (
        <option key={opt} value={opt}>
          {opt}
        </option>
      ))}
    </select>
  );
}

// ─── Toggle switch ───────────────────────────────────────────────────────

export function Toggle({
  checked,
  onChange,
  ariaLabel,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  ariaLabel?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      className={`${styles.toggle} ${checked ? styles.toggleOn : ''}`}
      onClick={() => onChange(!checked)}
    >
      <span className={styles.toggleThumb} />
    </button>
  );
}

// ─── SavedPill ──────────────────────────────────────────────────────────

export function SavedPill({ visible }: { visible: boolean }) {
  return (
    <div
      className={`${styles.savedPill} ${visible ? styles.savedPillVisible : ''}`}
      aria-live="polite"
      role="status"
    >
      ✓ Saved
    </div>
  );
}

// ─── Confirm dialog ─────────────────────────────────────────────────────

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  cancelLabel = 'Cancel',
  destructive = false,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel?: string;
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onCancel();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [open, onCancel]);

  if (!open) return null;
  return (
    <div
      className={styles.dialogBackdrop}
      role="dialog"
      aria-modal="true"
      aria-labelledby="dialog-title"
    >
      <div className={styles.dialog}>
        <h2 id="dialog-title" className={styles.dialogTitle}>
          {title}
        </h2>
        <p className={styles.dialogMessage}>{message}</p>
        <div className={styles.dialogActions}>
          <button
            type="button"
            className={`${styles.button} ${styles.buttonGhost}`}
            onClick={onCancel}
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            className={`${styles.button} ${destructive ? styles.buttonDestructive : ''}`}
            onClick={onConfirm}
            autoFocus
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── ResetSection link ──────────────────────────────────────────────────

export function ResetSectionLink({
  label,
  onClick,
}: {
  label: string;
  onClick: () => void;
}) {
  return (
    <button type="button" className={styles.resetLink} onClick={onClick}>
      ↻ {label}
    </button>
  );
}
