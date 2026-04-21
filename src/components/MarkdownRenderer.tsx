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
 * Security: Streamdown sanitizes rendered output via rehype-sanitize
 * (allowlist-based HTML element/attribute filtering) and rehype-harden
 * (blocks dangerous URL protocols). Raw HTML in markdown source is parsed
 * then sanitized, stripping script tags, event handlers, iframes, and
 * javascript: URLs. Link safety is disabled so links render as native
 * anchor elements with target="_blank" and rel="noopener noreferrer".
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
          /* Force dark syntax highlighting - the app has no .dark root class
             so the dual ["github-light","github-dark"] default resolves to
             github-light, giving code blocks a white background that clashes
             with the dark UI. */
          shikiTheme={['github-dark', 'github-dark']}
          /* Enable only the copy button on code blocks; disable download
             and all other block-level controls (table, mermaid). The parent
             ChatBubble provides its own CopyButton for the full message. */
          controls={{ code: { copy: true, download: false } }}
          /* Disable the link safety interstitial modal so links render as
             native <a> elements with href, target="_blank", and noopener.
             In a Tauri app the webview opens external links in the system
             browser, making the modal unnecessary friction. */
          linkSafety={{ enabled: false }}
        >
          {content}
        </Streamdown>
      </span>
    );
  },
);
