import { motion, AnimatePresence } from 'framer-motion';
import type React from 'react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { formatQuotedText } from '../utils/formatQuote';
import { quote } from '../config';
import { ImageThumbnails } from '../components/ImageThumbnails';
import { CommandSuggestion } from '../components/CommandSuggestion';
import { Tooltip } from '../components/Tooltip';
import type { AttachedImage } from '../types/image';
import { MAX_IMAGE_SIZE_BYTES } from '../types/image';
import { COMMANDS } from '../config/commands';

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
 * SVG overlay that traces a glowing comet-tail along the button's border.
 * Uses `pathLength="100"` so dash math is in clean percentages regardless
 * of the actual rect perimeter. Three layered strokes at staggered offsets
 * create a smooth fade-out tail that follows the rounded-rect path exactly.
 */
const BORDER_TRACE_RING = (
  <svg
    className="stop-ring-svg"
    viewBox="0 0 40 40"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect
      className="stop-trace-tail"
      x="1"
      y="1"
      width="38"
      height="38"
      rx="13"
      pathLength="100"
    />
    <rect
      className="stop-trace-mid"
      x="1"
      y="1"
      width="38"
      height="38"
      rx="13"
      pathLength="100"
    />
    <rect
      className="stop-trace-head"
      x="1"
      y="1"
      width="38"
      height="38"
      rx="13"
      pathLength="100"
    />
  </svg>
);

/** Hoisted static history (clock) icon — prevents re-allocation on every render. */
const HISTORY_ICON = (
  <svg
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <circle
      cx="12"
      cy="12"
      r="10"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
    />
    <polyline
      points="12 6 12 12 16 14"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

/** Hoisted static camera icon — triggers screenshot capture. */
const CAMERA_ICON = (
  <svg
    width="14"
    height="14"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <path
      d="M2 6 L2 2 L6 2 M10 2 L14 2 L14 6 M2 10 L2 14 L6 14 M10 14 L14 14 L14 10"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

/**
 * Maximum number of manually attached images per message. The backend allows
 * one additional image from /screen capture, for a total of 4 per message
 * (MAX_IMAGES_PER_MESSAGE in images.rs).
 */
export const MAX_IMAGES = 3;

/** Props for the AskBarView component. */
interface AskBarViewProps {
  /** The current user input text. */
  query: string;
  /** State setter to update the user input text. */
  setQuery: React.Dispatch<React.SetStateAction<string>>;
  /** True if the chat history is expanded or currently generating. */
  isChatMode: boolean;
  /** True if the AI is actively generating a response. */
  isGenerating: boolean;
  /** True while waiting for images to finish processing before submitting. */
  isSubmitPending?: boolean;
  /** Submit handler fired when the user commits their message. */
  onSubmit: () => void;
  /** Cancel handler fired when the user stops an active generation. */
  onCancel: () => void;
  /** Ref to the textarea input element for focus management. */
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
  /** Selected text from the host app captured at activation time, if any. */
  selectedText?: string;
  /**
   * Called when the compact history icon is clicked in ask-bar mode.
   * Omit to hide the history icon entirely.
   */
  onHistoryOpen?: () => void;
  /** Currently attached images (may still be processing in the background). */
  attachedImages: AttachedImage[];
  /** Called when the user pastes or drops image files. */
  onImagesAttached: (files: File[]) => void;
  /** Called when the user removes an attached image by ID. */
  onImageRemove: (id: string) => void;
  /** Called when the user clicks a thumbnail to preview it. */
  onImagePreview: (id: string) => void;
  /** Called when the user clicks the screenshot capture button. */
  onScreenshot: () => void;
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
  isSubmitPending = false,
  onSubmit,
  onCancel,
  inputRef,
  selectedText,
  onHistoryOpen,
  attachedImages,
  onImagesAttached,
  onImageRemove,
  onImagePreview,
  onScreenshot,
}: AskBarViewProps) {
  /** True when the UI should be locked — either generating or waiting for images. */
  const isBusy = isGenerating || isSubmitPending;
  const canSubmit =
    (query.trim().length > 0 || attachedImages.length > 0) && !isBusy;
  const isAtMaxImages = attachedImages.length >= MAX_IMAGES;
  const [isDragOver, setIsDragOver] = useState(false);

  // ─── Command suggestion state ─────────────────────────────────────────────

  /**
   * Index of the highlighted row in the suggestion popover. Reset to 0
   * whenever the query changes so a new filter result always starts at the top.
   */
  const [highlightedIndex, setHighlightedIndex] = useState(0);

  /**
   * When the user presses Escape, we store the query prefix that was active at
   * that moment. If the query later changes to a different prefix, the popover
   * reopens automatically; if it stays the same, the popover stays dismissed.
   * State (not ref) so that Escape triggers a re-render and hides the popover.
   */
  const [dismissedQuery, setDismissedQuery] = useState('');

  /**
   * Derived: show the suggestion popover when the query starts with "/" and
   * has not yet had a space added (user is still typing the trigger token),
   * the UI is not busy, and the user has not explicitly dismissed this prefix.
   */
  const rawQuery = query.trimStart();
  const showSuggestions =
    !isBusy &&
    rawQuery.startsWith('/') &&
    !rawQuery.includes(' ') &&
    rawQuery !== dismissedQuery;

  /** The active command prefix (e.g. "/sc"). Empty when not suggesting. */
  const commandPrefix = showSuggestions ? rawQuery : '';

  /** Commands that match the current prefix (memoized to keep stable reference). */
  const filteredCommands = useMemo(
    () =>
      showSuggestions
        ? COMMANDS.filter((cmd) => cmd.trigger.startsWith(commandPrefix))
        : [],
    [showSuggestions, commandPrefix],
  );

  // Reset the highlighted index whenever the command prefix changes
  // (user typed more characters and the results updated).
  /* eslint-disable @eslint-react/set-state-in-effect -- intentional: resetting
     highlighted index when the filter prefix changes drives no secondary effects
     and is the canonical pattern for derived-from-prop index resets. */
  useEffect(() => {
    setHighlightedIndex(0);
  }, [commandPrefix]);
  /* eslint-enable @eslint-react/set-state-in-effect */

  /** Applies the selected trigger by setting the query to "trigger " (with trailing space). */
  const handleCommandSelect = useCallback(
    (trigger: string) => {
      setDismissedQuery('');
      setHighlightedIndex(0);
      setQuery(trigger + ' ');
    },
    [setQuery],
  );

  /**
   * Auto-resizes the textarea to fit its content up to a maximum height.
   * Single forced reflow per input event ensures responsive text wrapping.
   * Also clears the dismissed-suggestion state so the popover can reopen
   * if the user has changed the command prefix since dismissing it.
   */
  const handleTextareaChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      const newValue = e.target.value;
      // Any keystroke clears the dismissed state so the popover can reopen
      // if the user types a new "/" prefix after having pressed Escape.
      setDismissedQuery('');
      setQuery(newValue);
      const el = e.target;
      el.style.height = 'auto'; // Reset to auto to trigger height recalculation
      el.style.height = `${Math.min(el.scrollHeight, 144)}px`;
    },
    [setQuery],
  );

  /**
   * Catches `Enter` without `Shift` to submit the form proactively,
   * avoiding accidental line breaks for power users.
   *
   * When the command suggestion popover is open, also handles:
   * - ArrowDown / ArrowUp: move the highlighted row (wraps around)
   * - Tab: complete the highlighted command trigger into the input
   * - Enter: if a valid row is highlighted, complete it; otherwise submit
   * - Escape: dismiss the popover without changing the query
   */
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (showSuggestions) {
        if (e.key === 'ArrowDown') {
          e.preventDefault();
          if (filteredCommands.length > 0) {
            setHighlightedIndex((i) => (i + 1) % filteredCommands.length);
          }
          return;
        }
        if (e.key === 'ArrowUp') {
          e.preventDefault();
          if (filteredCommands.length > 0) {
            setHighlightedIndex(
              (i) =>
                (i - 1 + filteredCommands.length) % filteredCommands.length,
            );
          }
          return;
        }
        if (e.key === 'Tab') {
          e.preventDefault();
          if (filteredCommands.length > 0) {
            const idx = Math.min(highlightedIndex, filteredCommands.length - 1);
            handleCommandSelect(filteredCommands[idx].trigger);
          }
          return;
        }
        if (e.key === 'Enter' && !e.shiftKey) {
          if (
            filteredCommands.length > 0 &&
            highlightedIndex < filteredCommands.length
          ) {
            const selectedTrigger = filteredCommands[highlightedIndex].trigger;
            if (rawQuery !== selectedTrigger) {
              // Partial match: complete the trigger into the input.
              e.preventDefault();
              handleCommandSelect(selectedTrigger);
              return;
            }
            // Exact match: fall through to normal submit below.
          }
          // No match, empty list, or exact trigger already typed: submit.
        }
        if (e.key === 'Escape') {
          e.preventDefault();
          setDismissedQuery(rawQuery);
          return;
        }
      }

      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        onSubmit();
      }
    },
    [
      showSuggestions,
      filteredCommands,
      highlightedIndex,
      handleCommandSelect,
      rawQuery,
      onSubmit,
    ],
  );

  /**
   * Filters and forwards valid image files to the parent for processing.
   * Rejects non-image files and files exceeding the 30MB size cap.
   */
  const processImageFiles = useCallback(
    (files: FileList | null) => {
      if (!files || isBusy) return;
      const remaining = MAX_IMAGES - attachedImages.length;
      if (remaining <= 0) return;

      const accepted: File[] = [];
      for (let i = 0; i < files.length && accepted.length < remaining; i++) {
        if (
          files[i].type.startsWith('image/') &&
          files[i].size <= MAX_IMAGE_SIZE_BYTES
        ) {
          accepted.push(files[i]);
        }
      }
      if (accepted.length > 0) {
        onImagesAttached(accepted);
      }
    },
    [isBusy, attachedImages.length, onImagesAttached],
  );

  /** Handles clipboard paste — extracts image items from clipboardData. */
  const handlePaste = useCallback(
    (e: React.ClipboardEvent) => {
      const items = e.clipboardData?.items;
      if (!items || isBusy) return;

      const remaining = MAX_IMAGES - attachedImages.length;
      if (remaining <= 0) return;

      const imageFiles: File[] = [];
      for (let i = 0; i < items.length && imageFiles.length < remaining; i++) {
        if (items[i].type.startsWith('image/')) {
          const file = items[i].getAsFile();
          if (file && file.size <= MAX_IMAGE_SIZE_BYTES) {
            imageFiles.push(file);
          }
        }
      }

      if (imageFiles.length === 0) return;
      e.preventDefault();
      onImagesAttached(imageFiles);
    },
    [isBusy, attachedImages.length, onImagesAttached],
  );

  const handleDragOver = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      if (!isBusy) setIsDragOver(true);
    },
    [isBusy],
  );

  const handleDragLeave = useCallback(() => {
    setIsDragOver(false);
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setIsDragOver(false);
      processImageFiles(e.dataTransfer?.files ?? null);
    },
    [processImageFiles],
  );

  return (
    <div
      className={`flex flex-col w-full shrink-0 ${isDragOver ? 'ring-2 ring-primary/40 ring-inset rounded-lg' : ''}`}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
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
      {attachedImages.length > 0 && (
        <div className="px-4 pt-2 pb-0">
          <ImageThumbnails
            items={attachedImages.map((img) => ({
              id: img.id,
              src: img.blobUrl,
              loading: img.filePath === null,
            }))}
            onPreview={onImagePreview}
            onRemove={onImageRemove}
            size={56}
          />
        </div>
      )}
      {/* Command suggestion renders above the input row in the normal DOM
          flow. Being inside the morphing container means the ResizeObserver
          detects the added height and grows the native window upward to reveal
          the popover. AnimatePresence + motion.div drive a smooth height
          transition so the window expansion feels intentional, not jarring. */}
      <AnimatePresence>
        {showSuggestions && (
          <motion.div
            key="command-suggestion"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{
              height: { duration: 0.2, ease: [0.16, 1, 0.3, 1] },
              opacity: { duration: 0.15 },
            }}
            style={{ overflow: 'hidden' }}
          >
            <CommandSuggestion
              commands={filteredCommands}
              highlightedIndex={highlightedIndex}
              onSelect={handleCommandSelect}
            />
          </motion.div>
        )}
      </AnimatePresence>
      <div className="relative">
        <div className="flex items-center w-full px-3 py-2.5 gap-2">
          <img
            src="/thuki-logo.png"
            alt="Thuki"
            className={`shrink-0 transition-all duration-300 ease-out ${
              isChatMode ? 'w-6 h-6 rounded-lg' : 'w-10 h-10 rounded-xl'
            }`}
            draggable={false}
          />

          {/* Compact history entry point: ask-bar mode only. In chat mode the
            history button lives in the ConversationView header. */}
          {!isChatMode && onHistoryOpen && (
            <button
              type="button"
              onClick={onHistoryOpen}
              aria-label="Open history"
              className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-text-primary hover:bg-white/8 transition-colors duration-150 cursor-pointer outline-none"
            >
              {HISTORY_ICON}
            </button>
          )}

          <textarea
            ref={inputRef}
            value={query}
            onChange={handleTextareaChange}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            disabled={isBusy}
            autoFocus
            rows={1}
            placeholder={isChatMode ? 'Reply...' : 'Ask Thuki anything...'}
            className="flex-1 min-w-0 bg-transparent border-none outline-none text-text-primary text-sm placeholder:text-text-secondary py-2 px-1 disabled:opacity-50 resize-none leading-relaxed"
          />

          {isAtMaxImages ? (
            <Tooltip label="Maximum 3 images attached">
              <button
                type="button"
                onClick={onScreenshot}
                disabled
                aria-label="Take screenshot"
                className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary transition-colors duration-150 disabled:opacity-40 disabled:cursor-default cursor-pointer"
              >
                {CAMERA_ICON}
              </button>
            </Tooltip>
          ) : (
            <Tooltip label="Take a screenshot">
              <button
                type="button"
                onClick={onScreenshot}
                disabled={isBusy}
                aria-label="Take screenshot"
                className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-text-primary hover:bg-white/8 transition-colors duration-150 disabled:opacity-40 disabled:cursor-default cursor-pointer"
              >
                {CAMERA_ICON}
              </button>
            </Tooltip>
          )}

          <motion.button
            type="button"
            onClick={isBusy ? onCancel : onSubmit}
            disabled={!canSubmit && !isBusy}
            whileHover={canSubmit || isBusy ? { scale: 1.08 } : undefined}
            whileTap={canSubmit || isBusy ? { scale: 0.92 } : undefined}
            className={`shrink-0 w-9 h-9 rounded-xl flex items-center justify-center transition-colors duration-200 ${
              isBusy
                ? 'stop-btn-ring bg-red-500/10 text-red-400 cursor-pointer'
                : canSubmit
                  ? 'bg-primary text-neutral cursor-pointer'
                  : 'bg-surface-elevated text-text-secondary cursor-default'
            }`}
            aria-label={isBusy ? 'Stop generating' : 'Send message'}
          >
            {isBusy ? (
              <>
                {BORDER_TRACE_RING}
                {STOP_ICON}
              </>
            ) : (
              ARROW_UP_ICON
            )}
          </motion.button>
        </div>
      </div>
    </div>
  );
}
