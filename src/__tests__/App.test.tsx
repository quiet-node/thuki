import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import App from '../App';
import {
  invoke,
  emitTauriEvent,
  enableChannelCapture,
  getLastChannel,
} from '../testUtils/mocks/tauri';
import { __mockWindow } from '../testUtils/mocks/tauri-window';

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

    expect(
      screen.getByPlaceholderText('Ask Thuki anything...'),
    ).toBeInTheDocument();
  });

  it('hides overlay on Escape key', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    // Confirm overlay is visible
    expect(
      screen.getByPlaceholderText('Ask Thuki anything...'),
    ).toBeInTheDocument();

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
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hi' });
      getLastChannel()?.simulateMessage({ type: 'Token', data: ' there!' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });

    // The assistant response should now be in the DOM
    expect(screen.getByText('Hi there!')).toBeInTheDocument();
  });

  it('shows selected context when provided', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay('some code snippet');

    expect(screen.getByText(/some code snippet/)).toBeInTheDocument();
  });

  it('enters hiding state on hide-request visibility event', async () => {
    render(<App />);
    await act(async () => {});

    // First show overlay
    await showOverlay();
    expect(
      screen.getByPlaceholderText('Ask Thuki anything...'),
    ).toBeInTheDocument();

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
    expect(
      screen.getByPlaceholderText('Ask Thuki anything...'),
    ).toBeInTheDocument();

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
    expect(container).not.toBeNull();

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
    expect(container).not.toBeNull();

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
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Reply' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
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

  it('applies justify-end layout when overlay opens with anchor', async () => {
    render(<App />);
    await act(async () => {});

    // Show overlay with a window anchor (upward-growth mode)
    await act(async () => {
      emitTauriEvent('thuki://visibility', {
        state: 'show',
        selected_text: null,
        window_anchor: { x: 100, bottom_y: 800, min_y: 50 },
      });
    });

    // The outer container should use justify-end for bottom-pinning
    const outer = document.querySelector('.justify-end');
    expect(outer).not.toBeNull();
  });

  describe('ResizeObserver window sizing with anchor', () => {
    let capturedCallback: ResizeObserverCallback | null = null;

    function spyOnResizeObserver() {
      const OriginalMock = globalThis.ResizeObserver;
      vi.spyOn(globalThis, 'ResizeObserver').mockImplementation(
        (callback: ResizeObserverCallback) => {
          capturedCallback = callback;
          return new OriginalMock(callback) as ResizeObserver;
        },
      );
    }

    function triggerResize(element: Element, contentHeight: number) {
      vi.spyOn(element, 'getBoundingClientRect').mockReturnValue({
        height: contentHeight,
        width: 600,
        top: 0,
        left: 0,
        right: 600,
        bottom: contentHeight,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      });
      if (capturedCallback) {
        capturedCallback(
          [{ target: element } as ResizeObserverEntry],
          {} as ResizeObserver,
        );
      }
    }

    it('calls set_window_frame with content height on first anchor event, not max height', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      // Show with anchor — bottom_y=884 means the window is at the bottom of a 900px screen
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_anchor: { x: 100, bottom_y: 884, min_y: 40 },
        });
      });

      invoke.mockClear();

      const container = document.querySelector('.morphing-container');
      expect(container).not.toBeNull();
      expect(capturedCallback).not.toBeNull();

      // Simulate first observer event: only the askbar is visible (~60px content)
      act(() => {
        triggerResize(container!, 60);
      });

      // REGRESSION: must use content height (60+48=108), NOT max height (648)
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 100,
        y: 884 - 108, // 776 — window bottom stays pinned, top moves to fit content
        width: 600,
        height: 108,
      });
      expect(invoke).not.toHaveBeenCalledWith(
        'set_window_frame',
        expect.objectContaining({ height: 648 }),
      );
    });

    it('grows incrementally: each resize event updates position and height', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_anchor: { x: 50, bottom_y: 800, min_y: 40 },
        });
      });

      const container = document.querySelector('.morphing-container');
      expect(container).not.toBeNull();

      // First event: askbar only
      invoke.mockClear();
      act(() => {
        triggerResize(container!, 60);
      });
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 50,
        y: 800 - 108,
        width: 600,
        height: 108,
      });

      // Second event: chat started, content grew
      invoke.mockClear();
      act(() => {
        triggerResize(container!, 200);
      });
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 50,
        y: 800 - 248,
        width: 600,
        height: 248,
      });
    });

    it('locks at max height and skips further resize events', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_anchor: { x: 100, bottom_y: 884, min_y: 40 },
        });
      });

      const container = document.querySelector('.morphing-container');
      expect(container).not.toBeNull();

      // Grow to max height (content=600 → window=648)
      invoke.mockClear();
      act(() => {
        triggerResize(container!, 600);
      });
      expect(invoke).toHaveBeenCalledWith(
        'set_window_frame',
        expect.objectContaining({ height: 648 }),
      );

      // Next event should be a no-op (isPreExpandedRef is now true)
      invoke.mockClear();
      act(() => {
        triggerResize(container!, 620);
      });
      expect(invoke).not.toHaveBeenCalledWith('set_window_frame', expect.anything());
    });

    it('uses setSize (not set_window_frame) after drag clears the anchor', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_anchor: { x: 100, bottom_y: 884, min_y: 40 },
        });
      });

      const container = document.querySelector('.morphing-container');
      expect(container).not.toBeNull();

      // Simulate drag: mousedown then mouseup clears the anchor
      act(() => {
        fireEvent.mouseDown(container!);
      });
      act(() => {
        fireEvent.mouseUp(window);
      });

      invoke.mockClear();
      __mockWindow.setSize.mockClear?.();

      // After drag, anchor is null — ResizeObserver should use setSize, not set_window_frame
      act(() => {
        triggerResize(container!, 60);
      });
      expect(invoke).not.toHaveBeenCalledWith('set_window_frame', expect.anything());
      expect(__mockWindow.setSize).toHaveBeenCalled();
    });
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
      getLastChannel()?.simulateMessage({
        type: 'Token',
        data: 'First response',
      });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });

    expect(screen.getByText('First response')).toBeInTheDocument();

    // Re-enable channel capture for second session
    enableChannelCapture();

    // Reopen overlay — should reset session
    await showOverlay();

    // Should be back to input bar mode with placeholder
    expect(
      screen.getByPlaceholderText('Ask Thuki anything...'),
    ).toBeInTheDocument();
    // Old messages should be gone
    expect(screen.queryByText('First response')).toBeNull();
  });
});
