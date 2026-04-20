import { useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { MarkdownRenderer } from './MarkdownRenderer';
import { LoadingStage } from './LoadingStage';

export interface ThinkingBlockProps {
  thinkingContent?: string;
  isThinking: boolean;
  isPending?: boolean;
  pendingLabel?: string;
}

const THINKING_LABEL = 'Thinking...';
const PENDING_LABEL = 'Warming up...';

/**
 * Collapsible thinking/reasoning section rendered above an AI response.
 *
 * While `isThinking` is true the block shows an animated "Thinking..." label.
 * When thinking completes the label changes to "Thinking process". The user
 * can click to toggle expansion at any time to see the reasoning content.
 */
export function ThinkingBlock({
  thinkingContent,
  isThinking,
  isPending = false,
  pendingLabel = PENDING_LABEL,
}: ThinkingBlockProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const hasThinkingContent = Boolean(thinkingContent?.trim());

  if (!hasThinkingContent && !isPending) return null;

  if (isPending) {
    return (
      <div data-testid="thinking-block" className="mb-2">
        <div data-testid="thinking-pending" className="inline-flex min-w-0">
          <LoadingStage label={pendingLabel} />
        </div>
      </div>
    );
  }

  // Strip "Thinking Process:" label that Gemma4 prepends to thinking tokens
  const displayContent = thinkingContent!
    .replace(/^\s*Thinking Process[:\s]*\n*/i, '')
    .trimStart();
  const summaryLabel = isThinking ? THINKING_LABEL : 'Thought process';
  const chevron = (
    <span
      data-testid="thinking-chevron"
      className="loading-label inline-block shrink-0 text-[9px] transition-transform duration-150"
      style={{
        transform: isExpanded ? 'rotate(180deg)' : 'rotate(90deg)',
      }}
    >
      &#9650;
    </span>
  );

  return (
    <div data-testid="thinking-block" className="mb-2">
      {/* Clickable summary row: chevron + label */}
      <button
        type="button"
        onClick={() => setIsExpanded((prev) => !prev)}
        className="flex items-center gap-2 cursor-pointer bg-transparent border-none p-0 text-left w-full"
        aria-expanded={isExpanded}
        aria-label="Toggle thinking details"
      >
        {isThinking ? (
          <span className="inline-flex min-w-0">
            <LoadingStage label={summaryLabel} labelPrefix={chevron} />
          </span>
        ) : (
          <>
            <span
              data-testid="thinking-chevron"
              className="inline-block text-[9px] text-text-secondary/55 transition-transform duration-150"
              style={{
                transform: isExpanded ? 'rotate(180deg)' : 'rotate(90deg)',
              }}
            >
              &#9650;
            </span>
            <span
              data-testid="thinking-summary-label"
              className="text-[11px] font-medium tracking-[0.01em] text-text-secondary/58"
            >
              {summaryLabel}
            </span>
          </>
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
            <div data-testid="timeline-rail" className="mt-1.5">
              {/* Clock + thinking text row */}
              <div className="flex gap-2">
                {/* Timeline rail */}
                <div className="flex flex-col items-center flex-shrink-0">
                  {/* Clock icon (no circle border, just the icon) */}
                  <div
                    data-testid="clock-icon"
                    className={`w-5 h-5 flex items-center justify-center ${isThinking ? 'animate-spin' : ''}`}
                  >
                    <svg
                      width="16"
                      height="16"
                      viewBox="0 0 16 16"
                      fill="none"
                      className="text-text-secondary/50"
                    >
                      <circle
                        cx="8"
                        cy="8"
                        r="7"
                        stroke="currentColor"
                        strokeWidth="1.2"
                      />
                      <line
                        x1="8"
                        y1="4"
                        x2="8"
                        y2="8"
                        stroke="currentColor"
                        strokeWidth="1.2"
                        strokeLinecap="round"
                      />
                      <line
                        x1="8"
                        y1="8"
                        x2="11"
                        y2="8"
                        stroke="currentColor"
                        strokeWidth="1.2"
                        strokeLinecap="round"
                      />
                    </svg>
                  </div>
                  {/* Vertical line */}
                  <div className="w-px flex-1 bg-text-secondary/20 min-h-[20px]" />
                </div>

                {/* Thinking text rendered as markdown (normal text color) */}
                <div className="flex-1 text-sm select-text min-w-0 opacity-70">
                  <MarkdownRenderer
                    content={displayContent}
                    isStreaming={isThinking}
                  />
                </div>
              </div>

              {/* Done row (separate, with extra top spacing) */}
              {!isThinking && (
                <div className="flex items-center gap-1.5 mt-3">
                  <div
                    data-testid="checkmark-icon"
                    className="w-5 h-5 flex items-center justify-center"
                  >
                    <svg
                      width="16"
                      height="16"
                      viewBox="0 0 16 16"
                      fill="none"
                      className="text-text-secondary/50"
                    >
                      <circle
                        cx="8"
                        cy="8"
                        r="7"
                        stroke="currentColor"
                        strokeWidth="1.2"
                      />
                      <path
                        d="M5 8.5L7 10.5L11 5.5"
                        stroke="currentColor"
                        strokeWidth="1.3"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      />
                    </svg>
                  </div>
                  <span className="text-xs text-text-secondary/50">Done</span>
                </div>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
