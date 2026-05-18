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
 * - Working pulse: animated ring when isWorking is true.
 * - Ready dot: small indicator dot when hasUnseen is true.
 */
export const MinimizedIcon = memo(function MinimizedIcon({
  isWorking,
  hasUnseen,
  onRestore,
}: MinimizedIconProps) {
  const downPosRef = useRef<{ x: number; y: number } | null>(null);
  const draggedRef = useRef(false);

  return (
    <button
      type="button"
      aria-label="Restore Thuki"
      className="relative flex items-center justify-center rounded-full bg-surface-elevated shadow-lg cursor-pointer select-none"
      style={{ width: 52, height: 52 }}
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
      {/* Thuki logo reused from AskBarView: public asset /thuki-logo.png, sized for 52px container. */}
      <img
        src="/thuki-logo.png"
        alt="Thuki"
        className="w-10 h-10 rounded-xl"
        draggable={false}
      />
      {isWorking && (
        <span
          data-testid="minimized-working"
          className="absolute inset-0 rounded-full ring-2 ring-primary/60 animate-pulse"
          aria-hidden="true"
        />
      )}
      {hasUnseen && (
        <span
          data-testid="minimized-ready-dot"
          className="absolute -top-0.5 -right-0.5 w-3 h-3 rounded-full bg-primary border border-surface-border"
          aria-hidden="true"
        />
      )}
    </button>
  );
});
