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
 * resize happens before paint and the strip never flashes clipped.
 *
 * `changeKey` is any value that changes when the card height changes (the
 * ambient download status). The fit re-runs whenever it changes identity.
 */
export function useFitOnboardingWindow(
  ref: RefObject<HTMLElement | null>,
  changeKey: unknown,
): void {
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const width = node.offsetWidth;
    const height = node.offsetHeight;
    if (width === 0 || height === 0) return;
    void (async () => {
      const win = getCurrentWindow();
      await win.setSize(new LogicalSize(width, height));
      await win.center();
    })();
  }, [ref, changeKey]);
}
