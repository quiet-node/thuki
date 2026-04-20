import { AnimatePresence, motion } from 'framer-motion';
import { useEffect, useRef, useState } from 'react';
import { MarkdownRenderer } from './MarkdownRenderer';
import { ErrorCard } from './ErrorCard';
import { CopyButton } from './CopyButton';
import { ImageThumbnails } from './ImageThumbnails';
import { ThinkingBlock } from './ThinkingBlock';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { formatQuotedText } from '../utils/formatQuote';
import { quote } from '../config';
import { COMMANDS, SCREEN_CAPTURE_PLACEHOLDER } from '../config/commands';
import { SearchWarningIcon } from './SearchWarningIcon';
import type { OllamaErrorKind } from '../hooks/useOllama';
import type {
  SearchResultPreview,
  SearchTraceStep,
  SearchWarning,
} from '../types/search';
import { SEARCH_WARNING_SEVERITY } from '../config/searchWarnings';
import { SearchTraceBlock } from './SearchTraceBlock';
import { SandboxSetupCard } from './SandboxSetupCard';

/**
 * Extracts a bare hostname from a URL for the sources footer. Strips the
 * leading `www.` prefix; falls back to the raw input if parsing fails.
 */
function domainOf(url: string): string {
  try {
    const host = new URL(url).hostname;
    return host.startsWith('www.') ? host.slice(4) : host;
  } catch {
    return url;
  }
}

/** Pseudo-random but deterministic 0–359 hue derived from a domain string.
 *  Lets every source get a distinct yet consistent color across renders. */
function domainHue(domain: string): number {
  let h = 0;
  for (let i = 0; i < domain.length; i++) {
    h = (h * 31 + domain.charCodeAt(i)) >>> 0;
  }
  return h % 360;
}

/**
 * Hand-picked palette of light, summery, slightly-cool gradient pairs for
 * letter avatars. Each entry is a two-stop linear-gradient suitable as the
 * `background` of a small circular badge. The domain hash selects one pair
 * deterministically so a given source always renders the same color.
 *
 * Picked to keep the palette pleasant and varied without clashing: no neon,
 * no muddy shades, all readable under white/90 letter text.
 */
const AVATAR_PALETTE: readonly string[] = [
  'linear-gradient(135deg, #ffb8a1, #ff8c77)', // peach coral
  'linear-gradient(135deg, #ffc3d5, #ff9cbd)', // cotton candy pink
  'linear-gradient(135deg, #a8d8ff, #7cb8ff)', // sky blue
  'linear-gradient(135deg, #a8e6cf, #7ecfb0)', // mint
  'linear-gradient(135deg, #c7b8ff, #a896ff)', // lavender
  'linear-gradient(135deg, #ffd3a5, #ffa978)', // sunset
  'linear-gradient(135deg, #9ee6d7, #6fc9b5)', // seafoam
  'linear-gradient(135deg, #fff0a5, #ffd96b)', // lemon sorbet
  'linear-gradient(135deg, #b8e0ff, #85b9ff)', // periwinkle
  'linear-gradient(135deg, #ffb6e1, #ff8cc8)', // bubblegum
  'linear-gradient(135deg, #c4eaa8, #9bd076)', // kiwi
  'linear-gradient(135deg, #ffc8a8, #ff9e78)', // papaya
] as const;

/** CSS gradient background for a letter avatar. Picks one of a hand-curated
 *  palette based on the domain hash for consistent but varied coloring. */
function avatarColor(domain: string): string {
  return AVATAR_PALETTE[domainHue(domain) % AVATAR_PALETTE.length];
}

/** Regex matching inline `[N]` citation markers in plain text. Captures the N. */
const CITATION_RE = /\[(\d+)\]/g;

/**
 * Walks the rendered answer DOM and replaces every plain-text `[N]` occurrence
 * with an anchor element that links to the matching source URL. Called inside
 * a `useEffect` that re-runs whenever the answer content or sources change so
 * streaming tokens stay in sync. Idempotent: already-wrapped `[N]` elements
 * carry a `data-citation` attribute and are skipped on subsequent passes.
 *
 * Security: only creates `<a>` elements with `textContent` — never inserts
 * arbitrary HTML — and caps the URL via the precomputed sources array. There
 * is no path for user input to reach this function other than through the
 * same SearXNG-validated URLs that populate the sources footer.
 */
function wrapCitations(
  root: HTMLElement,
  sources: SearchResultPreview[],
): void {
  // Collect text nodes first so we don't mutate the tree while iterating it.
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const targets: Text[] = [];
  let node = walker.nextNode() as Text | null;
  while (node) {
    const value = node.nodeValue;
    if (value && CITATION_RE.test(value)) {
      CITATION_RE.lastIndex = 0;
      targets.push(node);
    }
    node = walker.nextNode() as Text | null;
  }

  for (const text of targets) {
    const value = text.nodeValue as string;
    const parent = text.parentNode as ParentNode;
    const frag = document.createDocumentFragment();
    let lastIndex = 0;
    CITATION_RE.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = CITATION_RE.exec(value)) !== null) {
      const n = Number.parseInt(match[1], 10);
      const source = sources[n - 1];
      if (!source) continue;
      if (match.index > lastIndex) {
        frag.appendChild(
          document.createTextNode(value.slice(lastIndex, match.index)),
        );
      }
      const a = document.createElement('a');
      a.textContent = match[0];
      a.className = 'citation-link';
      a.setAttribute('data-citation', String(n));
      a.setAttribute('data-url', source.url);
      a.setAttribute('title', source.title || source.url);
      a.setAttribute('role', 'button');
      frag.appendChild(a);
      lastIndex = match.index + match[0].length;
    }
    if (lastIndex === 0) continue; // no valid numbered-source matches
    if (lastIndex < value.length) {
      frag.appendChild(document.createTextNode(value.slice(lastIndex)));
    }
    parent.replaceChild(frag, text);
  }
}

/**
 * Renders user message content with slash commands styled distinctly.
 * Only the FIRST occurrence of each command trigger is styled; duplicate
 * triggers render as plain text (the first one is the active command).
 */
function renderUserContent(content: string): React.ReactNode {
  const parts: React.ReactNode[] = [];
  let remaining = content;
  const styledCommands = new Set<string>();

  while (remaining.length > 0) {
    // Find the earliest command trigger in remaining text (skip already-styled ones)
    let earliest = -1;
    let matchedTrigger = '';
    for (const cmd of COMMANDS) {
      if (styledCommands.has(cmd.trigger)) continue;
      const idx = remaining.indexOf(cmd.trigger);
      if (idx !== -1 && (earliest === -1 || idx < earliest)) {
        const before = idx === 0 || remaining[idx - 1] === ' ';
        const after =
          idx + cmd.trigger.length >= remaining.length ||
          remaining[idx + cmd.trigger.length] === ' ';
        if (before && after) {
          earliest = idx;
          matchedTrigger = cmd.trigger;
        }
      }
    }

    if (earliest === -1) {
      parts.push(<span key={parts.length}>{remaining}</span>);
      break;
    }

    // Text before the command
    if (earliest > 0) {
      parts.push(
        <span key={parts.length}>{remaining.slice(0, earliest)}</span>,
      );
    }
    // The command itself, styled (first occurrence only)
    parts.push(
      <span key={parts.length} className="font-semibold text-[#7C2D12]">
        {matchedTrigger}
      </span>,
    );
    styledCommands.add(matchedTrigger);
    remaining = remaining.slice(earliest + matchedTrigger.length);
  }

  return <>{parts}</>;
}

interface ChatBubbleProps {
  /** The message role determines alignment and color treatment. */
  role: 'user' | 'assistant';
  /** The message content to render. AI messages support markdown. */
  content: string;
  /** Stagger index for orchestrated entrance choreography. */
  index: number;
  /** Selected text from the host app that was quoted alongside this message, if any. */
  quotedText?: string;
  /** Whether this bubble is actively streaming content from the LLM. */
  isStreaming?: boolean;
  /** When set, renders an ErrorCard callout instead of markdown. */
  errorKind?: OllamaErrorKind;
  /** Accumulated thinking/reasoning content from the model, if thinking mode was used. */
  thinkingContent?: string;
  /** Whether the model is currently in the thinking phase (streaming thinking tokens). */
  isThinking?: boolean;
  /** Absolute file paths of images attached to this message, if any. */
  imagePaths?: string[];
  /** Called when the user clicks a thumbnail to preview it. */
  onImagePreview?: (path: string) => void;
  /** Source URLs forwarded from the SearXNG results. Rendered as a clickable
   * footer below the answer; clicking opens the URL in the default browser. */
  searchSources?: SearchResultPreview[];
  /** Warnings emitted by the `/search` pipeline for this turn. Renders a
   * `SearchWarningIcon` beside the Sources collapsible when non-empty. */
  searchWarnings?: SearchWarning[];
  /** When true, renders a `SandboxSetupCard` instead of markdown or error bubble. */
  sandboxUnavailable?: boolean;
  /** User-facing search timeline data for `/search` turns. */
  searchTraces?: SearchTraceStep[];
  /** Whether the search pipeline is currently running. When true, renders a
   * `SearchTraceBlock` in loading state even before any traces arrive. */
  isSearching?: boolean;
}

/**
 * Framer Motion variants for individual chat bubbles.
 * Uses GPU-accelerated transforms (opacity, y, scale) for jank-free animation.
 * Spring physics provide natural, organic motion.
 */
const bubbleVariants = {
  hidden: { opacity: 0, y: 12, scale: 0.95 },
  visible: {
    opacity: 1,
    y: 0,
    scale: 1,
    transition: {
      type: 'spring' as const,
      stiffness: 380,
      damping: 26,
    },
  },
};

/**
 * Renders a chat message following industry-standard assistant UI conventions:
 *
 * - **User messages** — right-aligned bubble with warm gradient, quoted-text
 *   support, and an always-visible copy button below the bubble (right-aligned).
 * - **AI messages** — full-width plain text (no bubble), markdown-rendered, with
 *   an always-visible copy button below the text (left-aligned).
 *
 * Spring entrance animation is staggered by `index` to produce natural
 * choreography when multiple messages appear at once.
 */
export function ChatBubble({
  role,
  content,
  index,
  quotedText,
  isStreaming = false,
  imagePaths,
  onImagePreview,
  errorKind,
  thinkingContent,
  isThinking,
  searchSources,
  searchWarnings,
  sandboxUnavailable = false,
  searchTraces,
  isSearching = false,
}: ChatBubbleProps) {
  const isUser = role === 'user';
  const [sourcesOpen, setSourcesOpen] = useState(false);

  const activeWarnings = searchWarnings ?? [];
  const warningSeverity: 'error' | 'warn' | null = activeWarnings.some(
    (w) => SEARCH_WARNING_SEVERITY[w] === 'error',
  )
    ? 'error'
    : activeWarnings.length > 0
      ? 'warn'
      : null;

  /** Ref on the markdown container so `wrapCitations` can post-process
   *  the rendered DOM after every token update. */
  const answerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!searchSources || searchSources.length === 0) return;
    // Ref attaches synchronously on the JSX below, so `.current` is never
    // null when the effect fires on mount or any subsequent update.
    wrapCitations(answerRef.current!, searchSources);
  }, [content, searchSources]);

  /**
   * Two-way hover linking between inline citation anchors and the pill
   * footer: toggles `data-active-citation` on the container so CSS drives
   * the highlight state. Delegated event handlers mean we only bind one
   * listener per bubble regardless of citation/pill count.
   */
  const containerRef = useRef<HTMLDivElement>(null);
  const activateCitation = (n: string | null) => {
    // The bubble is always mounted when hover handlers can fire; non-null
    // assertion is guarded by the fact that the ref attaches synchronously
    // on the JSX below.
    const root = containerRef.current!;
    if (n) {
      root.setAttribute('data-active-citation', n);
    } else {
      root.removeAttribute('data-active-citation');
    }
  };

  const onAnswerMouseOver = (e: React.MouseEvent<HTMLDivElement>) => {
    const target = (e.target as HTMLElement).closest(
      '[data-citation]',
    ) as HTMLElement | null;
    if (target) activateCitation(target.getAttribute('data-citation'));
  };
  const onAnswerMouseOut = (e: React.MouseEvent<HTMLDivElement>) => {
    const target = (e.target as HTMLElement).closest('[data-citation]');
    if (target) activateCitation(null);
  };
  const onAnswerClick = (e: React.MouseEvent<HTMLDivElement>) => {
    const target = (e.target as HTMLElement).closest(
      '[data-citation]',
    ) as HTMLElement | null;
    if (!target) return;
    e.preventDefault();
    // `data-url` is always set when we build citation anchors in wrapCitations
    // and on source pills at render time, so the non-null assertion is safe.
    void invoke('open_url', { url: target.getAttribute('data-url')! });
  };

  return (
    <motion.div
      variants={bubbleVariants}
      initial="hidden"
      animate="visible"
      transition={{ delay: index * 0.06 }}
      className={`flex w-full ${isUser ? 'justify-end' : 'justify-start'}`}
    >
      {isUser ? (
        /* User bubble — max-width capped, stacks bubble + action bar */
        <div className="flex flex-col max-w-[80%]">
          <div className="chat-bubble chat-bubble-user relative px-4 py-2.5 text-sm leading-relaxed select-text rounded-2xl rounded-br-md">
            {quotedText && (
              <p className="border-l-2 border-white/40 pl-2 mb-2 italic text-xs text-white/60 whitespace-pre-wrap">
                {formatQuotedText(
                  quotedText,
                  quote.maxDisplayLines,
                  quote.maxDisplayChars,
                )}
              </p>
            )}
            {imagePaths && imagePaths.length > 0 && onImagePreview && (
              <div className="mb-2">
                <ImageThumbnails
                  items={imagePaths.map((p) => ({
                    id: p,
                    src:
                      p === SCREEN_CAPTURE_PLACEHOLDER
                        ? p
                        : p.startsWith('blob:')
                          ? p
                          : convertFileSrc(p),
                    loading: p.startsWith('blob:'),
                    placeholder: p === SCREEN_CAPTURE_PLACEHOLDER,
                  }))}
                  onPreview={onImagePreview}
                  size={48}
                />
              </div>
            )}
            {content && (
              <span className="text-white/95 font-medium whitespace-pre-wrap">
                {renderUserContent(content)}
              </span>
            )}
          </div>
          {content && (
            <div className="h-6 flex items-center px-1">
              <CopyButton content={content} align="right" />
            </div>
          )}
        </div>
      ) : (
        /* AI plain text — full width, no bubble chrome */
        <div
          ref={containerRef}
          data-testid="chat-bubble"
          className={`search-bubble flex flex-col w-full${warningSeverity === 'error' ? ' search-bubble--error' : ''}`}
          onMouseOver={onAnswerMouseOver}
          onMouseOut={onAnswerMouseOut}
          onClick={onAnswerClick}
        >
          <div
            ref={answerRef}
            className="text-sm leading-relaxed select-text py-1"
          >
            {(isSearching || (searchTraces && searchTraces.length > 0)) && (
              <SearchTraceBlock
                traces={searchTraces ?? []}
                isSearching={isSearching}
                sources={searchSources}
              />
            )}
            {thinkingContent && (
              <ThinkingBlock
                thinkingContent={thinkingContent}
                isThinking={isThinking ?? false}
              />
            )}
            {sandboxUnavailable ? (
              <SandboxSetupCard />
            ) : errorKind ? (
              <ErrorCard kind={errorKind} message={content} />
            ) : (
              <MarkdownRenderer content={content} isStreaming={isStreaming} />
            )}
          </div>
          {!errorKind && !sandboxUnavailable && !isStreaming && (
            <AnimatePresence initial={false}>
              {sourcesOpen && searchSources && searchSources.length > 0 && (
                <motion.div
                  key="sources"
                  data-testid="search-sources"
                  initial={{ height: 0, opacity: 0 }}
                  animate={{ height: 'auto', opacity: 1 }}
                  exit={{ height: 0, opacity: 0 }}
                  transition={{
                    height: {
                      duration: 0.3,
                      ease: [0.33, 1, 0.68, 1],
                    },
                    opacity: { duration: 0.2, delay: 0.05 },
                  }}
                  style={{ overflow: 'hidden' }}
                >
                  <div className="pt-3">
                    <p className="text-[10px] text-white/25 uppercase tracking-wider mb-1.5">
                      Sources
                    </p>
                    <div className="flex flex-col gap-0.5">
                      {searchSources.map((src, i) => {
                        const n = i + 1;
                        return (
                          <button
                            key={src.url}
                            type="button"
                            title={src.title || src.url}
                            data-citation={n}
                            data-url={src.url}
                            onMouseEnter={() => activateCitation(String(n))}
                            onMouseLeave={() => activateCitation(null)}
                            onClick={() =>
                              void invoke('open_url', { url: src.url })
                            }
                            className="source-row flex items-baseline gap-3 w-full text-left cursor-pointer py-0.5 group"
                          >
                            <span className="source-row-num shrink-0 w-5 text-xs text-white/25 tabular-nums">
                              {n}.
                            </span>
                            <span className="source-row-title flex-1 min-w-0 truncate text-sm text-white/60">
                              {src.title || src.url}
                            </span>
                            <span className="source-row-domain shrink-0 text-xs text-white/30 truncate max-w-[45%]">
                              {domainOf(src.url)}
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                </motion.div>
              )}
            </AnimatePresence>
          )}
          {!errorKind && !sandboxUnavailable && !isStreaming && (
            <div className="h-6 flex items-center gap-3">
              {/* shrink-0 wrapper prevents CopyButton's internal w-full from
                  pushing the sources trigger to the opposite end. */}
              <div className="shrink-0">
                <CopyButton content={content} align="left" />
              </div>
              {searchSources && searchSources.length > 0 && (
                <button
                  type="button"
                  onClick={() => setSourcesOpen((v) => !v)}
                  aria-expanded={sourcesOpen}
                  className="sources-trigger inline-flex items-center gap-2 cursor-pointer"
                >
                  <span aria-hidden className="inline-flex items-center">
                    {searchSources.slice(0, 3).map((src, i) => {
                      const domain = domainOf(src.url);
                      // SearXNG filters out empty-URL results before they reach
                      // the frontend, so `domain[0]` is always defined here —
                      // the `?` fallback is a belt-and-braces defensive default.
                      /* v8 ignore start */
                      const letter = (domain[0] ?? '?').toUpperCase();
                      /* v8 ignore stop */
                      const bg = avatarColor(domain);
                      return (
                        <span
                          key={src.url}
                          className="shrink-0 h-4.5 w-4.5 rounded-full inline-flex items-center justify-center text-[9px] font-semibold text-white/90"
                          style={{
                            background: bg,
                            border:
                              '1.5px solid var(--avatar-ring, rgba(26,26,26,1))',
                            marginLeft: i === 0 ? 0 : -6,
                          }}
                        >
                          {letter}
                        </span>
                      );
                    })}
                  </span>
                  <span className="text-[11px] text-white/50">
                    {searchSources.length}{' '}
                    {searchSources.length === 1 ? 'source' : 'sources'}
                  </span>
                </button>
              )}
              <SearchWarningIcon warnings={activeWarnings} />
            </div>
          )}
        </div>
      )}
    </motion.div>
  );
}
