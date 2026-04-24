import { AnimatePresence, motion } from 'framer-motion';
import type { RefObject } from 'react';

/**
 * Model picker is split into two exports so that the opening/closing state and
 * outside-click lifecycle live in {@link AskBarView}, which also owns the Tauri
 * window sizing via a morphing-container ResizeObserver. The list renders
 * **inline in the DOM flow** (not via a portal) so the ResizeObserver detects
 * the added height and grows the native window upward to reveal the menu.
 *
 * - {@link ModelPickerTrigger} - stateless chip button; wrapped in a `Tooltip`
 *   at the call site rather than internally.
 * - {@link ModelPickerList} - animated, full-width inline list shown above the
 *   ask bar input row, following the same pattern as `CommandSuggestion`.
 *
 * An earlier implementation rendered the menu via `createPortal(document.body)`
 * to escape the ask bar's `overflow-hidden` chrome, but the portal was still
 * bounded by the Tauri web view which is only ~80px tall in ask-bar mode, so
 * menus of 50-160px clipped. DOM-flow rendering grows the window naturally.
 */

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

/** Props for {@link ModelPickerTrigger}. */
export interface ModelPickerTriggerProps {
  /** Ref forwarded to the trigger button for outside-click discrimination. */
  triggerRef?: RefObject<HTMLButtonElement | null>;
  /** Whether the associated list is currently expanded. Drives `aria-expanded`. */
  isOpen: boolean;
  /** True while generation is active or another busy state gates the picker. */
  disabled: boolean;
  /** Called when the user toggles the menu open or closed via the chip. */
  onToggle: () => void;
}

/**
 * Chip-style button that toggles the model picker list. Stateless: the
 * parent owns `isOpen` so it can coordinate with the outside-click listener
 * and the inline {@link ModelPickerList} that renders above the input row.
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
      aria-haspopup="menu"
      disabled={disabled}
      onClick={onToggle}
      className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-text-primary hover:bg-white/8 transition-colors duration-150 disabled:opacity-40 disabled:cursor-default cursor-pointer outline-none"
    >
      {CHIP_ICON}
    </button>
  );
}

/** Props for {@link ModelPickerList}. */
export interface ModelPickerListProps {
  /** Ref forwarded to the list wrapper for outside-click discrimination. */
  listRef?: RefObject<HTMLDivElement | null>;
  /** Currently active model slug; the matching row renders an orange check. */
  activeModel: string;
  /** Full list of available model slugs from Ollama's tags endpoint. */
  models: string[];
  /** When true the list animates in; when false it animates out. */
  isOpen: boolean;
  /** Called with the chosen slug when the user clicks a row. */
  onSelect: (model: string) => void;
}

/**
 * Animated full-width list rendered above the ask bar input row. Sits inside
 * the morphing container (not a portal), so the existing ResizeObserver picks
 * up the added height and grows the Tauri window upward to reveal it.
 *
 * Visual layout:
 * - Outer wrapper is full-width with no right alignment so the window grows
 *   cleanly and there is no blank "void" on the left side.
 * - Inner `px-3 pt-2 pb-1` padding matches the ask bar's horizontal chrome.
 * - The card (`rounded-xl border bg-surface-elevated/40`) fills the full
 *   width between the padding, reading as a slight elevation on the main
 *   surface-base background.
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
          role="menu"
        >
          <div className="px-3 pt-2 pb-1">
            <div className="rounded-xl border border-surface-border bg-surface-elevated/40 overflow-hidden">
              {models.map((model) => (
                <button
                  key={model}
                  type="button"
                  role="menuitem"
                  aria-label={model}
                  aria-current={model === activeModel ? 'true' : undefined}
                  onClick={() => onSelect(model)}
                  className="flex items-center justify-between gap-3 w-full px-4 py-2.5 text-left text-sm text-text-primary hover:bg-white/5 transition-colors duration-120 cursor-pointer"
                >
                  <span className="flex-1 min-w-0 overflow-hidden text-ellipsis">
                    {model}
                  </span>
                  <svg
                    className="w-3.5 h-3.5 shrink-0 text-primary"
                    style={{ opacity: model === activeModel ? 1 : 0 }}
                    viewBox="0 0 16 16"
                    fill="none"
                    xmlns="http://www.w3.org/2000/svg"
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
              ))}
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
