import { useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { MarkdownRenderer } from './MarkdownRenderer';
import { LoadingStage } from './LoadingStage';

export interface ReasoningBlockProps {
  thinkingContent?: string;
  isThinking: boolean;
  isPending?: boolean;
  /**
   * Cue shown next to the dots while `isPending` is true. `null`/`undefined`
   * renders bare dots with no text - the caller (the engine-loading label,
   * shared with plain turns) is the single source of truth for this copy;
   * this component has no cue of its own.
   */
  pendingLabel?: string | null;
}

const REASONING_LABEL = 'Reasoning...';

/**
 * Classes shared byte-for-byte between the pending row and the clickable
 * summary row (both while `isThinking` and once done), so all three are
 * pixel-identical by construction rather than by hand-matching independent
 * class lists. Only the element tag and interactive attributes differ.
 */
const SUMMARY_ROW_CLASS = 'flex items-center gap-2 p-0 text-left w-full';

/**
 * Collapsible reasoning section rendered above an AI response.
 *
 * While `isThinking` is true the block shows an animated "Reasoning..." label.
 * When reasoning completes the label changes to "Reasoning". The user
 * can click to toggle expansion at any time to see the reasoning content.
 */
export function ReasoningBlock({
  thinkingContent,
  isThinking,
  isPending = false,
  pendingLabel = null,
}: ReasoningBlockProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const hasThinkingContent = Boolean(thinkingContent?.trim());

  if (!hasThinkingContent && !isPending) return null;

  if (isPending) {
    // Invisible placeholder, not a real chevron: there's nothing to expand
    // yet, so no click affordance should render or be announced. It exists
    // purely to reserve the exact width the real chevron occupies once
    // thinking starts, using the identical classes/markup that state uses
    // below, so the label lands at the same x position in both.
    const chevronSpacer = (
      <span
        data-testid="reasoning-chevron"
        aria-hidden="true"
        className="loading-label inline-block shrink-0 text-[9px] transition-transform duration-150 opacity-0"
        style={{ transform: 'rotate(90deg)' }}
      >
        &#9650;
      </span>
    );
    return (
      <div data-testid="reasoning-block" className="mb-2">
        <div data-testid="reasoning-pending" className={SUMMARY_ROW_CLASS}>
          <span className="inline-flex min-w-0">
            <LoadingStage label={pendingLabel} labelPrefix={chevronSpacer} />
          </span>
        </div>
      </div>
    );
  }

  // Strip "Thinking Process:" label that Gemma4 prepends to thinking tokens
  const displayContent = thinkingContent!
    .replace(/^\s*Thinking Process[:\s]*\n*/i, '')
    .trimStart();
  const summaryLabel = isThinking ? REASONING_LABEL : 'Reasoning';
  const chevron = (
    <span
      data-testid="reasoning-chevron"
      className="loading-label inline-block shrink-0 text-[9px] transition-transform duration-150"
      style={{
        transform: isExpanded ? 'rotate(180deg)' : 'rotate(90deg)',
      }}
    >
      &#9650;
    </span>
  );

  return (
    <div data-testid="reasoning-block" className="mb-2">
      {/* Clickable summary row: chevron + label. Same SUMMARY_ROW_CLASS the
          pending row above uses, plus the interactive-only extras. */}
      <button
        type="button"
        onClick={() => setIsExpanded((prev) => !prev)}
        className={`${SUMMARY_ROW_CLASS} cursor-pointer bg-transparent border-none`}
        aria-expanded={isExpanded}
        aria-label="Toggle reasoning details"
      >
        {isThinking ? (
          <span className="inline-flex min-w-0">
            <LoadingStage label={summaryLabel} labelPrefix={chevron} />
          </span>
        ) : (
          <>
            <span
              data-testid="reasoning-chevron"
              className="inline-block text-[9px] text-text-secondary/55 transition-transform duration-150"
              style={{
                transform: isExpanded ? 'rotate(180deg)' : 'rotate(90deg)',
              }}
            >
              &#9650;
            </span>
            <span
              data-testid="reasoning-summary-label"
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
