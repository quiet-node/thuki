import { motion } from 'framer-motion';
import type React from 'react';
import { useCallback, useState } from 'react';
import { formatQuotedText } from '../utils/formatQuote';
import { quote } from '../config';
import { ImageThumbnails } from '../components/ImageThumbnails';

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

/**
 * Props for the AskBarView component.
 */
/** Maximum number of images allowed per message. */
const MAX_IMAGES = 3;

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
  /**
   * Called when the compact history icon is clicked in ask-bar mode.
   * Omit to hide the history icon entirely.
   */
  onHistoryOpen?: () => void;
  /** Absolute file paths of currently attached images. */
  attachedImages: string[];
  /** Called when the user pastes or drops image files. */
  onImagesAttached: (paths: string[]) => void;
  /** Called when the user removes an attached image. */
  onImageRemove: (path: string) => void;
  /** Called when the user clicks a thumbnail to preview it. */
  onImagePreview: (path: string) => void;
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
  onHistoryOpen,
  attachedImages,
  onImagesAttached,
  onImageRemove,
  onImagePreview,
}: AskBarViewProps) {
  const canSubmit =
    (query.trim().length > 0 || attachedImages.length > 0) && !isGenerating;
  const [isDragOver, setIsDragOver] = useState(false);

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

  /** Extracts image files from a DataTransfer and forwards them for staging. */
  const processImageFiles = useCallback(
    (files: FileList | null) => {
      if (!files || isGenerating) return;
      const remaining = MAX_IMAGES - attachedImages.length;
      if (remaining <= 0) return;

      const imageFiles: File[] = [];
      for (let i = 0; i < files.length && imageFiles.length < remaining; i++) {
        if (files[i].type.startsWith('image/')) {
          imageFiles.push(files[i]);
        }
      }
      if (imageFiles.length === 0) return;

      const readPromises = imageFiles.map(
        (file) =>
          new Promise<ArrayBuffer>((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => resolve(reader.result as ArrayBuffer);
            /* v8 ignore start -- FileReader.onerror is a defensive callback that cannot fire in tests */
            reader.onerror = () => reject(reader.error);
            /* v8 ignore stop */
            reader.readAsArrayBuffer(file);
          }),
      );

      void Promise.all(readPromises).then((buffers) => {
        const byteArrays = buffers.map((buf) =>
          Array.from(new Uint8Array(buf)),
        );
        onImagesAttached(byteArrays as unknown as string[]);
      });
    },
    [isGenerating, attachedImages.length, onImagesAttached],
  );

  /** Handles clipboard paste — extracts image items from clipboardData. */
  const handlePaste = useCallback(
    (e: React.ClipboardEvent) => {
      const items = e.clipboardData?.items;
      if (!items || isGenerating) return;

      const remaining = MAX_IMAGES - attachedImages.length;
      if (remaining <= 0) return;

      const imageFiles: File[] = [];
      for (let i = 0; i < items.length && imageFiles.length < remaining; i++) {
        if (items[i].type.startsWith('image/')) {
          const file = items[i].getAsFile();
          if (file) imageFiles.push(file);
        }
      }

      if (imageFiles.length === 0) return;
      e.preventDefault();

      const readPromises = imageFiles.map(
        (file) =>
          new Promise<ArrayBuffer>((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => resolve(reader.result as ArrayBuffer);
            /* v8 ignore start -- FileReader.onerror is a defensive callback that cannot fire in tests */
            reader.onerror = () => reject(reader.error);
            /* v8 ignore stop */
            reader.readAsArrayBuffer(file);
          }),
      );

      void Promise.all(readPromises).then((buffers) => {
        const byteArrays = buffers.map((buf) =>
          Array.from(new Uint8Array(buf)),
        );
        onImagesAttached(byteArrays as unknown as string[]);
      });
    },
    [isGenerating, attachedImages.length, onImagesAttached],
  );

  const handleDragOver = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      if (!isGenerating) setIsDragOver(true);
    },
    [isGenerating],
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
            imagePaths={attachedImages}
            onPreview={onImagePreview}
            onRemove={onImageRemove}
            size={56}
          />
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

        {/* Compact history entry point — ask-bar mode only. In chat mode the
            history button lives in the ConversationView header. */}
        {!isChatMode && onHistoryOpen && (
          <button
            type="button"
            onClick={onHistoryOpen}
            aria-label="Open history"
            className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-text-primary hover:bg-white/8 transition-colors duration-150 cursor-pointer"
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
          {isGenerating ? (
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
  );
}
