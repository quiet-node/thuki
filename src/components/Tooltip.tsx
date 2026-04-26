/**
 * Lightweight animated tooltip for icon buttons.
 *
 * Renders below the trigger element via Portal so it escapes any
 * overflow clipping in the header bar. Animation is inspired by
 * hedera-glance's tooltip: opacity + scale + y, with a custom
 * cubic-bezier for a snappy, premium feel.
 */

import { AnimatePresence, motion } from 'framer-motion';
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

interface TooltipProps {
  /** Short label to display inside the tooltip. */
  label: string;
  /** The trigger element: usually an icon button. */
  children: React.ReactNode;
  /**
   * When true, the tooltip box preserves newlines in `label` and wraps long
   * lines at a ~320px max width. Single-line icon tooltips should leave this
   * off for the tight one-line presentation.
   */
  multiline?: boolean;
  /**
   * Where to render the tooltip relative to the trigger. Defaults to
   * `'bottom'` (matches the original header-bar behavior). Use `'top'` for
   * triggers near the bottom of the window where the bottom-anchored
   * tooltip would clip past the viewport edge.
   */
  placement?: 'top' | 'bottom';
  /** Extra classes appended to the wrapper div (e.g. flex layout helpers). */
  className?: string;
}

export function Tooltip({
  label,
  children,
  multiline = false,
  placement = 'bottom',
  className,
}: TooltipProps) {
  const [isVisible, setIsVisible] = useState(false);
  /** Defer portal mount until after first hover (lazy load). */
  const [hasActivated, setHasActivated] = useState(false);
  /**
   * `left` - clamped horizontal center of the tooltip box (px from viewport left).
   * `top` - vertical position below the trigger (px from viewport top).
   * `arrowOffset` - how far the arrow shifts from center (px) so it keeps pointing
   *   at the trigger even when the box is clamped away from the window edge.
   */
  const [coords, setCoords] = useState({ left: 0, top: 0, arrowOffset: 0 });
  const triggerRef = useRef<HTMLDivElement>(null);

  const updatePosition = () => {
    /* v8 ignore start */
    if (!triggerRef.current) return;
    /* v8 ignore stop */
    const rect = triggerRef.current.getBoundingClientRect();
    const rawLeft = rect.left + rect.width / 2;
    // Half-width estimate matched to the rendered max-width of each
    // variant. Single-line tooltips fit "Conversation history"
    // worst-case (~180px wide). Multiline tooltips render at
    // max-w-[220px], so a 110px halfWidth keeps the centered box
    // directly under the trigger even when the trigger sits near the
    // right edge of a typical Thuki overlay (600px wide).
    const tooltipHalfWidth = multiline ? 110 : 90;
    const edgePadding = 8;
    const left = Math.max(
      tooltipHalfWidth + edgePadding,
      Math.min(window.innerWidth - tooltipHalfWidth - edgePadding, rawLeft),
    );
    setCoords({
      left,
      // For top placement we anchor at the trigger's top edge with an 8px
      // gap; the outer wrapper translates the box up by its own height
      // (-100% on Y) so we never need to measure the box ahead of time.
      top: placement === 'top' ? rect.top - 8 : rect.bottom + 8,
      arrowOffset: rawLeft - left,
    });
  };

  const handleMouseEnter = () => {
    if (!hasActivated) setHasActivated(true);
    updatePosition();
    setIsVisible(true);
  };

  /**
   * Hides the tooltip. Fired on both `mouseleave` and `mousedown` so a click
   * on a tooltipped trigger that opens a popup (e.g. the model picker)
   * dismisses the tooltip instead of letting it overlap the popup. The
   * tooltip reappears normally on the next fresh hover.
   */
  const handleMouseLeave = () => {
    setIsVisible(false);
  };

  useEffect(() => {
    const handleWindowFocus = () => setIsVisible(false);
    window.addEventListener('focus', handleWindowFocus);
    return () => window.removeEventListener('focus', handleWindowFocus);
  }, []);

  return (
    <div
      ref={triggerRef}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      onMouseDown={handleMouseLeave}
      // React's synthetic focus events bubble, so a focused descendant
      // (e.g. an icon button wrapped by Tooltip) reveals the tooltip for
      // keyboard-only users without needing the mouse.
      onFocus={handleMouseEnter}
      onBlur={handleMouseLeave}
      className={`inline-flex${className ? ` ${className}` : ''}`}
    >
      {children}

      {hasActivated &&
        createPortal(
          <AnimatePresence>
            {isVisible && (
              /*
               * Outer div owns fixed positioning + centering transform.
               * Keeping it separate from the motion.div prevents Framer
               * Motion's transform pipeline from discarding translateX(-50%).
               */
              <div
                style={{
                  position: 'fixed',
                  left: coords.left,
                  top: coords.top,
                  // For top placement, also shift the box up by its own
                  // height so the bottom edge sits 8px above the trigger.
                  transform:
                    placement === 'top'
                      ? 'translate(-50%, -100%)'
                      : 'translateX(-50%)',
                  pointerEvents: 'none',
                  zIndex: 9999,
                }}
              >
                <motion.div
                  initial={{
                    opacity: 0,
                    scale: 0.92,
                    y: placement === 'top' ? 4 : -4,
                  }}
                  animate={{ opacity: 1, scale: 1, y: 0 }}
                  exit={{
                    opacity: 0,
                    scale: 0.92,
                    y: placement === 'top' ? 4 : -4,
                  }}
                  transition={{ duration: 0.18, ease: [0.23, 1, 0.32, 1] }}
                >
                  {/* Arrow pointing toward the trigger.
                      For bottom placement the arrow sits on the top edge;
                      for top placement it sits on the bottom edge. The
                      `left` is adjusted by `arrowOffset` so it tracks the
                      button center even when the tooltip box is clamped
                      sideways. */}
                  <div
                    aria-hidden="true"
                    style={{
                      left: `calc(50% + ${coords.arrowOffset}px)`,
                    }}
                    className={`absolute h-3 w-3 -translate-x-1/2 rotate-45 bg-surface-base ${
                      placement === 'top'
                        ? '-bottom-1.5 border-b border-r border-surface-border'
                        : '-top-1.5 border-l border-t border-surface-border'
                    }`}
                  />
                  <div
                    style={multiline ? { width: 225 } : undefined}
                    className={`relative rounded-lg border border-surface-border bg-surface-base px-2.5 py-1.5 text-[11px] text-text-primary shadow-chat ${
                      multiline
                        ? 'whitespace-pre-line leading-snug'
                        : 'whitespace-nowrap'
                    }`}
                  >
                    {label}
                  </div>
                </motion.div>
              </div>
            )}
          </AnimatePresence>,
          document.body,
        )}
    </div>
  );
}
