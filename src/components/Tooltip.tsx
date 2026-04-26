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
  /** Extra classes appended to the wrapper div (e.g. flex layout helpers). */
  className?: string;
}

export function Tooltip({
  label,
  children,
  multiline = false,
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
      top: rect.bottom + 8,
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
                  transform: 'translateX(-50%)',
                  pointerEvents: 'none',
                  zIndex: 9999,
                }}
              >
                <motion.div
                  initial={{ opacity: 0, scale: 0.92, y: -4 }}
                  animate={{ opacity: 1, scale: 1, y: 0 }}
                  exit={{ opacity: 0, scale: 0.92, y: -4 }}
                  transition={{ duration: 0.18, ease: [0.23, 1, 0.32, 1] }}
                >
                  {/* Arrow pointing up toward the trigger.
                      left is adjusted by arrowOffset so it tracks the button
                      center even when the tooltip box is clamped sideways. */}
                  <div
                    aria-hidden="true"
                    style={{
                      left: `calc(50% + ${coords.arrowOffset}px)`,
                    }}
                    className="absolute -top-1.5 h-3 w-3 -translate-x-1/2 rotate-45 border-l border-t border-surface-border bg-surface-base"
                  />
                  <div
                    style={multiline ? { width: 220 } : undefined}
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
