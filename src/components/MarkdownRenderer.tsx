import React, { memo, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Streamdown, type MathPlugin } from 'streamdown';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import { escapeCurrencyDollars } from '../utils/escapeCurrencyDollars';
import type { SearchResultPreview } from '../types/search';

interface MarkdownRendererProps {
  content: string;
  className?: string;
  /** Whether this content is actively being streamed from the LLM. */
  isStreaming?: boolean;
  /**
   * Search sources for inline `[N]` citation linking. When present, plain
   * `[N]` markers in the content are turned into citation chip anchors that
   * carry `data-citation`/`data-url` for the hover/click delegation in
   * ChatBubble. Rendered entirely through React (never by mutating the
   * DOM Streamdown owns): post-render DOM surgery detaches React's own
   * nodes mid-stream and crashes WebKit with `NotFoundError`, unmounting
   * the whole tree.
   */
  citationSources?: SearchResultPreview[];
}

/** Matches a bare numbered citation marker the search writer emits. */
const CITATION_MARKER_RE = /\[(\d+)\]/g;

/**
 * Rewrites plain-text `[N]` markers into markdown links targeting the
 * matching source URL so the citation renders through the normal markdown
 * pipeline. Markers with no matching source are left untouched. Spaces and
 * parentheses in the URL are percent-encoded so they cannot terminate the
 * `(destination)` and break the surrounding markdown.
 */
export function linkifyCitations(
  content: string,
  sources: SearchResultPreview[],
): string {
  return content.replace(CITATION_MARKER_RE, (marker, digits: string) => {
    const source = sources[Number.parseInt(digits, 10) - 1];
    if (!source) return marker;
    const url = source.url
      .replace(/ /g, '%20')
      .replace(/\(/g, '%28')
      .replace(/\)/g, '%29');
    return `[\\[${digits}\\]](${url})`;
  });
}

const mathPlugin: MathPlugin = {
  name: 'katex',
  type: 'math',
  remarkPlugin: remarkMath,
  rehypePlugin: rehypeKatex,
};

/**
 * Flattens a rendered anchor's React children to plain text so a citation
 * marker can be recognised. Streamdown hands a link's text as a string or a
 * single-element array; anything richer (a link wrapping bold/code) is not a
 * citation marker, so it collapses to the empty string and the anchor renders
 * normally. Exported for direct unit coverage of each child shape.
 */
export function childText(children: React.ReactNode): string {
  if (typeof children === 'string') return children;
  if (Array.isArray(children)) return children.map(childText).join('');
  return '';
}

/** Props Streamdown hands its anchor renderer (`node` is the hast node). */
type AnchorRenderProps = React.AnchorHTMLAttributes<HTMLAnchorElement> & {
  node?: unknown;
};

/**
 * Renders one anchor from Streamdown's markdown output. A `[N]` link produced
 * by `linkifyCitations` becomes a citation chip carrying the class and data
 * attributes ChatBubble's hover/click delegation expects, with no `href` so
 * the generic anchor-click handler leaves it to ChatBubble (a live `href`
 * would double-fire `open_url`). Any other anchor passes through unchanged.
 */
function renderCitationAnchor(
  sources: SearchResultPreview[],
  { node, children, ...rest }: AnchorRenderProps,
): React.ReactElement {
  void node;
  const marker = /^\[(\d+)\]$/.exec(childText(children));
  const source = marker
    ? sources[Number.parseInt(marker[1], 10) - 1]
    : undefined;
  if (marker && source) {
    return (
      <a
        className="citation-link"
        data-citation={marker[1]}
        data-url={source.url}
        title={source.title || source.url}
        role="button"
      >
        {children}
      </a>
    );
  }
  return <a {...rest}>{children}</a>;
}

/**
 * Builds the Streamdown `components` override that turns citation markers into
 * chips. Kept at module scope (not inside the component) so the anchor renderer
 * is not re-created per render and does not trip the nested-component lint.
 */
function citationComponents(sources: SearchResultPreview[]) {
  return {
    a: (props: AnchorRenderProps) => renderCitationAnchor(sources, props),
  };
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
 * Inline citations: when `citationSources` is provided, plain `[N]` markers
 * are rewritten to markdown links (`linkifyCitations`) and an `a` component
 * override renders each as a citation chip. This runs entirely through the
 * markdown pipeline: the previous implementation post-processed the rendered
 * DOM in a `useEffect`, which detached nodes React still owned and crashed
 * WebKit (`NotFoundError`) mid-stream, unmounting the whole tree.
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
  function MarkdownRenderer({
    content,
    className = '',
    isStreaming = false,
    citationSources,
  }) {
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

    // Citation-chip anchor override, only when this turn carries sources.
    const components = useMemo(
      () =>
        citationSources && citationSources.length > 0
          ? citationComponents(citationSources)
          : undefined,
      [citationSources],
    );

    const renderedContent = useMemo(
      () =>
        citationSources && citationSources.length > 0
          ? linkifyCitations(content, citationSources)
          : content,
      [content, citationSources],
    );

    return (
      <span className={`markdown-body ${className}`} onClick={handleClick}>
        <Streamdown
          mode={isStreaming ? 'streaming' : 'static'}
          components={components}
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
          {escapeCurrencyDollars(renderedContent)}
        </Streamdown>
      </span>
    );
  },
);
