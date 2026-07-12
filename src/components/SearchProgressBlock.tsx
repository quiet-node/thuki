import { useEffect, useId, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import type { SearchResultPreview, SearchStage } from '../types/search';
import { RequestStatusStrip } from './RequestStatusStrip';

/** Inner sources list: ~8–10 rows before scroll; keeps header on screen. */
const SOURCES_LIST_SCROLL_CLASS =
  'mt-2 ml-1.5 pl-3 border-l border-primary/20 flex flex-col gap-1.5 max-h-48 overflow-y-auto';

/**
 * Props for progressive web-search progress chrome
 * (`ask_model` + `SearchStatus` / `SearchSources`).
 *
 * Mounted during pure search and briefly during ChatBubble handoff exit
 * (Option D). After exit completes, ChatBubble keeps this unmounted.
 */
export interface SearchProgressBlockProps {
  /**
   * Coarse pipeline phase from the backend, or `null` once the turn has
   * finished searching (caller may still pass sources for a collapsed summary).
   */
  stage: SearchStage;
  /** Curated source list for this turn; grows when `SearchSources` arrives. */
  sources?: SearchResultPreview[];
  /** True while the search-augmented turn is still generating. */
  isSearching: boolean;
  /**
   * When true, force-collapse the source list and disable toggle so the
   * body height exit can run before ChatBubble fades the outer chrome.
   * Parent owns the outer opacity exit; this block owns collapse only.
   */
  isExiting?: boolean;
}

/**
 * Extracts a bare hostname from a URL for source rows. Strips a leading
 * `www.` prefix; falls back to the raw input if URL parsing fails.
 */
function domainOf(url: string): string {
  try {
    const host = new URL(url).hostname;
    return host.startsWith('www.') ? host.slice(4) : host;
  } catch {
    return url;
  }
}

/**
 * Deterministic 0–359 hue from a domain string so each source keeps a stable
 * letter-avatar color across re-renders without network favicon fetches.
 */
function domainHue(domain: string): number {
  let h = 0;
  for (let i = 0; i < domain.length; i++) {
    h = (h * 31 + domain.charCodeAt(i)) >>> 0;
  }
  return h % 360;
}

/**
 * Hand-picked gradient pairs for letter avatars (same palette as ChatBubble
 * source chips). Domain hash picks one pair deterministically.
 */
const AVATAR_PALETTE: readonly string[] = [
  'linear-gradient(135deg, #ffb8a1, #ff8c77)',
  'linear-gradient(135deg, #ffc3d5, #ff9cbd)',
  'linear-gradient(135deg, #a8d8ff, #7cb8ff)',
  'linear-gradient(135deg, #a8e6cf, #7ecfb0)',
  'linear-gradient(135deg, #c7b8ff, #a896ff)',
  'linear-gradient(135deg, #ffd3a5, #ffa978)',
  'linear-gradient(135deg, #9ee6d7, #6fc9b5)',
  'linear-gradient(135deg, #fff0a5, #ffd96b)',
  'linear-gradient(135deg, #b8e0ff, #85b9ff)',
  'linear-gradient(135deg, #ffb6e1, #ff8cc8)',
  'linear-gradient(135deg, #c4eaa8, #9bd076)',
  'linear-gradient(135deg, #ffc8a8, #ff9e78)',
] as const;

/**
 * Returns a CSS gradient background for a letter avatar keyed by domain.
 */
function avatarColor(domain: string): string {
  return AVATAR_PALETTE[domainHue(domain) % AVATAR_PALETTE.length];
}

/**
 * Human-readable header label for the current search stage while the pipeline
 * is live. Falls back to a neutral "Searching the web" when stage is null
 * mid-search (should not happen in practice).
 */
export function liveSearchStageLabel(stage: SearchStage): string {
  if (!stage) return 'Searching the web';
  switch (stage.kind) {
    case 'analyzing_query':
      return 'Analyzing query';
    case 'searching':
      return stage.gap ? 'Searching more angles' : 'Searching the web';
    case 'reading_sources':
      return stage.gap ? 'Reading additional pages' : 'Reading sources';
    case 'refining_search':
      return `Refining search (${stage.attempt}/${stage.total})`;
    case 'composing':
      return stage.gap ? 'Composing refined answer' : 'Composing answer';
    case 'verifying_sources':
      // C3 sources pill owns this state in ChatBubble; label kept for
      // exhaustiveness and any caller that still mounts the progress block.
      return 'Verifying sources...';
  }
}

/**
 * Builds the progress header: live stage copy, with `(N)` when sources exist.
 * Expand/collapse never rewrites this string; only stage advances do.
 */
function searchProgressHeaderLabel(
  stage: SearchStage,
  sourceCount: number,
): string {
  const stageLabel = liveSearchStageLabel(stage);
  return sourceCount > 0 ? `${stageLabel} (${sourceCount})` : stageLabel;
}

/**
 * Progressive-disclosure search chrome for built-in auto-search (Option D).
 *
 * Single expandable block during pure search. Phase header while live,
 * optional source list in the body (user can collapse for a clean view).
 * During handoff exit (`isExiting`), forces collapse so the list height
 * animates out before ChatBubble fades the outer row. Letter avatars only:
 * no favicon network fetches.
 */
export function SearchProgressBlock({
  stage,
  sources = [],
  isSearching,
  isExiting = false,
}: SearchProgressBlockProps) {
  const panelId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const sourceCount = sources.length;
  const hasSources = sourceCount > 0;

  // null = follow auto policy; boolean = user override
  const [userExpanded, setUserExpanded] = useState<boolean | null>(null);

  /**
   * Auto policy: expand when searching with sources. Collapse when no
   * sources yet. User toggle wins until sources go empty or the turn resets.
   * Handoff exit always forces collapsed so body AnimatePresence can run.
   */
  const autoExpanded = isSearching && hasSources;
  const expanded = isExiting ? false : (userExpanded ?? autoExpanded);

  // When sources first arrive during a live search, re-open unless the user
  // already forced collapse. Skip while exiting so we do not fight collapse.
  useEffect(() => {
    if (isExiting || !isSearching || !hasSources) return;
    setUserExpanded(null);
  }, [hasSources, isSearching, isExiting]);

  // Keep the progress header in view when the list expands (auto or user).
  // `nearest` avoids jumpy centering when the strip is already visible.
  useEffect(() => {
    if (!expanded || !hasSources || isExiting) return;
    /* v8 ignore next -- scrollIntoView is a host API absent in happy-dom */
    rootRef.current?.scrollIntoView?.({ block: 'nearest' });
  }, [expanded, hasSources, isExiting]);

  // Idle with no sources: nothing to show (footer chips handle post-answer).
  // Keep mounted while exiting even if `isSearching` flipped off mid-handoff.
  if (!isExiting && !isSearching && !hasSources) {
    return null;
  }

  // After the turn finishes, the action-bar sources chips own the list.
  if (!isExiting && !isSearching) {
    return null;
  }

  // Stage label always; count parens when sources exist. Collapse never swaps copy.
  const headerLabel = searchProgressHeaderLabel(stage, sourceCount);

  /**
   * Toggles expand/collapse. Only wired on the sources toggle button, which
   * is not rendered until at least one source exists. Label text is unchanged.
   * Toggle is `disabled` while exiting so collapse is not interrupted.
   */
  function handleToggle(): void {
    setUserExpanded((prev) => {
      const currently = prev ?? autoExpanded;
      return !currently;
    });
  }

  /**
   * Opens a source URL in the system browser via the existing Tauri command.
   */
  function openSource(url: string): void {
    void invoke('open_url', { url });
  }

  /**
   * Expand chevron: &#9650;, text-[9px], rotate 180 expanded / 90 collapsed.
   * Live strip passes this as `accessory` (dots → chevron → label).
   */
  const chevron = (
    <span
      data-testid="search-progress-chevron"
      aria-hidden
      className="inline-block shrink-0 text-[9px] text-text-secondary/55 transition-transform duration-150"
      style={{
        transform: expanded ? 'rotate(180deg)' : 'rotate(90deg)',
      }}
    >
      &#9650;
    </span>
  );

  return (
    <div
      ref={rootRef}
      data-testid="search-progress-block"
      data-exiting={isExiting ? 'true' : undefined}
      aria-busy={isExiting || undefined}
      className="mb-2"
    >
      <div className="flex min-w-0 items-center gap-2">
        {hasSources ? (
          <button
            type="button"
            data-testid="search-progress-toggle"
            aria-expanded={expanded}
            aria-controls={panelId}
            disabled={isExiting}
            onClick={handleToggle}
            className="flex min-w-0 flex-1 items-center gap-2 text-left cursor-pointer bg-transparent border-0 p-0 disabled:cursor-default"
          >
            <RequestStatusStrip label={headerLabel} accessory={chevron} />
          </button>
        ) : (
          <div data-testid="search-progress-header-row">
            <RequestStatusStrip label={headerLabel} />
          </div>
        )}
      </div>

      <AnimatePresence initial={false}>
        {expanded && hasSources ? (
          <motion.div
            id={panelId}
            key="search-progress-body"
            data-testid="search-progress-body"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{
              height: { duration: 0.22, ease: [0.33, 1, 0.68, 1] },
              opacity: { duration: 0.15 },
            }}
            style={{ overflow: 'hidden' }}
          >
            {/*
              Cap list height so a long source set scrolls inside the block
              instead of stretching the chat (header stays pinned above).
            */}
            <div
              data-testid="search-progress-source-list"
              className={SOURCES_LIST_SCROLL_CLASS}
            >
              {sources.map((src) => {
                const domain = domainOf(src.url);
                // Hostnames always start with a letter/digit in practice; the
                // '?' path is defensive for empty-domain edge cases from bad URLs.
                const letter = (domain.charAt(0) || '?').toUpperCase();
                const bg = avatarColor(domain);
                return (
                  <button
                    key={src.url}
                    type="button"
                    data-testid="search-progress-source-row"
                    title={src.title || src.url}
                    onClick={() => openSource(src.url)}
                    className="flex items-center gap-2 w-full text-left cursor-pointer bg-transparent border-0 p-0 min-w-0 group"
                  >
                    <span
                      aria-hidden
                      className="shrink-0 h-4.5 w-4.5 rounded-full inline-flex items-center justify-center text-[9px] font-semibold text-white/90"
                      style={{ background: bg }}
                    >
                      {letter}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-[12px] text-white/65 group-hover:text-white/85">
                      {src.title || src.url}
                    </span>
                    <span className="shrink-0 max-w-[40%] truncate text-[11px] text-white/30">
                      {domain}
                    </span>
                  </button>
                );
              })}
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}
