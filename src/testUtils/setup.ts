import '@testing-library/jest-dom/vitest';
import { act, cleanup } from '@testing-library/react';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, vi } from 'vitest';
import { handlers } from './mocks/handlers';
import { clearEventHandlers, resetChannelCapture } from './mocks/tauri';

export const server = setupServer(...handlers);

/**
 * Counter for deterministic blob URL generation in tests.
 * Reset between tests to ensure predictable URL values.
 */
let blobUrlCounter = 0;

beforeAll(() => {
  server.listen({ onUnhandledRequest: 'error' });
});

afterEach(async () => {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
  server.resetHandlers();
  cleanup();
  clearEventHandlers();
  resetChannelCapture();
  blobUrlCounter = 0;
  vi.restoreAllMocks();
});

afterAll(() => {
  server.close();
});

// ─── Browser API mocks (jsdom gaps) ─────────────────────────────────────────
//
// jsdom provides a DOM environment but doesn't implement all browser APIs.
// These mocks fill in the gaps for test scenarios.

/**
 * Mock ResizeObserver: allows tests to observe element resize events.
 *
 * The mock stores observed elements but doesn't actually call the callback
 * (since jsdom doesn't calculate layout). Tests can manually trigger resize
 * logic by calling observer.observe() then manually checking sizes or
 * dispatching resize events if needed.
 */
class MockResizeObserver {
  callback: ResizeObserverCallback;
  elements: Element[] = [];

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }
  observe(el: Element) {
    this.elements.push(el);
  }
  unobserve(el: Element) {
    this.elements = this.elements.filter((e) => e !== el);
  }
  disconnect() {
    this.elements = [];
  }
}

globalThis.ResizeObserver =
  MockResizeObserver as unknown as typeof ResizeObserver;

Object.defineProperty(navigator, 'clipboard', {
  value: {
    writeText: vi.fn(async () => {}),
    readText: vi.fn(async () => ''),
  },
  writable: true,
});

/**
 * Mock requestAnimationFrame: calls the callback synchronously.
 *
 * Tests expect synchronous execution; real requestAnimationFrame would batch
 * updates across multiple frames. The synchronous version simplifies test logic
 * (no need to await animation frames) and works well with Framer Motion's stub.
 */
globalThis.requestAnimationFrame = (cb: FrameRequestCallback) => {
  cb(0);
  return 0;
};
globalThis.cancelAnimationFrame = () => {};

/**
 * Mock URL.createObjectURL / revokeObjectURL: jsdom doesn't implement Blob URLs.
 * Returns a deterministic fake blob URL so tests can assert against it.
 */
URL.createObjectURL = vi.fn(
  () => `blob:http://localhost/fake-blob-${++blobUrlCounter}`,
);
URL.revokeObjectURL = vi.fn();

/**
 * Mock Range.getBoundingClientRect / getClientRects: jsdom doesn't implement
 * them. Lexical reads the range rect when syncing the DOM selection after a
 * programmatic edit (the AskBar input's controlled value sync calls
 * selectEnd()), which would otherwise throw "getBoundingClientRect is not a
 * function". Layout is irrelevant in jsdom, so an empty rect is sufficient.
 */
const ZERO_RECT: DOMRect = {
  x: 0,
  y: 0,
  width: 0,
  height: 0,
  top: 0,
  right: 0,
  bottom: 0,
  left: 0,
  toJSON: () => ({}),
};
if (!Range.prototype.getBoundingClientRect) {
  Range.prototype.getBoundingClientRect = () => ZERO_RECT;
}
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () =>
    ({
      length: 0,
      item: () => null,
      [Symbol.iterator]: function* () {},
    }) as unknown as DOMRectList;
}

/**
 * Polyfill ClipboardEvent: jsdom doesn't define it. Lexical's plain-text paste
 * handler references the global `ClipboardEvent` (objectKlassEquals(event,
 * ClipboardEvent)) when the AskBar input lets a non-image paste fall through,
 * so the bare reference throws without this stub. Paste mocks supply their own
 * clipboardData (RTL assigns it onto the event), so this only needs to exist.
 */
if (typeof globalThis.ClipboardEvent === 'undefined') {
  class ClipboardEvent extends Event {
    clipboardData: DataTransfer | null;
    constructor(
      type: string,
      eventInitDict: { clipboardData?: unknown } & EventInit = {},
    ) {
      super(type, eventInitDict);
      this.clipboardData =
        (eventInitDict.clipboardData as DataTransfer | null) ?? null;
    }
  }
  globalThis.ClipboardEvent =
    ClipboardEvent as unknown as typeof globalThis.ClipboardEvent;
}
