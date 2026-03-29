import { motion } from 'framer-motion';
import { MarkdownRenderer } from './MarkdownRenderer';

interface ChatBubbleProps {
  /** The message role determines alignment and color treatment. */
  role: 'user' | 'assistant';
  /** The message content to render. AI messages support markdown. */
  content: string;
  /** Stagger index for orchestrated entrance choreography. */
  index: number;
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
 * Renders an iMessage-inspired chat bubble with role-based styling.
 *
 * User messages appear right-aligned with a warm gradient (#ff8d5c → #e67a3e).
 * AI messages appear left-aligned with a frosted glass surface that harmonizes
 * with the dark overlay aesthetic.
 *
 * @param props Chat bubble properties including role, content, and stagger index.
 */
export function ChatBubble({ role, content, index }: ChatBubbleProps) {
  const isUser = role === 'user';

  return (
    <motion.div
      variants={bubbleVariants}
      initial="hidden"
      animate="visible"
      transition={{ delay: index * 0.06 }}
      className={`flex w-full ${isUser ? 'justify-end' : 'justify-start'}`}
    >
      <div
        className={`chat-bubble relative max-w-[80%] px-4 py-2.5 text-sm leading-relaxed select-text ${
          isUser
            ? 'chat-bubble-user rounded-2xl rounded-br-md'
            : 'chat-bubble-ai rounded-2xl rounded-bl-md'
        }`}
      >
        {isUser ? (
          <span className="text-white/95 font-medium">{content}</span>
        ) : (
          <MarkdownRenderer content={content} />
        )}
      </div>
    </motion.div>
  );
}
