import { motion } from 'framer-motion';
import { MarkdownRenderer } from './MarkdownRenderer';
import { CopyButton } from './CopyButton';

interface ChatBubbleProps {
  /** The message role determines alignment and color treatment. */
  role: 'user' | 'assistant';
  /** The message content to render. AI messages support markdown. */
  content: string;
  /** Stagger index for orchestrated entrance choreography. */
  index: number;
  /** Selected text from the host app that was quoted alongside this message, if any. */
  quotedText?: string;
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
 * A fixed 24px action bar below the bubble always reserves space for the copy
 * button, which fades in on hover — no layout shift.
 *
 * @param props Chat bubble properties including role, content, and stagger index.
 */
export function ChatBubble({
  role,
  content,
  index,
  quotedText,
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
      {/* group wrapper: owns max-width, stacks bubble + action bar, enables hover */}
      <div className="group flex flex-col max-w-[80%]">
        <div
          className={`chat-bubble relative px-4 py-2.5 text-sm leading-relaxed select-text ${
            isUser
              ? 'chat-bubble-user rounded-2xl rounded-br-md'
              : 'chat-bubble-ai rounded-2xl rounded-bl-md'
          }`}
        >
          {isUser ? (
            <>
              {quotedText && (
                <p className="border-l-2 border-white/40 pl-2 mb-2 italic text-xs text-white/60 line-clamp-2">
                  {quotedText.replace(/\s+/g, ' ').trim()}
                </p>
              )}
              <span className="text-white/95 font-medium">{content}</span>
            </>
          ) : (
            <MarkdownRenderer content={content} />
          )}
        </div>

        {/* Action bar — always 24px tall so layout never shifts on hover */}
        <div className="h-6 flex items-center px-1">
          <CopyButton content={content} align={isUser ? 'right' : 'left'} />
        </div>
      </div>
    </motion.div>
  );
}
