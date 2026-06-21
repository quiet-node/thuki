/**
 * Thuki-styled model picker popover for the Providers pane.
 *
 * Replaces the native `<select>` whose open list macOS renders with its own
 * chrome (system font, blue highlight, no filter). The closed trigger reuses
 * the existing `.dropdown` box; clicking it opens a popover that matches the
 * Settings surface and the overlay's `ModelPickerPanel`: a filter input, a
 * scroll-capped list, capability pills, a RAM-fit badge, accent selection, and
 * full keyboard navigation.
 *
 * The component is purely presentational and data-driven. Each {@link
 * ModelSelectItem} carries the row's text plus optional rich fields; the
 * built-in engine fills them (capabilities, size, context, fit) while Ollama
 * leaves them undefined, so an Ollama row degrades cleanly to a slug-only line.
 * Colours come from the window-global `--cap-*` / fit tokens, so the pills read
 * identically to the Library pane without sharing a component.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { RAM_FIT_LABEL, RAM_FIT_TOOLTIP } from '../../../utils/ramFit';
import type { RamFit } from '../../../types/starter';
import styles from './ModelSelect.module.css';

/** RAM-fit verdict to its colour class on this component's stylesheet. */
const FIT_CLASS: Record<RamFit, string> = {
  fits: styles.fitOk,
  tight: styles.fitTight,
  too_big: styles.fitHeavy,
};

/** Gap in pixels between the trigger and the popover. */
const POPOVER_GAP = 6;
/** Height used to decide drop direction before the popover is measured. */
const POPOVER_ESTIMATED_HEIGHT = 320;
/** Minimum gap kept between the popover and the viewport edges. */
const POPOVER_EDGE_MARGIN = 8;

/** One selectable model. Rich fields are optional so an Ollama row, which has
 * no metadata, renders as a slug-only line. */
export interface ModelSelectItem {
  /** Stable id, passed back to {@link ModelSelectProps.onChange} on select. */
  id: string;
  /** Primary row text and the closed-trigger label (display name, or slug). */
  label: string;
  /** Metadata sub-line ("8.2 GB · 128K · Google · Q4_K_M"); omitted for Ollama. */
  sub?: string;
  /** Vision capability; present only when known. Undefined hides the pill row. */
  vision?: boolean;
  /** Thinking capability; present only when known. */
  thinking?: boolean;
  /** RAM-fit verdict; renders a trailing coloured badge when present. */
  fit?: RamFit | null;
}

export interface ModelSelectProps {
  /** Currently selected item id. */
  value: string;
  /** Items to choose from. The host renders an empty state instead of this
   * component when there are none, so the list is always non-empty here. */
  items: ModelSelectItem[];
  /** Commits a new selection. */
  onChange: (id: string) => void;
  /** Accessible name for the trigger button. */
  ariaLabel: string;
  /** Trigger text when {@link value} matches no item (e.g. "Choose a model"). */
  placeholder?: string;
}

/** Fixed-position placement for the open popover. */
export interface PopoverPlacement {
  top: number;
  left: number;
  width: number;
}

/**
 * Computes the popover's fixed-viewport position from its trigger rectangle.
 * The popover matches the trigger width and drops below it, flipping above when
 * the space below cannot hold it and there is more room above; the left edge is
 * clamped so the popover never runs off either side. Pure so every branch is
 * unit tested without a real layout (measurement is a no-op under happy-dom).
 */
export function computePlacement(
  rect: { top: number; bottom: number; left: number; width: number },
  viewportHeight: number,
  viewportWidth: number,
  estimatedHeight: number = POPOVER_ESTIMATED_HEIGHT,
  gap: number = POPOVER_GAP,
): PopoverPlacement {
  const spaceBelow = viewportHeight - rect.bottom;
  const dropUp = spaceBelow < estimatedHeight + gap && rect.top > spaceBelow;
  const top = dropUp ? rect.top - estimatedHeight - gap : rect.bottom + gap;
  const maxLeft = viewportWidth - rect.width - POPOVER_EDGE_MARGIN;
  const left = Math.max(POPOVER_EDGE_MARGIN, Math.min(rect.left, maxLeft));
  return { top, left, width: rect.width };
}

const CHEVRON = (
  <svg
    className={styles.chev}
    viewBox="0 0 10 10"
    fill="none"
    aria-hidden="true"
  >
    <path
      d="M2.5 4l2.5 2.5L7.5 4"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

const SEARCH_ICON = (
  <svg
    className={styles.searchIcon}
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
    aria-hidden="true"
  >
    <circle cx="7" cy="7" r="4.5" />
    <path d="M11 11l3 3" strokeLinecap="round" />
  </svg>
);

const LISTBOX_ID = 'thuki-model-select-listbox';

/**
 * Controlled model picker: a trigger button that toggles a filterable popover.
 * Owns its open state, filter text, keyboard focus, and outside-click
 * dismissal; selection itself is lifted to the host via `onChange`.
 */
export function ModelSelect({
  value,
  items,
  onChange,
  ariaLabel,
  placeholder = '',
}: ModelSelectProps) {
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState('');
  const [highlightedIndex, setHighlightedIndex] = useState(0);
  const [placement, setPlacement] = useState<PopoverPlacement | null>(null);

  const triggerRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const listboxRef = useRef<HTMLDivElement>(null);

  const selected = items.find((i) => i.id === value);
  const triggerLabel = selected ? selected.label : placeholder;

  const filtered = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (needle === '') return items;
    return items.filter((i) => i.label.toLowerCase().includes(needle));
  }, [filter, items]);

  // Derive the safe highlight index inline so aria-activedescendant stays
  // consistent on the same render that `filtered` shrinks under the cursor.
  const safeIndex =
    filtered.length === 0 ? 0 : Math.min(highlightedIndex, filtered.length - 1);
  const activeId =
    filtered.length > 0 ? `${LISTBOX_ID}-option-${safeIndex}` : undefined;

  const close = useCallback(() => {
    setOpen(false);
    setFilter('');
    setHighlightedIndex(0);
    setPlacement(null);
  }, []);

  // Measure the trigger and place the popover at open time, in the click
  // handler rather than an effect, so the popover never paints before it is
  // placed. The measured rect is zero-sized under happy-dom, which
  // `computePlacement` tolerates; a missing rect (only when unmounted) keeps it
  // closed.
  const openPopover = useCallback(() => {
    const rect = triggerRef.current?.getBoundingClientRect();
    if (rect) {
      setPlacement(
        computePlacement(rect, window.innerHeight, window.innerWidth),
      );
      // Pre-highlight the active row so Enter right after opening commits the
      // current model rather than the first one, matching a native <select>.
      const selectedIndex = items.findIndex((i) => i.id === value);
      setHighlightedIndex(selectedIndex >= 0 ? selectedIndex : 0);
      setOpen(true);
    }
  }, [items, value]);

  // Dismiss on a pointer press outside the trigger and popover.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (
        triggerRef.current?.contains(target) ||
        popoverRef.current?.contains(target)
      ) {
        return;
      }
      close();
    };
    // The popover is fixed-positioned from a one-time trigger measurement, so
    // scrolling the Settings body or resizing the window would detach it from
    // the trigger; dismiss it instead of leaving it floating.
    document.addEventListener('mousedown', onPointerDown);
    window.addEventListener('scroll', close, true);
    window.addEventListener('resize', close);
    return () => {
      document.removeEventListener('mousedown', onPointerDown);
      window.removeEventListener('scroll', close, true);
      window.removeEventListener('resize', close);
    };
  }, [open, close]);

  // Keep the highlighted row visible when arrow keys move it off-screen.
  useEffect(() => {
    if (!activeId) return;
    const el = listboxRef.current?.querySelector<HTMLElement>(`#${activeId}`);
    /* v8 ignore next -- scrollIntoView is a host API absent in happy-dom */
    el?.scrollIntoView?.({ block: 'nearest' });
  }, [activeId]);

  const commit = (index: number) => {
    if (index < 0 || index >= filtered.length) return;
    onChange(filtered[index].id);
    close();
    // Return focus to the trigger so keyboard flow continues from it, the way
    // a native <select> does after a selection.
    triggerRef.current?.focus();
  };

  return (
    <div className={styles.root}>
      <button
        ref={triggerRef}
        type="button"
        className={styles.trigger}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        data-open={open || undefined}
        onClick={() => (open ? close() : openPopover())}
      >
        <span className={styles.triggerLabel}>{triggerLabel}</span>
        {CHEVRON}
      </button>

      {open && placement ? (
        <div
          ref={popoverRef}
          className={styles.popover}
          style={{
            top: placement.top,
            left: placement.left,
            width: placement.width,
          }}
        >
          <div className={styles.filter}>
            {SEARCH_ICON}
            <input
              type="text"
              role="combobox"
              aria-controls={LISTBOX_ID}
              aria-expanded="true"
              aria-activedescendant={activeId}
              aria-autocomplete="list"
              aria-label={`Filter ${ariaLabel}`}
              value={filter}
              autoFocus
              spellCheck={false}
              autoComplete="off"
              placeholder="Filter models…"
              onChange={(e) => {
                setFilter(e.target.value);
                setHighlightedIndex(0);
              }}
              onKeyDown={(e) => {
                if (e.key === 'ArrowDown') {
                  e.preventDefault();
                  if (filtered.length > 0) {
                    setHighlightedIndex((i) => (i + 1) % filtered.length);
                  }
                } else if (e.key === 'ArrowUp') {
                  e.preventDefault();
                  if (filtered.length > 0) {
                    setHighlightedIndex(
                      (i) => (i - 1 + filtered.length) % filtered.length,
                    );
                  }
                } else if (e.key === 'Home') {
                  e.preventDefault();
                  if (filtered.length > 0) setHighlightedIndex(0);
                } else if (e.key === 'End') {
                  e.preventDefault();
                  if (filtered.length > 0)
                    setHighlightedIndex(filtered.length - 1);
                } else if (e.key === 'Enter') {
                  e.preventDefault();
                  commit(safeIndex);
                } else if (e.key === 'Escape') {
                  e.preventDefault();
                  close();
                  triggerRef.current?.focus();
                } else if (e.key === 'Tab') {
                  // Let focus move on to the next control, but close the
                  // popover so it cannot be left open and detached behind it.
                  close();
                }
              }}
            />
          </div>

          <div
            ref={listboxRef}
            id={LISTBOX_ID}
            role="listbox"
            aria-label={ariaLabel}
            className={styles.scroll}
          >
            {filtered.length === 0 ? (
              <p className={styles.empty}>No models found.</p>
            ) : (
              filtered.map((item, index) => {
                const isActive = item.id === value;
                const isHighlighted = index === safeIndex;
                const showPills =
                  item.vision !== undefined || item.thinking !== undefined;
                return (
                  <button
                    key={item.id}
                    id={`${LISTBOX_ID}-option-${index}`}
                    type="button"
                    role="option"
                    aria-selected={isActive}
                    tabIndex={-1}
                    className={`${styles.option} ${
                      isHighlighted ? styles.optionHighlighted : ''
                    } ${isActive ? styles.optionSelected : ''}`}
                    onMouseEnter={() => setHighlightedIndex(index)}
                    onClick={() => commit(index)}
                  >
                    <span className={styles.optionBody}>
                      <span className={styles.optionName}>
                        <span
                          className={styles.optionNameText}
                          title={item.label}
                        >
                          {item.label}
                        </span>
                        {showPills ? (
                          <>
                            <span
                              className={`${styles.pill} ${styles.pillText}`}
                            >
                              Text
                            </span>
                            {item.vision ? (
                              <span
                                className={`${styles.pill} ${styles.pillVision}`}
                              >
                                Vision
                              </span>
                            ) : null}
                            {item.thinking ? (
                              <span
                                className={`${styles.pill} ${styles.pillThinking}`}
                              >
                                Thinking
                              </span>
                            ) : null}
                          </>
                        ) : null}
                      </span>
                      {item.sub ? (
                        <span className={styles.optionSub} title={item.sub}>
                          {item.sub}
                        </span>
                      ) : null}
                    </span>
                    {item.fit ? (
                      // Native title (not the Tooltip component): the row is a
                      // <button>, so a Tooltip's wrapper <div> would be invalid
                      // phrasing content nested inside it.
                      <span
                        className={`${styles.fit} ${FIT_CLASS[item.fit]}`}
                        title={RAM_FIT_TOOLTIP[item.fit]}
                      >
                        {RAM_FIT_LABEL[item.fit]}
                      </span>
                    ) : null}
                    <svg
                      className={`${styles.check} ${
                        isActive ? '' : styles.checkHidden
                      }`}
                      viewBox="0 0 16 16"
                      fill="none"
                      aria-hidden="true"
                    >
                      <path
                        d="M3 8l3.5 3.5L13 5"
                        stroke="currentColor"
                        strokeWidth="2.2"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      />
                    </svg>
                  </button>
                );
              })
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
}
