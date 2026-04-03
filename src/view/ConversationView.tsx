import { motion, AnimatePresence } from 'framer-motion';
import { useRef, useEffect } from 'react';
import { ChatBubble } from '../components/ChatBubble';
import { TypingIndicator } from '../components/TypingIndicator';
import { WindowControls } from '../components/WindowControls';
import type { Message } from '../hooks/useOllama';

/**
 * Props for the ConversationView component.
 * Describes the state of the active chat session.
 */
interface ConversationViewProps {
  /** Array of completed messages in the conversation. */
  messages: Message[];
  /** The actively streaming content for the current assistant response. */
  streamingContent: string;
  /** Whether the underlying LLM engine is currently generating a response. */
  isGenerating: boolean;
  /** Any active error message to display to the user. */
  error: string | null;
  /** Callback fired when the user requests to close the overlay. */
  onClose: () => void;
  /**
   * Called when the bookmark icon is clicked to persist the conversation.
   * Omit to hide the save button.
   */
  onSave?: () => void;
  /**
   * True once the conversation has been saved. Renders the bookmark filled
   * and disables the button.
   */
  isSaved?: boolean;
  /**
   * True when there is at least one completed AI response available to save.
   * Controls whether the bookmark button is interactive.
   */
  canSave?: boolean;
  /**
   * Called when the "History ▾" button is clicked.
   * Omit to hide the history button.
   */
  onHistoryOpen?: () => void;
}

/**
 * Renders the expanded chat history area of the Thuki application.
 *
 * Always fills its parent's available height (flex-1) so the window expands
 * to the morphing container's max-h-[600px] immediately — no dynamic height
 * calculation. Content beyond the visible area scrolls inside the flex child.
 *
 * Encapsulates the scrolling logic ("smart auto-scroll") that pins the view
 * to new arriving tokens unless the user intercedes by scrolling up manually.
 */
export function ConversationView({
  messages,
  streamingContent,
  isGenerating,
  error,
  onClose,
  onSave,
  isSaved,
  canSave,
  onHistoryOpen,
}: ConversationViewProps) {
  const scrollContainerRef = useRef<HTMLDivElement>(null);

  /** Threshold in pixels — if within this distance of the bottom, consider "near bottom". */
  const NEAR_BOTTOM_THRESHOLD = 60;

  /**
   * Tracks whether the view should auto-scroll to follow new content.
   *
   * Only **user-initiated** wheel events can disable auto-scroll (set to
   * `false` on upward scroll). This avoids false negatives from layout-induced
   * scroll events.
   *
   * Re-enabled when:
   * - The user scrolls back near the bottom (wheel deltaY > 0).
   * - A new message is added (`messages.length` changes).
   */
  const shouldAutoScrollRef = useRef(true);
  const prevMessagesLengthRef = useRef(0);

  /**
   * Wheel listener — the only mechanism that can disable auto-scroll.
   * Wheel events are exclusively user-initiated (never fired by programmatic
   * scrollTop changes or layout reflows), making them a reliable signal for
   * "user scrolled up to read earlier content."
   */
  useEffect(() => {
    const container = scrollContainerRef.current;
    /* v8 ignore start */
    if (!container) return;
    /* v8 ignore stop */

    const onWheel = (e: WheelEvent) => {
      if (e.deltaY < 0) {
        shouldAutoScrollRef.current = false;
      } else if (e.deltaY > 0) {
        requestAnimationFrame(() => {
          const { scrollTop, scrollHeight, clientHeight } = container;
          if (scrollHeight - scrollTop - clientHeight < NEAR_BOTTOM_THRESHOLD) {
            shouldAutoScrollRef.current = true;
          }
        });
      }
    };

    container.addEventListener('wheel', onWheel, { passive: true });
    return () => container.removeEventListener('wheel', onWheel);
  }, []);

  /**
   * Re-enable auto-scroll whenever a new message is added. Sending a message
   * is an explicit "I want to see the response" action.
   */
  useEffect(() => {
    if (messages.length > prevMessagesLengthRef.current) {
      shouldAutoScrollRef.current = true;
    }
    prevMessagesLengthRef.current = messages.length;
  }, [messages.length]);

  /**
   * Auto-scroll the chat container to the bottom when new content arrives,
   * but only if the user hasn't manually scrolled up.
   */
  useEffect(() => {
    const container = scrollContainerRef.current;
    /* v8 ignore start */
    if (!container) return;
    /* v8 ignore stop */

    if (!shouldAutoScrollRef.current) return;

    const raf = requestAnimationFrame(() => {
      container.scrollTop = container.scrollHeight;
    });

    return () => cancelAnimationFrame(raf);
  }, [messages, streamingContent]);

  return (
    <motion.div
      key="chat-area"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ opacity: { duration: 0.2 } }}
      className="chat-area flex-1 min-h-0 flex flex-col"
    >
      <WindowControls
        onClose={onClose}
        onSave={onSave}
        isSaved={isSaved}
        canSave={canSave}
        onHistoryOpen={onHistoryOpen}
      />

      <div
        ref={scrollContainerRef}
        className="chat-messages-scroll px-5 py-4 flex flex-col gap-3 flex-1 min-h-0 overflow-y-auto"
      >
        {messages.map((msg, i) => (
          <ChatBubble
            key={msg.id}
            role={msg.role}
            content={msg.content}
            quotedText={msg.quotedText}
            index={i}
          />
        ))}

        {/* Live-updating streaming bubble */}
        {streamingContent ? (
          <ChatBubble
            key="streaming"
            role="assistant"
            content={streamingContent}
            index={messages.length}
            isStreaming
          />
        ) : null}

        {/* Typing indicator (pulsing dots) shown before first token arrives */}
        {isGenerating && !streamingContent ? <TypingIndicator /> : null}

        {/* Transient error banner */}
        {error ? (
          <AnimatePresence>
            <motion.div
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              className="flex w-full justify-start mt-2"
            >
              <p className="text-red-400 text-xs px-4 py-2.5 rounded-2xl rounded-bl-md bg-red-950/30 border border-red-900/50 max-w-[80%]">
                {error}
              </p>
            </motion.div>
          </AnimatePresence>
        ) : null}
      </div>

      <motion.div
        initial={{ opacity: 0, scaleX: 0 }}
        animate={{ opacity: 1, scaleX: 1 }}
        transition={{
          type: 'spring',
          stiffness: 300,
          damping: 20,
          delay: 0.15,
        }}
        className="h-px shrink-0 bg-surface-border origin-center"
      />
    </motion.div>
  );
}
