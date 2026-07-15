import { useState, type ReactNode } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { MarkdownRenderer } from './MarkdownRenderer';
import { RequestStatusStrip } from './RequestStatusStrip';
import { avatarColor, domainOf } from '../utils/domainAvatar';

/** Minimal source shape for the under-reasoning chip (url drives avatar). */
export interface ReasoningSourceChipItem {
  title: string;
  url: string;
}

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
  /**
   * Web sources handed off from the search strip (design D). When non-empty,
   * a compact count chip with domain avatars sits under the reasoning summary
   * so dual live strips never stack. Full list stays in the message footer.
   */
  searchSources?: ReasoningSourceChipItem[];
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
 * Compact sources chip under the reasoning summary after search demotes.
 * Caller only mounts this when `sources` is non-empty.
 *
 * @param sources - Search hits to summarize (count + up to 3 domain avatars).
 * @returns Chip node with avatars and a count label.
 */
function ReasoningSourcesChip({
  sources,
}: {
  sources: ReasoningSourceChipItem[];
}): ReactNode {
  const count = sources.length;
  const label = count === 1 ? '1 source' : `${count} sources`;
  return (
    <div
      data-testid="reasoning-sources-chip"
      className="mt-1 ml-[22px] inline-flex items-center gap-1.5 text-[11px] text-text-secondary/50"
      aria-label={label}
    >
      <span aria-hidden className="inline-flex items-center">
        {sources.slice(0, 3).map((src, i) => {
          const domain = domainOf(src.url);
          /* v8 ignore start */
          const letter = (domain[0] ?? '?').toUpperCase();
          /* v8 ignore stop */
          const bg = avatarColor(domain);
          return (
            <span
              key={src.url}
              className="shrink-0 h-4 w-4 rounded-full inline-flex items-center justify-center text-[8px] font-semibold text-white/90"
              style={{
                background: bg,
                border: '1.5px solid var(--avatar-ring, rgba(26,26,26,1))',
                marginLeft: i === 0 ? 0 : -5,
              }}
            >
              {letter}
            </span>
          );
        })}
      </span>
      <span data-testid="reasoning-sources-chip-label">{label}</span>
    </div>
  );
}

/**
 * Collapsible reasoning section rendered above an AI response.
 *
 * While `isThinking` is true the block shows an animated "Reasoning..." label.
 * When reasoning completes the label changes to "Reasoning". The user
 * can click to toggle expansion at any time to see the reasoning content.
 * Optional `searchSources` render a quiet chip under the summary (design D).
 */
export function ReasoningBlock({
  thinkingContent,
  isThinking,
  isPending = false,
  pendingLabel = null,
  searchSources,
}: ReasoningBlockProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const hasThinkingContent = Boolean(thinkingContent?.trim());
  /**
   * Chip only while reasoning is the live strip (pending or streaming).
   * Once done, the message footer owns source chips; keep the summary quiet.
   */
  const sourcesForChip =
    (isThinking || isPending) && searchSources && searchSources.length > 0
      ? searchSources
      : null;

  if (!hasThinkingContent && !isPending) return null;

  if (isPending) {
    // Invisible placeholder, not a real chevron: there's nothing to expand
    // yet, so no click affordance should render or be announced. It exists
    // purely to reserve the exact width the real chevron occupies once
    // thinking starts, using the identical classes/markup that state uses
    // below, so the label lands at the same x position in both.
    // Passed as RequestStatusStrip accessory so order is dots → chevron → label.
    const chevronSpacer = (
      <span
        data-testid="reasoning-chevron"
        aria-hidden="true"
        className="inline-block shrink-0 text-[9px] transition-transform duration-150 opacity-0"
        style={{ transform: 'rotate(90deg)' }}
      >
        &#9650;
      </span>
    );
    return (
      <div data-testid="reasoning-block" className="mb-2">
        <div data-testid="reasoning-pending" className={SUMMARY_ROW_CLASS}>
          <RequestStatusStrip label={pendingLabel} accessory={chevronSpacer} />
        </div>
        {sourcesForChip ? (
          <ReasoningSourcesChip sources={sourcesForChip} />
        ) : null}
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
      className="inline-block shrink-0 text-[9px] text-text-secondary/55 transition-transform duration-150"
      style={{
        transform: isExpanded ? 'rotate(180deg)' : 'rotate(90deg)',
      }}
    >
      &#9650;
    </span>
  );

  return (
    <div data-testid="reasoning-block" className="mb-2">
      {/* Live: chevron as strip accessory (dots → chevron → label).
          Done: chevron left of static title only (no dots). */}
      <button
        type="button"
        onClick={() => setIsExpanded((prev) => !prev)}
        className={`${SUMMARY_ROW_CLASS} cursor-pointer bg-transparent border-none`}
        aria-expanded={isExpanded}
        aria-label="Toggle reasoning details"
      >
        {isThinking ? (
          <RequestStatusStrip label={summaryLabel} accessory={chevron} />
        ) : (
          <>
            {chevron}
            <span
              data-testid="reasoning-summary-label"
              className="request-status-strip__title font-medium tracking-[0.01em] text-text-secondary/58"
            >
              {summaryLabel}
            </span>
          </>
        )}
      </button>
      {sourcesForChip ? (
        <ReasoningSourcesChip sources={sourcesForChip} />
      ) : null}

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
