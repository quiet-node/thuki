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

export let lastChannel: Channel | null = null;

export function enableChannelCapture() {
  invoke.mockImplementation(
    async (_cmd: string, args?: Record<string, unknown>) => {
      if (args && 'onEvent' in args) {
        lastChannel = args.onEvent as Channel;
      }
    },
  );
}

export function resetChannelCapture() {
  lastChannel = null;
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
