import type { FocusEvent } from 'react';

/**
 * `onFocus` guard that drops focus when an element is focused programmatically
 * rather than by the user.
 *
 * When a Thuki panel (Settings or the overlay) is hidden and shown again,
 * `show_and_make_key()` calls AppKit's `makeFirstResponder:`, and WebKit
 * restores focus to the last-focused element. That programmatic refocus carries
 * no `relatedTarget`, whereas a real keyboard Tab always does. Without this
 * guard the restored element re-triggers `:focus-visible`, leaving an accent
 * ring around the last-clicked control every time the panel reopens.
 *
 * Blurring only the `relatedTarget === null` case removes that stray ring while
 * leaving genuine keyboard focus (and its ring) untouched. Attach it only to
 * navigational controls (sidebar tabs, segmented controls) that have no reason
 * to hold focus across a reopen, never to inputs or popovers that intentionally
 * autofocus.
 */
export function blurOnProgrammaticFocus(e: FocusEvent<HTMLElement>): void {
  if (e.relatedTarget === null) e.currentTarget.blur();
}
