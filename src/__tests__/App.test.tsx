import React from 'react';
import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
import App from '../App';
import {
  invoke,
  emitTauriEvent,
  enableChannelCapture,
  lastChannel,
} from '../test/mocks/tauri';

// Mock framer-motion to avoid rAF-loop issues in the test environment.
vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
      <div className={className} {...props}>
        {children}
      </div>
    ),
    span: ({ children, className, ...props }: React.HTMLAttributes<HTMLSpanElement>) => (
      <span className={className} {...props}>
        {children}
      </span>
    ),
    button: ({
      children,
      className,
      onClick,
      disabled,
      'aria-label': ariaLabel,
      ...props
    }: React.ButtonHTMLAttributes<HTMLButtonElement>) => (
      <button className={className} onClick={onClick} disabled={disabled} aria-label={ariaLabel} {...props}>
        {children}
      </button>
    ),
  },
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

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
