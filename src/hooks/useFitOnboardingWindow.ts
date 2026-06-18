import { useLayoutEffect, type RefObject } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';

/**
 * Sizes the native onboarding window to exactly fit the measured content card,
 * then re-centers it.
 *
 * The onboarding window is transparent, so any part of the window not covered
 * by the visible card still captures mouse clicks meant for the apps behind
 * Thuki. A fixed window taller than the card therefore leaves an invisible
 * click-blocking margin. Measuring the card and matching the window to it
 * removes that margin. The fit re-runs whenever `deps` change, so the window
 * tracks the card as the ambient download strip appears or grows a line.
 *
 * Measurement uses `offsetWidth`/`offsetHeight` (the layout border box), which
 * ignores the card's entrance transform, and runs in a layout effect so the
 * resize happens before paint and the card never flashes clipped.
 *
 * A `ResizeObserver` re-fits on ANY later content change (async data loading
 * in, a conditional line appearing), so the window can never end up shorter
 * than the card and clip its bottom. `changeKey` forces an immediate re-fit
 * for the known triggers without waiting for the observer's next callback.
 */
export function useFitOnboardingWindow(
  ref: RefObject<HTMLElement | null>,
  changeKey: unknown,
): void {
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const fit = () => {
      const width = node.offsetWidth;
      const height = node.offsetHeight;
      if (width === 0 || height === 0) return;
      void (async () => {
        const win = getCurrentWindow();
        await win.setSize(new LogicalSize(width, height));
        await win.center();
      })();
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(node);
    return () => observer.disconnect();
  }, [ref, changeKey]);
}
