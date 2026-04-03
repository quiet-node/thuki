import { vi } from 'vitest';

// ─── Channel mock ───────────────────────────────────────────────────────────

type ChannelCallback<T> = (message: T) => void;

export class Channel<T = unknown> {
  onmessage: ChannelCallback<T> = () => {};

  /** Test helper: simulate a message from the Rust backend. */
  simulateMessage(data: T) {
    this.onmessage(data);
  }
}

// ─── invoke mock ────────────────────────────────────────────────────────────

export const invoke = vi.fn<
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (cmd: string, args?: Record<string, any>) => Promise<any>
>(async () => {});

/**
 * Channel capture state (per test).
 *
 * Tests should use getLastChannel() to read the captured channel after calling ask().
 * Explicitly avoid relying on module-level state by calling resetChannelCapture()
 * in beforeEach or afterEach.
 */
let lastChannel: Channel | null = null;

/**
 * Get the last captured channel (set by enableChannelCapture when invoke is called with onEvent).
 * Returns null if no channel has been captured.
 */
export function getLastChannel(): Channel | null {
  return lastChannel;
}

/**
 * Enable channel capture: when invoke() is called with an onEvent argument,
 * that Channel will be stored in lastChannel for test use.
 *
 * IMPORTANT: Call resetChannelCapture() in afterEach to avoid state leaking between tests.
 */
export function enableChannelCapture() {
  invoke.mockImplementation(
    async (_cmd: string, args?: Record<string, unknown>) => {
      if (args && 'onEvent' in args) {
        lastChannel = args.onEvent as Channel;
      }
    },
  );
}

/**
 * Reset channel capture: clears lastChannel.
 * Call this in afterEach or between test scenarios to avoid state leaking.
 */
export function resetChannelCapture() {
  lastChannel = null;
}

/**
 * Enable channel capture AND provide per-command return values.
 *
 * Combines `enableChannelCapture` with command-specific mock responses in a
 * single `mockImplementation` call so neither overrides the other.
 *
 * @param responses - map of Tauri command name → resolved value
 */
export function enableChannelCaptureWithResponses(
  responses: Record<string, unknown>,
) {
  invoke.mockImplementation(
    async (cmd: string, args?: Record<string, unknown>) => {
      if (args && 'onEvent' in args) {
        lastChannel = args.onEvent as Channel;
      }
      if (Object.prototype.hasOwnProperty.call(responses, cmd)) {
        return responses[cmd];
      }
    },
  );
}

// ─── listen mock ────────────────────────────────────────────────────────────

type EventCallback<T = unknown> = (event: { payload: T }) => void;

const eventHandlers = new Map<string, Set<EventCallback>>();

export const listen = vi.fn(
  async <T>(event: string, handler: EventCallback<T>): Promise<() => void> => {
    if (!eventHandlers.has(event)) {
      eventHandlers.set(event, new Set());
    }
    const handlers = eventHandlers.get(event)!;
    handlers.add(handler as EventCallback);
    return () => {
      handlers.delete(handler as EventCallback);
    };
  },
);

export function emitTauriEvent<T>(event: string, payload: T) {
  const handlers = eventHandlers.get(event);
  if (handlers) {
    for (const handler of handlers) {
      handler({ payload });
    }
  }
}

export function clearEventHandlers() {
  eventHandlers.clear();
}
