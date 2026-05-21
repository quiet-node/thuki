import { vi } from 'vitest';

type FocusListener = (event: { payload: boolean }) => void;
const focusListeners: Set<FocusListener> = new Set();

let mockLabel = 'main';
/** Test helper: switch the active window label between renders. */
export function __setWindowLabel(label: string) {
  mockLabel = label;
}

/** Internal geometry state used by mockWindow and currentMonitor. */
let _windowGeometry = {
  x: 0,
  y: 0,
  scale: 1,
  width: 400,
  height: 700,
  monitorX: 0,
  monitorY: 0,
  monitorWidth: 1440,
  monitorHeight: 900,
  monitorNull: false,
};

/**
 * Test helper: configure the window position and monitor geometry used by
 * outerPosition(), scaleFactor(), and currentMonitor(). Call before the
 * action that triggers handleRestore geometry queries.
 */
export function __setWindowGeometry(opts: {
  x?: number;
  y?: number;
  scale?: number;
  width?: number;
  height?: number;
  monitorX?: number;
  monitorY?: number;
  monitorWidth?: number;
  monitorHeight?: number;
  monitorNull?: boolean;
}) {
  _windowGeometry = { ..._windowGeometry, ...opts };
  mockWindow.outerPosition.mockResolvedValue({
    x: _windowGeometry.x,
    y: _windowGeometry.y,
  });
  mockWindow.outerSize.mockResolvedValue({
    width: _windowGeometry.width,
    height: _windowGeometry.height,
  });
  mockWindow.scaleFactor.mockResolvedValue(_windowGeometry.scale);
}

const mockWindow = {
  get label() {
    return mockLabel;
  },
  setSize: vi.fn(async () => {}),
  setPosition: vi.fn(async () => {}),
  hide: vi.fn(async () => {}),
  show: vi.fn(async () => {}),
  setFocus: vi.fn(async () => {}),
  startDragging: vi.fn(async () => {}),
  outerPosition: vi.fn(async () => ({ x: 0, y: 0 })),
  outerSize: vi.fn(async () => ({ width: 400, height: 700 })),
  scaleFactor: vi.fn(async () => 1),
  /**
   * Mirrors Tauri's `Window.onFocusChanged` API. Returns an unlisten
   * function. Tests can drive it via `__emitFocus(true|false)`.
   */
  onFocusChanged: vi.fn(async (cb: FocusListener) => {
    focusListeners.add(cb);
    return () => {
      focusListeners.delete(cb);
    };
  }),
};

/** Test helper: fire a synthetic window focus event. */
export function __emitFocus(focused: boolean) {
  for (const cb of focusListeners) cb({ payload: focused });
}

/** Test helper: drop all listeners between tests. */
export function __resetFocusListeners() {
  focusListeners.clear();
}

export function getCurrentWindow() {
  return mockWindow;
}

/**
 * Mock for Tauri's currentMonitor() function from @tauri-apps/api/window.
 * Returns a Monitor-shaped object by default, or null when monitorNull is set.
 */
export const currentMonitor = vi.fn(async () => {
  if (_windowGeometry.monitorNull) return null;
  return {
    name: 'mock-monitor',
    size: {
      width: _windowGeometry.monitorWidth,
      height: _windowGeometry.monitorHeight,
    },
    position: { x: _windowGeometry.monitorX, y: _windowGeometry.monitorY },
    workArea: {
      position: { x: _windowGeometry.monitorX, y: _windowGeometry.monitorY },
      size: {
        width: _windowGeometry.monitorWidth,
        height: _windowGeometry.monitorHeight,
      },
    },
    scaleFactor: _windowGeometry.scale,
  };
});

export class LogicalSize {
  width: number;
  height: number;
  constructor(width: number, height: number) {
    this.width = width;
    this.height = height;
  }
}

export class LogicalPosition {
  x: number;
  y: number;
  constructor(x: number, y: number) {
    this.x = x;
    this.y = y;
  }
}

export { mockWindow as __mockWindow };
