import { AnimatePresence, motion } from 'framer-motion';
import { useEffect, useRef, useState } from 'react';

/**
 * Hoisted static SVG - chip-style trigger icon for the model picker.
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
      x="4.5"
      y="4.5"
      width="7"
      height="7"
      rx="1.5"
      stroke="currentColor"
      strokeWidth="1.5"
    />
    <path
      d="M6 2.5V4M8 2.5V4M10 2.5V4M6 12V13.5M8 12V13.5M10 12V13.5M2.5 6H4M2.5 8H4M2.5 10H4M12 6H13.5M12 8H13.5M12 10H13.5"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    />
  </svg>
);

/** Props for the ModelPicker component. */
export interface ModelPickerProps {
  /** Currently active model slug; highlighted in the popup. */
  activeModel: string;
  /** Full list of available model slugs from Ollama's tags endpoint. */
  models: string[];
  /** When true the trigger is inert (e.g. during generation). */
  disabled: boolean;
  /** Called with the chosen slug when the user picks a row. */
  onSelect: (model: string) => void;
}

/**
 * Right-side chip trigger that opens a slug-only popup anchored above
 * the ask bar's send button. The popup closes on outside click.
 *
 * Rendered inline inside AskBarView's bottom row: `absolute right-0 bottom-10`
 * keeps it within the ask bar's relative container so no portal is needed.
 */
export function ModelPicker({
  activeModel,
  models,
  disabled,
  onSelect,
}: ModelPickerProps) {
  const [isOpen, setIsOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  // Derived so a `disabled` flip (e.g. generation starts while popup is open)
  // hides the popup immediately without needing a state-syncing effect.
  const showPopup = isOpen && !disabled;

  useEffect(() => {
    if (!showPopup) return;
    const handleMouseDown = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener('mousedown', handleMouseDown);
    return () => document.removeEventListener('mousedown', handleMouseDown);
  }, [showPopup]);

  if (models.length === 0) return null;

  return (
    <div ref={rootRef} className="relative shrink-0">
      <button
        type="button"
        aria-label="Choose model"
        disabled={disabled}
        onClick={() => setIsOpen((open) => !open)}
        className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-text-primary hover:bg-white/8 transition-colors duration-150 disabled:opacity-40"
      >
        {CHIP_ICON}
      </button>

      <AnimatePresence>
        {showPopup && (
          <motion.div
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 6 }}
            transition={{ duration: 0.16 }}
            className="absolute right-0 bottom-10 w-56 overflow-hidden rounded-xl border border-surface-border bg-surface-base shadow-chat backdrop-blur-2xl"
          >
            {models.map((model) => (
              <button
                key={model}
                type="button"
                aria-label={model}
                aria-current={model === activeModel ? 'true' : undefined}
                onClick={() => {
                  onSelect(model);
                  setIsOpen(false);
                }}
                className={`block w-full truncate px-4 py-3 text-left text-sm ${
                  model === activeModel
                    ? 'bg-primary/10 text-text-primary'
                    : 'text-text-primary hover:bg-white/6'
                }`}
              >
                {model}
              </button>
            ))}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
