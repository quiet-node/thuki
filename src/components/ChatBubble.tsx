import { motion } from 'framer-motion';
import { MarkdownRenderer } from './MarkdownRenderer';
import { CopyButton } from './CopyButton';
import { formatQuotedText } from '../utils/formatQuote';
import { quote } from '../config';

interface ChatBubbleProps {
  /** The message role determines alignment and color treatment. */
  role: 'user' | 'assistant';
  /** The message content to render. AI messages support markdown. */
  content: string;
  /** Stagger index for orchestrated entrance choreography. */
  index: number;
  /** Selected text from the host app that was quoted alongside this message, if any. */
  quotedText?: string;
  /** Whether this bubble is actively streaming content from the LLM. */
  isStreaming?: boolean;
}

/**
 * Framer Motion variants for individual chat bubbles.
 * Uses GPU-accelerated transforms (opacity, y, scale) for jank-free animation.
 * Spring physics provide natural, organic motion.
 */
const bubbleVariants = {
  hidden: { opacity: 0, y: 12, scale: 0.95 },
  visible: {
    opacity: 1,
    y: 0,
    scale: 1,
    transition: {
      type: 'spring' as const,
      stiffness: 380,
      damping: 26,
    },
  },
};

/**
 * Renders a chat message following industry-standard assistant UI conventions:
 *
 * - **User messages** — right-aligned bubble with warm gradient, quoted-text
 *   support, and an always-visible copy button below the bubble (right-aligned).
 * - **AI messages** — full-width plain text (no bubble), markdown-rendered, with
 *   an always-visible copy button below the text (left-aligned).
 *
 * Spring entrance animation is staggered by `index` to produce natural
 * choreography when multiple messages appear at once.
 */
export function ChatBubble({
  role,
  content,
  index,
  quotedText,
  isStreaming = false,
}: ChatBubbleProps) {
  const isUser = role === 'user';

  return (
    <motion.div
      variants={bubbleVariants}
      initial="hidden"
      animate="visible"
      transition={{ delay: index * 0.06 }}
      className={`flex w-full ${isUser ? 'justify-end' : 'justify-start'}`}
    >
      {isUser ? (
        /* User bubble — max-width capped, group enables legacy hover compat */
        <div className="group flex flex-col max-w-[80%]">
          <div className="chat-bubble chat-bubble-user relative px-4 py-2.5 text-sm leading-relaxed select-text rounded-2xl rounded-br-md">
            {quotedText && (
              <p className="border-l-2 border-white/40 pl-2 mb-2 italic text-xs text-white/60 whitespace-pre-wrap">
                {formatQuotedText(
                  quotedText,
                  quote.maxDisplayLines,
                  quote.maxDisplayChars,
                )}
              </p>
            )}
            <span className="text-white/95 font-medium">{content}</span>
          </div>
          <div className="h-6 flex items-center px-1">
            <CopyButton content={content} align="right" />
          </div>
        </div>
      ) : (
        /* AI plain text — full width, no bubble chrome */
        <div className="flex flex-col w-full">
          <div className="text-sm leading-relaxed select-text py-1">
            <MarkdownRenderer content={content} isStreaming={isStreaming} />
          </div>
          <div className="h-6 flex items-center">
            <CopyButton content={content} align="left" />
          </div>
        </div>
      )}
    </motion.div>
  );
}
