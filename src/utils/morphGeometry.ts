/**
 * Pure geometry for the minimize/restore morph's edge-aware positioning.
 *
 * All coordinates are Tauri "logical" pixels: top-left origin, Y grows DOWN,
 * already divided by the display scale factor. There is no AppKit Y-flip in
 * this layer — callers must convert physical → logical (divide by scale)
 * before calling, and pass the results straight to `set_window_frame`.
 *
 * The floating mascot icon is a small square (`iconSize`, 68px) that the user
 * can park anywhere. On expand it grows into a `panel` (the full chat window).
 * To avoid clipping off a screen edge AND to avoid the icon visually jumping,
 * we pick which CORNER of the panel is pinned to the icon's corresponding
 * corner so the panel unfolds into the open space (the Floating UI
 * `flip`/`shift` rule). The pinned corner is also the CSS transform-origin and
 * where the persistent mascot is rendered, so the icon appears stationary
 * while the chat grows out of it.
 */

/** Which corner of the panel is pinned to the icon. */
export type MorphAnchor = 'tl' | 'tr' | 'bl' | 'br';

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface ExpandTarget {
  /** Pinned corner — drives transform-origin and the mascot's position. */
  anchor: MorphAnchor;
  /** Window top-left (logical) so the pinned corner sits on the icon. */
  x: number;
  y: number;
  /** True when the panel is bottom-anchored (it grew upward). */
  growsUpward: boolean;
}

/** Clamp `v` into `[lo, hi]`. When `lo > hi` (panel larger than the axis), the
 * lower bound wins, pinning to the monitor's near edge (best effort). */
function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(v, hi));
}

/**
 * Given the icon's screen rect, the monitor it's on, and the panel size,
 * decide the pinned corner and the resulting window top-left.
 *
 * Rule per axis: prefer growing in the natural direction (right / down) from
 * the icon's near corner; if that would overflow the monitor's far edge, pin
 * the opposite edge instead (grow left / up). A final clamp keeps the window
 * fully on the monitor even in degenerate cases (panel wider/taller than the
 * monitor → pins to the monitor's top-left).
 */
export function computeExpandTarget(
  icon: { x: number; y: number; size: number },
  monitor: Rect,
  panel: { w: number; h: number },
): ExpandTarget {
  // Growing right/down keeps the far edge on screen?
  const growRightFits = icon.x + panel.w <= monitor.x + monitor.w;
  const growDownFits = icon.y + panel.h <= monitor.y + monitor.h;

  const anchorRight = !growRightFits;
  const anchorBottom = !growDownFits;

  // Left anchors keep the icon's left edge as the panel's left; right anchors
  // pin the panel's right edge to the icon's right edge (icon.x + icon.size).
  let x = anchorRight ? icon.x + icon.size - panel.w : icon.x;
  let y = anchorBottom ? icon.y + icon.size - panel.h : icon.y;

  x = clamp(x, monitor.x, monitor.x + monitor.w - panel.w);
  y = clamp(y, monitor.y, monitor.y + monitor.h - panel.h);

  const anchor =
    `${anchorBottom ? 'b' : 't'}${anchorRight ? 'r' : 'l'}` as MorphAnchor;

  return { anchor, x, y, growsUpward: anchorBottom };
}

/**
 * Given the current (expanded) window rect and the pinned corner, compute the
 * top-left for the collapsed `iconSize`×`iconSize` window so that its pinned
 * corner stays exactly on the panel's pinned corner. The icon therefore
 * appears at the same screen point the chat folded into.
 */
export function computeCollapseTarget(
  frame: Rect,
  anchor: MorphAnchor,
  iconSize: number,
): { x: number; y: number } {
  const right = anchor === 'tr' || anchor === 'br';
  const bottom = anchor === 'bl' || anchor === 'br';
  return {
    x: right ? frame.x + frame.w - iconSize : frame.x,
    y: bottom ? frame.y + frame.h - iconSize : frame.y,
  };
}

/** Maps a `MorphAnchor` to a CSS `transform-origin` value. */
export function anchorToTransformOrigin(anchor: MorphAnchor): string {
  const vertical = anchor[0] === 'b' ? 'bottom' : 'top';
  const horizontal = anchor[1] === 'r' ? 'right' : 'left';
  return `${vertical} ${horizontal}`;
}
