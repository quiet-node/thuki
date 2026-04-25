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
  useMemo,
  useRef,
  useState,
} from 'react';

import styles from '../../styles/settings.module.css';
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
  children,
}: {
  label: string;
  helper?: ReactNode;
  error?: ConfigError | null;
  vertical?: boolean;
  children: ReactNode;
}) {
  const labelId = useId();
  return (
    <div
      className={vertical ? `${styles.row} ${styles.rowVertical}` : styles.row}
      role="group"
      aria-labelledby={labelId}
    >
      <label id={labelId} className={styles.rowLabel}>
        {label}
      </label>
      <div className={styles.rowControl}>
        {children}
        {helper ? <div className={styles.rowHelper}>{helper}</div> : null}
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
}: {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  maxLength?: number;
  ariaLabel?: string;
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
      rows={4}
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
  const decrement = () => {
    if (value - step >= min) onChange(value - step);
  };
  const increment = () => {
    if (value + step <= max) onChange(value + step);
  };
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

// ─── OrderedListEditor (model.available) ────────────────────────────────

export function OrderedListEditor({
  items,
  onChange,
  emptyMessage,
  addPlaceholder = 'model:tag',
}: {
  items: string[];
  onChange: (next: string[]) => void;
  emptyMessage?: string;
  addPlaceholder?: string;
}) {
  const [draft, setDraft] = useState('');
  const trimmed = useMemo(() => draft.trim(), [draft]);

  const move = (idx: number, dir: -1 | 1) => {
    const target = idx + dir;
    if (target < 0 || target >= items.length) return;
    const next = items.slice();
    [next[idx], next[target]] = [next[target], next[idx]];
    onChange(next);
  };
  const remove = (idx: number) => {
    onChange(items.filter((_, i) => i !== idx));
  };
  const add = () => {
    if (!trimmed) return;
    if (items.includes(trimmed)) return;
    onChange([...items, trimmed]);
    setDraft('');
  };

  return (
    <div className={styles.modelList}>
      {items.length === 0 && emptyMessage ? (
        <div className={styles.rowHelper}>{emptyMessage}</div>
      ) : null}
      {items.map((name, idx) => (
        <div
          key={name}
          className={`${styles.modelItem} ${idx === 0 ? styles.modelItemActive : ''}`}
        >
          {idx === 0 ? (
            <span className={styles.modelItemBadge}>Active</span>
          ) : null}
          <span className={styles.modelItemName}>{name}</span>
          <div className={styles.modelItemActions}>
            <button
              type="button"
              className={styles.iconBtn}
              aria-label={`Move ${name} up`}
              disabled={idx === 0}
              onClick={() => move(idx, -1)}
            >
              ▲
            </button>
            <button
              type="button"
              className={styles.iconBtn}
              aria-label={`Move ${name} down`}
              disabled={idx === items.length - 1}
              onClick={() => move(idx, 1)}
            >
              ▼
            </button>
            <button
              type="button"
              className={styles.iconBtn}
              aria-label={`Remove ${name}`}
              disabled={items.length === 1}
              onClick={() => remove(idx)}
            >
              ✕
            </button>
          </div>
        </div>
      ))}
      <div className={styles.modelAddRow}>
        <input
          type="text"
          className={styles.input}
          value={draft}
          placeholder={addPlaceholder}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            }
          }}
          aria-label="Add model"
        />
        <button
          type="button"
          className={styles.button}
          disabled={!trimmed || items.includes(trimmed)}
          onClick={add}
        >
          + Add
        </button>
      </div>
    </div>
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
