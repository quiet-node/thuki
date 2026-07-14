import { motion, AnimatePresence } from 'framer-motion';
import type React from 'react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { formatQuotedText } from '../utils/formatQuote';
import { LexicalAskBarInput } from './askbar/LexicalAskBarInput';
import type { AskBarKeyHandlers } from './askbar/LexicalAskBarInput';
import { useConfig } from '../contexts/ConfigContext';
import { ImageThumbnails } from '../components/ImageThumbnails';
import { CommandSuggestion } from '../components/CommandSuggestion';
import { ModelPicker } from '../components/ModelPicker';
import { Tooltip } from '../components/Tooltip';
import { CapabilityMismatchStrip } from '../components/CapabilityMismatchStrip';
import { DownloadStatusStrip } from '../components/DownloadStatusStrip';
import type { DownloadStripStatus } from '../components/DownloadStatusStrip';
import { AutoPrimeSkippedStrip } from '../components/AutoPrimeSkippedStrip';
import type { AutoPrimeSkippedStripProps } from '../components/AutoPrimeSkippedStrip';
import type { CapabilityMismatchMessage } from '../components/CapabilityMismatchStrip';
import { SearchTrustNotice } from '../components/SearchTrustNotice';
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
    width="16"
    height="16"
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
  /** Ref to the contentEditable input element for focus management. */
  inputRef: React.RefObject<HTMLDivElement | null>;
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
  /**
   * Called when the user clicks the model picker trigger. App.tsx owns the
   * open/close state and renders the ModelPickerPanel as an inline drawer.
   * In compose mode App.tsx gates this on `ollamaReachable`, so its presence
   * doubles as the signal that Ollama is reachable: the chip stays visible
   * even when there is no active model or zero installed models so the user
   * can recover by opening the picker.
   */
  onModelPickerToggle?: () => void;
  /** Whether the model picker panel is currently open (drives aria-expanded). */
  isModelPickerOpen?: boolean;
  /**
   * Capability mismatch message to render between the attachments row and
   * the input. `null` (or undefined) renders nothing. The host computes
   * this via `getCapabilityConflict` and passes it down. The host may
   * pass either a plain string (passive informational strip) or a
   * `{ text, url }` pair (clickable strip that opens the URL).
   */
  capabilityConflictMessage?: CapabilityMismatchMessage | null;
  /**
   * Ambient model-download status to render in the same slot as the
   * capability strip. `null` (or undefined) renders nothing. Set by the host
   * while a built-in model is downloading in the background so the ask bar
   * shows progress, readiness, or a retry without leaving the picker.
   */
  downloadStatus?: DownloadStripStatus | null;
  /**
   * Ambient warning that the built-in engine's memory gate skipped
   * auto-priming the active model (issue #296). Rendered in the same slot
   * as the download strip, but only in ask-bar mode: once a real chat turn
   * starts, the per-message `InsufficientMemory` error card takes over as
   * the surface for this same refusal. `null` (or undefined) renders
   * nothing.
   */
  autoPrimeSkipped?: AutoPrimeSkippedStripProps | null;
  /**
   * Whether a usable model is installed and active. When true, the send
   * affordance stays live even while a built-in model is downloading, so a
   * background download never holds the bar hostage; only when there is no
   * usable model yet does an in-flight download grey out send. Defaults to
   * false (treat as no usable model) for callers that do not track it.
   */
  hasUsableModel?: boolean;
  /**
   * When true, the input row plays a brief horizontal shake animation.
   * The host pulses this true / false to signal a refused submit.
   */
  shake?: boolean;
  /** Maximum number of manually attached images. Sourced from AppConfig. */
  maxImages: number;
  /**
   * Called once when the textarea transitions from empty to non-empty.
   * Used to trigger model pre-warming so Ollama is ready before the user
   * submits their first message.
   */
  onFirstKeystroke?: () => void;
}

/**
 * Renders the persistent bottom input bar of the application.
 *
 * Also hosts the first-use Auto search trust notice (elevated panel above the
 * logo/input row) when `behavior.auto_search` is on and the notice has not
 * been acknowledged. Notice is non-blocking: compose and send stay enabled.
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
  onModelPickerToggle,
  isModelPickerOpen,
  capabilityConflictMessage,
  downloadStatus,
  autoPrimeSkipped,
  hasUsableModel = false,
  shake = false,
  maxImages,
  onFirstKeystroke,
}: AskBarViewProps) {
  /** Quote limits + behavior flags from managed AppConfig. */
  const { quote, behavior } = useConfig();

  /**
   * Local hide after Got it so the card drops before config reload lands.
   * Session-only; next mount re-reads `searchNoticeAcknowledged` from config.
   */
  const [noticeDismissedLocally, setNoticeDismissedLocally] = useState(false);

  /**
   * First-use Auto search notice: when Auto search is on and the user has not
   * acknowledged yet (and not locally dismissed this session). Shown as soon
   * as the ask bar mounts, not gated on a search turn.
   */
  const showSearchTrustNotice =
    behavior.autoSearch &&
    !behavior.searchNoticeAcknowledged &&
    !noticeDismissedLocally;

  /**
   * Persist acknowledgement and hide the notice immediately.
   */
  const acknowledgeSearchNotice = useCallback(() => {
    setNoticeDismissedLocally(true);
    void invoke('set_config_field', {
      section: 'behavior',
      key: 'search_notice_acknowledged',
      value: true,
    });
  }, []);

  /**
   * Open Settings → Behavior (Auto search highlighted). Does not flip
   * auto_search; user must toggle there deliberately.
   */
  const openSearchSettings = useCallback(() => {
    void invoke('open_settings_to_behavior');
  }, []);

  /** True when the UI should be locked - either generating or waiting for images. */
  const isBusy = isGenerating || isSubmitPending;
  // A built-in model still downloading (or paused mid-download) greys the send
  // affordance ONLY while there is no usable model yet: the input stays editable
  // for drafting, but there is nothing to send to. Once a model is usable (the
  // download finished, or another model was installed in Settings), send stays
  // live and the download continues as an ambient strip, never a hostage.
  const isDownloadHolding =
    downloadStatus?.kind === 'downloading' ||
    downloadStatus?.kind === 'pausing' ||
    downloadStatus?.kind === 'verifying' ||
    downloadStatus?.kind === 'paused';
  const canSubmit =
    (query.trim().length > 0 || attachedImages.length > 0) &&
    !isBusy &&
    !(isDownloadHolding && !hasUsableModel);
  const isAtMaxImages = attachedImages.length >= maxImages;

  /** True briefly after a paste attempt is rejected because max images reached. */
  const [pasteMaxError, setPasteMaxError] = useState(false);

  useEffect(() => {
    if (!pasteMaxError) return;
    const timer = setTimeout(() => setPasteMaxError(false), 2000);
    return () => clearTimeout(timer);
  }, [pasteMaxError]);

  // ─── Model picker availability gate ───────────────────────────────────────

  /**
   * Prerequisites for rendering the chip trigger in the input bar.
   * Hidden in chat mode (the pill trigger moves to the WindowControls
   * header). The chip renders whenever the picker callback is wired up
   * regardless of model state: with no active model it surfaces the
   * "Pick a model" recovery affordance, and the caller is expected to
   * omit `onModelPickerToggle` (Ollama unreachable) when the chip should
   * stay hidden.
   */
  const modelPickerAvailable = Boolean(!isChatMode && onModelPickerToggle);

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
   * Mirrors the editor's text into the host query state. Clears the dismissed
   * suggestion so the popover can reopen when the user types a new "/" prefix
   * after pressing Escape. First-keystroke pre-warming is handled inside the
   * Lexical input.
   */
  const handleValueChange = useCallback(
    (newValue: string) => {
      setDismissedQuery('');
      setQuery(newValue);
    },
    [setQuery],
  );

  /**
   * Keyboard handlers for the slash-command popover, wired into the Lexical
   * input. Arrow / Tab / Escape only fire while the popover is open; Enter
   * (without Shift) always fires and either completes the highlighted command
   * or submits.
   */
  const keyHandlers = useMemo<AskBarKeyHandlers>(
    () => ({
      onEnterKey: () => {
        if (
          showSuggestions &&
          filteredCommands.length > 0 &&
          highlightedIndex < filteredCommands.length
        ) {
          const selectedTrigger = filteredCommands[highlightedIndex].trigger;
          if (lastSlashWord !== selectedTrigger) {
            handleCommandSelect(selectedTrigger);
            return;
          }
        }
        onSubmit();
      },
      onArrowDown: () => {
        if (filteredCommands.length > 0) {
          setHighlightedIndex((i) => (i + 1) % filteredCommands.length);
        }
      },
      onArrowUp: () => {
        if (filteredCommands.length > 0) {
          setHighlightedIndex(
            (i) => (i - 1 + filteredCommands.length) % filteredCommands.length,
          );
        }
      },
      onTab: () => {
        if (filteredCommands.length > 0) {
          const idx = Math.min(highlightedIndex, filteredCommands.length - 1);
          handleCommandSelect(filteredCommands[idx].trigger);
        }
      },
      onEscape: () => {
        setDismissedQuery(lastSlashWord);
      },
    }),
    [
      showSuggestions,
      filteredCommands,
      highlightedIndex,
      handleCommandSelect,
      lastSlashWord,
      onSubmit,
    ],
  );

  /**
   * Extracts image files from a clipboard paste. Returns true when images were
   * attached so the editor suppresses the default text paste; false to let the
   * editor paste text normally.
   */
  const handlePaste = useCallback(
    (clipboard: DataTransfer | null): boolean => {
      const items = clipboard?.items;
      if (!items || isBusy) return false;

      const remaining = maxImages - attachedImages.length;
      if (remaining <= 0) {
        const hasImageItem = Array.from(items).some((item) =>
          item.type.startsWith('image/'),
        );
        if (hasImageItem) setPasteMaxError(true);
        return false;
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

      if (imageFiles.length === 0) return false;
      onImagesAttached(imageFiles);
      return true;
    },
    [isBusy, attachedImages.length, maxImages, onImagesAttached],
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
        <p className="px-4 pt-2 pb-0 text-xs text-red-400">
          Max {maxImages} images
        </p>
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
      {capabilityConflictMessage && (
        <CapabilityMismatchStrip message={capabilityConflictMessage} />
      )}
      {downloadStatus ? (
        <DownloadStatusStrip status={downloadStatus} surface="askbar" />
      ) : null}
      {!isChatMode && autoPrimeSkipped ? (
        <AutoPrimeSkippedStrip {...autoPrimeSkipped} />
      ) : null}
      {showSearchTrustNotice ? (
        <div className="px-3 pt-2 pb-0">
          <SearchTrustNotice
            onAcknowledge={acknowledgeSearchNotice}
            onOpenSettings={openSearchSettings}
          />
        </div>
      ) : null}
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
      <motion.div
        className="relative"
        data-testid="ask-bar-row"
        animate={shake ? { x: [0, -4, 4, -3, 3, 0] } : { x: 0 }}
        transition={
          shake ? { duration: 0.5, ease: 'easeInOut' } : { duration: 0 }
        }
      >
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
            <Tooltip label="Conversation history">
              <button
                type="button"
                onClick={onHistoryOpen}
                aria-label="Open history"
                className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-primary hover:bg-primary/10 transition-colors duration-150 cursor-pointer outline-none"
              >
                {HISTORY_ICON}
              </button>
            </Tooltip>
          )}

          {/* Lexical-backed input. A single contentEditable means the caret is
              native and never drifts; command triggers highlight inline via a
              text-entity node. The host stays the source of truth for `query`. */}
          <div className="relative flex-1 min-w-0">
            {/* The compose surface stays editable while a response streams or
                a submit is pending, so the user can draft their next message
                without waiting. Sending is gated at submit time (handleSubmit
                no-ops while busy and the send button becomes Stop), never by
                disabling the input. */}
            <LexicalAskBarInput
              value={query}
              onValueChange={handleValueChange}
              onFirstKeystroke={onFirstKeystroke}
              placeholder={isChatMode ? 'Reply...' : 'Ask Thuki anything...'}
              inputRef={inputRef}
              contentEditableClassName="askbar-input thuki-text-base w-full bg-transparent outline-none text-text-primary py-2 px-1 whitespace-pre-wrap break-words"
              suggestionsOpen={showSuggestions}
              keyHandlers={keyHandlers}
              onPaste={handlePaste}
            />
          </div>

          {isAtMaxImages ? (
            <Tooltip label={`Maximum ${maxImages} images attached`}>
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
                className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-primary hover:bg-primary/10 transition-colors duration-150 disabled:opacity-40 disabled:cursor-default cursor-pointer"
              >
                {CAMERA_ICON}
              </button>
            </Tooltip>
          )}

          {modelPickerAvailable && onModelPickerToggle && (
            <Tooltip label="Choose model">
              <ModelPicker
                onClick={onModelPickerToggle}
                disabled={isBusy}
                isOpen={isModelPickerOpen ?? false}
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
      </motion.div>
    </div>
  );
}
