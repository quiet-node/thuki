/**
 * Sequential search progress → footer sources handoff phases.
 *
 * Search progress stays `live` for the whole search generation (including
 * reasoning and answer streaming). When the turn ends with sources, the
 * block keeps mounted long enough for exit animation (`exiting`) before
 * permanent unmount (`done`) so footer chips can take over.
 */

/** Lifecycle of search progress chrome relative to the finished turn. */
export type SearchHandoffPhase = 'idle' | 'live' | 'exiting' | 'done';

/** Inputs that drive handoff transitions from ChatBubble render props. */
export interface SearchHandoffSignals {
  /**
   * True while the search turn is still generating (`isSearching`).
   */
  showLiveSearch: boolean;
  /**
   * True once generation ended with sources (footer list ready).
   * Triggers exit retention when the block was live.
   */
  handedOff: boolean;
}

/**
 * Pure reducer for search handoff phase.
 *
 * @param prev - Phase from the previous render (or after an exit-complete).
 * @param signals - Live search vs handoff signals for this render.
 * @returns Next phase. `exiting` is sticky until {@link completeSearchHandoffExit}.
 */
export function nextSearchHandoffPhase(
  prev: SearchHandoffPhase,
  signals: SearchHandoffSignals,
): SearchHandoffPhase {
  const { showLiveSearch, handedOff } = signals;

  if (showLiveSearch) {
    return 'live';
  }

  // Search cancelled, verifying without body, or never started: clear a
  // stuck exit. Keep `done` only when we already finished a handoff and
  // content is still present (handled below via handedOff).
  if (!handedOff) {
    return 'idle';
  }

  // Handed off: retain search chrome only if it was showing (or mid-exit).
  if (prev === 'live' || prev === 'exiting') {
    return 'exiting';
  }

  // Never showed pure search this turn (e.g. thinking without search).
  return 'done';
}

/**
 * Marks exit animation finished so search progress stays unmounted.
 * No-op when not currently exiting (duplicate complete / fallback race).
 */
export function completeSearchHandoffExit(
  prev: SearchHandoffPhase,
): SearchHandoffPhase {
  return prev === 'exiting' ? 'done' : prev;
}

/**
 * How long search stays mounted with `isExiting` so the source-list height
 * collapse can start before the outer chrome is removed for the fade.
 * Slightly under the body height duration (0.22s) so fade overlaps the end
 * of collapse. Near-zero under reduced motion (caller).
 */
export const SEARCH_HANDOFF_COLLAPSE_LEAD_MS = 160;

/**
 * Fallback if `AnimatePresence.onExitComplete` never fires (unfocused
 * WKWebView can stall rAF; see App.tsx hide-commit notes). Slightly longer
 * than collapse lead + outer fade (~0.2s) so real animation usually wins;
 * caps stuck exiting at ~500ms.
 */
export const SEARCH_HANDOFF_EXIT_FALLBACK_MS = 500;
