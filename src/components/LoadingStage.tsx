import { TypingIndicator } from './TypingIndicator';

interface LoadingStageProps {
  /**
   * Optional label shown next to the 9-dot indicator. When `null` or
   * `undefined`, only the dots render (no label text). The label animates
   * with the shared shimmer sweep used for search/thinking stages.
   */
  label?: string | null;
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
export function LoadingStage({ label }: LoadingStageProps) {
  return (
    <span className="inline-flex items-center gap-3">
      <span className="shrink-0">
        <TypingIndicator />
      </span>
      {label ? (
        <span
          data-testid="loading-label"
          data-label={label}
          className="loading-label text-xs"
        >
          {label}
        </span>
      ) : null}
    </span>
  );
}
