import { vi } from 'vitest';

const mockWindow = {
  setSize: vi.fn(async () => {}),
  setPosition: vi.fn(async () => {}),
  hide: vi.fn(async () => {}),
  show: vi.fn(async () => {}),
  setFocus: vi.fn(async () => {}),
  startDragging: vi.fn(async () => {}),
};

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
