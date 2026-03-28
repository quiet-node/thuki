import { motion } from 'framer-motion';

/**
 * Container orchestration — delays each child dot by 150ms for a wave effect.
 */
const containerVariants = {
  hidden: { opacity: 0 },
  visible: {
    opacity: 1,
    transition: { staggerChildren: 0.15, delayChildren: 0.1 },
  },
};

/**
 * Individual dot animation — gentle vertical bounce with infinite repetition.
 * Uses translateY (GPU-accelerated) instead of top/margin for performance.
 */
const dotVariants = {
  hidden: { opacity: 0, y: 4 },
  visible: {
    opacity: 1,
    y: [0, -5, 0],
    transition: {
      y: { repeat: Infinity, duration: 0.8, ease: 'easeInOut' as const },
      opacity: { duration: 0.2 },
    },
  },
};

/**
 * Renders a three-dot typing indicator styled to match the AI chat bubble.
 * Appears left-aligned in the chat flow to signal that the AI is composing a response.
 */
export function TypingIndicator() {
  return (
    <div className="flex w-full justify-start">
      <motion.div
        variants={containerVariants}
        initial="hidden"
        animate="visible"
        className="chat-bubble chat-bubble-ai rounded-2xl rounded-bl-md px-5 py-3 flex items-center gap-1.5"
      >
        {[0, 1, 2].map((i) => (
          <motion.span
            key={i}
            variants={dotVariants}
            className="w-2 h-2 rounded-full bg-primary/70"
          />
        ))}
      </motion.div>
    </div>
  );
}
