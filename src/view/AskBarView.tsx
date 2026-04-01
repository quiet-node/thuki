import { motion } from 'framer-motion';
import type React from 'react';
import { useCallback } from 'react';
import { formatQuotedText } from '../utils/formatQuote';
import { quote } from '../config';

/**
 * Hoisted static SVG — prevents re-allocation on every render cycle.
 * @see Vercel React Best Practices §6.3 — Hoist Static JSX Elements
 */
const ARROW_UP_ICON = (
  <svg
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <path
      d="M8 13V3M8 3L3 8M8 3L13 8"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

/**
 * Hoisted static SVG — square stop icon displayed during active generation.
 */
const STOP_ICON = (
  <svg
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect x="3" y="3" width="10" height="10" rx="2" fill="currentColor" />
  </svg>
);

/**
 * Props for the AskBarView component.
 */
interface AskBarViewProps {
  /** The current user input text. */
  query: string;
  /** State setter to update the user input text. */
  setQuery: React.Dispatch<React.SetStateAction<string>>;
  /** True if the chat history is expanded or currently generating. */
  isChatMode: boolean;
  /** True if the AI is actively generating a response. */
  isGenerating: boolean;
  /** Submit handler fired when the user commits their message. */
  onSubmit: () => void;
  /** Cancel handler fired when the user stops an active generation. */
  onCancel: () => void;
  /** Ref to the textarea input element for focus management. */
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
  /** Selected text from the host app captured at activation time, if any. */
  selectedText?: string;
}

/**
 * Renders the persistent bottom input bar of the application.
 *
 * Window dragging is handled by the application root container via event
 * bubbling — mousedown events from this component propagate up naturally.
 */
export function AskBarView({
  query,
  setQuery,
  isChatMode,
  isGenerating,
  onSubmit,
  onCancel,
  inputRef,
  selectedText,
}: AskBarViewProps) {
  const canSubmit = query.trim().length > 0 && !isGenerating;

  /**
   * Auto-resizes the textarea to fit its content up to a maximum height.
   * Single forced reflow per input event ensures responsive text wrapping.
   */
  const handleTextareaChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      setQuery(e.target.value);
      const el = e.target;
      el.style.height = 'auto'; // Reset to auto to trigger height recalculation
      el.style.height = `${Math.min(el.scrollHeight, 144)}px`;
    },
    [setQuery],
  );

  /**
   * Catches `Enter` without `Shift` to submit the form proactively,
   * avoiding accidental line breaks for power users.
   */
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        onSubmit();
      }
    },
    [onSubmit],
  );

  return (
    <div className="flex flex-col w-full shrink-0">
      {selectedText && (
        <div className="px-4 pt-2 pb-0">
          <p className="italic text-xs text-text-secondary select-text whitespace-pre-wrap">
            &ldquo;
            {formatQuotedText(
              selectedText,
              quote.maxDisplayLines,
              quote.maxDisplayChars,
            )}
            &rdquo;
          </p>
        </div>
      )}
      <div className="flex items-center w-full px-3 py-2.5 gap-2">
        <img
          src="/thuki-logo.png"
          alt="Thuki"
          className={`shrink-0 transition-all duration-300 ease-out ${
            isChatMode ? 'w-6 h-6 rounded-lg' : 'w-10 h-10 rounded-xl'
          }`}
          draggable={false}
        />

        <textarea
          ref={inputRef}
          value={query}
          onChange={handleTextareaChange}
          onKeyDown={handleKeyDown}
          disabled={isGenerating}
          autoFocus
          rows={1}
          placeholder={isChatMode ? 'Reply...' : 'Ask Thuki anything...'}
          className="flex-1 min-w-0 bg-transparent border-none outline-none text-text-primary text-sm placeholder:text-text-secondary py-2 px-1 disabled:opacity-50 resize-none leading-relaxed"
        />

        <motion.button
          type="button"
          onClick={isGenerating ? onCancel : onSubmit}
          disabled={!canSubmit && !isGenerating}
          whileHover={canSubmit || isGenerating ? { scale: 1.08 } : undefined}
          whileTap={canSubmit || isGenerating ? { scale: 0.92 } : undefined}
          className={`shrink-0 w-9 h-9 rounded-xl flex items-center justify-center transition-colors duration-200 ${
            isGenerating
              ? 'stop-btn-ring bg-red-500/10 text-red-400 cursor-pointer'
              : canSubmit
                ? 'bg-primary text-neutral cursor-pointer'
                : 'bg-surface-elevated text-text-secondary cursor-default'
          }`}
          aria-label={isGenerating ? 'Stop generating' : 'Send message'}
        >
          {isGenerating ? STOP_ICON : ARROW_UP_ICON}
        </motion.button>
      </div>
    </div>
  );
}
