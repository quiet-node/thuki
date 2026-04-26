import { vi } from 'vitest';

type FocusListener = (event: { payload: boolean }) => void;
const focusListeners: Set<FocusListener> = new Set();

let mockLabel = 'main';
/** Test helper: switch the active window label between renders. */
export function __setWindowLabel(label: string) {
  mockLabel = label;
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
