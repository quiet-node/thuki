import { AnimatePresence, motion } from 'framer-motion';
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import { createPortal } from 'react-dom';

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

/** Hoisted static checkmark path used on the active row. */
const CHECK_ICON_PATH = (
  <path
    d="M3 8l3.5 3.5L13 5"
    stroke="currentColor"
    strokeWidth="2.2"
    strokeLinecap="round"
    strokeLinejoin="round"
  />
);

/** Fixed target width for the portal menu in pixels. */
const MENU_WIDTH = 220;
/** Viewport-edge padding used when clamping the left position. */
const EDGE_PADDING = 8;
/** Vertical gap between the trigger and the menu. */
const MENU_GAP = 8;

/** Screen position for the portal menu, computed from the trigger rect. */
interface MenuPosition {
  top: number;
  left: number;
}

/** Props for the {@link ModelPicker} component. */
export interface ModelPickerProps {
  /** Currently active model slug; the matching row renders an orange tick. */
  activeModel: string;
  /** Full list of available model slugs from Ollama's tags endpoint. */
  models: string[];
  /** When true the trigger is inert (e.g. during generation) and any open menu closes. */
  disabled: boolean;
  /** Called with the chosen slug when the user picks a row. */
  onSelect: (model: string) => void;
}

/**
 * Single self-contained model picker rendered as a portal menu.
 *
 * The menu escapes the ask bar's morphing container (which sets
 * `overflow-hidden`) by rendering into `document.body` via
 * {@link createPortal}. That keeps the Thuki window size stable while the
 * menu floats above it like a native macOS NSMenu.
 *
 * Positioning algorithm:
 *   1. Read the trigger's `getBoundingClientRect` on open.
 *   2. Right-align the menu to the trigger, clamped to 8px from the left edge.
 *   3. Prefer opening above the trigger. If that would clip above the
 *      viewport, open below instead. Uses a two-phase rAF measurement:
 *      render once to measure the menu height, then adjust `top`.
 *   4. Re-run on every scroll / resize / window blur while the menu is open.
 *
 * All listeners (scroll, resize, mousedown, keydown) are attached in a single
 * effect gated on {@link showMenu} and removed on close or unmount.
 */
export function ModelPicker({
  activeModel,
  models,
  disabled,
  onSelect,
}: ModelPickerProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [position, setPosition] = useState<MenuPosition | null>(null);

  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  /**
   * Combined open gate: hides the menu if the picker becomes disabled or
   * empty while the user intent (`isOpen`) is still true. The underlying
   * `isOpen` state is still reset to false by the disabled-sync effect
   * below so re-enabling does not reopen a stale menu.
   */
  const showMenu = isOpen && !disabled && models.length > 0;

  /** Recomputes the menu position from the current trigger rect. */
  const updatePosition = useCallback(() => {
    const trigger = triggerRef.current;
    /* v8 ignore start -- trigger ref is always set while the menu can be open;
       guard is defensive for concurrent unmount. */
    if (!trigger) return;
    /* v8 ignore stop */
    const rect = trigger.getBoundingClientRect();
    const left = Math.max(EDGE_PADDING, rect.right - MENU_WIDTH);

    const menuEl = menuRef.current;
    const menuHeight = menuEl?.offsetHeight ?? 0;
    let top = rect.top - menuHeight - MENU_GAP;
    if (top < 0) {
      top = rect.bottom + MENU_GAP;
    }
    setPosition({ top, left });
  }, []);

  /**
   * First-frame position: read the rect synchronously so the menu mounts
   * at an approximate spot, then the effect below re-measures height and
   * flips above/below on the following frame.
   */
  /* eslint-disable @eslint-react/set-state-in-effect -- intentional: seeding
     the initial menu position from the trigger rect is exactly what a layout
     effect is for. The rAF inside the next effect corrects the top coordinate
     once the menu has laid out and a real height is available. */
  useLayoutEffect(() => {
    if (!showMenu) {
      setPosition(null);
      return;
    }
    const trigger = triggerRef.current;
    /* v8 ignore start -- showMenu implies trigger is mounted; defensive guard. */
    if (!trigger) return;
    /* v8 ignore stop */
    const rect = trigger.getBoundingClientRect();
    const left = Math.max(EDGE_PADDING, rect.right - MENU_WIDTH);
    // Start above the trigger by an estimated offset so the first paint
    // is close to final. The rAF below corrects based on real height.
    setPosition({ top: rect.top - MENU_GAP, left });
  }, [showMenu]);
  /* eslint-enable @eslint-react/set-state-in-effect */

  /**
   * Attaches all live listeners for the open menu and re-measures once the
   * menu has laid out so the above/below flip uses the real height.
   */
  useEffect(() => {
    if (!showMenu) return;

    // Re-measure after the portal has rendered once.
    const rafId = requestAnimationFrame(updatePosition);

    const handleScroll = () => {
      requestAnimationFrame(updatePosition);
    };
    const handleResize = () => {
      requestAnimationFrame(updatePosition);
    };
    const handleMouseDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (triggerRef.current?.contains(target)) return;
      if (menuRef.current?.contains(target)) return;
      setIsOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setIsOpen(false);
      }
    };

    window.addEventListener('scroll', handleScroll, { passive: true });
    window.addEventListener('resize', handleResize, { passive: true });
    document.addEventListener('mousedown', handleMouseDown);
    document.addEventListener('keydown', handleKeyDown);

    return () => {
      cancelAnimationFrame(rafId);
      window.removeEventListener('scroll', handleScroll);
      window.removeEventListener('resize', handleResize);
      document.removeEventListener('mousedown', handleMouseDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [showMenu, updatePosition]);

  /**
   * When the picker becomes disabled (e.g. generation starts), collapse
   * the open intent so re-enabling does not reopen a stale menu.
   */
  /* eslint-disable @eslint-react/set-state-in-effect -- intentional: mirror the
     disabled prop into the local open state so a mid-open disable cleanly
     closes. No secondary effects are triggered by this reset. */
  useEffect(() => {
    if (disabled && isOpen) setIsOpen(false);
  }, [disabled, isOpen]);
  /* eslint-enable @eslint-react/set-state-in-effect */

  const handleToggle = useCallback(() => {
    setIsOpen((prev) => !prev);
  }, []);

  const handleRowClick = useCallback(
    (model: string) => {
      onSelect(model);
      setIsOpen(false);
    },
    [onSelect],
  );

  if (models.length === 0) return null;

  /* v8 ignore next 2 -- SSR guard; Tauri + happy-dom always provide document. */
  const portalTarget = typeof document !== 'undefined' ? document.body : null;

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        aria-label="Choose model"
        aria-expanded={isOpen}
        aria-haspopup="menu"
        disabled={disabled}
        onClick={handleToggle}
        className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-text-primary hover:bg-white/8 transition-colors duration-150 disabled:opacity-40 disabled:cursor-default cursor-pointer"
      >
        {CHIP_ICON}
      </button>
      {portalTarget &&
        createPortal(
          <AnimatePresence>
            {showMenu && position && (
              <motion.div
                ref={menuRef}
                key="model-picker-menu"
                role="menu"
                initial={{ opacity: 0, y: 6, scale: 0.98 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: 6, scale: 0.98 }}
                transition={{ duration: 0.16, ease: [0.16, 1, 0.3, 1] }}
                style={{ top: position.top, left: position.left }}
                className="fixed min-w-[220px] rounded-xl border border-surface-border bg-surface-base shadow-chat backdrop-blur-2xl p-1.5 z-[200]"
              >
                {models.map((model) => {
                  const active = model === activeModel;
                  return (
                    <button
                      key={model}
                      type="button"
                      role="menuitem"
                      aria-label={model}
                      aria-current={active ? 'true' : undefined}
                      onClick={() => handleRowClick(model)}
                      className="flex items-center justify-between gap-2.5 px-3 py-2 rounded-lg w-full text-left text-sm text-text-primary whitespace-nowrap cursor-pointer hover:bg-white/5 transition-colors duration-120"
                    >
                      <span className="flex-1 min-w-0 overflow-hidden text-ellipsis">
                        {model}
                      </span>
                      <svg
                        className="w-3.5 h-3.5 shrink-0 text-primary"
                        style={{ opacity: active ? 1 : 0 }}
                        viewBox="0 0 16 16"
                        fill="none"
                        xmlns="http://www.w3.org/2000/svg"
                        aria-hidden="true"
                      >
                        {CHECK_ICON_PATH}
                      </svg>
                    </button>
                  );
                })}
              </motion.div>
            )}
          </AnimatePresence>,
          portalTarget,
        )}
    </>
  );
}
