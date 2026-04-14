import { useState, useEffect, useRef } from 'react';
import { AnimatePresence, motion } from 'framer-motion';

export interface ThinkingBlockProps {
  thinkingContent: string;
  isThinking: boolean;
  durationMs?: number;
}

/**
 * Extracts the first sentence from the thinking content to use as a summary.
 * Matches up to the first period, exclamation, question mark, or newline.
 * Falls back to the full string if no sentence-ending punctuation is found.
 */
function extractSummary(content: string): string {
  const match = content.match(/^(.+?)[.!?\n]/);
  return match ? match[1] : content;
}

/**
 * Formats the thinking duration into a human-readable string.
 */
function formatDuration(ms: number): string {
  if (ms < 1000) return 'less than a second';
  const seconds = Math.round(ms / 1000);
  return `${seconds} second${seconds === 1 ? '' : 's'}`;
}

/**
 * Collapsible thinking/reasoning section rendered above an AI response.
 *
 * While `isThinking` is true the block auto-expands, showing a timeline rail
 * with a spinning clock icon and streaming thinking tokens. When thinking
 * completes (isThinking transitions false) the block auto-collapses to a
 * one-line summary. The user can click to toggle expansion at any time.
 */
export function ThinkingBlock({
  thinkingContent,
  isThinking,
  durationMs,
}: ThinkingBlockProps) {
  const [isExpanded, setIsExpanded] = useState(isThinking);
  const prevIsThinkingRef = useRef(isThinking);

  /* eslint-disable @eslint-react/set-state-in-effect -- intentional: syncing
     expanded state with isThinking prop transitions (false->true expands,
     true->false collapses). This is a controlled prop-to-state sync. */
  useEffect(() => {
    if (isThinking && !prevIsThinkingRef.current) {
      setIsExpanded(true);
    } else if (!isThinking && prevIsThinkingRef.current) {
      setIsExpanded(false);
    }
    prevIsThinkingRef.current = isThinking;
  }, [isThinking]);
  /* eslint-enable @eslint-react/set-state-in-effect */

  if (!thinkingContent) return null;

  const summary = isThinking ? 'Thinking...' : extractSummary(thinkingContent);
  const durationText =
    !isThinking && durationMs !== undefined
      ? `Thought for ${formatDuration(durationMs)}`
      : null;

  return (
    <div data-testid="thinking-block" className="mb-2">
      {/* Clickable summary row */}
      <button
        type="button"
        onClick={() => setIsExpanded((prev) => !prev)}
        className="flex items-center gap-2 cursor-pointer bg-transparent border-none p-0 text-left w-full"
        aria-expanded={isExpanded}
        aria-label="Toggle thinking details"
      >
        {/* Chevron: ▲ rotated 90deg (collapsed, pointing right) or 180deg (expanded, pointing down) */}
        <span
          data-testid="thinking-chevron"
          className="text-[10px] text-text-secondary inline-block transition-transform duration-150"
          style={{ transform: isExpanded ? 'rotate(180deg)' : 'rotate(90deg)' }}
        >
          &#9650;
        </span>
        <span className="text-sm text-text-secondary italic">{summary}</span>
        {durationText && (
          <span className="text-xs text-text-secondary/50 ml-1">
            {durationText}
          </span>
        )}
      </button>

      {/* Expandable content */}
      <AnimatePresence>
        {isExpanded && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.2 }}
            className="overflow-hidden"
          >
            <div className="flex gap-3 mt-2">
              {/* Timeline rail */}
              <div
                data-testid="timeline-rail"
                className="flex flex-col items-center gap-1 py-1"
              >
                {/* Clock icon */}
                <div
                  data-testid="clock-icon"
                  className={`w-5 h-5 rounded-full border border-text-secondary/40 flex items-center justify-center ${isThinking ? 'animate-spin' : ''}`}
                >
                  <svg
                    width="12"
                    height="12"
                    viewBox="0 0 12 12"
                    fill="none"
                    className="text-text-secondary/70"
                  >
                    <circle
                      cx="6"
                      cy="6"
                      r="5"
                      stroke="currentColor"
                      strokeWidth="1.2"
                    />
                    <line
                      x1="6"
                      y1="3"
                      x2="6"
                      y2="6"
                      stroke="currentColor"
                      strokeWidth="1.2"
                      strokeLinecap="round"
                    />
                    <line
                      x1="6"
                      y1="6"
                      x2="8"
                      y2="6"
                      stroke="currentColor"
                      strokeWidth="1.2"
                      strokeLinecap="round"
                    />
                  </svg>
                </div>

                {/* Vertical line */}
                <div className="w-px flex-1 bg-text-secondary/20 min-h-[20px]" />

                {/* Checkmark icon (only when done) */}
                {!isThinking && (
                  <div
                    data-testid="checkmark-icon"
                    className="w-5 h-5 rounded-full border border-text-secondary/40 flex items-center justify-center"
                  >
                    <svg
                      width="10"
                      height="10"
                      viewBox="0 0 10 10"
                      fill="none"
                      className="text-text-secondary/70"
                    >
                      <path
                        d="M2 5.5L4 7.5L8 3"
                        stroke="currentColor"
                        strokeWidth="1.3"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      />
                    </svg>
                  </div>
                )}
              </div>

              {/* Thinking text */}
              <div className="flex-1 text-sm text-text-secondary/70 whitespace-pre-wrap py-1 select-text">
                {thinkingContent}
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
