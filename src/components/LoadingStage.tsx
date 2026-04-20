import type { ReactNode } from 'react';
import { TypingIndicator } from './TypingIndicator';

interface LoadingStageProps {
  /**
   * Optional label shown next to the 9-dot indicator. When `null` or
   * `undefined`, only the dots render (no label text). The label animates
   * with the shared shimmer sweep used for search/thinking stages.
   */
  label?: string | null;
  /**
   * Compact layout used in secondary UI surfaces like the search trace
   * disclosure header, where the stage label should stay supportive rather
   * than dominate the response body.
   */
  compact?: boolean;
  /**
   * Optional inline prefix rendered immediately before the label text. Used by
   * disclosure-style headers where the chevron should read as part of the
   * title rather than as a separate trailing control.
   */
  labelPrefix?: ReactNode;
}

/**
 * Shared loading row: 9-dot `TypingIndicator` on the left, an optional
 * shimmer-animated label on the right. Used as the stable loading state for
 * both `/search` stages ("Classifying query", "Searching the web") and the
 * `/think` flow ("Thinking..."). Keeps the visual pattern consistent across
 * long-running backend processes.
 *
 * The outer wrapper is a `span` with `inline-flex` so the component can sit
 * inside a `<button>` (e.g. the thinking-block disclosure row) without
 * producing invalid block-inside-button HTML.
 */
export function LoadingStage({
  label,
  compact = false,
  labelPrefix,
}: LoadingStageProps) {
  return (
    <span className={`inline-flex items-center ${compact ? 'gap-2' : 'gap-3'}`}>
      <span className="shrink-0">
        <TypingIndicator />
      </span>
      {label ? (
        <span
          data-testid="loading-stage-title"
          className={`inline-flex min-w-0 items-center ${compact ? 'gap-1 text-[11px] leading-none' : 'gap-1.5 text-xs'}`}
        >
          {labelPrefix ? (
            <span
              data-testid="loading-label-prefix"
              className="inline-flex shrink-0 items-center"
            >
              {labelPrefix}
            </span>
          ) : null}
          <span
            data-testid="loading-label"
            data-label={label}
            className="loading-label min-w-0"
          >
            {label}
          </span>
        </span>
      ) : null}
    </span>
  );
}
