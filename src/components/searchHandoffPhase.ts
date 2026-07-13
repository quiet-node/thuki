/**
 * Sequential search → reasoning/answer handoff phases (Option D).
 *
 * Pure search keeps {@link SearchProgressBlock} mounted (`live`). When
 * reasoning or answer content arrives, the block must stay mounted long
 * enough for its exit animation (`exiting`) before it is permanently
 * unmounted (`done`). Phase exists so exit retention is explicit and
 * testable without relying on Framer Motion's retain-while-exiting
 * behavior in the test mock.
 */

/** Lifecycle of search progress chrome relative to reasoning/answer. */
export type SearchHandoffPhase = 'idle' | 'live' | 'exiting' | 'done';

/** Inputs that drive handoff transitions from ChatBubble render props. */
export interface SearchHandoffSignals {
  /**
   * True while pure search chrome should show: searching, not verifying,
   * and reasoning/answer has not started yet.
   */
  showLiveSearch: boolean;
  /**
   * True once thinking content, thinking-pending, or answer body exists.
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
