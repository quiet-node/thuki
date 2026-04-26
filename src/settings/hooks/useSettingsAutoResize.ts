/**
 * Auto-sizes the Settings NSWindow to the active tab's content.
 *
 * Why: each tab has a different natural height (AI ~520, Display ~720,
 * Web ~860). Using a single fixed window height wastes whitespace on
 * light tabs and forces unnecessary scroll on heavy ones. This hook
 * makes the window hug its content with a smooth animation on tab
 * switch, while still capping at MAX_HEIGHT so the window never
 * overflows the screen.
 *
 * Mechanism:
 * - A `ResizeObserver` on the supplied content element fires whenever
 *   the natural content height changes (tab switch, textarea grow,
 *   marker banner appear/disappear).
 * - Target window height = `content + chromeHeight`, clamped to
 *   `[MIN_HEIGHT, MAX_HEIGHT]`. Above MAX_HEIGHT the body retains its
 *   `overflow-y: auto` so the user can still scroll the tail.
 * - The first measurement snaps the window without animating so the
 *   panel does not visibly settle on open.
 * - Subsequent changes interpolate between the last sent size and the
 *   new target via `requestAnimationFrame`, easing out over
 *   `ANIMATE_MS`. Each frame issues one `setSize`; macOS resizes the
 *   real NSWindow at 60 Hz.
 * - Tiny corrections (`< NEGLIGIBLE_DELTA_PX`) are no-ops to avoid
 *   thrashing on sub-pixel ResizeObserver entries.
 */

import { useEffect, useLayoutEffect, useRef } from 'react';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

const ANIMATE_MS = 220;
/** Hard floor: settings panel below this is unusable on macOS. */
const MIN_HEIGHT = 280;
/** Hard ceiling: prevents the window from exceeding a 13" laptop display. */
const MAX_HEIGHT = 900;
/** Settings is intentionally a fixed-width column. */
const SETTINGS_WIDTH = 580;
/** Sub-pixel ResizeObserver chatter is dropped below this threshold. */
const NEGLIGIBLE_DELTA_PX = 4;

const easeOutCubic = (t: number) => 1 - Math.pow(1 - t, 3);

function clampHeight(h: number): number {
  return Math.max(MIN_HEIGHT, Math.min(MAX_HEIGHT, h));
}

/**
 * Observes `contentRef.current.scrollHeight` and animates the OS window
 * to fit. `chromeHeight` is the constant offset from content to total
 * window height (window padding + WindowControls + tab bar + banner +
 * body padding). `revision` is any value that changes whenever the
 * caller knows the content has been replaced (e.g. the active tab id);
 * the hook re-measures synchronously on every revision change so a
 * tab swap that does not trigger a ResizeObserver entry (because the
 * new tree mounts inside the same wrapper element within a single
 * paint) still drives a resize.
 */
export function useSettingsAutoResize(
  contentRef: React.RefObject<HTMLElement | null>,
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
   * `handleResize` is recreated on every effect run, so a separate
   * effect that wants to invoke it (the revision-driven remeasure
   * below) has to reach it through a ref to avoid stale closures.
   */
  const handleResizeRef = useRef<() => void>(() => {});

  useEffect(() => {
    const el = contentRef.current;
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
        // First tick: snap without animation so the window does not
        // visibly settle when the panel mounts.
        initialisedRef.current = true;
        lastSentRef.current = target;
        void getCurrentWindow().setSize(
          new LogicalSize(SETTINGS_WIDTH, target),
        );
        return;
      }
      // initialisedRef guards above guarantee lastSentRef is set here.
      const last = lastSentRef.current as number;
      if (Math.abs(target - last) < NEGLIGIBLE_DELTA_PX) return;
      cancelAnim();
      fromRef.current = last;
      toRef.current = target;
      startTimeRef.current = performance.now();
      rafRef.current = requestAnimationFrame(tick);
    };

    const observer = new ResizeObserver(handleResize);
    observer.observe(el);
    handleResizeRef.current = handleResize;
    // ResizeObserver is spec'd to fire once on observe(), but happy-dom
    // and a few engines skip the initial tick. Fire manually so the
    // window snaps to its measured size on mount regardless.
    handleResize();

    return () => {
      observer.disconnect();
      cancelAnim();
    };
  }, [contentRef]);

  // Force a re-measure whenever the caller-supplied revision changes
  // (e.g. on tab switch). useLayoutEffect runs after DOM mutations are
  // committed but before the browser paints, so `scrollHeight` reflects
  // the freshly-mounted tab's natural height.
  useLayoutEffect(() => {
    handleResizeRef.current();
  }, [revision, chromeHeight]);
}
