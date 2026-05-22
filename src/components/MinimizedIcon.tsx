import { memo, useRef } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';

interface MinimizedIconProps {
  /** True while a response is still streaming in the background. */
  isWorking: boolean;
  /** True when a response finished while minimized and has not been seen. */
  hasUnseen: boolean;
  /** Restore the parked conversation. */
  onRestore: () => void;
}

const DRAG_THRESHOLD_PX = 6;

/**
 * Floating minimized icon shown when the chat overlay is collapsed.
 *
 * Renders the Thuki logo in a small circular button. Supports:
 * - Dragging: pointer move past threshold calls the native window drag.
 * - Restore: plain click (no drag) calls onRestore.
 * - Working: the mascot does a soft elastic "jelly wobble" and a warm jewel
 *   breathes at its bottom-right corner while a response streams.
 * - Done: the mascot does one elastic pop and the jewel holds a steady amber
 *   when a reply landed while minimized and has not been seen.
 */
export const MinimizedIcon = memo(function MinimizedIcon({
  isWorking,
  hasUnseen,
  onRestore,
}: MinimizedIconProps) {
  const downPosRef = useRef<{ x: number; y: number } | null>(null);
  const draggedRef = useRef(false);

  // Wobble while working; one-shot pop the moment a reply lands; otherwise still.
  const logoMotion = isWorking
    ? ' minimized-logo-working'
    : hasUnseen
      ? ' minimized-logo-done'
      : '';

  return (
    <button
      type="button"
      aria-label="Restore Thuki"
      className="flex w-full h-full items-center justify-center cursor-pointer select-none bg-transparent p-0 border-0"
      onPointerDown={(e) => {
        downPosRef.current = { x: e.clientX, y: e.clientY };
        draggedRef.current = false;
      }}
      onPointerMove={(e) => {
        if (!downPosRef.current) return;
        const dx = e.clientX - downPosRef.current.x;
        const dy = e.clientY - downPosRef.current.y;
        if (Math.hypot(dx, dy) > DRAG_THRESHOLD_PX && !draggedRef.current) {
          draggedRef.current = true;
          void getCurrentWindow().startDragging();
        }
      }}
      onPointerUp={() => {
        const wasDrag = draggedRef.current;
        downPosRef.current = null;
        draggedRef.current = false;
        if (!wasDrag) onRestore();
      }}
    >
      {/* Logo + status jewel as one unit, centered in the (larger) window with
          margin so the working wobble / completion pop can overshoot the logo
          without being clipped by the window frame. The jewel is positioned
          relative to this 48px wrapper, so it tracks the logo's bottom-right
          corner rather than the window's. */}
      <span className="relative inline-flex">
        {/* Thuki logo: bare 48px natural shape, no background, no rounded crop. */}
        <img
          src="/thuki-logo.png"
          alt="Thuki"
          className={`w-12 h-12${logoMotion}`}
          draggable={false}
        />
        {isWorking && (
          <span
            data-testid="minimized-working"
            className="minimized-jewel minimized-jewel-working"
            aria-hidden="true"
          />
        )}
        {!isWorking && hasUnseen && (
          <span
            data-testid="minimized-ready-dot"
            className="minimized-jewel minimized-jewel-done"
            aria-hidden="true"
          />
        )}
      </span>
    </button>
  );
});
