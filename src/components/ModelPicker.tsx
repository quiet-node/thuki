import { AnimatePresence, motion } from 'framer-motion';
import type { RefObject } from 'react';

/**
 * Hoisted static SVG - chip-style trigger icon for the model picker.
 * Redrawn to occupy ~88% of the 16x16 canvas so visual weight matches
 * the adjacent camera and send icons in the ask bar.
 * @see Vercel React Best Practices - Hoist Static JSX Elements
 */
const CHIP_ICON = (
  <svg
    width="14"
    height="14"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect
      x="3"
      y="3"
      width="10"
      height="10"
      rx="1.5"
      stroke="currentColor"
      strokeWidth="1.5"
    />
    <path
      d="M5 1V3M8 1V3M11 1V3M5 13V15M8 13V15M11 13V15M1 5H3M1 8H3M1 11H3M13 5H15M13 8H15M13 11H15"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    />
  </svg>
);

/** Props for the {@link ModelPickerTrigger} component. */
export interface ModelPickerTriggerProps {
  /** Ref forwarded to the underlying button so callers can manage focus / outside-click. */
  triggerRef?: RefObject<HTMLButtonElement | null>;
  /** True while the associated popup is visible - drives `aria-expanded`. */
  isOpen: boolean;
  /** When true the trigger is inert (e.g. during generation). */
  disabled: boolean;
  /** Fires on click to toggle the popup open/closed. */
  onToggle: () => void;
}

/**
 * Right-side chip button that opens the model picker popup.
 *
 * The popup itself lives in {@link ModelPickerList} and is rendered in the
 * ask bar's upper DOM-flow slot so the morphing container can grow the
 * native window to reveal it without being clipped by `overflow-hidden`.
 */
export function ModelPickerTrigger({
  triggerRef,
  isOpen,
  disabled,
  onToggle,
}: ModelPickerTriggerProps) {
  return (
    <button
      ref={triggerRef}
      type="button"
      aria-label="Choose model"
      aria-expanded={isOpen}
      disabled={disabled}
      onClick={onToggle}
      className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-text-primary hover:bg-white/8 transition-colors duration-150 disabled:opacity-40 disabled:cursor-default cursor-pointer"
    >
      {CHIP_ICON}
    </button>
  );
}

/** Props for the {@link ModelPickerList} component. */
export interface ModelPickerListProps {
  /** Ref forwarded to the outer list container for outside-click detection. */
  listRef?: RefObject<HTMLDivElement | null>;
  /** Currently active model slug; highlighted in the popup. */
  activeModel: string;
  /** Full list of available model slugs from Ollama's tags endpoint. */
  models: string[];
  /** True when the list should be visible. */
  isOpen: boolean;
  /** Called with the chosen slug when the user picks a row. */
  onSelect: (model: string) => void;
}

/**
 * Animated popup rendered inline above the ask bar input row.
 *
 * Uses a height animation inside `AnimatePresence` so the morphing
 * container's `ResizeObserver` can smoothly grow the native window as
 * the list mounts. Renders nothing when `isOpen` is false or the
 * `models` list is empty.
 */
export function ModelPickerList({
  listRef,
  activeModel,
  models,
  isOpen,
  onSelect,
}: ModelPickerListProps) {
  return (
    <AnimatePresence>
      {isOpen && models.length > 0 && (
        <motion.div
          ref={listRef}
          key="model-picker-list"
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: 'auto', opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={{
            height: { duration: 0.2, ease: [0.16, 1, 0.3, 1] },
            opacity: { duration: 0.15 },
          }}
          style={{ overflow: 'hidden' }}
        >
          <div className="flex justify-end px-3 pt-2 pb-1">
            <div className="w-56 overflow-hidden rounded-xl border border-surface-border bg-surface-base shadow-chat backdrop-blur-2xl">
              {models.map((model) => (
                <button
                  key={model}
                  type="button"
                  aria-label={model}
                  aria-current={model === activeModel ? 'true' : undefined}
                  onClick={() => onSelect(model)}
                  className={`block w-full truncate px-4 py-3 text-left text-sm cursor-pointer ${
                    model === activeModel
                      ? 'bg-primary/10 text-text-primary'
                      : 'text-text-primary hover:bg-white/6'
                  }`}
                >
                  {model}
                </button>
              ))}
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
