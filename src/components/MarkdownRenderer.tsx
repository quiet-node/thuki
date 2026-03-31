import React, { memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

interface MarkdownRendererProps {
  content: string;
  className?: string;
}

/** Remark plugins applied to every render. Stable reference prevents re-initialization. */
const remarkPlugins = [remarkGfm];

/**
 * Renders markdown content as React elements using react-markdown.
 *
 * Secure by design: react-markdown converts markdown to a React element tree
 * via `createElement`, never using `dangerouslySetInnerHTML`. Raw HTML in
 * markdown source is stripped by default, preventing XSS without an external
 * sanitizer. Supports GitHub Flavored Markdown (tables, strikethrough, task
 * lists, autolinks) via the remark-gfm plugin.
 *
 * Memoized to skip re-renders when props are unchanged, which matters during
 * LLM token streaming where sibling bubbles would otherwise re-render on
 * every new token.
 */
export const MarkdownRenderer: React.FC<MarkdownRendererProps> = memo(
  function MarkdownRenderer({ content, className = '' }) {
    return (
      <span className={`markdown-body ${className}`}>
        <ReactMarkdown remarkPlugins={remarkPlugins}>{content}</ReactMarkdown>
      </span>
    );
  },
);
