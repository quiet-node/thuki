/**
 * Auto-sizes the Settings NSWindow to the active tab's content.
 *
 * Why: each tab has a different natural height (AI ~480, Display ~720,
 * Web ~860). Using a single fixed window height wastes whitespace on
 * light tabs and forces unnecessary scroll on heavy ones. This hook
 * makes the window hug its content with a smooth animation on tab
 * switch, while still capping at MAX_HEIGHT so the window never
 * overflows the screen.
 *
 * Mechanism:
 * - Caller passes the content element directly (driven by a state-backed
 *   callback ref so the effect re-runs when the element mounts; a
 *   plain `useRef` would not, which was the original cause of the
 *   "first render returned null, effect saw null, never re-ran"
 *   bug).
 * - A `ResizeObserver` on that element fires whenever its natural
 *   height changes (textarea grow, banner appear).
 * - A separate layout-effect re-measures synchronously on every
 *   `revision` change (active tab id) since React unmount+mount of
 *   the panel children inside the same wrapper does not always
 *   trigger a ResizeObserver entry within a single paint frame.
 * - Target window height = `content + chromeHeight`, clamped to
 *   `[MIN_HEIGHT, MAX_HEIGHT]`. Above MAX_HEIGHT the body retains its
 *   `overflow-y: auto` so the user can still scroll the tail.
 * - The first measurement snaps the window without animating so the
 *   panel does not visibly settle on open. Subsequent changes
 *   interpolate via `requestAnimationFrame` with ease-out cubic.
 * - Sub-pixel deltas (`< NEGLIGIBLE_DELTA_PX`) are no-ops.
 */

import { useEffect, useLayoutEffect, useRef } from 'react';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

const ANIMATE_MS = 220;
/** Hard floor: settings panel below this is unusable on macOS. */
const MIN_HEIGHT = 280;
/**
 * Hard ceiling: keeps the panel comfortably small even on a 13" laptop.
 * Tabs whose natural content exceeds this (Web's full timeouts list)
 * scroll inside `.body` rather than push the window taller.
 */
const MAX_HEIGHT = 700;
/** Settings is intentionally a fixed-width column. */
const SETTINGS_WIDTH = 580;
/** Sub-pixel ResizeObserver chatter is dropped below this threshold. */
const NEGLIGIBLE_DELTA_PX = 4;

const easeOutCubic = (t: number) => 1 - Math.pow(1 - t, 3);

function clampHeight(h: number): number {
  return Math.max(MIN_HEIGHT, Math.min(MAX_HEIGHT, h));
}

/**
 * Animates the OS window to fit `el.scrollHeight + chromeHeight`. Pass
 * `null` for `el` while the content is not yet mounted; the hook will
 * (re-)attach the ResizeObserver when a non-null element arrives.
 */
export function useSettingsAutoResize(
  el: HTMLElement | null,
  chromeHeight: number,
  revision: unknown,
): void {
  const rafRef = useRef<number | null>(null);
  const initialisedRef = useRef(false);
  const lastSentRef = useRef<number | null>(null);
  const startTimeRef = useRef(0);
  const fromRef = useRef(0);
  const toRef = useRef(0);
  const chromeRef = useRef(chromeHeight);
  chromeRef.current = chromeHeight;

  /**
   * Stable handle to the latest `handleResize` so the revision-driven
   * layout effect can fire it without listing the closure values that
   * change every render in its own dependency array.
   */
  const handleResizeRef = useRef<() => void>(() => {});

  useEffect(() => {
    if (!el) return;

    const cancelAnim = () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };

    const tick = (now: number) => {
      const elapsed = now - startTimeRef.current;
      const t = Math.min(1, elapsed / ANIMATE_MS);
      const eased = easeOutCubic(t);
      const h = Math.round(
        fromRef.current + (toRef.current - fromRef.current) * eased,
      );
      if (h !== lastSentRef.current) {
        lastSentRef.current = h;
        void getCurrentWindow().setSize(new LogicalSize(SETTINGS_WIDTH, h));
      }
      if (t < 1) {
        rafRef.current = requestAnimationFrame(tick);
      } else {
        rafRef.current = null;
      }
    };

    const handleResize = () => {
      const target = clampHeight(el.scrollHeight + chromeRef.current);
      if (!initialisedRef.current) {
        initialisedRef.current = true;
        lastSentRef.current = target;
        void getCurrentWindow().setSize(
          new LogicalSize(SETTINGS_WIDTH, target),
        );
        return;
      }
      // initialisedRef guard above guarantees lastSentRef is set here.
      const last = lastSentRef.current as number;
      if (Math.abs(target - last) < NEGLIGIBLE_DELTA_PX) return;
      cancelAnim();
      fromRef.current = last;
      toRef.current = target;
      startTimeRef.current = performance.now();
      rafRef.current = requestAnimationFrame(tick);
    };

    handleResizeRef.current = handleResize;
    const observer = new ResizeObserver(handleResize);
    observer.observe(el);
    // Spec says ResizeObserver fires once on observe; fire manually to
    // cover engines that skip that initial tick (and to snap before
    // the browser would paint).
    handleResize();

    return () => {
      observer.disconnect();
      cancelAnim();
    };
  }, [el]);

  useLayoutEffect(() => {
    handleResizeRef.current();
  }, [revision, chromeHeight]);
}
