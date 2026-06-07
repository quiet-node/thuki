import React, { memo, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Streamdown, type MathPlugin } from 'streamdown';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import { escapeCurrencyDollars } from '../utils/escapeCurrencyDollars';

interface MarkdownRendererProps {
  content: string;
  className?: string;
  /** Whether this content is actively being streamed from the LLM. */
  isStreaming?: boolean;
}

const mathPlugin: MathPlugin = {
  name: 'katex',
  type: 'math',
  remarkPlugin: remarkMath,
  rehypePlugin: rehypeKatex,
};

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
 * KaTeX is appended after rehype-sanitize in Streamdown's pipeline and
 * is therefore not covered by the allowlist. XSS safety for math content
 * relies on KaTeX's own output escaping and the default trust:false setting,
 * which blocks arbitrary-HTML LaTeX macros such as \href and \htmlClass.
 *
 * External links: a bare `<a target="_blank">` does nothing in a Tauri
 * WKWebView (the webview will not navigate to the system browser on its
 * own), so anchor clicks are intercepted here and routed through the
 * `open_url` command, which opens the URL in the user's default browser.
 * `open_url` only accepts http/https, so non-web schemes are rejected
 * there. This is the same mechanism `TipBar` uses for its links.
 *
 * Currency disambiguation: `remark-math` would otherwise parse the text
 * between two currency dollars (e.g. "raise $1M ... reach $1M") as one
 * giant inline-math run. `escapeCurrencyDollars` escapes `$<digit>` before
 * the content reaches the parser so currency renders as plain text while
 * genuine `$x$` / `$$...$$` math is preserved. The `.katex-display`
 * overflow rule in App.css is the structural backstop that keeps any wide
 * math inside its own box.
 *
 * Memoized to skip re-renders when props are unchanged, which matters
 * during LLM token streaming where sibling bubbles would otherwise
 * re-render on every new token.
 */
export const MarkdownRenderer: React.FC<MarkdownRendererProps> = memo(
  function MarkdownRenderer({ content, className = '', isStreaming = false }) {
    /**
     * Delegated anchor-click handler. Streamdown renders links as native
     * `<a target="_blank">` elements, which a Tauri WKWebView silently
     * ignores. Intercept the click, hand the href to `open_url`, and let
     * the backend open it in the default browser. Non-anchor clicks (text,
     * code copy button, etc.) fall through untouched.
     */
    const handleClick = useCallback((e: React.MouseEvent<HTMLElement>) => {
      const anchor = (e.target as HTMLElement).closest('a');
      const href = anchor?.getAttribute('href');
      if (!href) return;
      e.preventDefault();
      void invoke('open_url', { url: href }).catch((err: unknown) => {
        console.error('failed to open link', href, err);
      });
    }, []);

    return (
      <span className={`markdown-body ${className}`} onClick={handleClick}>
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
          plugins={{ math: mathPlugin }}
        >
          {escapeCurrencyDollars(content)}
        </Streamdown>
      </span>
    );
  },
);
