import { AnimatePresence, motion } from 'framer-motion';
import { useEffect, useRef, useState } from 'react';
import { MarkdownRenderer } from './MarkdownRenderer';
import { ErrorCard } from './ErrorCard';
import { CopyButton } from './CopyButton';
import { ReplaceButton } from './ReplaceButton';
import { ImageThumbnails } from './ImageThumbnails';
import { ReasoningBlock } from './ReasoningBlock';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { formatQuotedText } from '../utils/formatQuote';
import { useConfig } from '../contexts/ConfigContext';
import { COMMANDS, SCREEN_CAPTURE_PLACEHOLDER } from '../config/commands';
import type { EngineErrorKind } from '../hooks/useModel';
import type { SearchResultPreview, SearchStage } from '../types/search';
import { SearchProgressBlock } from './SearchProgressBlock';
import { cleanForRender } from '../utils/sanitizeAssistantContent';

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

/**
 * Friendly model name for the `InsufficientMemory` card title (issue #296):
 * the display-name mapping when known (built-in ids are raw slugs), else the
 * raw id, else a neutral fallback for a message with no attribution. Shared by
 * both the initial-failure fetch path and the carried-figures render path so
 * the three-way fallback has a single set of branches to cover.
 */
function resolveMemoryModelName(
  modelName: string | undefined,
  displayNames: Record<string, string> | undefined,
): string {
  return (modelName && displayNames?.[modelName]) ?? modelName ?? 'This model';
}

/**
 * Hoisted static SVG glyph for the model attribution chip. Mirrors the
 * chip icon used by the model picker so the attribution visually couples
 * to the picker UI. Rendered as a child of a color-controlled span.
 */
const ATTRIB_CHIP_ICON = (
  <svg
    width="9"
    height="9"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect
      x="3"
      y="3"
      width="10"
      height="10"
      rx="1.5"
      stroke="currentColor"
      strokeWidth="1.5"
    />
    <path
      d="M5 1V3M8 1V3M11 1V3M5 13V15M8 13V15M11 13V15M1 5H3M1 8H3M1 11H3M13 5H15M13 8H15M13 11H15"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    />
  </svg>
);

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
  errorKind?: EngineErrorKind;
  /** Opens the model picker from an `EngineStartFailed` error card so a failed
   *  model load is never a dead end. Forwarded to the ErrorCard. */
  onSwitchModel?: () => void;
  /** Replays the turn with the pre-load memory gate bypassed (issue #296).
   *  Forwarded to the ErrorCard's `InsufficientMemory` branch as the "Load
   *  anyway" action. */
  onLoadAnyway?: () => void;
  /** Accumulated thinking/reasoning content from the model, if thinking mode was used. */
  thinkingContent?: string;
  /** Whether a `/think` turn is waiting for the first thinking tokens. */
  isThinkingPending?: boolean;
  /**
   * Cue shown in place of the reasoning block while `isThinkingPending` is
   * true - the same engine-loading label shown next to a plain turn's
   * typing dots (`null` before the loading threshold elapses, so a fast/warm
   * turn shows no text either).
   */
  pendingLabel?: string | null;
  /** Whether the model is currently in the thinking phase (streaming thinking tokens). */
  isThinking?: boolean;
  /** Absolute file paths of images attached to this message, if any. */
  imagePaths?: string[];
  /** Called when the user clicks a thumbnail to preview it. */
  onImagePreview?: (path: string) => void;
  /** When set, renders a Replace button in the action bar that writes this
   * message's content back into the source app (for `/rewrite` & `/refine`). */
  onReplace?: (text: string) => Promise<boolean>;
  /** Source URLs for a web-search answer. Click opens the URL in the browser. */
  searchSources?: SearchResultPreview[];
  /**
   * Coarse web-search phase (`SearchStatus`). Drives
   * {@link SearchProgressBlock} while the turn is searching.
   */
  searchStage?: SearchStage;
  /** True while web search is in flight for this assistant message. */
  isSearching?: boolean;
  /** When set on an assistant message, renders a chip-style attribution badge beside the CopyButton so the user sees which model produced this response. */
  modelName?: string;
  /**
   * Friendly display name per model id. When `modelName` has an entry
   * (built-in models, whose ids are the raw "repo:file.gguf" slug), the
   * attribution chip renders the friendly name; ids without an entry render
   * verbatim (already clean for Ollama / OpenAI). Keeps the chip consistent
   * with the model picker and the titlebar pill.
   */
  displayNames?: Record<string, string>;
  /**
   * Pre-fetched memory-fit figures for an `InsufficientMemory` re-attribution
   * (issue #296). When present, the card renders these carried numbers together
   * with `modelName` and the per-message async `estimate_model_fit` fetch is
   * skipped, so the model name and GB figures never disagree during a "Switch
   * model" swap. Absent on the initial failure, where the fetch supplies them.
   */
  memoryFit?: { requiredBytes: number; availableBytes: number };
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
 * - **User messages** - right-aligned bubble with warm gradient, quoted-text
 *   support, and an always-visible copy button below the bubble (right-aligned).
 * - **AI messages** - full-width plain text (no bubble), markdown-rendered, with
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
  onReplace,
  errorKind,
  onSwitchModel,
  onLoadAnyway,
  thinkingContent,
  isThinkingPending,
  pendingLabel,
  isThinking,
  searchSources,
  searchStage = null,
  isSearching = false,
  modelName,
  displayNames,
  memoryFit,
}: ChatBubbleProps) {
  const isUser = role === 'user';
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const quote = useConfig().quote;
  // Render-time defense for legacy assistant content that may carry
  // special turn-boundary tokens leaked by older Ollama versions or
  // mis-tuned models. Backend now strips these on write, so the scrub is
  // a no-op for fresh replies; history stored before that strip existed
  // relies on it. User input never contains these markers naturally so we
  // skip the scrub for user bubbles.
  const displayContent = isUser ? content : cleanForRender(content);

  /** Citation audit/repair after answer tokens; Done is still withheld. */
  const isVerifyingSources = searchStage?.kind === 'verifying_sources';
  const hasSearchSources = Boolean(searchSources && searchSources.length > 0);
  /**
   * Reasoning or answer body has started. Option D sequential handoff:
   * unmount SearchProgressBlock so only Reasoning (then answer) is live.
   * Sources stay available post-answer via the footer chips / citations.
   */
  const handedOffFromSearch = Boolean(
    thinkingContent || isThinkingPending || displayContent,
  );
  /**
   * SearchProgressBlock covers retrieve/read/compose only during pure
   * search. Unmounts once reasoning/answer starts, and during verifying
   * (C3 sources pill owns that status).
   */
  const showSearchProgress =
    isSearching && !isVerifyingSources && !handedOffFromSearch;
  /**
   * Action bar is hidden while tokens stream. Re-open for the C3 verifying
   * pill (still isStreaming, only when sources exist) and for the finished turn.
   */
  const showActionBar =
    !errorKind && (!isStreaming || (isVerifyingSources && hasSearchSources));

  /**
   * Machine-readable figures for the `InsufficientMemory` error card
   * (issue #296) on the INITIAL failure, fetched lazily once the card actually
   * needs them rather than threaded down as a prop from every ancestor.
   * `undefined` while the fetch is in flight or if it fails; the ErrorCard falls
   * back to its generic message render in that case, so a failed fetch never
   * crashes the bubble. Not reset when `errorKind` moves away from
   * `InsufficientMemory`: `ErrorCard` only reads figures for that kind, so a
   * stale estimate lingering in state is inert until the next fetch overwrites
   * it. Overridden entirely by `memoryFit` when the message carries pre-fetched
   * figures (the "Switch model" re-attribution path), so the fetch is skipped
   * there and this stale value can never leak onto a re-attributed card.
   */
  const [insufficientMemoryInfo, setInsufficientMemoryInfo] = useState<
    | { modelName: string; requiredBytes: number; availableBytes: number }
    | undefined
  >(undefined);

  useEffect(() => {
    if (errorKind !== 'InsufficientMemory') return;
    // Carried figures win: the re-attribution path (issue #296) hands the card
    // the newly-picked model's numbers directly, so skip the redundant fetch
    // whose async resolution is exactly the stale window we are eliminating.
    if (memoryFit) return;
    let cancelled = false;
    invoke<{
      required_bytes: number;
      available_bytes: number;
      verdict: string;
    }>('estimate_model_fit', { modelId: modelName })
      .then((estimate) => {
        if (cancelled) return;
        setInsufficientMemoryInfo({
          modelName: resolveMemoryModelName(modelName, displayNames),
          requiredBytes: estimate.required_bytes,
          availableBytes: estimate.available_bytes,
        });
      })
      .catch(() => {
        // Leave undefined; ErrorCard renders its generic fallback.
      });
    return () => {
      cancelled = true;
    };
  }, [errorKind, modelName, displayNames, memoryFit]);

  /**
   * Figures actually rendered by the `InsufficientMemory` card. Carried
   * `memoryFit` (set atomically with `modelName` on a "Switch model" swap) takes
   * precedence over the initial-failure fetch state, so the name and GB numbers
   * always describe the same model and never render the previous model's data
   * (issue #296). Falls back to the fetched state on the initial failure.
   */
  const resolvedMemoryInfo = memoryFit
    ? {
        modelName: resolveMemoryModelName(modelName, displayNames),
        requiredBytes: memoryFit.requiredBytes,
        availableBytes: memoryFit.availableBytes,
      }
    : insufficientMemoryInfo;

  /**
   * Stacks up to three domain letter avatars for the sources trigger or the
   * verifying pill. Call only when `hasSearchSources` is true. `pulse`
   * enables the C3 chip-pulse while citation audit runs.
   */
  function renderSourceAvatars(pulse: boolean): React.ReactNode {
    return (
      <span
        aria-hidden
        className={`inline-flex items-center${pulse ? ' sources-chips-pulse' : ''}`}
      >
        {searchSources!.slice(0, 3).map((src, i) => {
          const domain = domainOf(src.url);
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
                border: '1.5px solid var(--avatar-ring, rgba(26,26,26,1))',
                marginLeft: i === 0 ? 0 : -6,
              }}
            >
              {letter}
            </span>
          );
        })}
      </span>
    );
  }

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
    // Scope to inline citation anchors only. Source-row buttons in the
    // footer also carry `data-citation` (for hover-link highlighting) and
    // own their own click handler; matching them here would double-fire
    // `open_url`, opening the URL twice in the browser.
    const target = (e.target as HTMLElement).closest(
      'a[data-citation]',
    ) as HTMLElement | null;
    if (!target) return;
    e.preventDefault();
    // `data-url` is always set when MarkdownRenderer builds citation
    // anchors, so the non-null assertion is safe.
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
        /* User bubble - max-width capped, stacks bubble + action bar */
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
              <span className="thuki-text-base text-white/95 font-medium whitespace-pre-wrap">
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
        /* AI plain text - full width, no bubble chrome */
        <div
          ref={containerRef}
          data-testid="chat-bubble"
          className="search-bubble flex flex-col w-full"
          onMouseOver={onAnswerMouseOver}
          onMouseOut={onAnswerMouseOut}
          onClick={onAnswerClick}
        >
          <div className="text-sm leading-relaxed select-text py-1">
            {showSearchProgress ? (
              <SearchProgressBlock
                stage={searchStage}
                sources={searchSources}
                isSearching={isSearching}
              />
            ) : null}
            {(thinkingContent || isThinkingPending) && (
              <ReasoningBlock
                thinkingContent={thinkingContent}
                isPending={isThinkingPending ?? false}
                pendingLabel={pendingLabel}
                isThinking={isThinking ?? false}
              />
            )}
            {errorKind ? (
              <ErrorCard
                kind={errorKind}
                message={content}
                onSwitchModel={onSwitchModel}
                onLoadAnyway={onLoadAnyway}
                insufficientMemoryInfo={resolvedMemoryInfo}
              />
            ) : (
              <MarkdownRenderer
                content={displayContent}
                isStreaming={isStreaming}
                citationSources={searchSources}
              />
            )}
          </div>
          {!errorKind && !isStreaming && (
            <AnimatePresence initial={false}>
              {sourcesOpen && hasSearchSources && (
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
                      {searchSources!.map((src, i) => {
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
          {showActionBar && (
            <div
              className={`h-6 flex items-center gap-3${isVerifyingSources ? ' mt-1.5' : ''}`}
            >
              {isVerifyingSources && hasSearchSources ? (
                <span
                  data-testid="sources-verifying-pill"
                  role="status"
                  aria-live="polite"
                  className="sources-verifying-pill"
                >
                  {renderSourceAvatars(true)}
                  Verifying sources...
                </span>
              ) : (
                <>
                  {/* shrink-0 wrapper prevents CopyButton's internal w-full from
                      pushing the sources trigger to the opposite end. */}
                  <div className="shrink-0">
                    <CopyButton content={displayContent} align="left" />
                  </div>
                  {onReplace && (
                    <ReplaceButton
                      content={displayContent}
                      onReplace={onReplace}
                    />
                  )}
                  {hasSearchSources && (
                    <button
                      type="button"
                      onClick={() => setSourcesOpen((v) => !v)}
                      aria-expanded={sourcesOpen}
                      className="sources-trigger inline-flex items-center gap-2 cursor-pointer"
                    >
                      {renderSourceAvatars(false)}
                      <span className="text-[11px] text-white/50">
                        {searchSources!.length}{' '}
                        {searchSources!.length === 1 ? 'source' : 'sources'}
                      </span>
                    </button>
                  )}
                  {/* Model attribution chip: visually couples the response to the
                      model-picker UI so users can see which model produced it. */}
                  {modelName && (
                    <span
                      data-testid="model-attribution"
                      className="inline-flex items-center gap-[5px] px-[6px] py-[2px] pr-[8px] rounded-md border border-primary/15 bg-primary/5 text-[10.5px] tracking-[0.01em] text-text-secondary w-fit transition-[background-color,border-color,color] duration-150 hover:text-text-primary hover:bg-primary/10 hover:border-primary/25"
                    >
                      <span className="text-primary/85 shrink-0 flex items-center">
                        {ATTRIB_CHIP_ICON}
                      </span>
                      <span className="max-w-[100px] truncate">
                        {displayNames?.[modelName] ?? modelName}
                      </span>
                    </span>
                  )}
                </>
              )}
            </div>
          )}
        </div>
      )}
    </motion.div>
  );
}
