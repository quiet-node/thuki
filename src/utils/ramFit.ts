/**
 * User-facing labels for the backend's RAM-fit verdict, shared by every
 * surface that shows the hint (the Library and Discover panes). Colour classes
 * stay per-component because they reference each pane's own CSS-module classes;
 * only the wording is shared here so the vocabulary cannot drift between panes.
 */

import type { RamFit } from '../types/starter';

/** Maps a RAM-fit verdict to the word shown next to a model. */
export const RAM_FIT_LABEL: Record<RamFit, string> = {
  fits: 'Comfortable',
  tight: 'Tight',
  too_big: 'Heavy',
};

/** One-line explanation shown when hovering a RAM-fit label, so the verdict
 * is not cryptic. Phrased around this Mac's memory, not raw numbers. */
export const RAM_FIT_TOOLTIP: Record<RamFit, string> = {
  fits: 'Runs with memory to spare on this Mac.',
  tight: 'Runs, but close to this Mac’s memory limit.',
  too_big: 'Larger than this Mac’s memory comfortably holds; expect slowdowns.',
};
