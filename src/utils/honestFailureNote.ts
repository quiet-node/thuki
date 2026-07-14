import type { SearchFailReason } from '../hooks/useModel';

/**
 * User-facing body for total citation failure. Must match
 * `HONEST_FAILURE_NOTE_BODY` in `src-tauri/src/websearch/cite_check.rs`.
 * Backend appends this plain string (no markdown emphasis); ChatBubble
 * peels it off and styles it as the L3 hairline rail.
 */
export const HONEST_FAILURE_NOTE_BODY =
  "Thuki found sources but could not verify the answer's citations against the page text. Treat specific claims carefully.";

/**
 * User-facing body for a search that could not reach the web at all. Rendered
 * in the same L3 hairline-rail note as {@link HONEST_FAILURE_NOTE_BODY}; driven
 * by the backend's `StreamChunk::SearchFailed { reason: 'unreachable' }`.
 */
export const SEARCH_UNREACHABLE_NOTE_BODY =
  "Couldn't reach the web to verify this. This answer is from the model's own knowledge and may be out of date. Check your internet connection and try again.";

/**
 * User-facing body for a search that reached the web but found nothing current.
 * Rendered in the same L3 hairline-rail note; driven by the backend's
 * `StreamChunk::SearchFailed { reason: 'no_results' }`.
 */
export const SEARCH_NO_RESULTS_NOTE_BODY =
  "Searched the web but found nothing current for this. This answer is from the model's own knowledge. Try rephrasing your question.";

/**
 * Maps a search-failure reason to its L3 hairline-rail note body. Pure.
 */
export function searchFailNoteBody(reason: SearchFailReason): string {
  return reason === 'unreachable'
    ? SEARCH_UNREACHABLE_NOTE_BODY
    : SEARCH_NO_RESULTS_NOTE_BODY;
}

export interface HonestFailureNoteSplit {
  /** Answer markdown with the trailing honesty note removed (may be empty). */
  body: string;
  /** Stable note body when present; always the unwrapped plain string. */
  note: string | null;
}

/**
 * Detect and split a trailing total-citation-failure honesty note from
 * assistant content. Matches the stable body string, optionally wrapped in
 * legacy markdown italics (`*...*`) for messages stored before FE-owned style.
 * Pure: no side effects.
 */
export function splitHonestFailureNote(
  content: string,
): HonestFailureNoteSplit {
  if (!content) {
    return { body: content, note: null };
  }

  const trimmed = content.trimEnd();
  // Current backend: plain body. Legacy: markdown italic wrappers.
  const suffixes = [HONEST_FAILURE_NOTE_BODY, `*${HONEST_FAILURE_NOTE_BODY}*`];

  for (const suffix of suffixes) {
    if (trimmed === suffix) {
      return { body: '', note: HONEST_FAILURE_NOTE_BODY };
    }
    // Backend joins with a blank line when answer body is non-empty.
    const sep = `\n\n${suffix}`;
    if (trimmed.endsWith(sep)) {
      return {
        body: trimmed.slice(0, -sep.length).trimEnd(),
        note: HONEST_FAILURE_NOTE_BODY,
      };
    }
  }

  return { body: content, note: null };
}
