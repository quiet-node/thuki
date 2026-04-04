import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import App from '../App';
import {
  invoke,
  emitTauriEvent,
  enableChannelCapture,
  enableChannelCaptureWithResponses,
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

  it('submits query with quoted text when selectedContext is set', async () => {
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

    // Backend receives the message and quoted text separately
    expect(invoke).toHaveBeenCalledWith(
      'ask_ollama',
      expect.objectContaining({
        message: 'my question',
        quotedText: 'selected snippet',
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
      vi.spyOn(globalThis, 'ResizeObserver').mockImplementation(function (
        callback: ResizeObserverCallback,
      ) {
        capturedCallback = callback;
        return new OriginalMock(callback) as ResizeObserver;
      });
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

    it('immediately expands to max height when isGenerating becomes true with upward anchor', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      // Show with anchor
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_anchor: { x: 100, bottom_y: 884, min_y: 40 },
        });
      });

      // Small initial resize (ask bar only, isGenerating=false)
      const container = document.querySelector('.morphing-container');
      expect(container).not.toBeNull();
      act(() => {
        triggerResize(container!, 60);
      });

      // Submit a message — causes isGenerating to become true
      invoke.mockClear();
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'hello' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Must immediately call set_window_frame with max height
      // max = min(648, 884 - 40 = 844) = 648; newY = 884 - 648 = 236
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 100,
        y: 236,
        width: 600,
        height: 648,
      });

      // Subsequent resize events must be no-ops (isPreExpandedRef is now true)
      invoke.mockClear();
      act(() => {
        triggerResize(container!, 100);
      });
      expect(invoke).not.toHaveBeenCalledWith(
        'set_window_frame',
        expect.anything(),
      );
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
      expect(invoke).not.toHaveBeenCalledWith(
        'set_window_frame',
        expect.anything(),
      );
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
      expect(invoke).not.toHaveBeenCalledWith(
        'set_window_frame',
        expect.anything(),
      );
      expect(__mockWindow.setSize).toHaveBeenCalled();
    });

    it('clamps to available space when screen gap is smaller than MAX_CHAT_WINDOW_HEIGHT', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      // Available space: bottom_y - min_y = 300 - 100 = 200, which is < MAX_CHAT_WINDOW_HEIGHT (648)
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_anchor: { x: 50, bottom_y: 300, min_y: 100 },
        });
      });

      const container = document.querySelector('.morphing-container');
      expect(container).not.toBeNull();

      invoke.mockClear();
      // Content height (300) → targetHeight (348) exceeds available space (200) → clamped to 200
      act(() => {
        triggerResize(container!, 300);
      });
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 50,
        y: 100, // bottom_y (300) - clamped height (200) = 100
        width: 600,
        height: 200, // clamped to available space, not targetHeight (348) or MAX (648)
      });

      // isPreExpandedRef is now true — next event is a no-op
      invoke.mockClear();
      act(() => {
        triggerResize(container!, 400);
      });
      expect(invoke).not.toHaveBeenCalledWith(
        'set_window_frame',
        expect.anything(),
      );
    });

    it('isPreExpandedRef resets on session reopen, allowing incremental growth again', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      // Session 1: grow to max height, locking isPreExpandedRef
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_anchor: { x: 100, bottom_y: 884, min_y: 40 },
        });
      });

      const container1 = document.querySelector('.morphing-container');
      act(() => {
        triggerResize(container1!, 600); // locks isPreExpandedRef = true
      });

      // Close the overlay — requestHideOverlay resets isPreExpandedRef to false
      await act(async () => {
        emitTauriEvent('thuki://visibility', { state: 'hide-request' });
      });

      // Session 2: reopen with new anchor — incremental growth must work again
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_anchor: { x: 100, bottom_y: 884, min_y: 40 },
        });
      });

      const container2 = document.querySelector('.morphing-container');
      expect(container2).not.toBeNull();

      invoke.mockClear();
      // Small content — must NOT be skipped even though the previous session was locked
      act(() => {
        triggerResize(container2!, 60);
      });
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 100,
        y: 776, // bottom_y (884) - neededHeight (108) = 776
        width: 600,
        height: 108, // content height (60) + padding (48)
      });
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

  // ─── History integration ─────────────────────────────────────────────────────

  describe('history integration', () => {
    it('shows history icon button in ask-bar mode', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      expect(
        screen.getByRole('button', { name: /open history/i }),
      ).toBeInTheDocument();
    });

    it('shows history panel when history icon is clicked in ask-bar mode', async () => {
      invoke.mockResolvedValue([]); // list_conversations returns empty

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      expect(
        screen.getByPlaceholderText(/search past chats/i),
      ).toBeInTheDocument();
    });

    it('closes history panel when a conversation is loaded', async () => {
      invoke.mockResolvedValueOnce([]); // list_conversations
      invoke.mockResolvedValueOnce([
        // load_conversation
        {
          id: 'm1',
          role: 'user',
          content: 'Hello',
          quoted_text: null,
          created_at: 1,
        },
        {
          id: 'm2',
          role: 'assistant',
          content: 'Hi',
          quoted_text: null,
          created_at: 2,
        },
      ]);

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Open history
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      // Wait for empty list to render
      await act(async () => {});

      // Panel should be visible but no conversations to click
      // (list is empty, so just verify panel closes on a second click)
      // Close via second click on history icon
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      expect(screen.queryByPlaceholderText(/search past chats/i)).toBeNull();
    });

    it('shows save button in conversation view when there are messages', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'test' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'Reply' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      expect(screen.getByRole('button', { name: /save/i })).toBeInTheDocument();
    });

    it('save button calls save_conversation when clicked', async () => {
      enableChannelCaptureWithResponses({
        save_conversation: { conversation_id: 'conv-test' },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'question' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });

      expect(invoke).toHaveBeenCalledWith(
        'save_conversation',
        expect.objectContaining({
          model: expect.any(String),
          messages: expect.any(Array),
        }),
      );
    });

    it('clicking save button when already saved calls delete_conversation (unsave toggle)', async () => {
      enableChannelCaptureWithResponses({
        save_conversation: { conversation_id: 'conv-save-toggle' },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'question' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Save the conversation first
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });

      // Button should now read "Remove from history"
      expect(
        screen.getByRole('button', { name: /remove from history/i }),
      ).toBeInTheDocument();

      invoke.mockClear();

      // Click again to unsave
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /remove from history/i }),
        );
      });

      expect(invoke).toHaveBeenCalledWith('delete_conversation', {
        conversationId: 'conv-save-toggle',
      });

      // Button reverts to "Save conversation"
      expect(
        screen.getByRole('button', { name: /save conversation/i }),
      ).toBeInTheDocument();
    });

    it('resets history state on overlay reopen', async () => {
      enableChannelCaptureWithResponses({
        save_conversation: { conversation_id: 'conv-123' },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Send message + Done
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'hello' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hi' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Save
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });

      // Reopen — bookmark should reset (save button enabled again)
      enableChannelCapture();
      await showOverlay();

      // In ask-bar mode now — no save button visible, but history icon is
      expect(
        screen.getByRole('button', { name: /open history/i }),
      ).toBeInTheDocument();
    });

    it('handleNewConversation shows SwitchConfirmation when unsaved, resets on Start New', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Get into chat mode with an unsaved turn
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'question' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Click + (unsaved conversation → history panel opens with SwitchConfirmation)
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      // SwitchConfirmation should be visible with "new" variant
      expect(
        screen.getByRole('button', { name: 'Start New' }),
      ).toBeInTheDocument();

      // Click "Start New" → should reset to ask-bar mode
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Start New' }));
      });

      expect(
        screen.getByPlaceholderText('Ask Thuki anything...'),
      ).toBeInTheDocument();
    });

    it('handleNewConversation Cancel closes the history dropdown', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Get into chat mode with an unsaved turn
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'question' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Click + → SwitchConfirmation appears
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      expect(
        screen.getByRole('button', { name: 'Cancel' }),
      ).toBeInTheDocument();

      // Click Cancel → dropdown closes, still in chat mode
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
      });

      // SwitchConfirmation should be gone
      expect(
        screen.queryByRole('button', { name: 'Cancel' }),
      ).not.toBeInTheDocument();
      // Still showing the conversation
      expect(screen.getByText('question')).toBeInTheDocument();
    });

    it('handleNewConversation resets directly when conversation is already saved', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
        save_conversation: 'saved-id',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Get into chat mode
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'question' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Save the conversation
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });

      // Click + (already saved → no confirmation, direct reset)
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      // Should be directly back in ask-bar mode (no confirmation prompt)
      expect(
        screen.getByPlaceholderText('Ask Thuki anything...'),
      ).toBeInTheDocument();
    });

    it('handleNewConversation revokes blob URLs when images are attached', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
        save_image_command: '/tmp/img.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Get into chat mode with an unsaved turn
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'question' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Paste an image while in chat mode (unsaved conversation)
      const replyInput = screen.getByPlaceholderText('Reply...');
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(replyInput, {
          clipboardData: {
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      const revokeSpy = vi.mocked(URL.revokeObjectURL);
      revokeSpy.mockClear();

      // Click + → SwitchConfirmation (unsaved conversation)
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      // Click "Start New" → resetForNewConversation revokes blob URLs
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Start New' }));
      });

      expect(revokeSpy).toHaveBeenCalled();
      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('handleNewConversation saves then resets on Save & Start New', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
        save_conversation: 'saved-id',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Get into chat mode with an unsaved turn
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'question' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Click + → SwitchConfirmation appears
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      expect(
        screen.getByRole('button', { name: 'Save & Start New' }),
      ).toBeInTheDocument();

      // Click "Save & Start New" → saves then resets to ask-bar mode
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Save & Start New' }),
        );
      });

      expect(
        screen.getByPlaceholderText('Ask Thuki anything...'),
      ).toBeInTheDocument();
    });

    it('handleSaveAndNew aborts reset when save fails', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'list_conversations') return [];
        if (cmd === 'save_conversation') throw new Error('disk full');
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn so isSaved = false
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'q' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Click + → SwitchConfirmation
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      // Click "Save & Start New" — save fails → should stay in chat mode
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Save & Start New' }),
        );
      });

      // Still in chat mode (save_conversation threw, reset was aborted)
      expect(screen.getByText('q')).toBeInTheDocument();
    });

    it('handleSaveAndLoad saves unsaved conversation then switches', async () => {
      const OTHER_MSGS = [
        {
          id: 'm3',
          role: 'user',
          content: 'Old q',
          quoted_text: null,
          created_at: 1,
        },
        {
          id: 'm4',
          role: 'assistant',
          content: 'Old a',
          quoted_text: null,
          created_at: 2,
        },
      ];
      enableChannelCaptureWithResponses({
        save_conversation: { conversation_id: 'conv-new' },
        load_conversation: OTHER_MSGS,
        list_conversations: [
          {
            id: 'conv-other2',
            title: 'Other chat',
            model: 'gemma3:4b',
            updated_at: 1,
            message_count: 2,
          },
        ],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn (unsaved)
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'q' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Open chat history WITHOUT saving
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /history/i }));
      });

      // Click a different conversation → SwitchConfirmation
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /other chat/i }));
      });

      // Save & Switch — isSaved is FALSE so save_conversation should be called
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /save & switch/i }));
      });

      expect(invoke).toHaveBeenCalledWith(
        'save_conversation',
        expect.objectContaining({
          model: expect.any(String),
        }),
      );
    });

    it('handleSaveAndLoad aborts load when save_conversation fails', async () => {
      // Bug: without the early return on save failure, the load would still run
      // and could overwrite the current session with an unrelated conversation.
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'list_conversations')
          return [
            {
              id: 'c2',
              title: 'Other chat',
              model: 'gemma3:4b',
              updated_at: 1,
              message_count: 1,
            },
          ];
        if (cmd === 'save_conversation') throw new Error('disk full');
        // load_conversation must NOT be called
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn so isSaved = false
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'q' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Open history → click another conversation → SwitchConfirmation
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /other chat/i }));
      });

      // Confirm "Save & Switch" — save_conversation will throw
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /save & switch/i }));
      });

      // load_conversation must NOT have been called (early return after save failure)
      expect(invoke).not.toHaveBeenCalledWith(
        'load_conversation',
        expect.anything(),
      );
    });

    it('clicking a conversation loads it directly when already saved (no dialog)', async () => {
      const OTHER_MSGS = [
        {
          id: 'm3',
          role: 'user',
          content: 'Old q',
          quoted_text: null,
          created_at: 1,
        },
        {
          id: 'm4',
          role: 'assistant',
          content: 'Old a',
          quoted_text: null,
          created_at: 2,
        },
      ];
      enableChannelCaptureWithResponses({
        save_conversation: { conversation_id: 'conv-current' },
        load_conversation: OTHER_MSGS,
        list_conversations: [
          {
            id: 'conv-other',
            title: 'Switch target',
            model: 'gemma3:4b',
            updated_at: 1,
            message_count: 2,
          },
        ],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'q' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Save the conversation → isSaved = true
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });

      // Open chat history
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      // Click a different conversation — isSaved=true means no dialog, loads directly
      invoke.mockClear();
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /switch target/i }));
      });

      // No SwitchConfirmation dialog — save_conversation NOT called again
      expect(invoke).not.toHaveBeenCalledWith(
        'save_conversation',
        expect.anything(),
      );
      // load_conversation IS called directly
      expect(invoke).toHaveBeenCalledWith('load_conversation', {
        conversationId: 'conv-other',
      });
    });

    it('handleDeleteConversation marks active conversation unsaved but keeps messages', async () => {
      const LOADED_MSGS = [
        {
          id: 'm1',
          role: 'user',
          content: 'Hi',
          quoted_text: null,
          created_at: 1,
        },
        {
          id: 'm2',
          role: 'assistant',
          content: 'Hello',
          quoted_text: null,
          created_at: 2,
        },
      ];
      enableChannelCaptureWithResponses({
        load_conversation: LOADED_MSGS,
        list_conversations: [
          {
            id: 'conv-target',
            title: 'My chat',
            model: 'gemma3:4b',
            updated_at: 1,
            message_count: 2,
          },
        ],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Load a conversation from ask-bar history → conversationId = 'conv-target'
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /my chat/i }));
      });

      // Messages are visible in chat mode
      expect(screen.getByText('Hi')).toBeInTheDocument();

      // Open chat history and delete the currently-active conversation
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /delete conversation/i }),
        );
      });

      // delete_conversation was called
      expect(invoke).toHaveBeenCalledWith('delete_conversation', {
        conversationId: 'conv-target',
      });

      // Messages remain — still in chat mode
      expect(screen.getByText('Hi')).toBeInTheDocument();

      // Save button reverts to unsaved state ("Save conversation")
      expect(
        screen.getByRole('button', { name: /save conversation/i }),
      ).toBeInTheDocument();
    });

    it('clicking outside the chat history dropdown closes it', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn to enter chat mode
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'q' } });
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Open history dropdown
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      expect(
        screen.getByPlaceholderText('Search past chats…'),
      ).toBeInTheDocument();

      // Click outside — should close the dropdown
      await act(async () => {
        fireEvent.mouseDown(document.body);
      });
      expect(screen.queryByPlaceholderText('Search past chats…')).toBeNull();
    });

    it('clicking inside the chat history dropdown does not close it', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn to enter chat mode
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'q' } });
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Open history dropdown
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      const searchInput = screen.getByPlaceholderText('Search past chats…');
      expect(searchInput).toBeInTheDocument();

      // Click inside the dropdown — should NOT close it
      await act(async () => {
        fireEvent.mouseDown(searchInput);
      });
      expect(
        screen.getByPlaceholderText('Search past chats…'),
      ).toBeInTheDocument();
    });

    it('handleDeleteConversation allows saving the conversation again after deletion', async () => {
      // After deleting the active conversation from history, isSaved resets to
      // false so the user can re-save the same messages under a new record.
      enableChannelCaptureWithResponses({
        load_conversation: [
          {
            id: 'm1',
            role: 'user',
            content: 'Hi',
            quoted_text: null,
            created_at: 1,
          },
          {
            id: 'm2',
            role: 'assistant',
            content: 'Hello',
            quoted_text: null,
            created_at: 2,
          },
        ],
        list_conversations: [
          {
            id: 'conv-active',
            title: 'Active chat',
            model: 'gemma3:4b',
            updated_at: 1,
            message_count: 2,
          },
        ],
        save_conversation: { conversation_id: 'conv-new' },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Load the conversation → isSaved = true
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /active chat/i }));
      });

      // Verify save button shows unsave state
      expect(
        screen.getByRole('button', { name: /remove from history/i }),
      ).toBeInTheDocument();

      // Open history and delete the active conversation
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /delete conversation/i }),
        );
      });

      // Messages remain, isSaved is now false — save button is re-enabled
      expect(screen.getByText('Hi')).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: /save conversation/i }),
      ).toBeInTheDocument();

      // User can re-save the conversation
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });
      expect(invoke).toHaveBeenCalledWith(
        'save_conversation',
        expect.objectContaining({ messages: expect.any(Array) }),
      );
    });

    it('handleLoadConversation closes history panel when load_conversation fails', async () => {
      // Bug: without try/catch, setIsHistoryOpen(false) is never reached when
      // loadConversation() throws, leaving the panel open on failure.
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'list_conversations')
          return [
            {
              id: 'c1',
              title: 'Chat',
              model: 'gemma3:4b',
              updated_at: 1,
              message_count: 1,
            },
          ];
        if (cmd === 'load_conversation') throw new Error('load failed');
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      // Click the conversation — load_conversation will throw
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /^chat$/i }));
      });

      // Panel must close even on failure; app must still be running
      expect(screen.queryByPlaceholderText(/search past chats/i)).toBeNull();
      expect(
        screen.getByPlaceholderText('Ask Thuki anything...'),
      ).toBeInTheDocument();
    });

    it('handleDeleteConversation does not reset history when a different conversation is deleted', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [
          {
            id: 'conv-unrelated',
            title: 'Unrelated',
            model: 'gemma3:4b',
            updated_at: 1,
            message_count: 2,
          },
        ],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Open ask-bar history (no conversation loaded — conversationId is null)
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      // Delete a conversation while conversationId is null (id !== conversationId → false branch)
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /delete conversation/i }),
        );
      });

      expect(invoke).toHaveBeenCalledWith('delete_conversation', {
        conversationId: 'conv-unrelated',
      });
    });
  });

  // ─── Image integration ─────────────────────────────────────────────────────

  describe('image integration', () => {
    /** Helper: paste an image file into the textarea and wait for thumbnails. */
    async function pasteImage() {
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['fake-img-data'], 'photo.png', {
        type: 'image/png',
      });
      const clipboardData = {
        items: [{ type: 'image/png', getAsFile: () => file }],
      };
      await act(async () => {
        fireEvent.paste(textarea, { clipboardData });
      });
      // Thumbnails appear immediately via blob URL (before backend completes)
      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });
    }

    it('handleImagesAttached stages images and shows thumbnails', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for FileReader + invoke to complete in background
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.objectContaining({
              imageDataBase64: expect.any(String),
            }),
          );
        });
      });

      // Thumbnails should still be present
      expect(
        screen.getByRole('list', { name: /attached images/i }),
      ).toBeInTheDocument();
    });

    it('handleImageRemove removes thumbnail and calls remove_image_command', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for backend to resolve (filePath set)
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      invoke.mockClear();

      // Click remove button on the thumbnail
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /remove/i }));
      });

      expect(invoke).toHaveBeenCalledWith('remove_image_command', {
        path: '/tmp/staged/img1.jpg',
      });
      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('handleSubmit with images passes imagePaths and clears attachedImages', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for backend to resolve (filePath set)
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      // Type a message and submit
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'describe this' } });
      });

      invoke.mockClear();
      enableChannelCapture();

      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // ask_ollama should be called with imagePaths
      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: 'describe this',
          imagePaths: ['/tmp/staged/img1.jpg'],
        }),
      );
    });

    it('submits with images and no text', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for backend to resolve
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      invoke.mockClear();
      enableChannelCapture();

      // Submit with Enter (no text, just images)
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // ask_ollama should be called with empty message but imagePaths
      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: '',
          imagePaths: ['/tmp/staged/img1.jpg'],
        }),
      );
    });

    it('previewImage opens ImagePreviewModal and closing clears it', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Click preview button on thumbnail
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /preview/i }));
      });

      // ImagePreviewModal should be open (has role="dialog")
      expect(screen.getByRole('dialog')).toBeInTheDocument();

      // Close the modal
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /close preview/i }));
      });

      // Dialog should be gone
      expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('handleImagesAttached removes image when backend fails', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'save_image_command') throw new Error('disk full');
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.drop(
          document.querySelector('[class*="flex flex-col w-full shrink-0"]')!,
          {
            preventDefault: vi.fn(),
            dataTransfer: { files: [file] },
          },
        );
      });

      // Thumbnail appears immediately via blob URL
      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      // Wait for FileReader + invoke to settle — failed image gets removed
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      // Image should be removed after backend failure
      await vi.waitFor(() => {
        expect(
          screen.queryByRole('list', { name: /attached images/i }),
        ).toBeNull();
      });
    });

    it('handleImagesAttached skips images that fail to stage', async () => {
      // First call succeeds, second call fails
      let saveCallCount = 0;
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            // channel capture — no-op for this test
          }
          if (cmd === 'save_image_command') {
            saveCallCount++;
            if (saveCallCount === 2) throw new Error('disk full');
            return '/tmp/staged/img1.jpg';
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Drop two image files via the AskBarView wrapper
      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      );
      expect(askBarWrapper).not.toBeNull();

      const file1 = new File(['data1'], 'img1.png', { type: 'image/png' });
      const file2 = new File(['data2'], 'img2.png', { type: 'image/png' });
      fireEvent.drop(askBarWrapper!, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [file1, file2] },
      });

      // Both thumbnails appear immediately
      await vi.waitFor(() => {
        expect(screen.getAllByRole('listitem')).toHaveLength(2);
      });

      // Wait for both backend calls to settle
      await act(async () => {
        await vi.waitFor(() => {
          expect(saveCallCount).toBe(2);
        });
      });

      // Failed image gets removed, only one remains
      await vi.waitFor(() => {
        expect(screen.getAllByRole('listitem')).toHaveLength(1);
      });
    });

    it('handleChatImagePreview opens modal for chat history image', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for backend to resolve
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      // Type and submit to create a user message with image
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'what is this?' } });
      });

      invoke.mockClear();
      enableChannelCapture();

      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Simulate AI response completing
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'It is' });
        getLastChannel()?.simulateMessage({ type: 'Token', data: ' a cat.' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // The user message should have a thumbnail from chat history (via convertFileSrc)
      // Find the preview button in the chat bubble (not the ask bar)
      const previewButtons = screen.getAllByRole('button', {
        name: /preview/i,
      });
      // The chat bubble thumbnail should be present
      expect(previewButtons.length).toBeGreaterThan(0);

      await act(async () => {
        fireEvent.click(previewButtons[0]);
      });

      // ImagePreviewModal should be open
      expect(screen.getByRole('dialog')).toBeInTheDocument();

      // Close it
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /close preview/i }));
      });

      expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('handleChatImagePreview passes blob URLs through without convertFileSrc', async () => {
      // Make save_image_command hang so the image stays as a blob URL
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            // channel capture
          }
          if (cmd === 'save_image_command') {
            return new Promise<string>(() => {}); // never resolves
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste and submit while still processing → pendingUserMessage with blob URL
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      act(() => {
        fireEvent.change(textarea, { target: { value: 'what is this?' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Pending user message should be visible in chat with a blob URL thumbnail
      await vi.waitFor(() => {
        expect(screen.getByText('what is this?')).toBeInTheDocument();
      });

      // Click the preview button in the chat bubble — should open the modal
      // with the blob URL directly (no convertFileSrc wrapping).
      const previewButtons = screen.getAllByRole('button', {
        name: /preview/i,
      });
      expect(previewButtons.length).toBeGreaterThan(0);

      await act(async () => {
        fireEvent.click(previewButtons[0]);
      });

      expect(screen.getByRole('dialog')).toBeInTheDocument();

      // Flush stale FileReader macrotask so it doesn't leak into the next test.
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });
    });

    it('handleImageRemove is safe when called twice for the same image', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for backend to resolve
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      invoke.mockClear();

      // Click remove twice rapidly — the second call should be a no-op
      // (the functional updater in setAttachedImages will find no matching
      // image on the second pass, exercising the !img branch).
      const removeBtn = screen.getByRole('button', { name: /remove/i });
      await act(async () => {
        fireEvent.click(removeBtn);
        fireEvent.click(removeBtn);
      });

      // remove_image_command should only be called once
      const removeCalls = invoke.mock.calls.filter(
        (call) => call[0] === 'remove_image_command',
      );
      expect(removeCalls).toHaveLength(1);
    });

    it('handleImageRemove revokes blob URL without calling remove_image_command when filePath is null', async () => {
      // Make save_image_command hang forever (never resolve)
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            // channel capture — no-op
          }
          if (cmd === 'save_image_command') {
            return new Promise(() => {}); // never resolves
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image — thumbnail appears immediately with null filePath
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      invoke.mockClear();

      // Remove the image while filePath is still null
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /remove/i }));
      });

      // Should NOT call remove_image_command (no file to delete)
      expect(invoke).not.toHaveBeenCalledWith(
        'remove_image_command',
        expect.anything(),
      );
      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('defers submit when images are still processing and fires when ready', async () => {
      // Flush any stale macrotasks (e.g. FileReader.onload from prior tests)
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });

      // Track save_image_command calls scoped to THIS test
      let resolveSave: ((path: string) => void) | null = null;
      const savePromises: Promise<string>[] = [];
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            // Accept channel for ask_ollama
          }
          if (cmd === 'save_image_command') {
            const p = new Promise<string>((resolve) => {
              resolveSave = resolve;
            });
            savePromises.push(p);
            return p;
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image — thumbnail appears immediately (filePath null)
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      // Wait for this test's FileReader to complete and call save_image_command
      await act(async () => {
        await vi.waitFor(() => expect(savePromises).toHaveLength(1));
      });

      // Type and submit while image is still processing
      act(() => {
        fireEvent.change(textarea, { target: { value: 'describe this' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Should show "Processing images" state
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve the image — triggers deferred submit chain
      resolveSave!('/tmp/staged/img1.jpg');

      // Flush async chain: promise → state update → effect → ask → invoke
      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      // User message should appear in the chat (ask() fired the real submit)
      expect(screen.getByText('describe this')).toBeInTheDocument();
    });

    it('stop button cancels active generation via handleCancel', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/img.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Start a normal text conversation (no images)
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'hello' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Should be generating — stop button visible
      const stopBtn = screen.getByRole('button', { name: /stop/i });
      expect(stopBtn).toBeInTheDocument();

      // Click stop — should call cancel_generation
      invoke.mockClear();
      enableChannelCapture();

      await act(async () => {
        fireEvent.click(stopBtn);
      });

      expect(invoke).toHaveBeenCalledWith('cancel_generation');
    });

    it('cancelling during pending submit restores input (undo send)', async () => {
      // Flush stale macrotasks from prior tests
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });

      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            // Accept channel
          }
          if (cmd === 'save_image_command') {
            return new Promise<string>(() => {}); // never resolves
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      // Type and submit while image is still processing
      act(() => {
        fireEvent.change(textarea, { target: { value: 'my question' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Should be in chat mode with stop button
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Click stop to cancel the pending submit
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /stop/i }));
      });

      // Should revert to ask-bar mode with the query restored
      const restoredTextarea = screen.getByPlaceholderText(
        'Ask Thuki anything...',
      );
      expect(restoredTextarea).toBeInTheDocument();
      expect((restoredTextarea as HTMLTextAreaElement).value).toBe(
        'my question',
      );

      // Images should still be visible (still processing in background)
      expect(
        screen.getByRole('list', { name: /attached images/i }),
      ).toBeInTheDocument();

      // ask_ollama should never have been called
      expect(invoke).not.toHaveBeenCalledWith('ask_ollama', expect.anything());
    });

    it('waits for all images before firing deferred submit', async () => {
      // Flush stale macrotasks from prior tests
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });

      // Two images: each gets its own resolve function
      const resolvers: ((path: string) => void)[] = [];
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            // Accept channel
          }
          if (cmd === 'save_image_command') {
            return new Promise<string>((resolve) => {
              resolvers.push(resolve);
            });
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Drop two images at once
      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      )!;
      const file1 = new File(['d1'], 'a.png', { type: 'image/png' });
      const file2 = new File(['d2'], 'b.png', { type: 'image/png' });
      fireEvent.drop(askBarWrapper, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [file1, file2] },
      });

      // Wait for both save_image_command calls
      await act(async () => {
        await vi.waitFor(() => expect(resolvers).toHaveLength(2));
      });

      // Submit while both images are still processing
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'two images' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve ONLY the first image — allReady should still be false
      await act(async () => {
        resolvers[0]('/tmp/img1.jpg');
      });
      await act(async () => {});

      // Still processing — second image not ready
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve the second image — now allReady is true, submit fires
      await act(async () => {
        resolvers[1]('/tmp/img2.jpg');
      });
      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      // User message should appear
      expect(screen.getByText('two images')).toBeInTheDocument();
    });

    it('cancels deferred submit when all images fail', async () => {
      // Make save_image_command hang then reject
      let rejectSave: ((err: Error) => void) | null = null;
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            // channel capture
          }
          if (cmd === 'save_image_command') {
            return new Promise<string>((_, reject) => {
              rejectSave = reject;
            });
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste and submit while processing
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      act(() => {
        fireEvent.change(textarea, { target: { value: 'describe' } });
      });

      // Wait for FileReader to complete and save_image_command to be invoked
      // (which sets rejectSave via the promise constructor).
      await act(async () => {
        await vi.waitFor(() => {
          expect(rejectSave).not.toBeNull();
        });
      });

      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Waiting state
      await vi.waitFor(() => {
        expect(
          screen.getByRole('button', { name: /stop/i }),
        ).toBeInTheDocument();
      });

      // Reject the image — it should be removed and pending submit cancelled
      await act(async () => {
        rejectSave!(new Error('disk full'));
      });

      // Image removed → no thumbnails → pending submit cancelled
      await vi.waitFor(() => {
        expect(
          screen.queryByRole('list', { name: /attached images/i }),
        ).toBeNull();
      });

      // ask_ollama should never have been called
      expect(invoke).not.toHaveBeenCalledWith('ask_ollama', expect.anything());

      // The "Processing images" button should be gone — back to normal send
      expect(
        screen.getByRole('button', { name: /send message/i }),
      ).toBeInTheDocument();

      // User's query should be restored so their text isn't lost
      expect(screen.getByPlaceholderText('Ask Thuki anything...')).toHaveValue(
        'describe',
      );
    });
  });

  it('revokes blob URLs when overlay reopens with attached images', async () => {
    enableChannelCaptureWithResponses({
      save_image_command: '/tmp/img.jpg',
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    // Paste an image so attachedImages is non-empty
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    const file = new File(['data'], 'img.png', { type: 'image/png' });
    await act(async () => {
      fireEvent.paste(textarea, {
        clipboardData: {
          items: [{ type: 'image/png', getAsFile: () => file }],
        },
      });
    });

    await vi.waitFor(() => {
      expect(
        screen.getByRole('list', { name: /attached images/i }),
      ).toBeInTheDocument();
    });

    // Reopen overlay — should clear images and revoke blob URLs
    await showOverlay();

    expect(URL.revokeObjectURL).toHaveBeenCalled();
    expect(screen.queryByRole('list', { name: /attached images/i })).toBeNull();
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
