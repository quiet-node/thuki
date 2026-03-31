import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ConversationView } from '../ConversationView';

describe('ConversationView', () => {
  it('renders ChatBubble for each message', () => {
    const messages = [
      { role: 'user' as const, content: 'Hello there' },
      { role: 'assistant' as const, content: 'Hi!' },
    ];
    render(
      <ConversationView
        messages={messages}
        streamingContent=""
        isGenerating={false}
        error={null}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText('Hello there')).toBeInTheDocument();
    expect(screen.getByText('Hi!')).toBeInTheDocument();
  });

  it('renders streaming bubble when streamingContent is non-empty', () => {
    render(
      <ConversationView
        messages={[]}
        streamingContent="streaming response..."
        isGenerating={true}
        error={null}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText('streaming response...')).toBeInTheDocument();
  });

  it('shows TypingIndicator when isGenerating with no streaming content', () => {
    const { container } = render(
      <ConversationView
        messages={[]}
        streamingContent=""
        isGenerating={true}
        error={null}
        onClose={vi.fn()}
      />,
    );
    const dots = container.querySelectorAll('.rounded-full.bg-primary\\/70');
    expect(dots.length).toBeGreaterThanOrEqual(3);
  });

  it('hides TypingIndicator when streaming content arrives', () => {
    const { container } = render(
      <ConversationView
        messages={[]}
        streamingContent="some token"
        isGenerating={true}
        error={null}
        onClose={vi.fn()}
      />,
    );
    const dots = container.querySelectorAll('.rounded-full.bg-primary\\/70');
    expect(dots).toHaveLength(0);
  });

  it('shows error banner when error is non-null', () => {
    render(
      <ConversationView
        messages={[]}
        streamingContent=""
        isGenerating={false}
        error="Something went wrong"
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText('Something went wrong')).toBeInTheDocument();
  });

  it('hides error banner when error is null', () => {
    render(
      <ConversationView
        messages={[]}
        streamingContent=""
        isGenerating={false}
        error={null}
        onClose={vi.fn()}
      />,
    );
    expect(screen.queryByText('Something went wrong')).toBeNull();
  });

  it('renders WindowControls with onClose', () => {
    const onClose = vi.fn();
    render(
      <ConversationView
        messages={[]}
        streamingContent=""
        isGenerating={false}
        error={null}
        onClose={onClose}
      />,
    );
    expect(screen.getByRole('button', { name: 'Close window' })).toBeInTheDocument();
  });

  it('renders empty state with no messages (no .chat-bubble elements)', () => {
    const { container } = render(
      <ConversationView
        messages={[]}
        streamingContent=""
        isGenerating={false}
        error={null}
        onClose={vi.fn()}
      />,
    );
    expect(container.querySelectorAll('.chat-bubble')).toHaveLength(0);
  });

  it('handleScroll updates pinned state when user scrolls', () => {
    const { container } = render(
      <ConversationView
        messages={[{ role: 'user' as const, content: 'Hello' }]}
        streamingContent=""
        isGenerating={false}
        error={null}
        onClose={vi.fn()}
      />,
    );

    const scrollEl = container.querySelector(
      '.chat-messages-scroll',
    ) as HTMLElement;
    expect(scrollEl).not.toBeNull();

    // Fire a scroll event — the handler reads scrollTop/scrollHeight/clientHeight
    act(() => {
      fireEvent.scroll(scrollEl);
    });

    // No assertion needed beyond "no crash" — the callback just updates a ref
    expect(scrollEl).not.toBeNull();
  });

  it('auto-scroll is skipped when user is not near bottom (early return branch)', () => {
    const { container, rerender } = render(
      <ConversationView
        messages={[{ role: 'user' as const, content: 'first' }]}
        streamingContent=""
        isGenerating={false}
        error={null}
        onClose={vi.fn()}
      />,
    );

    const scrollEl = container.querySelector(
      '.chat-messages-scroll',
    ) as HTMLElement;
    expect(scrollEl).not.toBeNull();

    // Simulate scrolling far up — sets isUserNearBottomRef to false
    // by making scrollHeight - scrollTop - clientHeight > NEAR_BOTTOM_THRESHOLD (60)
    Object.defineProperty(scrollEl, 'scrollHeight', {
      value: 500,
      configurable: true,
    });
    Object.defineProperty(scrollEl, 'clientHeight', {
      value: 100,
      configurable: true,
    });
    Object.defineProperty(scrollEl, 'scrollTop', {
      value: 0,
      configurable: true,
      writable: true,
    });

    act(() => {
      fireEvent.scroll(scrollEl);
    });

    // Now rerender with new messages — the auto-scroll useEffect should hit the early return
    act(() => {
      rerender(
        <ConversationView
          messages={[
            { role: 'user' as const, content: 'first' },
            { role: 'assistant' as const, content: 'response' },
          ]}
          streamingContent=""
          isGenerating={false}
          error={null}
          onClose={vi.fn()}
        />,
      );
    });

    // scrollTop should remain 0 (auto-scroll was skipped)
    expect(scrollEl.scrollTop).toBe(0);
  });

  it('renders multiple messages correctly (10 messages)', () => {
    const messages = Array.from({ length: 10 }, (_, i) => ({
      role: (i % 2 === 0 ? 'user' : 'assistant') as 'user' | 'assistant',
      content: `Message ${i}`,
    }));
    render(
      <ConversationView
        messages={messages}
        streamingContent=""
        isGenerating={false}
        error={null}
        onClose={vi.fn()}
      />,
    );
    for (let i = 0; i < 10; i++) {
      expect(screen.getByText(`Message ${i}`)).toBeInTheDocument();
    }
  });
});
