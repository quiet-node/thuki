/**
 * macOS-style traffic light window controls.
 *
 * Renders a thin header bar with close/minimize/zoom dots on the left.
 * On hover, the close button reveals an × icon; minimize and zoom dots
 * remain grayed as decorative elements (panel windows do not support
 * minimize or fullscreen).
 *
 * The bar surface doubles as a drag handle for window repositioning.
 * A subtle divider at the bottom visually separates the controls from
 * the chat messages area below.
 */

import { memo } from 'react';

interface WindowControlsProps {
  /** Triggers the overlay hide animation sequence. */
  onClose: () => void;
  /** Initiates native window drag from the bar surface. */
  onDragStart: (e: React.MouseEvent) => void;
}

/** Decorative dot color for inactive buttons. */
const INACTIVE_DOT = 'rgba(255, 255, 255, 0.12)';

export const WindowControls = memo(function WindowControls({
  onClose,
  onDragStart,
}: WindowControlsProps) {
  return (
    <div className="shrink-0">
      <div
        onMouseDown={onDragStart}
        className="group flex items-center px-4 py-2.5"
      >
        {/* Close button — reveals × icon on group hover */}
        <button
          type="button"
          onClick={onClose}
          className="w-3 h-3 rounded-full bg-[#FF5F57] flex items-center justify-center transition-transform duration-150 hover:scale-125 active:scale-90"
          aria-label="Close window"
        >
          <svg
            width="6"
            height="6"
            viewBox="0 0 6 6"
            className="opacity-0 group-hover:opacity-100 transition-opacity duration-150"
            aria-hidden="true"
          >
            <path
              d="M0.5 0.5L5.5 5.5M5.5 0.5L0.5 5.5"
              stroke="rgba(0,0,0,0.6)"
              strokeWidth="1.2"
              strokeLinecap="round"
            />
          </svg>
        </button>

        {/* Minimize — decorative only */}
        <div
          className="w-3 h-3 rounded-full ml-2"
          style={{ backgroundColor: INACTIVE_DOT }}
          aria-hidden="true"
        />

        {/* Zoom — decorative only */}
        <div
          className="w-3 h-3 rounded-full ml-2"
          style={{ backgroundColor: INACTIVE_DOT }}
          aria-hidden="true"
        />
      </div>

      {/* Divider between controls and chat area */}
      <div className="h-px bg-surface-border" />
    </div>
  );
});
