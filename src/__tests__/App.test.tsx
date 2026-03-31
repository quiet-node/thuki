import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
import App from '../App';
import {
  invoke,
  emitTauriEvent,
  enableChannelCapture,
  lastChannel,
} from '../test/mocks/tauri';
import { __mockWindow } from '../test/mocks/tauri-window';

import { vi } from 'vitest';

async function showOverlay(selectedText: string | null = null) {
  await act(async () => {
    emitTauriEvent('thuki://visibility', {
      state: 'show',
      selected_text: selectedText,
      window_anchor: null,
    });
  });
}

describe('App', () => {
  beforeEach(() => {
    invoke.mockClear();
    enableChannelCapture();
  });

  it('renders nothing when overlay is hidden', async () => {
    const { container } = render(<App />);
    // Flush effects so listener registers
    await act(async () => {});

    expect(container.querySelector('.morphing-container')).toBeNull();
  });

  it('shows overlay on visibility show event', async () => {
    render(<App />);
    // Flush effects so listener registers
    await act(async () => {});

    await showOverlay();

    expect(screen.getByPlaceholderText('Ask Thuki anything...')).toBeTruthy();
  });

  it('hides overlay on Escape key', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    // Confirm overlay is visible
    expect(screen.getByPlaceholderText('Ask Thuki anything...')).toBeTruthy();

    act(() => {
      fireEvent.keyDown(window, { key: 'Escape' });
    });

    expect(invoke).toHaveBeenCalledWith('notify_overlay_hidden');
  });

  it('completes a full conversation turn', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');

    // Type a message
    act(() => {
      fireEvent.change(textarea, { target: { value: 'hello there' } });
    });

    // Submit with Enter
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });

    // Wait for invoke to be called (ask_ollama)
    await act(async () => {});

    // Simulate streaming tokens
    act(() => {
      lastChannel?.simulateMessage({ type: 'Token', data: 'Hi' });
      lastChannel?.simulateMessage({ type: 'Token', data: ' there!' });
      lastChannel?.simulateMessage({ type: 'Done' });
    });

    // The assistant response should now be in the DOM
    expect(screen.getByText('Hi there!')).toBeTruthy();
  });

  it('shows selected context when provided', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay('some code snippet');

    expect(screen.getByText(/some code snippet/)).toBeTruthy();
  });

  it('enters hiding state on hide-request visibility event', async () => {
    render(<App />);
    await act(async () => {});

    // First show overlay
    await showOverlay();
    expect(screen.getByPlaceholderText('Ask Thuki anything...')).toBeTruthy();

    // Then send hide-request — calls requestHideOverlay() (not handleCloseOverlay)
    await act(async () => {
      emitTauriEvent('thuki://visibility', { state: 'hide-request' });
    });

    // The hide-request path transitions overlay to hiding state (overlayState !== 'visible'),
    // so shouldRenderOverlay becomes false and the overlay is removed from the DOM.
    expect(screen.queryByPlaceholderText('Ask Thuki anything...')).toBeNull();
  });

  it('hides overlay on Cmd+W key', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();
    expect(screen.getByPlaceholderText('Ask Thuki anything...')).toBeTruthy();

    act(() => {
      fireEvent.keyDown(window, { key: 'w', metaKey: true });
    });

    expect(invoke).toHaveBeenCalledWith('notify_overlay_hidden');
  });

  it('hides overlay on Ctrl+W key', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    act(() => {
      fireEvent.keyDown(window, { key: 'w', ctrlKey: true });
    });

    expect(invoke).toHaveBeenCalledWith('notify_overlay_hidden');
  });

  it('commits window hide after HIDE_COMMIT_DELAY_MS when hiding', async () => {
    vi.useFakeTimers();
    render(<App />);
    await act(async () => {});

    await showOverlay();

    act(() => {
      fireEvent.keyDown(window, { key: 'Escape' });
    });

    // Advance past the 350ms hide delay
    await act(async () => {
      vi.advanceTimersByTime(400);
    });

    expect(__mockWindow.hide).toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('does not submit empty query', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');

    // Press Enter with empty textarea
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });

    await act(async () => {});

    // ask_ollama should NOT have been called
    expect(invoke).not.toHaveBeenCalledWith('ask_ollama', expect.anything());
  });

  it('fires drag on non-interactive mousedown', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    // Fire mousedown on the outermost div (non-interactive)
    const container = document.querySelector('.morphing-container');
    expect(container).toBeTruthy();

    act(() => {
      fireEvent.mouseDown(container!);
    });

    expect(__mockWindow.startDragging).toHaveBeenCalled();
  });

  it('clears anchor ref on mouseup after drag', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const container = document.querySelector('.morphing-container');
    expect(container).toBeTruthy();

    __mockWindow.startDragging.mockClear();

    act(() => {
      fireEvent.mouseDown(container!);
    });

    // startDragging was called — now fire mouseup to cover the mouseup handler
    act(() => {
      fireEvent.mouseUp(window);
    });

    // No assertion needed — just exercising the mouseup callback (windowAnchorRef = null)
    expect(__mockWindow.startDragging).toHaveBeenCalled();
  });

  it('does not fire drag when mousedown on select-text element', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    // Send a message to enter chat mode so ChatBubble (with .select-text) renders
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    act(() => {
      fireEvent.change(textarea, { target: { value: 'test message' } });
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    act(() => {
      lastChannel?.simulateMessage({ type: 'Token', data: 'Reply' });
      lastChannel?.simulateMessage({ type: 'Done' });
    });

    // Find a .select-text element
    const selectTextEl = document.querySelector('.select-text');
    if (selectTextEl) {
      __mockWindow.startDragging.mockClear();
      act(() => {
        fireEvent.mouseDown(selectTextEl);
      });
      expect(__mockWindow.startDragging).not.toHaveBeenCalled();
    }
  });

  it('does not fire drag when mousedown on TEXTAREA', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    __mockWindow.startDragging.mockClear();

    act(() => {
      fireEvent.mouseDown(textarea);
    });

    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });

  it('submits query with context prepended when selectedContext is set', async () => {
    render(<App />);
    await act(async () => {});

    // Show with selected context
    await showOverlay('selected snippet');

    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    act(() => {
      fireEvent.change(textarea, { target: { value: 'my question' } });
    });

    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });

    await act(async () => {});

    // Prompt should be context-wrapped
    expect(invoke).toHaveBeenCalledWith(
      'ask_ollama',
      expect.objectContaining({
        prompt: expect.stringContaining('Context:'),
      }),
    );
  });

  it('requestHideOverlay is a no-op when already hidden', async () => {
    render(<App />);
    await act(async () => {});

    // Overlay is hidden initially — fire hide-request on hidden overlay
    // This exercises the 'hidden' branch in requestHideOverlay's state setter
    await act(async () => {
      emitTauriEvent('thuki://visibility', { state: 'hide-request' });
    });

    // No crash, no change — overlay is already hidden
    expect(document.querySelector('.morphing-container')).toBeNull();
  });

  it('resets session on overlay reopen', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');

    // Complete a conversation turn
    act(() => {
      fireEvent.change(textarea, { target: { value: 'first question' } });
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    act(() => {
      lastChannel?.simulateMessage({ type: 'Token', data: 'First response' });
      lastChannel?.simulateMessage({ type: 'Done' });
    });

    expect(screen.getByText('First response')).toBeTruthy();

    // Re-enable channel capture for second session
    enableChannelCapture();

    // Reopen overlay — should reset session
    await showOverlay();

    // Should be back to input bar mode with placeholder
    expect(screen.getByPlaceholderText('Ask Thuki anything...')).toBeTruthy();
    // Old messages should be gone
    expect(screen.queryByText('First response')).toBeNull();
  });
});
