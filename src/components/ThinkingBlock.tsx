import { useState, useEffect, useRef } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { MarkdownRenderer } from './MarkdownRenderer';

export interface ThinkingBlockProps {
  thinkingContent: string;
  isThinking: boolean;
  durationMs?: number;
}

const THINKING_TEXT = 'Thinking...';
/** How many milliseconds between position advances along the text. */
const SWEEP_STEP_MS = 80;
/** How many characters on each side of the peak are partially illuminated. */
const SPREAD = 3;
/** Base opacity for characters outside the glow zone. */
const BASE_OPACITY = 0.35;

/**
 * Computes a smooth bell-curve opacity for each character based on its
 * distance from the current sweep position. Characters near the peak
 * glow brightly while neighbors taper off smoothly, creating a flowing
 * wave of light across the text.
 */
function getCharOpacity(charIndex: number, sweepPos: number): number {
  const len = THINKING_TEXT.length;
  // Shortest distance on the circular loop
  const raw = Math.abs(charIndex - sweepPos);
  const dist = Math.min(raw, len - raw);
  if (dist > SPREAD) return BASE_OPACITY;
  // Cosine falloff: 1.0 at center, tapering to BASE_OPACITY at edge
  const t = dist / SPREAD;
  return BASE_OPACITY + (1 - BASE_OPACITY) * Math.cos((t * Math.PI) / 2);
}

/**
 * Animated "Thinking..." label with a smooth sweeping glow.
 * A soft highlight wave travels across the text, illuminating nearby
 * characters with a cosine falloff for a premium, fluid feel.
 */
function ThinkingLabel() {
  const [sweepPos, setSweepPos] = useState(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    timerRef.current = setInterval(() => {
      setSweepPos((prev) => (prev + 1) % THINKING_TEXT.length);
    }, SWEEP_STEP_MS);
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, []);

  return (
    <span className="text-sm text-text-secondary" data-testid="thinking-label">
      {THINKING_TEXT.split('').map((char, i) => (
        <span
          key={i}
          className="transition-opacity duration-150 ease-in-out"
          style={{ opacity: getCharOpacity(i, sweepPos) }}
        >
          {char}
        </span>
      ))}
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
