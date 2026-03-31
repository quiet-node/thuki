import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, vi } from 'vitest';
import { handlers } from './mocks/handlers';
import { clearEventHandlers, resetChannelCapture } from './mocks/tauri';

export const server = setupServer(...handlers);

beforeAll(() => {
  server.listen({ onUnhandledRequest: 'error' });
});

afterEach(() => {
  server.resetHandlers();
  cleanup();
  clearEventHandlers();
  resetChannelCapture();
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
