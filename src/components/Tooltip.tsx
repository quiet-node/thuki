/**
 * Lightweight animated tooltip for icon buttons.
 *
 * Renders below the trigger element via Portal so it escapes any
 * overflow clipping in the header bar. Animation is inspired by
 * hedera-glance's tooltip: opacity + scale + y, with a custom
 * cubic-bezier for a snappy, premium feel.
 */

import { AnimatePresence, motion } from 'framer-motion';
import { useRef, useState } from 'react';
import { createPortal } from 'react-dom';

interface TooltipProps {
  /** Short label to display inside the tooltip. */
  label: string;
  /** The trigger element — usually an icon button. */
  children: React.ReactNode;
}

export function Tooltip({ label, children }: TooltipProps) {
  const [isVisible, setIsVisible] = useState(false);
  /** Defer portal mount until after first hover (lazy load). */
  const [hasActivated, setHasActivated] = useState(false);
  /**
   * `left` — clamped horizontal center of the tooltip box (px from viewport left).
   * `top` — vertical position below the trigger (px from viewport top).
   * `arrowOffset` — how far the arrow shifts from center (px) so it keeps pointing
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
    // Conservative half-width estimate for the widest label ("Conversation history").
    // Keeps the tooltip box fully inside the viewport near window edges.
    const tooltipHalfWidth = 90;
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

  const handleMouseLeave = () => {
    setIsVisible(false);
  };

  return (
    <div
      ref={triggerRef}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      className="inline-flex"
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
                  <div className="relative rounded-lg border border-surface-border bg-surface-base px-2.5 py-1.5 text-[11px] text-text-primary shadow-chat whitespace-nowrap">
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
