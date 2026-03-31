import React, { useMemo } from 'react';
import DOMPurify from 'dompurify';
import { marked } from 'marked';

interface MarkdownRendererProps {
  content: string;
  className?: string;
}

/**
 * Safely renders markdown content by parsing it to HTML and rigorously sanitizing
 * the output using DOMPurify to prevent XSS attacks. Ensures the LLM cannot execute
 * arbitrary JS in the Tauri Webview.
 *
 * @param props Content string and optional CSS class names.
 * @returns Sanitized, rendered HTML within a span.
 */
export const MarkdownRenderer: React.FC<MarkdownRendererProps> = ({
  content,
  className = '',
}) => {
  const safeHtml = useMemo(() => {
    if (!content) return '';
    try {
      // Parse markdown synchronously
      const rawHtml = marked.parse(content, { async: false }) as string;
      // Sanitize the HTML to prevent XSS
      return DOMPurify.sanitize(rawHtml);
    /* v8 ignore start */
    } catch (e) { // marked.parse() cannot throw in the ESM test environment
      console.error('Markdown rendering error', e);
      return '<i>Error rendering text</i>';
    }
    /* v8 ignore stop */
  }, [content]);

  return (
    <span
      className={`markdown-body ${className}`}
      dangerouslySetInnerHTML={{ __html: safeHtml }}
    />
  );
};
