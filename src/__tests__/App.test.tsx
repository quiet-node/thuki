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
      window_x: null,
      window_y: null,
      screen_bottom_y: null,
    });
  });
}

describe('App', () => {
  beforeEach(() => {
    invoke.mockClear();
    enableChannelCapture();
  });

  it('fetches model picker state on mount and refreshes it when the overlay shows', async () => {
    invoke.mockReset();
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
      },
    });

    render(<App />);
    await act(async () => {});

    expect(invoke).toHaveBeenCalledWith('get_model_picker_state');

    invoke.mockClear();

    await showOverlay();

    expect(invoke).toHaveBeenCalledWith('get_model_picker_state');
  });

  it('renders the model picker when the overlay is visible and models load', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
      },
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toBeInTheDocument();
  });

  it('saves the conversation with the currently selected model', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
      },
      save_conversation: { conversation_id: 'conv-1' },
      generate_title: undefined,
      set_active_model: undefined,
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {
      fireEvent.click(screen.getByRole('menuitem', { name: 'qwen2.5:7b' }));
    });

    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    fireEvent.change(textarea, { target: { value: 'hello there' } });
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    await act(async () => {});

    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hi there!' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });

    fireEvent.click(screen.getByLabelText('Save conversation'));

    expect(invoke).toHaveBeenCalledWith(
      'save_conversation',
      expect.objectContaining({ model: 'qwen2.5:7b' }),
    );
  });

  it('grows upward when near bottom screen edge', async () => {
    const { container } = render(<App />);
    await act(async () => {});

    await act(async () => {
      emitTauriEvent('thuki://visibility', {
        state: 'show',
        selected_text: null,
        window_x: 50,
        window_y: 1000,
        screen_bottom_y: 1100,
      });
    });

    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'hi' } });
    });
    await act(async () => {
      fireEvent.keyDown(textarea, { key: 'Enter', code: 'Enter' });
    });
    // This should morph into max-height window
    await act(async () => {
      await new Promise((r) => requestAnimationFrame(r));
    });
    expect(
      (container.querySelector('.morphing-container') as HTMLElement).style
        .height,
    ).toBe('600px');
  });

  it('keeps full chat height after clicking the expanded upward chat surface', async () => {
    const { container } = render(<App />);
    await act(async () => {});

    await act(async () => {
      emitTauriEvent('thuki://visibility', {
        state: 'show',
        selected_text: null,
        window_x: 50,
        window_y: 1000,
        screen_bottom_y: 1100,
      });
    });

    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'hi' } });
      fireEvent.keyDown(textarea, { key: 'Enter', code: 'Enter' });
    });

    const morphingContainer = container.querySelector(
      '.morphing-container',
    ) as HTMLElement;
    expect(morphingContainer.style.height).toBe('600px');

    const chatArea = container.querySelector('.chat-area');
    expect(chatArea).not.toBeNull();

    act(() => {
      fireEvent.mouseDown(chatArea!);
      fireEvent.mouseUp(window);
    });

    expect(morphingContainer.style.height).toBe('600px');
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

  it('hides overlay on Escape key and cancels an active /search turn', async () => {
    vi.useFakeTimers();
    enableChannelCapture();
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    act(() => {
      fireEvent.change(textarea, { target: { value: '/search rust async' } });
    });
    await act(async () => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });

    invoke.mockClear();

    await act(async () => {
      fireEvent.keyDown(window, { key: 'Escape' });
      vi.advanceTimersByTime(351);
      await Promise.resolve();
    });

    expect(invoke).toHaveBeenCalledWith('cancel_generation');
    expect(invoke).toHaveBeenCalledWith('notify_overlay_hidden');
    expect(screen.queryByPlaceholderText('Ask Thuki anything...')).toBeNull();
    vi.useRealTimers();
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

    // Then send hide-request - calls requestHideOverlay() (not handleCloseOverlay)
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

  it('clears upward growth on mouseup after drag', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const container = document.querySelector('.morphing-container');
    expect(container).not.toBeNull();

    __mockWindow.startDragging.mockClear();

    act(() => {
      fireEvent.mouseDown(container!);
    });

    // startDragging was called; fire mouseup to cover the mouseup handler
    act(() => {
      fireEvent.mouseUp(window);
    });

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

  it('applies justify-end when window is near screen bottom', async () => {
    render(<App />);
    await act(async () => {});

    // Show overlay near screen bottom: window_y=750, screen_bottom=900.
    // 750 + MAX_CHAT_WINDOW_HEIGHT(648) = 1398 > 900 → grows upward.
    await act(async () => {
      emitTauriEvent('thuki://visibility', {
        state: 'show',
        selected_text: null,
        window_x: 100,
        window_y: 750,
        screen_bottom_y: 900,
      });
    });

    const outer = document.querySelector('.justify-end');
    expect(outer).not.toBeNull();
  });

  it('applies justify-start when window has room below', async () => {
    render(<App />);
    await act(async () => {});

    // Show overlay near top: window_y=100, screen_bottom=900.
    // 100 + 648 = 748 < 900 → grows downward.
    await act(async () => {
      emitTauriEvent('thuki://visibility', {
        state: 'show',
        selected_text: null,
        window_x: 100,
        window_y: 100,
        screen_bottom_y: 900,
      });
    });

    const outer = document.querySelector('.justify-start');
    expect(outer).not.toBeNull();
    expect(document.querySelector('.justify-end')).toBeNull();
  });

  describe('ResizeObserver upward growth', () => {
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

    it('commits exact height when not streaming (initial ask bar)', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      // window_y=804, screen_bottom=900. bottomY = 804+80 = 884.
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_x: 100,
          window_y: 804,
          screen_bottom_y: 900,
        });
      });

      invoke.mockClear();

      const container = document.querySelector('.morphing-container');
      expect(container).not.toBeNull();

      // Not streaming yet, so exact height is committed (no buffer)
      act(() => {
        triggerResize(container!, 60);
      });

      // bottomY(884) - targetHeight(108) = 776
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 100,
        y: 776,
        width: 600,
        height: 108,
      });
    });

    it('uses setSize (not set_window_frame) after drag clears upward growth', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_x: 100,
          window_y: 804,
          screen_bottom_y: 900,
        });
      });

      const container = document.querySelector('.morphing-container');
      expect(container).not.toBeNull();

      // Drag clears upward growth
      act(() => {
        fireEvent.mouseDown(container!);
      });
      act(() => {
        fireEvent.mouseUp(window);
      });

      invoke.mockClear();
      __mockWindow.setSize.mockClear?.();

      act(() => {
        triggerResize(container!, 60);
      });
      expect(invoke).not.toHaveBeenCalledWith(
        'set_window_frame',
        expect.anything(),
      );
      expect(__mockWindow.setSize).toHaveBeenCalled();
    });

    it('resets upward growth on session reopen', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      // Session 1: near bottom, grows upward
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_x: 100,
          window_y: 804,
          screen_bottom_y: 900,
        });
      });

      const container1 = document.querySelector('.morphing-container');
      act(() => {
        triggerResize(container1!, 60);
      });

      // Close
      await act(async () => {
        emitTauriEvent('thuki://visibility', { state: 'hide-request' });
      });

      // Session 2: reopen near bottom again
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_x: 100,
          window_y: 804,
          screen_bottom_y: 900,
        });
      });

      const container2 = document.querySelector('.morphing-container');
      expect(container2).not.toBeNull();

      invoke.mockClear();
      act(() => {
        triggerResize(container2!, 60);
      });
      // bottomY = 804+80 = 884. 884-108 = 776.
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 100,
        y: 776,
        width: 600,
        height: 108,
      });
    });
  });

  it('requestHideOverlay is a no-op when already hidden', async () => {
    render(<App />);
    await act(async () => {});

    // Overlay is hidden initially - fire hide-request on hidden overlay
    // This exercises the 'hidden' branch in requestHideOverlay's state setter
    await act(async () => {
      emitTauriEvent('thuki://visibility', { state: 'hide-request' });
    });

    // No crash, no change - overlay is already hidden
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
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: 'gemma4:e2b',
          all: ['gemma4:e2b'],
        },
        list_conversations: [],
      });

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

      // Reopen - bookmark should reset (save button enabled again)
      enableChannelCapture();
      await showOverlay();

      // In ask-bar mode now - no save button visible, but history icon is
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

      // Click "Save & Start New" - save fails → should stay in chat mode
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
            model: 'gemma4:e2b',
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

      // Save & Switch - isSaved is FALSE so save_conversation should be called
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /save & switch/i }));
      });

      expect(invoke).toHaveBeenCalledWith(
        'save_conversation',
        expect.objectContaining({
          messages: expect.any(Array),
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
              model: 'gemma4:e2b',
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

      // Confirm "Save & Switch" - save_conversation will throw
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
            model: 'gemma4:e2b',
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

      // Click a different conversation - isSaved=true means no dialog, loads directly
      invoke.mockClear();
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /switch target/i }));
      });

      // No SwitchConfirmation dialog - save_conversation NOT called again
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
            model: 'gemma4:e2b',
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

      // Messages remain - still in chat mode
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

      // Click outside - should close the dropdown
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

      // Click inside the dropdown - should NOT close it
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
            model: 'gemma4:e2b',
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

      // Messages remain, isSaved is now false - save button is re-enabled
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
              model: 'gemma4:e2b',
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

      // Click the conversation - load_conversation will throw
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
            model: 'gemma4:e2b',
            updated_at: 1,
            message_count: 2,
          },
        ],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Open ask-bar history (no conversation loaded - conversationId is null)
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

      // Wait for FileReader + invoke to settle - failed image gets removed
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
            // channel capture - no-op for this test
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

    it('dropping image onto root window div attaches image in ask-bar mode', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const rootDiv = document.querySelector('.h-screen')!;
      expect(rootDiv).not.toBeNull();
      const file = new File(['data'], 'photo.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.drop(rootDiv, {
          preventDefault: vi.fn(),
          dataTransfer: { files: [file] },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });
    });

    it('dropping image onto root window div attaches image in chat mode (second image after conversation)', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Send a plain text message and complete the generation to enter chat mode
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'hello' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Complete the AI response so isGenerating becomes false
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hi!' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      // Confirm we are in chat mode with generation complete
      expect(screen.getByPlaceholderText('Reply...')).toBeInTheDocument();

      // Now in chat mode. Drop image onto root div (not AskBarView specifically)
      const rootDiv = document.querySelector('.h-screen')!;
      expect(rootDiv).not.toBeNull();
      const file = new File(['data'], 'second.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.drop(rootDiv, {
          preventDefault: vi.fn(),
          dataTransfer: { files: [file] },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });
    });

    it('dragOver anywhere in window shows violet ring on AskBarView when under max', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const rootDiv = document.querySelector('.h-screen')!;
      expect(rootDiv).not.toBeNull();
      fireEvent.dragOver(rootDiv, { preventDefault: vi.fn() });

      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      )!;
      expect(askBarWrapper.classList.contains('ring-2')).toBe(true);
      expect(askBarWrapper.classList.contains('ring-red-500/60')).toBe(false);
    });

    it('dragOver shows red ring and max label when already at max images', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste 3 images to reach max
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      for (let i = 0; i < 3; i++) {
        const file = new File([`data${i}`], `img${i}.png`, {
          type: 'image/png',
        });
        await act(async () => {
          fireEvent.paste(textarea, {
            clipboardData: {
              items: [{ type: 'image/png', getAsFile: () => file }],
            },
          });
        });
      }

      // Wait for 3 thumbnails
      await vi.waitFor(() => {
        expect(screen.getAllByRole('listitem')).toHaveLength(3);
      });

      // Now drag over; should show red ring and max label
      const rootDiv = document.querySelector('.h-screen')!;
      fireEvent.dragOver(rootDiv, { preventDefault: vi.fn() });

      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      )!;
      expect(askBarWrapper.classList.contains('ring-red-500/60')).toBe(true);
      expect(screen.getByText('Max 3 images')).toBeInTheDocument();
    });

    it('dragLeave when cursor exits window clears drag-over ring', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const rootDiv = document.querySelector('.h-screen')!;
      fireEvent.dragOver(rootDiv, { preventDefault: vi.fn() });
      // relatedTarget null simulates cursor leaving the window entirely
      fireEvent.dragLeave(rootDiv, { relatedTarget: null });

      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      )!;
      expect(askBarWrapper.classList.contains('ring-2')).toBe(false);
    });

    it('dragOver when generating does not show drag-over ring', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Submit to trigger isGenerating
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'hi' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      const rootDiv = document.querySelector('.h-screen')!;
      fireEvent.dragOver(rootDiv, { preventDefault: vi.fn() });

      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      )!;
      expect(askBarWrapper.classList.contains('ring-2')).toBe(false);
    });

    it('handleRootDrop ignores drop during generation', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: 'hi' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      const rootDiv = document.querySelector('.h-screen')!;
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      fireEvent.drop(rootDiv, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [file] },
      });

      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('handleRootDrop ignores drop with no dataTransfer files', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const rootDiv = document.querySelector('.h-screen')!;
      fireEvent.drop(rootDiv, { preventDefault: vi.fn() });

      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('handleRootDrop ignores drop when already at max images', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img.jpg',
      });
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      for (let i = 0; i < 3; i++) {
        const img = new File([`d${i}`], `i${i}.png`, { type: 'image/png' });
        await act(async () => {
          fireEvent.paste(textarea, {
            clipboardData: {
              items: [{ type: 'image/png', getAsFile: () => img }],
            },
          });
        });
      }
      await vi.waitFor(() => {
        expect(screen.getAllByRole('listitem')).toHaveLength(3);
      });

      const rootDiv = document.querySelector('.h-screen')!;
      const extra = new File(['extra'], 'extra.png', { type: 'image/png' });
      fireEvent.drop(rootDiv, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [extra] },
      });

      // Still exactly 3 - the drop was rejected
      expect(screen.getAllByRole('listitem')).toHaveLength(3);
    });

    it('handleRootDrop ignores non-image files', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const rootDiv = document.querySelector('.h-screen')!;
      const doc = new File(['text'], 'doc.txt', { type: 'text/plain' });
      fireEvent.drop(rootDiv, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [doc] },
      });

      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
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

      // Click the preview button in the chat bubble - should open the modal
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

      // Click remove twice rapidly - the second call should be a no-op
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
            // channel capture - no-op
          }
          if (cmd === 'save_image_command') {
            return new Promise(() => {}); // never resolves
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image - thumbnail appears immediately with null filePath
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

      // Paste an image; thumbnail appears immediately (filePath null)
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

      // Resolve the image; triggers deferred submit chain
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

      // Should be generating - stop button visible
      const stopBtn = screen.getByRole('button', { name: /stop/i });
      expect(stopBtn).toBeInTheDocument();

      // Click stop - should call cancel_generation
      invoke.mockClear();
      enableChannelCapture();

      await act(async () => {
        fireEvent.click(stopBtn);
      });

      expect(invoke).toHaveBeenCalledWith('cancel_generation');
    });

    it('stop button hard-aborts an active /search turn and resets search mode', async () => {
      let resolveSearch!: () => void;
      let resolveCancel!: () => void;

      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'search_pipeline') {
          return new Promise<void>((res) => {
            resolveSearch = res;
          });
        }
        if (cmd === 'cancel_generation') {
          return new Promise<void>((res) => {
            resolveCancel = res;
          });
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/search what is Rust?' },
        });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const stopBtn = screen.getByRole('button', { name: /stop/i });
      expect(stopBtn).toBeInTheDocument();

      act(() => {
        fireEvent.click(stopBtn);
      });

      expect(invoke).toHaveBeenCalledWith('cancel_generation');
      expect(screen.queryByRole('button', { name: /stop/i })).toBeNull();
      expect(textarea).not.toBeDisabled();

      act(() => {
        fireEvent.change(textarea, { target: { value: 'hello' } });
      });
      expect(textarea).toHaveValue('hello');

      await act(async () => {
        resolveCancel?.();
        resolveSearch?.();
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const calls = invoke.mock.calls.filter(
        (c) => c[0] === 'ask_ollama' || c[0] === 'search_pipeline',
      );
      const last = calls[calls.length - 1];
      expect(last[0]).toBe('ask_ollama');
      expect(last[1]).toMatchObject({ message: 'hello' });
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

      // Resolve ONLY the first image - allReady should still be false
      await act(async () => {
        resolvers[0]('/tmp/img1.jpg');
      });
      await act(async () => {});

      // Still processing - second image not ready
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve the second image - now allReady is true, submit fires
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

      // Reject the image - it should be removed and pending submit cancelled
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

      // The "Processing images" button should be gone - back to normal send
      expect(
        screen.getByRole('button', { name: /send message/i }),
      ).toBeInTheDocument();

      // User's query should be restored so their text isn't lost
      expect(screen.getByPlaceholderText('Ask Thuki anything...')).toHaveValue(
        'describe',
      );
    });
  });

  // ─── Screenshot integration ────────────────────────────────────────────────

  describe('screenshot integration', () => {
    it('clicking screenshot button invokes capture_screenshot', async () => {
      enableChannelCaptureWithResponses({ capture_screenshot_command: null });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Take screenshot' }),
        );
      });

      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith('capture_screenshot_command');
        });
      });
    });

    it('does nothing when capture_screenshot returns null (cancelled)', async () => {
      enableChannelCaptureWithResponses({ capture_screenshot_command: null });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Take screenshot' }),
        );
      });

      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith('capture_screenshot_command');
        });
      });

      // save_image_command must NOT have been called
      const saveCalls = invoke.mock.calls.filter(
        ([cmd]) => cmd === 'save_image_command',
      );
      expect(saveCalls).toHaveLength(0);
    });

    it('does not invoke capture_screenshot_command when at max images', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img.jpg',
        capture_screenshot_command: null,
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Attach 3 images via paste to reach the limit.
      const pasteOneImage = async () => {
        const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
        const file = new File(['data'], 'photo.png', { type: 'image/png' });
        await act(async () => {
          fireEvent.paste(textarea, {
            clipboardData: {
              items: [{ type: 'image/png', getAsFile: () => file }],
            },
          });
        });
      };
      await pasteOneImage();
      await pasteOneImage();
      await pasteOneImage();

      const btn = screen.getByRole('button', { name: 'Take screenshot' });
      expect(btn).toBeDisabled();

      invoke.mockClear();
      fireEvent.click(btn);
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('capture_screenshot_command');
    });

    it('attaches screenshot image when capture_screenshot returns base64', async () => {
      const fakeBase64 = btoa('fake screenshot bytes');
      enableChannelCaptureWithResponses({
        capture_screenshot_command: fakeBase64,
        save_image_command: '/tmp/screenshot.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Take screenshot' }),
        );
      });

      // Wait for invoke(capture_screenshot) → FileReader → invoke(save_image_command)
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.objectContaining({ imageDataBase64: expect.any(String) }),
          );
        });
      });
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

    // Reopen overlay - should clear images and revoke blob URLs
    await showOverlay();

    expect(URL.revokeObjectURL).toHaveBeenCalled();
    expect(screen.queryByRole('list', { name: /attached images/i })).toBeNull();
  });

  it('revokes blob URLs when overlay hides with attached images', async () => {
    enableChannelCaptureWithResponses({
      save_image_command: '/tmp/img.jpg',
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

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

    const revokeSpy = vi.mocked(URL.revokeObjectURL);
    revokeSpy.mockClear();

    // Hide overlay via Escape - requestHideOverlay should revoke blob URLs
    await act(async () => {
      fireEvent.keyDown(window, { key: 'Escape' });
    });

    expect(revokeSpy).toHaveBeenCalled();
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

    // Reopen overlay - should reset session
    await showOverlay();

    // Should be back to input bar mode with placeholder
    expect(
      screen.getByPlaceholderText('Ask Thuki anything...'),
    ).toBeInTheDocument();
    // Old messages should be gone
    expect(screen.queryByText('First response')).toBeNull();
  });

  // ─── /screen command ─────────────────────────────────────────────────────────

  describe('/screen command', () => {
    it('invokes capture_full_screen_command and calls ask with screenshot path', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Use "/screen " (with trailing space) so the suggestion popover is dismissed
      // and Enter goes to the submit handler directly.
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/screen ' } });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('capture_full_screen_command');
      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          imagePaths: ['/tmp/screen.jpg'],
          message: '/screen',
        }),
      );
    });

    it('keeps the /screen trigger in the message sent to the backend', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/screen what is this error?' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: '/screen what is this error?',
          imagePaths: ['/tmp/screen.jpg'],
        }),
      );
    });

    it('detects /screen anywhere in the message, not just at start', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: 'hello /screen there' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('capture_full_screen_command');
      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: 'hello /screen there',
          imagePaths: ['/tmp/screen.jpg'],
        }),
      );
    });

    it('does not call ask when capture_full_screen_command throws', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'capture_full_screen_command') {
          throw new Error('Permission denied');
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Use "/screen " (with trailing space) so the suggestion popover is dismissed
      // and Enter goes directly to the submit handler.
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/screen ' } });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('capture_full_screen_command');
      expect(invoke).not.toHaveBeenCalledWith('ask_ollama', expect.anything());
      // The actual Rust error message is surfaced directly.
      expect(screen.getByText('Permission denied')).toBeInTheDocument();
    });

    it('surfaces string errors from Tauri invoke directly', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'capture_full_screen_command') {
          // Tauri v2 rejects with the Err(String) value as a plain string.
          return Promise.reject(
            'Screen Recording permission is required to use /screen.',
          );
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/screen ' } });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(
        screen.getByText(
          'Screen Recording permission is required to use /screen.',
        ),
      ).toBeInTheDocument();
    });

    it('handles non-Error non-string rejection values', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'capture_full_screen_command') {
          return Promise.reject(42);
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/screen ' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(screen.getByText('42')).toBeInTheDocument();
    });

    it('clears capture error when a new submit is attempted', async () => {
      enableChannelCapture();
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'capture_full_screen_command') {
          throw new Error('capture failed');
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      // First attempt fails; error banner appears.
      act(() => {
        fireEvent.change(textarea, { target: { value: '/screen ' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      expect(screen.getByText('capture failed')).toBeInTheDocument();

      // Typing a new query and submitting normal text clears the error banner.
      act(() => {
        fireEvent.change(textarea, { target: { value: 'hello' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      expect(screen.queryByText('capture failed')).toBeNull();
    });

    it('merges screenshot path with existing attached images', async () => {
      // Set up mocks: save_image_command for image attachment, then screen capture.
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/attached.jpg',
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image first. This exercises the filter/map on attachedImages inside
      // handleScreenSubmit, covering the lines for non-null filePath images.
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['img'], 'photo.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      // Wait for the image to be processed (filePath resolved).
      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith(
          'save_image_command',
          expect.anything(),
        );
        expect(screen.getAllByRole('listitem')).toHaveLength(1);
      });

      // Now type /screen and submit.
      act(() => {
        fireEvent.change(textarea, { target: { value: '/screen describe' } });
      });

      vi.useFakeTimers();
      try {
        await act(async () => {
          fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
          await Promise.resolve();
          await Promise.resolve();
        });

        expect(invoke).toHaveBeenCalledWith('capture_full_screen_command');
        expect(invoke).toHaveBeenCalledWith(
          'ask_ollama',
          expect.objectContaining({
            message: '/screen describe',
            imagePaths: ['/tmp/attached.jpg', '/tmp/screen.jpg'],
          }),
        );

        await act(async () => {
          getLastChannel()?.simulateMessage({ type: 'Token', data: 'done' });
          getLastChannel()?.simulateMessage({ type: 'Done' });
          await Promise.resolve();
          await Promise.resolve();
        });
      } finally {
        vi.useRealTimers();
      }
    });

    it('handles /screen with selected context', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay('some context');

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/screen explain' } });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: '/screen explain',
          quotedText: 'some context',
          imagePaths: ['/tmp/screen.jpg'],
        }),
      );
    });

    it('shows pending chat bubble immediately on submit before capture resolves', async () => {
      let resolveCapture!: (path: string) => void;
      enableChannelCaptureWithResponses({
        capture_full_screen_command: new Promise<string>((res) => {
          resolveCapture = res;
        }),
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/screen check this' } });
      });

      // Submit; capture is now in-flight (pending)
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Before capture resolves: query should be cleared and app in pending mode
      expect((textarea as HTMLTextAreaElement).value).toBe('');

      // Resolve the capture and let async work settle
      await act(async () => {
        resolveCapture('/tmp/screen.jpg');
      });
      await act(async () => {});

      // After capture resolves: ask_ollama should be called
      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({ message: '/screen check this' }),
      );
    });

    it('restores query with cleanQuery text when capture fails mid-message', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'capture_full_screen_command') {
          throw new Error('Screen capture timed out');
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/screen what is this?' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Query should be restored with the full original message
      expect((textarea as HTMLTextAreaElement).value).toBe(
        '/screen what is this?',
      );
      expect(screen.getByText('Screen capture timed out')).toBeInTheDocument();
    });

    it('uses blobUrl for still-processing attached images in the pending bubble', async () => {
      // save_image_command never resolves: image stays in null-filePath state.
      // Use enableChannelCaptureWithResponses so channel capture (for ask_ollama)
      // still works alongside the custom per-command responses.
      enableChannelCaptureWithResponses({
        save_image_command: new Promise<string>(() => {}),
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image; save_image_command hangs, so filePath stays null
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['img'], 'photo.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      // Submit /screen immediately; image still processing (filePath === null)
      act(() => {
        fireEvent.change(textarea, { target: { value: '/screen ' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Capture succeeded; ask_ollama called with only the screenshot
      // (the attached image never resolved its filePath)
      expect(invoke).toHaveBeenCalledWith('capture_full_screen_command');
      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          imagePaths: ['/tmp/screen.jpg'],
        }),
      );
    });

    it('cancelling during in-flight capture prevents ask from being called', async () => {
      let resolveCapture!: (path: string) => void;
      enableChannelCaptureWithResponses({
        capture_full_screen_command: new Promise<string>((res) => {
          resolveCapture = res;
        }),
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/screen ' } });
      });

      // Submit; capture is now in-flight
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Cancel while capture is pending (Stop button)
      const stopButton = screen.getByRole('button', { name: /stop|cancel/i });
      act(() => {
        fireEvent.click(stopButton);
      });

      // Resolve the capture after cancel
      await act(async () => {
        resolveCapture('/tmp/screen.jpg');
      });
      await act(async () => {});

      // ask_ollama must NOT be called since the user cancelled
      expect(invoke).not.toHaveBeenCalledWith('ask_ollama', expect.anything());
    });
  });

  // ─── /think command ─────────────────────────────────────────────────────────

  describe('/think command', () => {
    it('sends think:true to ask_ollama and keeps /think prefix in message', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/think why is the sky blue?' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: '/think why is the sky blue?',
          think: true,
        }),
      );
    });

    it('shows a warming-up placeholder first, then swaps it to the thinking row when thinking tokens arrive', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/think explain recursion' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      expect(screen.getByTestId('thinking-block')).toBeInTheDocument();
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Warming up...',
      );
      expect(
        screen.queryByRole('button', { name: 'Toggle thinking details' }),
      ).toBeNull();

      act(() => {
        getLastChannel()?.simulateMessage({
          type: 'ThinkingToken',
          data: 'Let me think this through.',
        });
      });

      expect(screen.queryByText('Warming up...')).toBeNull();
      expect(
        screen.getByRole('button', { name: 'Toggle thinking details' }),
      ).toBeInTheDocument();
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Thinking...',
      );
    });

    it('does nothing when /think has no query and no images', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/think' } });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('ask_ollama', expect.anything());
    });

    it('detects /think anywhere in the message, not just at start', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: 'hello /think world' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: 'hello /think world',
          think: true,
        }),
      );
    });

    it('forwards selected context when /think is used with quoted text', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay('some selected text');

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/think explain this code' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: '/think explain this code',
          quotedText: 'some selected text',
          think: true,
        }),
      );
    });

    it('sends think:true with /think followed by only a space', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/think ' } });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      // "/think " with only a space after prefix, no actual query, no images => no submit
      expect(invoke).not.toHaveBeenCalledWith('ask_ollama', expect.anything());
    });
  });

  // ─── Multi-command ──────────────────────────────────────────────────────────

  describe('Multi-command support', () => {
    it('sends /screen /think with both screen capture and think:true', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/screen /think explain this' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('capture_full_screen_command');
      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: '/screen /think explain this',
          imagePaths: ['/tmp/screen.jpg'],
          think: true,
        }),
      );
    });

    it('sends /think /screen with both screen capture and think:true', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/think /screen explain this' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('capture_full_screen_command');
      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: '/think /screen explain this',
          imagePaths: ['/tmp/screen.jpg'],
          think: true,
        }),
      );
    });
  });

  // ─── Utility commands ───────────────────────────────────────────────────────

  describe('Utility commands (buildPrompt routing)', () => {
    it('routes /rewrite command through buildPrompt and calls ask_ollama with composed prompt', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/rewrite fix this text' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_ollama');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Please help rewrite the text below');
        expect(args.message).toContain('fix this text');
      });
    });

    it('routes /translate with language arg through buildPrompt', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/translate jpn hello world' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_ollama');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Target language: jpn');
        expect(args.message).toContain('Text: hello world');
      });
    });

    it('/think and utility command compose: /think /tldr some long text', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/think /tldr some long text' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_ollama');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Summarize the following text');
        expect(args.message).toContain('some long text');
        expect(args.think).toBe(true);
      });
    });

    it('utility command with no input text does not call ask_ollama', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/rewrite' } });
      });

      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('ask_ollama', expect.anything());
    });

    it('utility command returns null composedPrompt when no usable input is found', async () => {
      // /translate with only a language code (no text to translate) makes buildPrompt return null.
      // strippedMessage = 'jpn' (non-empty, bypasses the early guard) but buildPrompt gets
      // lang='jpn', typedRemainder='', selected='', so returns null.
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/translate jpn' } });
      });

      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('ask_ollama', expect.anything());
    });

    it('utility command uses selected context when available', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      // Activate overlay with selected text as context
      await showOverlay('original selected text');

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      // Type a command with extra instruction so strippedMessage is non-empty
      // (bypasses the "no content" early guard) and selectedContext is also set.
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/rewrite make it concise' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_ollama');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Please help rewrite the text below');
        expect(args.message).toContain('original selected text');
        expect(args.quotedText).toBe('original selected text');
      });
    });

    it('utility command with bare trigger uses selected context as display text', async () => {
      // strippedMessage is empty, selectedContext is present, images bypass the
      // early-return guard. displayText falls through to selectedContext?.trim().
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/ctx.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay('my selected text');

      // Paste an image and wait for backend resolution so hasPendingImages is false
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });
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

      // Submit just the command trigger (strippedMessage will be '')
      act(() => {
        fireEvent.change(textarea, { target: { value: '/rewrite' } });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_ollama');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        // The prompt should use selectedContext as $INPUT
        expect(args.message).toContain('my selected text');
      });
    });

    it('displays stripped user input in chat bubble, not the prompt template', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/rewrite fix this text' },
        });
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      // renderUserContent splits command triggers into separate spans.
      // Check body textContent to confirm the full original query appears.
      await vi.waitFor(() => {
        expect(document.body.textContent).toContain('/rewrite fix this text');
      });
    });

    it('utility command with resolved attached images passes imagePaths and revokes blob URLs', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image and wait for backend resolution
      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      const file = new File(['fake-img-data'], 'photo.png', {
        type: 'image/png',
      });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });
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

      // Type /rewrite command and submit
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/rewrite fix this prose' },
        });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_ollama');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Please help rewrite the text below');
        expect(args.imagePaths).toEqual(['/tmp/staged/img1.jpg']);
      });
    });

    it('utility command with pending images defers submit until images resolve', async () => {
      // Flush stale macrotasks from prior tests
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });

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

      // Paste an image - thumbnail appears immediately (filePath null)
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

      // Type /rewrite and submit while image is still processing
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/rewrite make it clearer' },
        });
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Should show pending state (stop button visible)
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve the image - triggers deferred submit chain
      resolveSave!('/tmp/staged/img1.jpg');

      // Flush async chain: promise -> state update -> effect -> ask -> invoke
      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      // renderUserContent splits command triggers into separate spans.
      // Check body textContent to confirm the full original query appears.
      expect(document.body.textContent).toContain('/rewrite make it clearer');
    });
  });

  // ─── /search command ───────────────────────────────────────────────────────

  describe('/search command', () => {
    it('routes /search submissions to search_pipeline with the stripped query', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/search rust async' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      expect(invoke).toHaveBeenCalledWith(
        'search_pipeline',
        expect.objectContaining({ message: 'rust async' }),
      );
    });

    it('moves selected context into the /search user bubble and clears the ask bar preview', async () => {
      enableChannelCapture();
      const { container } = render(<App />);
      await act(async () => {});
      await showOverlay('selected snippet');

      const findSelectedSnippet = () =>
        screen.getAllByText(/selected snippet/i, { selector: 'p' });

      expect(findSelectedSnippet()).toHaveLength(1);
      expect(container.querySelectorAll('p.text-text-secondary')).toHaveLength(
        1,
      );

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/search explain this selection' },
        });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      expect(textarea).toHaveValue('');
      expect(findSelectedSnippet()).toHaveLength(1);
      expect(container.querySelectorAll('p.text-text-secondary')).toHaveLength(
        0,
      );
      expect(
        container.querySelectorAll('p[class*="text-white/60"]'),
      ).toHaveLength(1);
    });

    it('keeps searchActive after a clarify trace with question tokens', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/search who is him' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const firstChannel = getLastChannel();
      await act(async () => {
        firstChannel!.onmessage({
          type: 'Trace',
          step: {
            id: 'clarify',
            kind: 'clarify',
            status: 'completed',
            title: 'Waiting for clarification',
            summary: 'Search is paused until you clarify who or what you mean.',
          },
        });
        firstChannel!.onmessage({ type: 'Token', content: 'Which person?' });
        firstChannel!.onmessage({ type: 'Done' });
      });
      await act(async () => {
        await Promise.resolve();
      });

      const followupInvokeCountBefore = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      act(() => {
        fireEvent.change(textarea, { target: { value: 'Donald Trump' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      const followupInvokeCountAfter = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      expect(followupInvokeCountAfter).toBe(followupInvokeCountBefore + 1);
      const searchCalls = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      );
      expect(searchCalls[searchCalls.length - 1][1]).toMatchObject({
        message: 'Donald Trump',
      });
    });

    it('continues routing follow-ups through search_pipeline after a clarify trace', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/search who is him' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const firstChannel = getLastChannel();
      await act(async () => {
        firstChannel!.onmessage({
          type: 'Trace',
          step: {
            id: 'clarify',
            kind: 'clarify',
            status: 'completed',
            title: 'Waiting for clarification',
            summary: 'Search is paused until you clarify who or what you mean.',
          },
        });
        firstChannel!.onmessage({ type: 'Token', content: 'Which person?' });
        firstChannel!.onmessage({ type: 'Done' });
      });
      // Flush askSearch promise + .then() so isGenerating updates.
      await act(async () => {
        await Promise.resolve();
      });

      // Follow-up without /search prefix should still route to search_pipeline.
      const followupInvokeCountBefore = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      act(() => {
        fireEvent.change(textarea, { target: { value: 'Donald Trump' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      const followupInvokeCountAfter = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      expect(followupInvokeCountAfter).toBe(followupInvokeCountBefore + 1);
      const searchCalls = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      );
      expect(searchCalls[searchCalls.length - 1][1]).toMatchObject({
        message: 'Donald Trump',
      });
    });

    it('drops searchActive after a final Token+Done turn so the next submit uses ask_ollama', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/search rust' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const channel = getLastChannel();
      await act(async () => {
        channel!.onmessage({ type: 'Searching', queries: [] });
        channel!.onmessage({ type: 'Token', content: 'Rust is fast.' });
        channel!.onmessage({ type: 'Done' });
      });
      // Flush the askSearch promise + .then() so searchActive resets to false.
      await act(async () => {
        await Promise.resolve();
      });

      act(() => {
        fireEvent.change(textarea, { target: { value: 'hello' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const calls = invoke.mock.calls.filter(
        (c) => c[0] === 'ask_ollama' || c[0] === 'search_pipeline',
      );
      const last = calls[calls.length - 1];
      expect(last[0]).toBe('ask_ollama');
      expect(last[1]).toMatchObject({ message: 'hello' });
    });

    it('follow-up after a clarify trace still routes through search_pipeline', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/search ambiguous' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      const firstChannel = getLastChannel();
      await act(async () => {
        firstChannel!.onmessage({
          type: 'Trace',
          step: {
            id: 'clarify',
            kind: 'clarify',
            status: 'completed',
            title: 'Waiting for clarification',
            summary: 'Search is paused until you clarify who or what you mean.',
          },
        });
        firstChannel!.onmessage({ type: 'Token', content: 'First clarify?' });
        firstChannel!.onmessage({ type: 'Done' });
      });
      await act(async () => {
        await Promise.resolve();
      });

      // User types their own clarification and submits - still routes to
      // search_pipeline because searchActive persisted (final=false on clarify).
      const countBefore = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      act(() => {
        fireEvent.change(textarea, { target: { value: 'Einstein' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      const countAfter = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      expect(countAfter).toBe(countBefore + 1);
      const allSearchCalls = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      );
      expect(allSearchCalls[allSearchCalls.length - 1][1]).toMatchObject({
        message: 'Einstein',
      });
    });

    it('ignores empty /search submissions with no text after the trigger', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/search' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      expect(invoke.mock.calls.some((c) => c[0] === 'search_pipeline')).toBe(
        false,
      );
    });

    it('lets /screen override search continuation mid-conversation', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/s.jpg',
      });
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
      act(() => {
        fireEvent.change(textarea, { target: { value: '/search him' } });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      const channel = getLastChannel();
      await act(async () => {
        channel!.onmessage({
          type: 'Trace',
          step: {
            id: 'clarify',
            kind: 'clarify',
            status: 'completed',
            title: 'Waiting for clarification',
            summary: 'Search is paused until you clarify who or what you mean.',
          },
        });
        channel!.onmessage({ type: 'Token', content: 'Which?' });
        channel!.onmessage({ type: 'Done' });
      });

      // With searchActive still on, /screen must take precedence.
      act(() => {
        fireEvent.change(textarea, {
          target: { value: '/screen what is this' },
        });
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      expect(invoke).toHaveBeenCalledWith('capture_full_screen_command');
    });
  });

  describe('Onboarding', () => {
    it('shows onboarding screen when thuki://onboarding event fires', async () => {
      enableChannelCaptureWithResponses({
        check_accessibility_permission: false,
        check_screen_recording_permission: false,
      });

      render(<App />);
      await act(async () => {});

      await act(async () => {
        emitTauriEvent('thuki://onboarding', { stage: 'permissions' });
      });

      expect(screen.getByText("Let's get Thuki set up")).toBeInTheDocument();
    });

    it('does not show onboarding on normal visibility event', async () => {
      render(<App />);
      await act(async () => {});

      await showOverlay();

      expect(screen.queryByText("Let's get Thuki set up")).toBeNull();
    });

    it('renders normal ask bar when overlay is shown without onboarding', async () => {
      render(<App />);
      await act(async () => {});

      await showOverlay();

      expect(
        screen.getByPlaceholderText('Ask Thuki anything...'),
      ).toBeInTheDocument();
    });

    it('dismisses onboarding and shows ask bar when onComplete is called', async () => {
      invoke.mockResolvedValue(undefined);

      render(<App />);
      await act(async () => {});

      await act(async () => {
        emitTauriEvent('thuki://onboarding', { stage: 'intro' });
      });

      expect(screen.getByText('Before you dive in')).toBeInTheDocument();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /get started/i }));
      });

      expect(screen.queryByText('Before you dive in')).toBeNull();
    });
  });
});
