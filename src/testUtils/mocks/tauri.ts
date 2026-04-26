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
 * Default model-picker state used by tests that do not opt into a specific
 * inventory. Tests that need the no-model state should mock
 * `get_model_picker_state` to `{ active: null, all: [], ollamaReachable: true }`
 * (S2) or `{ active: null, all: [], ollamaReachable: false }` (S1) explicitly.
 */
export const TEST_DEFAULT_MODEL_PICKER_STATE = {
  active: 'gemma4:e2b',
  all: ['gemma4:e2b'],
  ollamaReachable: true,
} as const;

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
 * Default seed for `get_model_picker_state`. Tests that do not opt into a
 * specific model inventory still need a non-null active model so the
 * capability-mismatch strip does not block submits with the "no model
 * selected" copy. Real Ollama responses look like this on a fresh install
 * with one model pulled.
 */
const DEFAULT_MODEL_PICKER_STATE = {
  active: 'gemma4:e2b',
  all: ['gemma4:e2b'],
  ollamaReachable: true,
} as const;

/**
 * Enable channel capture: when invoke() is called with an onEvent argument,
 * that Channel will be stored in lastChannel for test use.
 *
 * Tests that do not specify their own `get_model_picker_state` response get a
 * default single-model inventory so the no-model gate does not block their
 * submits. Tests that need to assert no-model behaviour should use
 * `enableChannelCaptureWithResponses({ get_model_picker_state: { active: null,
 * all: [] } })` instead.
 *
 * IMPORTANT: Call resetChannelCapture() in afterEach to avoid state leaking between tests.
 */
export function enableChannelCapture() {
  invoke.mockImplementation(
    async (cmd: string, args?: Record<string, unknown>) => {
      if (args && 'onEvent' in args) {
        lastChannel = args.onEvent as Channel;
      }
      if (cmd === 'get_model_picker_state') {
        return DEFAULT_MODEL_PICKER_STATE;
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
      // Same default-seeding rationale as `enableChannelCapture`: tests
      // that do not explicitly mock `get_model_picker_state` still need a
      // non-null active model so the capability strip does not block submits.
      if (cmd === 'get_model_picker_state') {
        return DEFAULT_MODEL_PICKER_STATE;
      }
    },
  );
}

// ─── convertFileSrc mock ────────────────────────────────────────────────────

/** Returns a passthrough URL for test rendering (no Tauri asset protocol). */
export function convertFileSrc(path: string): string {
  return `asset://localhost/${encodeURIComponent(path)}`;
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
