import {
  motion,
  AnimatePresence,
  useMotionValue,
  useSpring,
} from 'framer-motion';
import { useRef, useEffect, useLayoutEffect, useState } from 'react';
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

  /** Threshold in pixels — if within this distance of the bottom, consider "near bottom". */
  const NEAR_BOTTOM_THRESHOLD = 60;

  /**
   * Tracks whether the view should auto-scroll to follow new content.
   *
   * Only **user-initiated** wheel events can disable auto-scroll (set to
   * `false` on upward scroll). This avoids false negatives from layout-induced
   * scroll events: the `useLayoutEffect` height-measurement cycle temporarily
   * sets `height: auto` on the spring-animated parent, which can clamp
   * `scrollTop` to 0 and fire a deferred scroll event that looks like the user
   * scrolled away from the bottom, even though they didn't.
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

  /* v8 ignore start -- useLayoutEffect + DOM measurement requires a real browser */
  useLayoutEffect(() => {
    const node = motionRef.current;
    if (!node) return;

    // Temporarily remove the spring-driven height so the browser lays out
    // children at their natural sizes. This runs before paint — no flicker.
    const prev = node.style.height;
    node.style.height = 'auto';
    const naturalH = Math.ceil(node.getBoundingClientRect().height);

    // Compute the actual flex-available space by reading the parent container's
    // clientHeight (capped by its max-h-[600px]) and subtracting sibling heights
    // (AskBarView). Without this, the spring would target 600px while the flex
    // algorithm renders the motion.div at ~548px — the mismatch makes the scroll
    // container 52px taller than the visible area, hiding the latest streamed
    // content behind the input bar.
    let maxAvailable = naturalH;
    const parent = node.parentElement;
    if (parent) {
      let siblingH = 0;
      for (const child of parent.children) {
        if (child !== node) {
          siblingH += (child as HTMLElement).offsetHeight;
        }
      }
      maxAvailable = parent.clientHeight - siblingH;
    }

    node.style.height = prev;
    // eslint-disable-next-line @eslint-react/set-state-in-effect -- intentional: measure DOM in useLayoutEffect before paint, then feed the spring
    setTargetHeight(Math.min(naturalH, Math.max(maxAvailable, 0)));
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
