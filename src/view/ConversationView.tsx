import { motion, AnimatePresence, useMotionValue, useSpring } from 'framer-motion';
import { useRef, useCallback, useEffect, useLayoutEffect, useState } from 'react';
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
}

/**
 * Renders the expanded chat history area of the Thuki application.
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
}: ConversationViewProps) {
  const scrollContainerRef = useRef<HTMLDivElement>(null);

  /**
   * Tracks whether the user is "pinned" near the bottom of the scroll
   * container. When pinned, new streaming tokens auto-scroll the view.
   * When the user manually scrolls up, pinning is released so they can
   * read older messages undisturbed.
   */
  const isUserNearBottomRef = useRef(true);

  /** Threshold in pixels — if within this distance of the bottom, consider "pinned". */
  const NEAR_BOTTOM_THRESHOLD = 60;

  /**
   * Scroll event handler — updates the pinned state based on the user's
   * current scroll position relative to the bottom of the container.
   */
  const handleScroll = useCallback(() => {
    const container = scrollContainerRef.current;
    /* v8 ignore start */
    if (!container) return; // defensive null guard, ref always populated when handler fires
    /* v8 ignore stop */
    const { scrollTop, scrollHeight, clientHeight } = container;
    isUserNearBottomRef.current =
      scrollHeight - scrollTop - clientHeight < NEAR_BOTTOM_THRESHOLD;
  }, []);

  /**
   * Auto-scroll the chat container to the bottom — but only when the user
   * is pinned near the bottom. This prevents yanking the user back down
   * if they are reading old messages while generation occurs.
   */
  useEffect(() => {
    if (!isUserNearBottomRef.current) return;

    const container = scrollContainerRef.current;
    /* v8 ignore start */
    if (!container) return; // defensive null guard, ref always populated when effect fires
    /* v8 ignore stop */

    const raf = requestAnimationFrame(() => {
      container.scrollTop = container.scrollHeight;
    });

    return () => cancelAnimationFrame(raf);
  }, [messages, streamingContent]);

  /**
   * Re-pin to bottom whenever the user sends a new message.
   * Ensures the view snaps to the latest query immediately.
   */
  useEffect(() => {
    if (messages.length > 0) {
      const lastMsg = messages[messages.length - 1];
      if (lastMsg.role === 'user') {
        isUserNearBottomRef.current = true;
      }
    }
  }, [messages]);

  /**
   * Spring-driven height that smoothly tracks the growing content.
   *
   * Framer Motion's `height: 'auto'` measures the target once at mount and
   * snaps when the spring finishes — causing a visible jump when streaming
   * tokens grow the content beyond the initial measurement. Instead, we
   * temporarily flip the element to `height: auto` inside a `useLayoutEffect`
   * (before the browser paints), measure the natural height, restore the
   * spring value, and feed the measurement to a spring. The user never sees
   * the temporary auto state. The spring smoothly chases the growing content.
   *
   * Capped at `MAX_CONVERSATION_HEIGHT` so the flex chain stays intact and
   * the scroll container can scroll when content exceeds the available space.
   */
  const motionRef = useRef<HTMLDivElement>(null);
  const [targetHeight, setTargetHeight] = useState(0);

  /** Cap so the spring settles at the available space and the scroll container takes over. */
  const MAX_CONVERSATION_HEIGHT = 600;

  /* v8 ignore start -- useLayoutEffect + DOM measurement requires a real browser */
  useLayoutEffect(() => {
    const node = motionRef.current;
    if (!node) return;
    // Temporarily remove the spring-driven height so the browser can lay out
    // children at their natural sizes. This runs before paint, so no flicker.
    const prev = node.style.height;
    node.style.height = 'auto';
    const naturalH = Math.ceil(node.getBoundingClientRect().height);
    node.style.height = prev;
    setTargetHeight(Math.min(naturalH, MAX_CONVERSATION_HEIGHT));
  }, [messages, streamingContent, isGenerating, error]);
  /* v8 ignore stop */

  const heightMotion = useMotionValue(0);
  const heightSpring = useSpring(heightMotion, { stiffness: 300, damping: 30 });

  useLayoutEffect(() => {
    heightMotion.set(targetHeight);
  }, [targetHeight, heightMotion]);

  return (
    <motion.div
      ref={motionRef}
      key="chat-area"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ opacity: { duration: 0.2 } }}
      style={{ height: heightSpring, overflow: 'hidden' }}
      className="chat-area min-h-0 flex flex-col"
    >
      <WindowControls onClose={onClose} />

      <div
        ref={scrollContainerRef}
        onScroll={handleScroll}
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
