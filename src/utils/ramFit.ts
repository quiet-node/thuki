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
