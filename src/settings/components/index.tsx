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
  helper,
  children,
}: {
  heading: string;
  /** Optional description rendered in a `?` tooltip next to the heading, for
   * explaining what the whole section is about (scope), distinct from the
   * per-row helpers that explain individual settings. */
  helper?: string;
  children: ReactNode;
}) {
  return (
    <section className={styles.section}>
      <div className={styles.sectionHeading}>
        {heading}
        {helper ? (
          <Tooltip label={helper} multiline>
            <button
              type="button"
              className={styles.infoBtn}
              aria-label={`About ${heading}`}
            >
              ?
            </button>
          </Tooltip>
        ) : null}
      </div>
      {children}
    </section>
  );
}

export function SettingRow({
  label,
  labelAccessory,
  helper,
  error,
  vertical = false,
  tooltipPlacement = 'bottom',
  rightAlign = false,
  children,
}: {
  label: string;
  /**
   * Optional decoration rendered after the label text inside the label
   * element (e.g. animated underline). Not part of the accessible name.
   */
  labelAccessory?: ReactNode;
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
          <span className={styles.rowLabelText}>
            {label}
            {labelAccessory}
          </span>
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
  formatValue,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  onChange: (next: number) => void;
  ariaLabel?: string;
  /**
   * Optional custom formatter for the value chip and the `aria-valuetext`.
   * Wins over `unit`. Use when an enum-like slider (e.g. font-weight at
   * 400/500/600/700) should surface descriptive labels ("Regular", "Medium")
   * instead of the raw number. Returning the empty string is allowed and
   * blanks the chip.
   */
  formatValue?: (n: number) => string;
}) {
  // Track local value during a continuous drag so the displayed value
  // updates per pixel, but only fire onChange on commit (mouse-up / blur).
  // Otherwise every intermediate frame triggers a debounced save (which
  // collapses to one anyway, but the UI thread does a lot of useless work).
  //
  // `localRef` mirrors `local` synchronously inside `onChange`. Reading from
  // the ref (not the closure-captured `local`) lets the commit handlers see
  // the latest value even when both `onChange` and `onMouseUp`/`onKeyUp`
  // fire in the same React event tick (the common case for single-click
  // track jumps and single-press keyboard nudges). Without the ref the
  // commit handler would compare the *previous* render's `local` to `value`
  // and silently skip the save when both are equal.
  const [local, setLocal] = useState(value);
  const localRef = useRef(value);
  const draggingRef = useRef(false);
  useEffect(() => {
    // Sync external value into local state only when the user is not
    // actively dragging; otherwise the prop update would clobber the
    // in-progress drag position.
    if (!draggingRef.current) {
      setLocal(value);
      localRef.current = value;
    }
  }, [value]);

  const commit = () => {
    draggingRef.current = false;
    const next = localRef.current;
    if (next !== value) onChange(next);
  };

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
        aria-valuetext={
          formatValue
            ? formatValue(local)
            : unit
              ? `${local} ${unit}`
              : `${local}`
        }
        onChange={(e: ChangeEvent<HTMLInputElement>) => {
          const next = Number(e.target.value);
          draggingRef.current = true;
          localRef.current = next;
          setLocal(next);
        }}
        onMouseUp={commit}
        onTouchEnd={commit}
        onBlur={commit}
        onKeyUp={commit}
      />
      <div className={styles.valChip} aria-hidden>
        {formatValue ? formatValue(local) : unit ? `${local} ${unit}` : local}
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

// ─── Reduced-motion helper ──────────────────────────────────────────────

/**
 * Whether the OS is set to reduce motion. Callers use it to skip non-essential
 * animations (dialog exit fade, success ticks) and jump straight to the settled
 * state.
 *
 * @returns `true` when `prefers-reduced-motion: reduce` matches, else `false`
 *   (including when `matchMedia` is unavailable).
 */
export function prefersReducedMotion(): boolean {
  return (
    window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false
  );
}

// ─── Confirm dialog ─────────────────────────────────────────────────────

/**
 * Exit-animation duration for `ConfirmDialog`, in milliseconds. When `open`
 * flips to false the dialog stays mounted this long so the leave animation
 * (`dialogBackdropOut` / `dialogPanelOut` in `settings.module.css`) can play,
 * then it unmounts. Mirrors the entrance timing. Exported so tests advance
 * fake timers by the exact value rather than a guessed one.
 */
export const DIALOG_EXIT_MS = 160;

/**
 * Modal confirm used for irreversible or high-impact Settings actions.
 *
 * Focus policy: non-destructive dialogs autofocus the confirm button (Enter
 * affirms). Destructive dialogs autofocus Cancel instead so a held/repeated
 * Enter from the field that opened the dialog cannot activate the destructive
 * action (common form+dialog race).
 *
 * @param open Whether the dialog is open (exit animation defers unmount).
 * @param title Dialog title (`aria-labelledby` target).
 * @param message Body copy explaining the consequence.
 * @param confirmLabel Label for the affirmative button.
 * @param cancelLabel Label for the dismiss button. Default `Cancel`.
 * @param destructive When true, styles confirm as destructive and autofocuses Cancel.
 * @param primary Accent-fill confirm when not destructive.
 * @param onConfirm Affirmative action callback.
 * @param onCancel Dismiss callback (also Escape).
 */
export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  cancelLabel = 'Cancel',
  destructive = false,
  primary = false,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel?: string;
  destructive?: boolean;
  /** Accent-fill the confirm button (the affirmative primary action). Ignored
   * when `destructive` is set, which takes visual precedence. */
  primary?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  // Deferred-unmount lifecycle so a close reads as smooth as the entrance:
  // `rendered` gates the DOM, the derived `closing` drives the exit CSS. State
  // that reacts to an `open` change is adjusted during render (React's
  // recommended pattern over an effect): re-opening shows immediately, and
  // reduced motion unmounts at once with no partial fade. The exit timer is the
  // only side effect, and it clears itself in its own cleanup, which also
  // cancels the pending unmount when the dialog is re-opened mid-exit.
  const [rendered, setRendered] = useState(open);
  const [prevOpen, setPrevOpen] = useState(open);
  if (open !== prevOpen) {
    setPrevOpen(open);
    if (open) {
      setRendered(true);
    } else if (rendered && prefersReducedMotion()) {
      setRendered(false);
    }
  }
  const closing = rendered && !open;

  useEffect(() => {
    if (!closing) return;
    const id = window.setTimeout(() => setRendered(false), DIALOG_EXIT_MS);
    return () => window.clearTimeout(id);
  }, [closing]);

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

  if (!rendered) return null;
  return (
    <div
      className={styles.dialogBackdrop}
      data-closing={closing ? 'true' : undefined}
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
            // Destructive: land focus on Cancel so Enter cannot chain-confirm.
            autoFocus={destructive}
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            className={`${styles.button} ${
              destructive
                ? styles.buttonDestructive
                : primary
                  ? styles.buttonPrimary
                  : ''
            }`}
            onClick={onConfirm}
            // Non-destructive only: Enter affirms. Destructive never autofocuses
            // the wipe/ack button (see component JSDoc focus policy).
            autoFocus={!destructive}
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
