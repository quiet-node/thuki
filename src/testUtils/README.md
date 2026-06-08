# Test Utilities

This directory contains setup, configuration, and mocks for the vitest test suite.

## Files

### `setup.ts`

Global test setup and browser API mocks:

- **MSW (Mock Service Worker)**: Intercepts HTTP requests for realistic Ollama streaming
- **ResizeObserver mock**: Tests can observe element size changes without a real layout engine
- **Clipboard API mock**: Tests can read/write clipboard without user permission dialogs
- **matchMedia mock**: Tests can detect responsive design breakpoints
- **requestAnimationFrame mock**: Synchronous rAF for deterministic animation testing

All mocks are set up in `beforeAll` and cleaned up in `afterEach`.

### `mocks/` Directory

Test doubles for Tauri and UI libraries:

#### `tauri.ts`

Mocks Tauri IPC layer:

- **`Channel<T>`**: Simulates Tauri's streaming Channel API. Tests call `simulateMessage()` to emit backend messages.
- **`invoke()`**: Mocked command invocation. Set up with `enableChannelCapture()` to automatically capture streaming Channels.
- **`listen()` / `emitTauriEvent()`**: Event system for window visibility, drag, etc.

**Channel Capture Pattern:**

```typescript
import { enableChannelCapture, getLastChannel, resetChannelCapture } from '../../testUtils/mocks/tauri';

beforeEach(() => {
  enableChannelCapture();
  resetChannelCapture(); // Clear state from previous test
});

it('streams data', async () => {
  render(<MyComponent />);
  await act(async () => {
    // Trigger code that calls invoke() with onEvent Channel
    fireEvent.click(button);
  });

  // Get the Channel that invoke() captured
  const channel = getLastChannel();
  act(() => {
    channel?.simulateMessage({ type: 'Token', data: 'hello' });
    channel?.simulateMessage({ type: 'Done' });
  });

  expect(screen.getByText('hello')).toBeInTheDocument();
});
```

**Important**: Always call `resetChannelCapture()` in `beforeEach` or `afterEach` to avoid state leaking between tests.

#### `tauri-window.ts`

Mocks Tauri's window management API:

- **`setSize()`, `getPhysicalSize()`**: Window dimension control
- **`hide()`, `show()`**: Visibility control
- **`startDragging()`, `stopDragging()`**: Drag handling

Mocked methods can be spied on with `vi.spyOn()` to verify calls.

#### `framer-motion.tsx`

Lightweight stub for Framer Motion:

- Replaces `motion.*` components with plain HTML elements
- Strips motion-specific props (`animate`, `initial`, `exit`, `whileHover`, etc.) to avoid React warnings
- Allows tests to assert on real DOM structure without animation complexity

**Why needed**: Vitest's synchronous `requestAnimationFrame` shim causes infinite recursion with Framer Motion's animation batching in real mode. The stub avoids this by skipping animations entirely (acceptable for structural tests).

#### `handlers.ts`

MSW HTTP request handlers for Ollama streaming:

- Mocks `/api/generate` POST endpoint
- Streams NDJSON format (one JSON object per line) matching real Ollama behavior
- Allows tests to exercise streaming token accumulation

## Adding New Mocks

To mock a new module (e.g., a third-party library):

1. Create a file in `mocks/` matching the library name or module path
2. Export mocked implementations of the module's public API
3. Add an alias to `vitest.config.ts`:
   ```typescript
   alias: {
     'my-library': resolve(__dirname, 'src/testUtils/mocks/my-library.ts'),
   }
   ```
4. Import the library normally in components; vitest will use the mock in tests

## Best Practices

- **Minimize mock complexity**: Mocks should be as simple as possible while still supporting the test scenario
- **Document unexpected behavior**: If a mock differs from the real API (e.g., synchronous rAF), document it with a comment
- **Clean up state**: Always reset mock state in `afterEach` to prevent test interference
- **Use `vi.spyOn()` carefully**: Spying on mocks can hide issues if the real API changes

## Common Patterns

### Verify Command Was Called

```typescript
import { invoke } from '../../testUtils/mocks/tauri';

it('calls ask_ollama on submit', async () => {
  render(<App />);
  fireEvent.change(textarea, { target: { value: 'hello' } });
  fireEvent.keyDown(textarea, { key: 'Enter' });

  expect(invoke).toHaveBeenCalledWith('ask_ollama', expect.objectContaining({
    prompt: 'hello'
  }));
});
```

### Simulate Event

```typescript
import { emitTauriEvent } from '../../testUtils/mocks/tauri';

it('shows overlay on visibility event', async () => {
  render(<App />);

  await act(async () => {
    emitTauriEvent('thuki://visibility', { state: 'show' });
  });

  expect(screen.getByPlaceholderText('Ask Thuki anything...')).toBeInTheDocument();
});
```

### Spy on Mock Implementation

```typescript
import { __mockWindow } from '../../testUtils/mocks/tauri-window';

it('hides window on close', async () => {
  // __mockWindow methods are already vi.fn() mocks
  __mockWindow.hide.mockClear();

  render(<App />);
  fireEvent.click(screen.getByRole('button', { name: 'Close' }));

  expect(__mockWindow.hide).toHaveBeenCalled();
});
```
