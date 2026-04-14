import { motion } from 'framer-motion';
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
  /** Whether the underlying LLM engine is currently generating a response. */
  isGenerating: boolean;
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
  /**
   * Called when the new-conversation (+) button is clicked.
   * Omit to hide the button.
   */
  onNewConversation?: () => void;
  /** Called when the user clicks a thumbnail to preview it. */
  onImagePreview?: (path: string) => void;
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
  isGenerating,
  onClose,
  onSave,
  isSaved,
  canSave,
  onHistoryOpen,
  onNewConversation,
  onImagePreview,
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
   * Re-enable auto-scroll only when the user sends a new message.
   * Sending a message is an explicit "I want to see the response" action.
   * When an assistant message is finalized (streaming completes), we preserve
   * the current scroll lock state so the user can keep reading where they are.
   */
  useEffect(() => {
    if (messages.length > prevMessagesLengthRef.current) {
      const newest = messages[messages.length - 1];
      if (newest?.role === 'user') {
        shouldAutoScrollRef.current = true;
      }
    }
    prevMessagesLengthRef.current = messages.length;
  }, [messages.length, messages]);

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
  }, [messages, isGenerating]);

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
        onNewConversation={onNewConversation}
        onHistoryOpen={onHistoryOpen}
      />

      <div
        ref={scrollContainerRef}
        className="chat-messages-scroll px-5 py-4 flex flex-col gap-3 flex-1 min-h-0 overflow-y-auto"
      >
        {messages.map((msg, i) => {
          const isLastAssistant =
            isGenerating &&
            i === messages.length - 1 &&
            msg.role === 'assistant';

          // Hide the empty assistant placeholder; the TypingIndicator
          // already covers this visual state. When thinking content is
          // present, render the bubble so the ThinkingBlock is visible.
          if (isLastAssistant && !msg.content && !msg.thinkingContent)
            return null;

          return (
            <ChatBubble
              key={msg.id}
              role={msg.role}
              content={msg.content}
              quotedText={msg.quotedText}
              index={i}
              isStreaming={isLastAssistant}
              imagePaths={msg.imagePaths}
              onImagePreview={onImagePreview}
              errorKind={msg.errorKind}
              thinkingContent={msg.thinkingContent}
              isThinking={
                isLastAssistant && !msg.content && !!msg.thinkingContent
              }
            />
          );
        })}

        {/* Typing indicator (pulsing dots) shown before first token arrives */}
        {isGenerating &&
        messages[messages.length - 1]?.role === 'assistant' &&
        !messages[messages.length - 1]?.content &&
        !messages[messages.length - 1]?.thinkingContent ? (
          <TypingIndicator />
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
