import { useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { MarkdownRenderer } from './MarkdownRenderer';

export interface ThinkingBlockProps {
  thinkingContent: string;
  isThinking: boolean;
  durationMs?: number;
}

/**
 * Spinning starburst icon + italic "Thinking..." text.
 * Matches Claude.ai's thinking indicator: a rotating sparkle/starburst
 * icon with smooth CSS animation, paired with static dimmed italic text.
 */
function ThinkingLabel() {
  return (
    <span
      className="flex items-center gap-2 text-sm"
      data-testid="thinking-label"
    >
      <svg
        width="16"
        height="16"
        viewBox="0 0 16 16"
        fill="none"
        className="animate-spin text-primary"
        style={{ animationDuration: '1.5s' }}
      >
        {/* 8-ray starburst */}
        {[0, 45, 90, 135, 180, 225, 270, 315].map((angle) => (
          <line
            key={angle}
            x1="8"
            y1="8"
            x2="8"
            y2="2"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            transform={`rotate(${angle} 8 8)`}
            opacity={0.4 + (angle / 315) * 0.6}
          />
        ))}
      </svg>
      <span className="text-text-secondary/60 italic">Thinking...</span>
    </span>
  );
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
  const [isExpanded, setIsExpanded] = useState(false);

  if (!thinkingContent) return null;

  // Strip "Thinking Process:" label that Gemma4 prepends to thinking tokens
  const displayContent = thinkingContent
    .replace(/^\s*Thinking Process[:\s]*\n*/i, '')
    .trimStart();

  const durationText =
    !isThinking && durationMs !== undefined
      ? `Thought for ${formatDuration(durationMs)}`
      : null;

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
        {/* Chevron: ▲ rotated 90deg (collapsed, pointing right) or 180deg (expanded, pointing down) */}
        <span
          data-testid="thinking-chevron"
          className="text-[10px] text-text-secondary inline-block transition-transform duration-150"
          style={{ transform: isExpanded ? 'rotate(180deg)' : 'rotate(90deg)' }}
        >
          &#9650;
        </span>
        {isThinking ? (
          <ThinkingLabel />
        ) : (
          <span className="text-sm text-text-secondary/60">{durationText}</span>
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
