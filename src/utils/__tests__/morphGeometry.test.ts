import { describe, it, expect } from 'vitest';
import {
  computeExpandTarget,
  computeCollapseTarget,
  anchorToTransformOrigin,
  type MorphAnchor,
} from '../morphGeometry';

// A 1440x900 monitor at the origin, a 48px icon, and a 400x700 panel.
const MON = { x: 0, y: 0, w: 1440, h: 900 };
const PANEL = { w: 400, h: 700 };
const ICON = 48;

describe('computeExpandTarget', () => {
  it('icon in the top-left zone → anchor top-left, grows down-right', () => {
    const r = computeExpandTarget({ x: 100, y: 80, size: ICON }, MON, PANEL);
    expect(r.anchor).toBe('tl');
    expect(r.x).toBe(100); // keeps the icon's left edge
    expect(r.y).toBe(80); // keeps the icon's top edge
    expect(r.growsUpward).toBe(false);
  });

  it('icon near the right edge → anchor top-right, shifts left', () => {
    // Icon flush against the right edge (x 1392 + 48 = 1440). Growing right
    // (1392 + 400 = 1792 > 1440) overflows → anchor right.
    const r = computeExpandTarget({ x: 1392, y: 80, size: ICON }, MON, PANEL);
    expect(r.anchor).toBe('tr');
    // panel right edge pinned to icon right edge (1392 + 48 = 1440),
    // so left = 1440 - 400 = 1040.
    expect(r.x).toBe(1392 + ICON - PANEL.w);
    expect(r.y).toBe(80);
    expect(r.growsUpward).toBe(false);
  });

  it('icon near the bottom edge → anchor bottom-left, grows upward', () => {
    // icon.y 850 + panel 700 = 1550 > 900 → overflow bottom → anchor bottom.
    const r = computeExpandTarget({ x: 100, y: 850, size: ICON }, MON, PANEL);
    expect(r.anchor).toBe('bl');
    expect(r.x).toBe(100);
    // panel bottom edge pinned to icon bottom edge (850 + 48 = 898),
    // so top = 898 - 700 = 198.
    expect(r.y).toBe(850 + ICON - PANEL.h);
    expect(r.growsUpward).toBe(true);
  });

  it('icon in the bottom-right corner → anchor bottom-right, grows up-left', () => {
    const r = computeExpandTarget({ x: 1392, y: 850, size: ICON }, MON, PANEL);
    expect(r.anchor).toBe('br');
    expect(r.x).toBe(1392 + ICON - PANEL.w);
    expect(r.y).toBe(850 + ICON - PANEL.h);
    expect(r.growsUpward).toBe(true);
  });

  it('icon in the upper-left area (fits both ways) → top-left', () => {
    // Must sit high enough that the 700px panel fits below it (y + 700 <= 900
    // → y <= 200) and left enough that 400px fits to the right.
    const r = computeExpandTarget({ x: 700, y: 150, size: ICON }, MON, PANEL);
    expect(r.anchor).toBe('tl');
    expect(r.x).toBe(700);
    expect(r.y).toBe(150);
  });

  it('boundary: panel right edge exactly on the monitor edge stays top-left', () => {
    // icon.x chosen so icon.x + panel.w === monitor right (1440).
    const iconX = 1440 - PANEL.w; // 1040
    const r = computeExpandTarget({ x: iconX, y: 80, size: ICON }, MON, PANEL);
    expect(r.anchor).toBe('tl'); // `<=` is inclusive → still fits
    expect(r.x).toBe(iconX);
  });

  it('boundary: one pixel past the edge flips to right anchor', () => {
    const iconX = 1440 - PANEL.w + 1; // 1041 → overflows by 1
    const r = computeExpandTarget({ x: iconX, y: 80, size: ICON }, MON, PANEL);
    expect(r.anchor).toBe('tr');
    expect(r.x).toBe(iconX + ICON - PANEL.w);
  });

  it('panel wider than the monitor → clamps to the monitor left edge', () => {
    const narrow = { x: 0, y: 0, w: 300, h: 900 };
    const r = computeExpandTarget({ x: 250, y: 80, size: ICON }, narrow, PANEL);
    // 250 + 400 > 300 → anchor right → x = 250+48-400 = -102 → clamp lo=0.
    expect(r.anchor).toBe('tr');
    expect(r.x).toBe(0);
  });

  it('panel taller than the monitor → clamps to the monitor top edge', () => {
    const shortMon = { x: 0, y: 0, w: 1440, h: 500 };
    const r = computeExpandTarget(
      { x: 100, y: 400, size: ICON },
      shortMon,
      PANEL,
    );
    // 400 + 700 > 500 → anchor bottom → y = 400+48-700 = -252 → clamp lo=0.
    expect(r.anchor).toBe('bl');
    expect(r.y).toBe(0);
  });

  it('second monitor with non-zero offset → right edge uses monitor.x + w', () => {
    const mon2 = { x: 1440, y: 0, w: 1920, h: 1080 };
    // icon near mon2's right edge: 3260 + 400 = 3660 > 1440+1920=3360 → right.
    const r = computeExpandTarget({ x: 3260, y: 100, size: ICON }, mon2, PANEL);
    expect(r.anchor).toBe('tr');
    expect(r.x).toBe(3260 + ICON - PANEL.w);
    // comfortably inside vertically.
    expect(r.y).toBe(100);
    expect(r.growsUpward).toBe(false);
  });

  it('second monitor with non-zero offset, icon in its left zone → top-left', () => {
    const mon2 = { x: 1440, y: 0, w: 1920, h: 1080 };
    const r = computeExpandTarget({ x: 1500, y: 100, size: ICON }, mon2, PANEL);
    expect(r.anchor).toBe('tl');
    expect(r.x).toBe(1500);
    expect(r.y).toBe(100);
  });
});

describe('computeCollapseTarget', () => {
  // The expanded chat occupies this rect.
  const FRAME = { x: 1058, y: 80, w: 400, h: 700 };

  it('top-left anchor keeps the frame top-left', () => {
    expect(computeCollapseTarget(FRAME, 'tl', ICON)).toEqual({
      x: 1058,
      y: 80,
    });
  });

  it('top-right anchor pins the icon to the frame top-right', () => {
    expect(computeCollapseTarget(FRAME, 'tr', ICON)).toEqual({
      x: 1058 + 400 - ICON,
      y: 80,
    });
  });

  it('bottom-left anchor pins the icon to the frame bottom-left', () => {
    expect(computeCollapseTarget(FRAME, 'bl', ICON)).toEqual({
      x: 1058,
      y: 80 + 700 - ICON,
    });
  });

  it('bottom-right anchor pins the icon to the frame bottom-right', () => {
    expect(computeCollapseTarget(FRAME, 'br', ICON)).toEqual({
      x: 1058 + 400 - ICON,
      y: 80 + 700 - ICON,
    });
  });

  it(
    'round-trips with computeExpandTarget: expanding from a corner then ' +
      'collapsing returns the icon to its origin',
    () => {
      // Icon parked near the bottom-right of the monitor (on-screen).
      const icon = { x: 1392, y: 850, size: ICON };
      const exp = computeExpandTarget(icon, MON, PANEL);
      // The expanded window occupies (exp.x, exp.y, panel).
      const frame = { x: exp.x, y: exp.y, w: PANEL.w, h: PANEL.h };
      const back = computeCollapseTarget(frame, exp.anchor, ICON);
      // The collapsed 48px window returns to the icon's original top-left.
      expect(back).toEqual({ x: icon.x, y: icon.y });
    },
  );
});

describe('anchorToTransformOrigin', () => {
  it.each<[MorphAnchor, string]>([
    ['tl', 'top left'],
    ['tr', 'top right'],
    ['bl', 'bottom left'],
    ['br', 'bottom right'],
  ])('%s → %s', (anchor, origin) => {
    expect(anchorToTransformOrigin(anchor)).toBe(origin);
  });
});
