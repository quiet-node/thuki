import React, { memo } from 'react';
import { Streamdown } from 'streamdown';

interface MarkdownRendererProps {
  content: string;
  className?: string;
  /** Whether this content is actively being streamed from the LLM. */
  isStreaming?: boolean;
}

/**
 * Renders markdown content using Streamdown, a streaming-aware markdown
 * renderer that handles incomplete syntax and memoizes completed blocks.
 *
 * During streaming, only the last in-progress block re-renders on each
 * token. Completed paragraphs are memoized and never reflow, eliminating
 * the bubble-height jitter caused by full markdown re-parsing.
 *
 * Memoized to skip re-renders when props are unchanged, which matters
 * during LLM token streaming where sibling bubbles would otherwise
 * re-render on every new token.
 */
export const MarkdownRenderer: React.FC<MarkdownRendererProps> = memo(
  function MarkdownRenderer({ content, className = '', isStreaming = false }) {
    return (
      <span className={`markdown-body ${className}`}>
        <Streamdown
          mode={isStreaming ? 'streaming' : 'static'}
          controls={false}
        >
          {content}
        </Streamdown>
      </span>
    );
  },
);
