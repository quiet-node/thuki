import { motion, AnimatePresence } from 'framer-motion';
import type React from 'react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { formatQuotedText } from '../utils/formatQuote';
import { useConfig } from '../contexts/ConfigContext';
import { ImageThumbnails } from '../components/ImageThumbnails';
import { CommandSuggestion } from '../components/CommandSuggestion';
import { ModelPickerList, ModelPickerTrigger } from '../components/ModelPicker';
import { Tooltip } from '../components/Tooltip';
import type { AttachedImage } from '../types/image';
import { MAX_IMAGE_SIZE_BYTES } from '../types/image';
import { COMMANDS } from '../config/commands';

/**
 * Hoisted static SVG - prevents re-allocation on every render cycle.
 * @see Vercel React Best Practices §6.3 - Hoist Static JSX Elements
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
 * Hoisted static SVG - square stop icon displayed during active generation.
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

/** Hoisted static history (clock) icon - prevents re-allocation on every render. */
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

/** Hoisted static camera icon - triggers screenshot capture. */
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
 * Renders text with command triggers highlighted in violet for the mirror div.
 * Only the first occurrence of each command is highlighted; duplicates render plain.
 */
export function renderHighlightedText(text: string): React.ReactNode {
  const parts: React.ReactNode[] = [];
  let remaining = text;
  const highlighted = new Set<string>();

  while (remaining.length > 0) {
    let earliest = -1;
    let matchedTrigger = '';
    for (const cmd of COMMANDS) {
      if (highlighted.has(cmd.trigger)) continue;
      const idx = remaining.indexOf(cmd.trigger);
      if (idx !== -1 && (earliest === -1 || idx < earliest)) {
        const before = idx === 0 || remaining[idx - 1] === ' ';
        const after =
          idx + cmd.trigger.length >= remaining.length ||
          remaining[idx + cmd.trigger.length] === ' ';
        if (before && after) {
          earliest = idx;
          matchedTrigger = cmd.trigger;
        }
      }
    }

    if (earliest === -1) {
      parts.push(<span key={parts.length}>{remaining}</span>);
      break;
    }

    if (earliest > 0) {
      parts.push(
        <span key={parts.length}>{remaining.slice(0, earliest)}</span>,
      );
    }
    parts.push(
      <span key={parts.length} className="text-violet-400">
        {matchedTrigger}
      </span>,
    );
    highlighted.add(matchedTrigger);
    remaining = remaining.slice(earliest + matchedTrigger.length);
  }

  return <>{parts}</>;
}

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
  /** Called when the user pastes image files. */
  onImagesAttached: (files: File[]) => void;
  /** Called when the user removes an attached image by ID. */
  onImageRemove: (id: string) => void;
  /** Called when the user clicks a thumbnail to preview it. */
  onImagePreview: (id: string) => void;
  /** Called when the user clicks the screenshot capture button. */
  onScreenshot: () => void;
  /**
   * Drag state passed down from the root window handler.
   * "normal" = violet ring; "max" = red ring + label; undefined = no ring.
   */
  isDragOver?: 'normal' | 'max';
  /** Currently active Ollama model slug. Enables the model picker when set. */
  activeModel?: string;
  /** Full list of model slugs available for selection in the picker. */
  availableModels?: string[];
  /** Called when the user picks a new active model from the picker. */
  onModelSelect?: (model: string) => void;
}

/**
 * Renders the persistent bottom input bar of the application.
 *
 * Window dragging is handled by the application root container via event
 * bubbling - mousedown events from this component propagate up naturally.
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
  isDragOver,
  activeModel,
  availableModels,
  onModelSelect,
}: AskBarViewProps) {
  /** Ref to the mirror div behind the textarea for command highlighting. */
  const mirrorRef = useRef<HTMLDivElement>(null);

  /** Quote display limits resolved from the managed AppConfig. */
  const quote = useConfig().quote;

  /** True when the UI should be locked - either generating or waiting for images. */
  const isBusy = isGenerating || isSubmitPending;
  const canSubmit =
    (query.trim().length > 0 || attachedImages.length > 0) && !isBusy;
  const isAtMaxImages = attachedImages.length >= MAX_IMAGES;

  /** True briefly after a paste attempt is rejected because max images reached. */
  const [pasteMaxError, setPasteMaxError] = useState(false);

  useEffect(() => {
    if (!pasteMaxError) return;
    const timer = setTimeout(() => setPasteMaxError(false), 2000);
    return () => clearTimeout(timer);
  }, [pasteMaxError]);

  // ─── Model picker availability gate ───────────────────────────────────────

  /** Prerequisites for rendering the model picker chip. */
  const modelPickerAvailable = Boolean(
    activeModel &&
    availableModels &&
    availableModels.length > 0 &&
    onModelSelect,
  );

  /** Whether the inline model picker list is currently expanded. */
  const [isModelPickerOpen, setIsModelPickerOpen] = useState(false);
  const modelPickerTriggerRef = useRef<HTMLButtonElement>(null);
  const modelPickerListRef = useRef<HTMLDivElement>(null);

  /** Combined gate: don't show the list while busy or without data. */
  const showModelPicker = isModelPickerOpen && !isBusy && modelPickerAvailable;

  /* Auto-close the picker when generation starts. */
  /* eslint-disable @eslint-react/set-state-in-effect -- intentional: mirror the
     busy prop into the local open state so a mid-open busy flip cleanly closes
     the list. No secondary effects are triggered by this reset. */
  useEffect(() => {
    if (isBusy && isModelPickerOpen) setIsModelPickerOpen(false);
  }, [isBusy, isModelPickerOpen]);
  /* eslint-enable @eslint-react/set-state-in-effect */

  /* Outside-click closes the picker. Listener attached only while open. */
  useEffect(() => {
    if (!showModelPicker) return;
    const handleMouseDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (modelPickerTriggerRef.current?.contains(target)) return;
      if (modelPickerListRef.current?.contains(target)) return;
      setIsModelPickerOpen(false);
    };
    document.addEventListener('mousedown', handleMouseDown);
    return () => document.removeEventListener('mousedown', handleMouseDown);
  }, [showModelPicker]);

  const toggleModelPicker = useCallback(() => {
    setIsModelPickerOpen((open) => !open);
  }, []);

  const handleModelRowSelect = useCallback(
    (model: string) => {
      onModelSelect?.(model);
      setIsModelPickerOpen(false);
    },
    [onModelSelect],
  );

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
   * Finds the last word starting with "/" in the query to use as the active
   * command prefix. This allows command suggestions to appear anywhere in the
   * text, not just at the start.
   */
  const rawQuery = query.trimStart();
  const lastSlashWord = useMemo(() => {
    // Find the last word that starts with "/"
    const match = rawQuery.match(/(?:^|\s)(\/\S*)$/);
    return match ? match[1] : '';
  }, [rawQuery]);

  const showSuggestions =
    !isBusy && lastSlashWord.length > 0 && lastSlashWord !== dismissedQuery;

  /** The active command prefix (e.g. "/sc"). Empty when not suggesting. */
  const commandPrefix = showSuggestions ? lastSlashWord : '';

  /** Commands already present in the text before the current slash word. */
  const usedCommands = useMemo(() => {
    const textBeforeSlash = rawQuery.slice(
      0,
      rawQuery.length - lastSlashWord.length,
    );
    return new Set(
      COMMANDS.filter((cmd) => {
        const idx = textBeforeSlash.indexOf(cmd.trigger);
        if (idx === -1) return false;
        const before = idx === 0 || textBeforeSlash[idx - 1] === ' ';
        const after =
          idx + cmd.trigger.length >= textBeforeSlash.length ||
          textBeforeSlash[idx + cmd.trigger.length] === ' ';
        return before && after;
      }).map((cmd) => cmd.trigger),
    );
  }, [rawQuery, lastSlashWord]);

  /** Commands that match the current prefix, excluding already-used ones. */
  const filteredCommands = useMemo(
    () =>
      showSuggestions
        ? COMMANDS.filter(
            (cmd) =>
              cmd.trigger.startsWith(commandPrefix) &&
              !usedCommands.has(cmd.trigger),
          )
        : [],
    [showSuggestions, commandPrefix, usedCommands],
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

  /**
   * Applies the selected trigger by replacing the partial slash word at the
   * end of the query with the full trigger + trailing space.
   */
  const handleCommandSelect = useCallback(
    (trigger: string) => {
      setDismissedQuery('');
      setHighlightedIndex(0);
      // Replace the partial slash word at the end with the completed trigger
      const beforeSlash = rawQuery.slice(
        0,
        rawQuery.length - lastSlashWord.length,
      );
      setQuery(beforeSlash + trigger + ' ');
    },
    [setQuery, rawQuery, lastSlashWord],
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
            if (lastSlashWord !== selectedTrigger) {
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
          setDismissedQuery(lastSlashWord);
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
      lastSlashWord,
      onSubmit,
    ],
  );

  /** Syncs the mirror div scroll position with the textarea. */
  const handleTextareaScroll = useCallback(() => {
    /* v8 ignore start -- both refs are always set by React when this fires */
    if (!mirrorRef.current || !inputRef.current) return;
    /* v8 ignore stop */
    mirrorRef.current.scrollTop = inputRef.current.scrollTop;
  }, [inputRef]);

  /** Handles clipboard paste - extracts image items from clipboardData. */
  const handlePaste = useCallback(
    (e: React.ClipboardEvent) => {
      const items = e.clipboardData?.items;
      if (!items || isBusy) return;

      const remaining = MAX_IMAGES - attachedImages.length;
      if (remaining <= 0) {
        const hasImageItem = Array.from(items).some((item) =>
          item.type.startsWith('image/'),
        );
        if (hasImageItem) setPasteMaxError(true);
        return;
      }

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

  // Suppress the paste error label while a drag is active so the drag-state
  // ring and label always agree. Once the drag ends, the paste error (if still
  // within its 2 s window) reappears.
  const showMaxLabel = isDragOver === 'max' || (pasteMaxError && !isDragOver);
  const ringClass =
    isDragOver === 'max'
      ? 'ring-2 ring-red-500/60 ring-inset rounded-lg'
      : isDragOver === 'normal'
        ? 'ring-2 ring-primary/40 ring-inset rounded-lg'
        : '';

  return (
    <div className={`flex flex-col w-full shrink-0 ${ringClass}`}>
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
      {showMaxLabel && (
        <p className="px-4 pt-2 pb-0 text-xs text-red-400">Max 3 images</p>
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
      {/* Model picker list renders inline above the input row in the same
          DOM-flow pattern as CommandSuggestion. The morphing-container
          ResizeObserver picks up the added height and grows the Tauri
          window upward to reveal the menu without clipping. */}
      {modelPickerAvailable && activeModel && availableModels && (
        <ModelPickerList
          listRef={modelPickerListRef}
          activeModel={activeModel}
          models={availableModels}
          isOpen={showModelPicker}
          onSelect={handleModelRowSelect}
        />
      )}
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

          <div className="relative flex-1 min-w-0">
            {/* Mirror div: renders the same text with highlighted commands.
                Sits behind the transparent textarea so colored spans show through. */}
            <div
              ref={mirrorRef}
              aria-hidden="true"
              className="absolute inset-0 pointer-events-none bg-transparent text-text-primary text-sm py-2 px-1 leading-relaxed whitespace-pre-wrap break-words overflow-hidden"
            >
              {renderHighlightedText(query)}
            </div>
            <textarea
              ref={inputRef}
              value={query}
              onChange={handleTextareaChange}
              onKeyDown={handleKeyDown}
              onPaste={handlePaste}
              onScroll={handleTextareaScroll}
              disabled={isBusy}
              autoFocus
              rows={1}
              placeholder={isChatMode ? 'Reply...' : 'Ask Thuki anything...'}
              className="relative w-full bg-transparent border-none outline-none text-transparent text-sm placeholder:text-text-secondary py-2 px-1 disabled:opacity-50 resize-none leading-relaxed"
              style={{ caretColor: 'var(--color-text-primary)' }}
            />
          </div>

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

          {modelPickerAvailable && (
            <Tooltip label="Choose model">
              <ModelPickerTrigger
                triggerRef={modelPickerTriggerRef}
                isOpen={showModelPicker}
                disabled={isBusy}
                onToggle={toggleModelPicker}
              />
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
