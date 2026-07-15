import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import type { SearchResultPreview, SearchStage } from '../types/search';
import { pinChatMessagesToBottom } from '../utils/scrollChat';
import { avatarColor, domainOf } from '../utils/domainAvatar';
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
  /**
   * Live read of ConversationView's auto-scroll gate (owner of
   * `shouldAutoScrollRef`). Returns `true` while the user is pinned to the
   * bottom and `false` the instant they wheel up to read history. The block
   * only hard-pins the chat scroller when this returns non-`false`, so a
   * user who scrolled up mid-search is never yanked back down. A function
   * (not a boolean) because the gate flips on a ref the wheel handler mutates
   * without a re-render, so a snapshot would be stale by pin time. Absent in
   * isolation tests, where an unconditional pin is the intended behavior.
   */
  shouldAutoScroll?: () => boolean;
  /**
   * Auto-expand the source list while reading. When false (e.g. answer tokens
   * started streaming), auto policy collapses the list to free room; the strip
   * stays mounted and the user can re-expand via the chevron.
   */
  preferSourcesExpanded?: boolean;
  /**
   * After reasoning, answer-stream phases use inventory copy `Sources (N)`
   * instead of replaying "Reading sources" / "Composing answer". Verify still
   * uses the live verifying stage label. Three-dot strip stays either way.
   */
  postReasoningSourcesLabel?: boolean;
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
 *
 * During the verify stage the footer C3 pill owns the "Verifying sources..."
 * copy, so the strip shows the neutral inventory `Sources (N)` instead to avoid
 * repeating it, EXCEPT on a reasoned turn where the restored strip under
 * Reasoning keeps the live verify label (deliberate; a handoff test locks it).
 * Off verify, inventory is used only after reasoning so the restored strip does
 * not re-claim "Reading sources" during the answer stream.
 *
 * @param stage - Live pipeline stage.
 * @param sourceCount - Number of sources for the `(N)` suffix.
 * @param postReasoning - Marks a reasoned turn's restored strip.
 * @returns Header string for the strip toggle.
 */
export function searchProgressHeaderLabel(
  stage: SearchStage,
  sourceCount: number,
  postReasoning = false,
): string {
  const isVerifying = Boolean(stage && stage.kind === 'verifying_sources');
  const useInventory = isVerifying ? !postReasoning : postReasoning;
  const stageLabel = useInventory ? 'Sources' : liveSearchStageLabel(stage);
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
  shouldAutoScroll,
  preferSourcesExpanded = true,
  postReasoningSourcesLabel = false,
}: SearchProgressBlockProps) {
  const panelId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const sourceCount = sources.length;
  const hasSources = sourceCount > 0;

  // null = follow auto policy; boolean = user override
  const [userExpanded, setUserExpanded] = useState<boolean | null>(null);

  /**
   * Auto policy: expand while searching with sources and the parent still
   * prefers expansion (reading phase). Collapse when answer streaming starts
   * (`preferSourcesExpanded` false) so the answer has room; user can re-open.
   * Handoff exit always forces collapsed so body AnimatePresence can run.
   */
  const autoExpanded = isSearching && hasSources && preferSourcesExpanded;

  // When a live search first gains sources (enters the auto-expand state),
  // drop any user override so the fresh batch re-opens the list. Done as a
  // render-time transition adjustment keyed on the auto-expand condition
  // rather than a state-in-effect, so it lands in the same commit and never a
  // frame late. React bails out once the condition stops changing; the
  // `userExpanded !== null` guard keeps the setState converging.
  const autoExpandActive =
    isSearching && !isExiting && hasSources && preferSourcesExpanded;
  const prevAutoExpandActiveRef = useRef(autoExpandActive);
  if (prevAutoExpandActiveRef.current !== autoExpandActive) {
    prevAutoExpandActiveRef.current = autoExpandActive;
    if (autoExpandActive && userExpanded !== null) {
      setUserExpanded(null);
    }
  }

  // Answer started: clear any user-expanded override so the list auto-collapses
  // for room; user can still re-expand via the chevron afterward.
  const prevPreferExpandedRef = useRef(preferSourcesExpanded);
  if (prevPreferExpandedRef.current !== preferSourcesExpanded) {
    prevPreferExpandedRef.current = preferSourcesExpanded;
    if (!preferSourcesExpanded && userExpanded !== null) {
      setUserExpanded(null);
    }
  }

  const expanded = isExiting ? false : (userExpanded ?? autoExpanded);

  // Latest expand/exit flags for animation callbacks (exit must not pin).
  const expandedRef = useRef(expanded);
  expandedRef.current = expanded;
  const isExitingRef = useRef(isExiting);
  isExitingRef.current = isExiting;

  /**
   * Hard-pin the chat scroller so header + source list fully enter view.
   * Uses `.chat-messages-scroll` scrollTop rather than scrollIntoView:
   * browser ancestor heuristics no-op or under-scroll when the strip is
   * already partially visible at the bottom edge under a long answer.
   * Called after expand height animation finishes and when sourceCount
   * changes while already expanded (list grows under a long answer).
   * ConversationView's ResizeObserver covers mid-animation frames.
   */
  const pinProgressInView = useCallback((): void => {
    // Exit / collapse can still fire onAnimationComplete; skip pin then.
    // Defensive guard; expand path always has both true.
    /* v8 ignore start */
    if (!expandedRef.current || isExitingRef.current) return;
    /* v8 ignore stop */
    // Honor ConversationView's manual-scroll gate: never yank a user who
    // scrolled up to read history back to the bottom. Absent gate (isolation
    // tests) pins unconditionally, preserving follow-live-output behavior.
    if (shouldAutoScroll?.() === false) return;
    pinChatMessagesToBottom(rootRef.current);
  }, [shouldAutoScroll]);

  // Re-pin when source count grows while expanded (no expand animation).
  useEffect(() => {
    if (!expanded || !hasSources || isExiting) return;
    pinProgressInView();
  }, [expanded, hasSources, isExiting, sourceCount, pinProgressInView]);

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
  // Post-reasoning answer stream: "Sources (N)"; verify keeps stage copy.
  const headerLabel = searchProgressHeaderLabel(
    stage,
    sourceCount,
    postReasoningSourcesLabel,
  );

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
      {/*
        Screen-reader-only announcement of the same string the visible
        header shows (`headerLabel`), so SR and sighted users hear one
        phase name. Uses inventory `Sources (N)` on non-reasoned verify
        turns: the footer `sources-verifying-pill` owns "Verifying
        sources..." there, and announcing the live stage string here
        would double it. Reasoned verify keeps the live verify copy in
        both places on purpose (strip under Reasoning + pill).
      */}
      <span
        data-testid="search-progress-live-region"
        role="status"
        aria-live="polite"
        className="sr-only"
      >
        {headerLabel}
      </span>
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
            onAnimationComplete={pinProgressInView}
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
