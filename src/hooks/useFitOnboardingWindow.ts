import { useLayoutEffect, useRef, type RefObject } from 'react';
import { invoke } from '@tauri-apps/api/core';

/**
 * Window (ms) after a step spawns during which the onboarding window keeps
 * re-centering as the card settles. A static card still reflows shortly after
 * mount (web-font load, async content sizing the matrix), so centering only the
 * very first fit leaves the window off-center once the reflow lands. Centering
 * every fit within this window absorbs the settle; centering stops afterwards so
 * a later interaction (clicking Download) or a manual drag does not snap the
 * window back to the middle.
 */
const SETTLE_MS = 1200;

/**
 * Sizes the native onboarding window to exactly fit the measured content card
 * and centers it at spawn, then resizes it in place afterwards.
 *
 * The onboarding window is transparent, so any part of the window not covered
 * by the visible card still captures mouse clicks meant for the apps behind
 * Thuki. A fixed window taller than the card therefore leaves an invisible
 * click-blocking margin. Measuring the card and matching the window to it
 * removes that margin.
 *
 * Sizing and centering are delegated to the `fit_onboarding_window` backend
 * command: positioning the window from JS did not reliably re-center it, so the
 * resize and the optional center run atomically on the macOS main thread with
 * the same `center()` Tauri uses at show time. The command centers only while
 * the step is within its spawn settle window (see `SETTLE_MS`); later fits pass
 * `center: false` and resize in place.
 *
 * Measurement uses `offsetWidth`/`offsetHeight` (the layout border box), which
 * ignores the card's entrance transform, and runs in a layout effect so the
 * resize is requested before paint. A `ResizeObserver` re-fits on any later
 * content change (async data loading in, a conditional line appearing, the
 * ambient download strip growing a line); `changeKey` forces an immediate
 * re-fit for the known triggers. Fit work is coalesced into a single
 * `requestAnimationFrame` so a burst of observer callbacks collapses to one
 * request.
 */
export function useFitOnboardingWindow(
  ref: RefObject<HTMLElement | null>,
  changeKey: unknown,
): void {
  // Deadline for the spawn settle window, set once on the first mount of this
  // step and preserved across the effect re-running when `changeKey` changes.
  const settleUntilRef = useRef(0);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    if (settleUntilRef.current === 0) {
      settleUntilRef.current = Date.now() + SETTLE_MS;
    }

    let frame = 0;

    const runFit = () => {
      const width = node.offsetWidth;
      const height = node.offsetHeight;
      if (width === 0 || height === 0) return;
      const center = Date.now() < settleUntilRef.current;
      void invoke('fit_onboarding_window', { width, height, center });
    };

    // Coalesce a burst of callbacks (the mount fit and the observer's initial
    // fire arrive in the same tick) into one request by cancelling the
    // previously scheduled frame. `cancelAnimationFrame(0)` is a harmless no-op.
    const scheduleFit = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(runFit);
    };

    scheduleFit();
    const observer = new ResizeObserver(scheduleFit);
    observer.observe(node);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [ref, changeKey]);
}
