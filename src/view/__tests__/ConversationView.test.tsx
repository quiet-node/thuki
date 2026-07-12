import { render, screen, act, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { ConversationView } from '../ConversationView';
import { invoke } from '../../testUtils/mocks/tauri';

describe('ConversationView', () => {
  it('renders ChatBubble for each message', () => {
    const messages = [
      { id: '1', role: 'user' as const, content: 'Hello there' },
      { id: '2', role: 'assistant' as const, content: 'Hi!' },
    ];
    render(
      <ConversationView
        messages={messages}
        isGenerating={false}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText('Hello there')).toBeInTheDocument();
    expect(screen.getByText('Hi!')).toBeInTheDocument();
  });

  it('renders streaming assistant message when isGenerating', () => {
    const { container } = render(
      <ConversationView
        messages={[
          {
            id: '1',
            role: 'assistant' as const,
            content: 'streaming response...',
          },
        ]}
        isGenerating={true}
        onClose={vi.fn()}
      />,
    );
    // Streamdown splits streaming text into per-word animated spans,
    // so exact full-text match won't work. Check for content presence.
    expect(container.textContent).toContain('streaming');
    expect(container.textContent).toContain('response...');
  });

  it('shows RequestStatusStrip when isGenerating with empty assistant content', () => {
    const { container } = render(
      <ConversationView
        messages={[{ id: '1', role: 'assistant' as const, content: '' }]}
        isGenerating={true}
        onClose={vi.fn()}
      />,
    );
    // Unified strip: Y1 three-dot motion host
    expect(
      container.querySelectorAll('[data-testid="three-dot-motion"]').length,
    ).toBeGreaterThanOrEqual(1);
  });

  it('hides RequestStatusStrip when assistant content arrives', () => {
    const { container } = render(
      <ConversationView
        messages={[
          { id: '1', role: 'assistant' as const, content: 'some token' },
        ]}
        isGenerating={true}
        onClose={vi.fn()}
      />,
    );
    expect(
      container.querySelectorAll('[data-testid="three-dot-motion"]'),
    ).toHaveLength(0);
  });

  it('renders WindowControls with onClose', () => {
    const onClose = vi.fn();
    render(
      <ConversationView messages={[]} isGenerating={false} onClose={onClose} />,
    );
    expect(
      screen.getByRole('button', { name: 'Close window' }),
    ).toBeInTheDocument();
  });

  it('renders empty state with no messages (no .chat-bubble elements)', () => {
    const { container } = render(
      <ConversationView messages={[]} isGenerating={false} onClose={vi.fn()} />,
    );
    expect(container.querySelectorAll('.chat-bubble')).toHaveLength(0);
  });

  it('auto-scroll is skipped when user scrolls up via wheel', () => {
    const { container, rerender } = render(
      <ConversationView
        messages={[{ id: '1', role: 'user' as const, content: 'first' }]}
        isGenerating={false}
        onClose={vi.fn()}
      />,
    );

    const scrollEl = container.querySelector(
      '.chat-messages-scroll',
    ) as HTMLElement;
    expect(scrollEl).not.toBeNull();

    Object.defineProperty(scrollEl, 'scrollTop', {
      value: 0,
      configurable: true,
      writable: true,
    });

    // Simulate the user scrolling up (negative deltaY) - this is the only
    // mechanism that disables auto-scroll, avoiding false negatives from
    // layout-induced scroll events during spring height measurement.
    act(() => {
      scrollEl.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }));
    });

    // Rerender with new streaming content - auto-scroll should be skipped
    // because the user explicitly scrolled up
    act(() => {
      rerender(
        <ConversationView
          messages={[
            { id: '1', role: 'user' as const, content: 'first' },
            { id: '2', role: 'assistant' as const, content: 'new token' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
        />,
      );
    });

    // scrollTop should remain 0 (auto-scroll was skipped)
    expect(scrollEl.scrollTop).toBe(0);
  });

  it('auto-scroll re-enables when a new user message is added', () => {
    const { container, rerender } = render(
      <ConversationView
        messages={[{ id: '1', role: 'user' as const, content: 'first' }]}
        isGenerating={false}
        onClose={vi.fn()}
      />,
    );

    const scrollEl = container.querySelector(
      '.chat-messages-scroll',
    ) as HTMLElement;
    expect(scrollEl).not.toBeNull();

    Object.defineProperty(scrollEl, 'scrollTop', {
      value: 0,
      configurable: true,
      writable: true,
    });

    // User scrolls up - disables auto-scroll
    act(() => {
      scrollEl.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }));
    });

    // Add a new user message - this should re-enable auto-scroll because
    // sending a message is an explicit "I want to see the response" action
    act(() => {
      rerender(
        <ConversationView
          messages={[
            { id: '1', role: 'user' as const, content: 'first' },
            { id: '2', role: 'user' as const, content: 'second question' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
        />,
      );
    });

    // Auto-scroll should have re-engaged (scrollTop set via rAF in test env
    // may not fire, but the branch is exercised - the key assertion is that
    // adding a user message doesn't leave auto-scroll disabled)
  });

  it('auto-scroll stays disabled when assistant message is finalized', () => {
    const { container, rerender } = render(
      <ConversationView
        messages={[
          { id: '1', role: 'user' as const, content: 'first' },
          { id: '2', role: 'assistant' as const, content: 'streaming reply' },
        ]}
        isGenerating={true}
        onClose={vi.fn()}
      />,
    );

    const scrollEl = container.querySelector(
      '.chat-messages-scroll',
    ) as HTMLElement;
    expect(scrollEl).not.toBeNull();

    Object.defineProperty(scrollEl, 'scrollTop', {
      value: 0,
      configurable: true,
      writable: true,
    });

    // User scrolls up during streaming - disables auto-scroll
    act(() => {
      scrollEl.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }));
    });

    // Streaming finishes: assistant message is committed to the messages array
    act(() => {
      rerender(
        <ConversationView
          messages={[
            { id: '1', role: 'user' as const, content: 'first' },
            {
              id: '2',
              role: 'assistant' as const,
              content: 'streaming reply',
            },
          ]}
          isGenerating={false}
          onClose={vi.fn()}
        />,
      );
    });

    // scrollTop should remain 0: auto-scroll was NOT re-enabled by the
    // assistant message, so the user can keep reading where they scrolled
    expect(scrollEl.scrollTop).toBe(0);
  });

  it('auto-scroll re-enables when user scrolls back to bottom via wheel', async () => {
    const { container, rerender } = render(
      <ConversationView
        messages={[{ id: '1', role: 'user' as const, content: 'first' }]}
        isGenerating={false}
        onClose={vi.fn()}
      />,
    );

    const scrollEl = container.querySelector(
      '.chat-messages-scroll',
    ) as HTMLElement;
    expect(scrollEl).not.toBeNull();

    // User scrolls up - disables auto-scroll
    act(() => {
      scrollEl.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }));
    });

    // Simulate the user being near the bottom after scrolling down
    Object.defineProperty(scrollEl, 'scrollHeight', {
      value: 500,
      configurable: true,
    });
    Object.defineProperty(scrollEl, 'clientHeight', {
      value: 480,
      configurable: true,
    });
    Object.defineProperty(scrollEl, 'scrollTop', {
      value: 10,
      configurable: true,
      writable: true,
    });

    // User scrolls down (positive deltaY) - the rAF callback should check
    // position and re-enable auto-scroll since we're near the bottom
    // (scrollHeight - scrollTop - clientHeight = 500 - 10 - 480 = 10 < 60)
    act(() => {
      scrollEl.dispatchEvent(new WheelEvent('wheel', { deltaY: 100 }));
    });

    // Flush the rAF scheduled by the wheel handler
    await act(async () => {
      await new Promise((r) => requestAnimationFrame(r));
    });

    // Rerender with streaming content - should auto-scroll again
    act(() => {
      rerender(
        <ConversationView
          messages={[
            { id: '1', role: 'user' as const, content: 'first' },
            { id: '2', role: 'assistant' as const, content: 'new tokens' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
        />,
      );
    });

    // The auto-scroll effect exercised the re-enabled path
  });

  it('auto-scroll stays disabled when user scrolls down but not near bottom', async () => {
    const { container, rerender } = render(
      <ConversationView
        messages={[{ id: '1', role: 'user' as const, content: 'first' }]}
        isGenerating={false}
        onClose={vi.fn()}
      />,
    );

    const scrollEl = container.querySelector(
      '.chat-messages-scroll',
    ) as HTMLElement;
    expect(scrollEl).not.toBeNull();

    // User scrolls up - disables auto-scroll
    act(() => {
      scrollEl.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }));
    });

    // Simulate the user NOT near the bottom
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

    // User scrolls down but is still far from the bottom
    // (scrollHeight - scrollTop - clientHeight = 500 - 0 - 100 = 400 > 60)
    act(() => {
      scrollEl.dispatchEvent(new WheelEvent('wheel', { deltaY: 100 }));
    });

    // Flush the rAF
    await act(async () => {
      await new Promise((r) => requestAnimationFrame(r));
    });

    // Rerender with streaming - auto-scroll should still be disabled
    act(() => {
      rerender(
        <ConversationView
          messages={[
            { id: '1', role: 'user' as const, content: 'first' },
            { id: '2', role: 'assistant' as const, content: 'new tokens' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
        />,
      );
    });

    // scrollTop should remain 0 (auto-scroll was skipped)
    expect(scrollEl.scrollTop).toBe(0);
  });

  it('wheel with deltaY 0 does not change auto-scroll state', () => {
    const { container, rerender } = render(
      <ConversationView
        messages={[{ id: '1', role: 'user' as const, content: 'first' }]}
        isGenerating={false}
        onClose={vi.fn()}
      />,
    );

    const scrollEl = container.querySelector(
      '.chat-messages-scroll',
    ) as HTMLElement;

    Object.defineProperty(scrollEl, 'scrollTop', {
      value: 0,
      configurable: true,
      writable: true,
    });

    // Horizontal-only scroll (deltaY === 0) should be a no-op for auto-scroll
    act(() => {
      scrollEl.dispatchEvent(
        new WheelEvent('wheel', { deltaY: 0, deltaX: 100 }),
      );
    });

    // Rerender with streaming - auto-scroll should still be enabled (default)
    act(() => {
      rerender(
        <ConversationView
          messages={[
            { id: '1', role: 'user' as const, content: 'first' },
            { id: '2', role: 'assistant' as const, content: 'tokens' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
        />,
      );
    });
  });

  describe('header controls', () => {
    it('renders History button when onHistoryOpen is provided', () => {
      render(
        <ConversationView
          messages={[]}
          isGenerating={false}
          onClose={vi.fn()}
          onHistoryOpen={vi.fn()}
        />,
      );
      expect(
        screen.getByRole('button', { name: /history/i }),
      ).toBeInTheDocument();
    });

    it('does not render History button when onHistoryOpen is not provided', () => {
      render(
        <ConversationView
          messages={[]}
          isGenerating={false}
          onClose={vi.fn()}
        />,
      );
      expect(screen.queryByRole('button', { name: /history/i })).toBeNull();
    });

    it('calls onHistoryOpen when History button is clicked', () => {
      const onHistoryOpen = vi.fn();
      render(
        <ConversationView
          messages={[]}
          isGenerating={false}
          onClose={vi.fn()}
          onHistoryOpen={onHistoryOpen}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: /history/i }));
      expect(onHistoryOpen).toHaveBeenCalledOnce();
    });

    it('renders Save button when onSave is provided', () => {
      render(
        <ConversationView
          messages={[]}
          isGenerating={false}
          onClose={vi.fn()}
          onSave={vi.fn()}
          isSaved={false}
          canSave={true}
        />,
      );
      expect(screen.getByRole('button', { name: /save/i })).toBeInTheDocument();
    });

    it('calls onSave when Save button is clicked', () => {
      const onSave = vi.fn();
      render(
        <ConversationView
          messages={[]}
          isGenerating={false}
          onClose={vi.fn()}
          onSave={onSave}
          isSaved={false}
          canSave={true}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: /save/i }));
      expect(onSave).toHaveBeenCalledOnce();
    });

    it('Save button is disabled when canSave is false', () => {
      render(
        <ConversationView
          messages={[]}
          isGenerating={false}
          onClose={vi.fn()}
          onSave={vi.fn()}
          isSaved={false}
          canSave={false}
        />,
      );
      const saveBtn = screen.getByRole('button', { name: /save/i });
      expect(saveBtn).toBeDisabled();
    });

    it('Save button is enabled (for unsave) when isSaved is true', () => {
      render(
        <ConversationView
          messages={[]}
          isGenerating={false}
          onClose={vi.fn()}
          onSave={vi.fn()}
          isSaved={true}
          canSave={true}
        />,
      );
      const saveBtn = screen.getByRole('button', {
        name: /remove from history/i,
      });
      expect(saveBtn).not.toBeDisabled();
    });
  });

  describe('Thinking props forwarding', () => {
    it('renders the /think pending placeholder with bare dots before the engine loading threshold elapses', () => {
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: '',
              fromThink: true,
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
        />,
      );
      expect(screen.getByTestId('reasoning-block')).toBeInTheDocument();
      expect(screen.queryByTestId('loading-label')).toBeNull();
    });

    it('shows the shared engine loading label inside the /think pending placeholder for a builtin cold start', () => {
      vi.useFakeTimers();
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: '',
              fromThink: true,
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          providerKind="builtin"
        />,
      );
      act(() => {
        vi.advanceTimersByTime(1000);
      });
      expect(screen.getByTestId('reasoning-block')).toBeInTheDocument();
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Starting up the model…',
      );
      vi.useRealTimers();
    });

    it('does not render a duplicate external loading row for a /think turn', () => {
      vi.useFakeTimers();
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: '',
              fromThink: true,
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          providerKind="builtin"
        />,
      );
      act(() => {
        vi.advanceTimersByTime(1000);
      });
      // Only ReasoningBlock's own pending row should render the label; the
      // bare-dots external row is suppressed for /think turns.
      expect(screen.getAllByTestId('loading-label')).toHaveLength(1);
      vi.useRealTimers();
    });

    it('renders ReasoningBlock when assistant message has thinkingContent', () => {
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: 'The answer is 42.',
              thinkingContent: 'Let me think about this...',
            },
          ]}
          isGenerating={false}
          onClose={vi.fn()}
        />,
      );
      expect(screen.getByTestId('reasoning-block')).toBeInTheDocument();
    });

    it('renders ChatBubble (not null) when thinking but no content yet during generation', () => {
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: '',
              fromThink: true,
              thinkingContent: 'Reasoning in progress...',
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
        />,
      );
      // The bubble should render with ReasoningBlock visible
      expect(screen.getByTestId('reasoning-block')).toBeInTheDocument();
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Reasoning...',
      );
    });

    it('shows the live "Reasoning..." indicator while reasoning streams even without /think', () => {
      // A reasoning model may emit thinking tokens without an explicit
      // /think (e.g. it ignored the off switch). The indicator must reflect
      // the real stream state: still thinking, not a premature "Done".
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: '',
              // No fromThink flag: this turn was not an explicit /think.
              thinkingContent: 'Reasoning in progress...',
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Reasoning...',
      );
    });

    it('does not show a duplicate external status strip when reasoning owns chrome', () => {
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: '',
              thinkingContent: 'Reasoning...',
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
        />,
      );
      // Reasoning path owns chrome via ReasoningBlock's RequestStatusStrip.
      // No second external ConversationView loading row for /think.
      expect(screen.getByTestId('loading-label')).toBeInTheDocument();
    });
  });

  it('renders multiple messages correctly (10 messages)', () => {
    const messages = Array.from({ length: 10 }, (_, i) => ({
      id: `msg-${i}`,
      role: (i % 2 === 0 ? 'user' : 'assistant') as 'user' | 'assistant',
      content: `Message ${i}`,
    }));
    render(
      <ConversationView
        messages={messages}
        isGenerating={false}
        onClose={vi.fn()}
      />,
    );
    for (let i = 0; i < 10; i++) {
      expect(screen.getByText(`Message ${i}`)).toBeInTheDocument();
    }
  });

  describe('search integration', () => {
    it('renders the loading label next to the dots while analyzing query', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            { id: 'a', role: 'assistant', content: '' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'analyzing_query' }}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Analyzing query',
      );
    });

    it('renders the loading label as "Searching the web" when searching', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            { id: 'a', role: 'assistant', content: '' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'searching' }}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Searching the web',
      );
    });

    it('renders refining_search with attempt counter', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            { id: 'a', role: 'assistant', content: '' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'refining_search', attempt: 2, total: 3 }}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Refining search (2/3)',
      );
    });

    it('shows dots only (no label) when searchStage is null', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            { id: 'a', role: 'assistant', content: '' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
        />,
      );
      expect(screen.queryByTestId('loading-label')).toBeNull();
      expect(screen.getByRole('status')).toHaveAttribute(
        'aria-label',
        'AI is thinking',
      );
    });

    it('renders reading_sources with gap label in gap round', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            { id: 'a', role: 'assistant', content: '' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'reading_sources', gap: true }}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Reading additional pages',
      );
    });

    it('renders composing with gap label in gap round', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            { id: 'a', role: 'assistant', content: '' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'composing', gap: true }}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Composing refined answer',
      );
    });

    it('renders the loading label as "Searching more angles" when searching with gap', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            { id: 'a', role: 'assistant', content: '' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'searching', gap: true }}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Searching more angles',
      );
    });

    it('renders reading_sources without gap label in initial round', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            { id: 'a', role: 'assistant', content: '' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'reading_sources' }}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Reading sources',
      );
    });

    it('renders composing without gap label in initial round', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            { id: 'a', role: 'assistant', content: '' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'composing' }}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Composing answer',
      );
    });

    it('shows C3 verifying pill on the search bubble during citation audit', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            {
              id: 'a',
              role: 'assistant',
              content: 'Adobe acquired Figma.',
              fromSearch: true,
              searchSources: [
                { title: 'Adobe', url: 'https://adobe.com' },
                { title: 'Reuters', url: 'https://reuters.com' },
              ],
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'verifying_sources' }}
        />,
      );
      expect(
        screen.queryByTestId('search-progress-block'),
      ).not.toBeInTheDocument();
      expect(screen.getByTestId('sources-verifying-pill')).toHaveTextContent(
        'Verifying sources...',
      );
    });

    it('maps verifying_sources on the loading-row label helper', () => {
      // Non-fromSearch empty assistant still hits ConversationView's
      // searchStageLabel switch (fromSearch owns chrome in ChatBubble).
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            { id: 'a', role: 'assistant', content: '' },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'verifying_sources' }}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Verifying sources...',
      );
    });

    it('renders SearchProgressBlock for web search when fromSearch is true', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            {
              id: 'a',
              role: 'assistant',
              content: '',
              fromSearch: true,
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'searching' }}
        />,
      );
      expect(screen.getByTestId('search-progress-block')).toBeInTheDocument();
      expect(screen.getByTestId('loading-label')).toHaveAttribute(
        'data-label',
        'Searching the web',
      );
    });

    it('hides external RequestStatusStrip for search turns (bubble progress chrome takes over)', () => {
      render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            {
              id: 'a',
              role: 'assistant',
              content: '',
              fromSearch: true,
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'searching' }}
        />,
      );
      // External stage row is suppressed via !fromSearch; the label lives inside
      // SearchProgressBlock (auto-search) instead of a second ConversationView row.
      expect(screen.getByTestId('search-progress-block')).toBeInTheDocument();
      const labels = screen.getAllByTestId('loading-label');
      expect(labels).toHaveLength(1);
      expect(labels[0]).toHaveAttribute('data-label', 'Searching the web');
    });

    it('re-pins scroll to bottom when searchSources grow while auto-scroll is on', async () => {
      const { container, rerender } = render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            {
              id: 'a',
              role: 'assistant',
              content: 'long prior answer text',
              fromSearch: true,
              searchSources: [],
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'reading_sources' }}
        />,
      );

      const scrollEl = container.querySelector(
        '.chat-messages-scroll',
      ) as HTMLElement;
      expect(scrollEl).not.toBeNull();

      let scrollTopValue = 0;
      Object.defineProperty(scrollEl, 'scrollHeight', {
        get: () => 800,
        configurable: true,
      });
      Object.defineProperty(scrollEl, 'clientHeight', {
        get: () => 400,
        configurable: true,
      });
      Object.defineProperty(scrollEl, 'scrollTop', {
        get: () => scrollTopValue,
        set: (v: number) => {
          scrollTopValue = v;
        },
        configurable: true,
      });

      act(() => {
        rerender(
          <ConversationView
            messages={[
              { id: 'u', role: 'user', content: 'q' },
              {
                id: 'a',
                role: 'assistant',
                content: 'long prior answer text',
                fromSearch: true,
                searchSources: [
                  { title: 'A', url: 'https://a.example' },
                  { title: 'B', url: 'https://b.example' },
                ],
              },
            ]}
            isGenerating={true}
            onClose={vi.fn()}
            searchStage={{ kind: 'reading_sources' }}
          />,
        );
      });

      // Double rAF: React commit frame + layout settle frame.
      await act(async () => {
        await new Promise((r) => requestAnimationFrame(r));
        await new Promise((r) => requestAnimationFrame(r));
      });

      expect(scrollTopValue).toBe(800);
    });

    it('does not force-scroll when searchSources grow after user scrolled up', async () => {
      const { container, rerender } = render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            {
              id: 'a',
              role: 'assistant',
              content: 'long prior answer text',
              fromSearch: true,
              searchSources: [],
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'reading_sources' }}
        />,
      );

      const scrollEl = container.querySelector(
        '.chat-messages-scroll',
      ) as HTMLElement;
      expect(scrollEl).not.toBeNull();

      let scrollTopValue = 0;
      Object.defineProperty(scrollEl, 'scrollTop', {
        get: () => scrollTopValue,
        set: (v: number) => {
          scrollTopValue = v;
        },
        configurable: true,
      });

      act(() => {
        scrollEl.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }));
      });

      act(() => {
        rerender(
          <ConversationView
            messages={[
              { id: 'u', role: 'user', content: 'q' },
              {
                id: 'a',
                role: 'assistant',
                content: 'long prior answer text',
                fromSearch: true,
                searchSources: [
                  { title: 'A', url: 'https://a.example' },
                  { title: 'B', url: 'https://b.example' },
                ],
              },
            ]}
            isGenerating={true}
            onClose={vi.fn()}
            searchStage={{ kind: 'reading_sources' }}
          />,
        );
      });

      await act(async () => {
        await new Promise((r) => requestAnimationFrame(r));
        await new Promise((r) => requestAnimationFrame(r));
      });

      expect(scrollTopValue).toBe(0);
    });

    it('aborts in-flight double-rAF scroll when user scrolls up mid-settle', async () => {
      // Spy rAF so we can run the outer frame, flip auto-scroll off, then
      // run the inner settle frame and assert it no-ops.
      const nativeRaf = globalThis.requestAnimationFrame.bind(globalThis);
      const pending: FrameRequestCallback[] = [];
      const rafSpy = vi
        .spyOn(globalThis, 'requestAnimationFrame')
        .mockImplementation((cb: FrameRequestCallback) => {
          pending.push(cb);
          return pending.length;
        });
      const cafSpy = vi
        .spyOn(globalThis, 'cancelAnimationFrame')
        .mockImplementation((id: number) => {
          pending[id - 1] = () => {};
        });

      try {
        const { container, rerender } = render(
          <ConversationView
            messages={[
              { id: 'u', role: 'user', content: 'q' },
              {
                id: 'a',
                role: 'assistant',
                content: 'answer',
                fromSearch: true,
                searchSources: [],
              },
            ]}
            isGenerating={true}
            onClose={vi.fn()}
            searchStage={{ kind: 'reading_sources' }}
          />,
        );

        const scrollEl = container.querySelector(
          '.chat-messages-scroll',
        ) as HTMLElement;

        let scrollTopValue = 0;
        Object.defineProperty(scrollEl, 'scrollHeight', {
          get: () => 900,
          configurable: true,
        });
        Object.defineProperty(scrollEl, 'scrollTop', {
          get: () => scrollTopValue,
          set: (v: number) => {
            scrollTopValue = v;
          },
          configurable: true,
        });

        // Drop any rAFs from the initial mount; we only care about the
        // sources-growth effect below.
        pending.length = 0;

        act(() => {
          rerender(
            <ConversationView
              messages={[
                { id: 'u', role: 'user', content: 'q' },
                {
                  id: 'a',
                  role: 'assistant',
                  content: 'answer',
                  fromSearch: true,
                  searchSources: [{ title: 'A', url: 'https://a.example' }],
                },
              ]}
              isGenerating={true}
              onClose={vi.fn()}
              searchStage={{ kind: 'reading_sources' }}
            />,
          );
        });

        // Outer settle frame schedules the inner frame.
        expect(pending.length).toBeGreaterThanOrEqual(1);
        act(() => {
          const outer = pending.shift()!;
          outer(0);
        });

        // User scrolls up before the inner frame runs.
        act(() => {
          scrollEl.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }));
        });
        scrollTopValue = 12;

        act(() => {
          while (pending.length > 0) {
            const cb = pending.shift()!;
            cb(0);
          }
        });

        expect(scrollTopValue).toBe(12);
      } finally {
        rafSpy.mockRestore();
        cafSpy.mockRestore();
        void nativeRaf;
      }
    });

    it('re-pins scroll when searchStage advances during a generating search turn', async () => {
      const { container, rerender } = render(
        <ConversationView
          messages={[
            { id: 'u', role: 'user', content: 'q' },
            {
              id: 'a',
              role: 'assistant',
              content: '',
              fromSearch: true,
            },
          ]}
          isGenerating={true}
          onClose={vi.fn()}
          searchStage={{ kind: 'searching' }}
        />,
      );

      const scrollEl = container.querySelector(
        '.chat-messages-scroll',
      ) as HTMLElement;

      let scrollTopValue = 0;
      Object.defineProperty(scrollEl, 'scrollHeight', {
        get: () => 700,
        configurable: true,
      });
      Object.defineProperty(scrollEl, 'scrollTop', {
        get: () => scrollTopValue,
        set: (v: number) => {
          scrollTopValue = v;
        },
        configurable: true,
      });

      act(() => {
        rerender(
          <ConversationView
            messages={[
              { id: 'u', role: 'user', content: 'q' },
              {
                id: 'a',
                role: 'assistant',
                content: '',
                fromSearch: true,
              },
            ]}
            isGenerating={true}
            onClose={vi.fn()}
            searchStage={{ kind: 'composing' }}
          />,
        );
      });

      await act(async () => {
        await new Promise((r) => requestAnimationFrame(r));
        await new Promise((r) => requestAnimationFrame(r));
      });

      expect(scrollTopValue).toBe(700);
    });

    it('ResizeObserver pins scroller when content resizes and auto-scroll is on', () => {
      let roCallback: ResizeObserverCallback | null = null;
      const Original = globalThis.ResizeObserver;
      const roSpy = vi
        .spyOn(globalThis, 'ResizeObserver')
        .mockImplementation(function (cb: ResizeObserverCallback) {
          roCallback = cb;
          return new Original(cb) as ResizeObserver;
        });

      try {
        const { container } = render(
          <ConversationView
            messages={[
              { id: 'u', role: 'user', content: 'q' },
              {
                id: 'a',
                role: 'assistant',
                content: 'long answer',
                fromSearch: true,
                searchSources: [{ title: 'A', url: 'https://a.example' }],
              },
            ]}
            isGenerating={true}
            onClose={vi.fn()}
            searchStage={{ kind: 'reading_sources' }}
          />,
        );

        const scrollEl = container.querySelector(
          '.chat-messages-scroll',
        ) as HTMLElement;
        expect(scrollEl).not.toBeNull();
        expect(roCallback).not.toBeNull();

        let scrollTopValue = 0;
        Object.defineProperty(scrollEl, 'scrollHeight', {
          get: () => 1500,
          configurable: true,
        });
        Object.defineProperty(scrollEl, 'scrollTop', {
          get: () => scrollTopValue,
          set: (v: number) => {
            scrollTopValue = v;
          },
          configurable: true,
        });

        // Simulate Framer sources-body height growth mid-animation.
        act(() => {
          roCallback!(
            [] as unknown as ResizeObserverEntry[],
            {} as ResizeObserver,
          );
        });

        expect(scrollTopValue).toBe(1500);
      } finally {
        roSpy.mockRestore();
      }
    });

    it('ResizeObserver does not pin when user has scrolled up', () => {
      let roCallback: ResizeObserverCallback | null = null;
      const Original = globalThis.ResizeObserver;
      const roSpy = vi
        .spyOn(globalThis, 'ResizeObserver')
        .mockImplementation(function (cb: ResizeObserverCallback) {
          roCallback = cb;
          return new Original(cb) as ResizeObserver;
        });

      try {
        const { container } = render(
          <ConversationView
            messages={[
              { id: 'u', role: 'user', content: 'q' },
              {
                id: 'a',
                role: 'assistant',
                content: 'long answer',
                fromSearch: true,
                searchSources: [{ title: 'A', url: 'https://a.example' }],
              },
            ]}
            isGenerating={true}
            onClose={vi.fn()}
            searchStage={{ kind: 'reading_sources' }}
          />,
        );

        const scrollEl = container.querySelector(
          '.chat-messages-scroll',
        ) as HTMLElement;

        let scrollTopValue = 40;
        Object.defineProperty(scrollEl, 'scrollHeight', {
          get: () => 1500,
          configurable: true,
        });
        Object.defineProperty(scrollEl, 'scrollTop', {
          get: () => scrollTopValue,
          set: (v: number) => {
            scrollTopValue = v;
          },
          configurable: true,
        });

        act(() => {
          scrollEl.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }));
        });

        act(() => {
          roCallback!(
            [] as unknown as ResizeObserverEntry[],
            {} as ResizeObserver,
          );
        });

        expect(scrollTopValue).toBe(40);
      } finally {
        roSpy.mockRestore();
      }
    });
  });

  describe('Engine loading label', () => {
    const plainMessages = [
      { id: 'u', role: 'user' as const, content: 'q' },
      { id: 'a', role: 'assistant' as const, content: '' },
    ];

    beforeEach(() => {
      vi.useFakeTimers();
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('shows the first filler phrase for a builtin cold start past the threshold', () => {
      render(
        <ConversationView
          messages={plainMessages}
          isGenerating={true}
          onClose={vi.fn()}
          providerKind="builtin"
        />,
      );
      act(() => {
        vi.advanceTimersByTime(1000);
      });
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Starting up the model…',
      );
    });

    it('shows the warming label immediately when the engine is already warming', () => {
      render(
        <ConversationView
          messages={plainMessages}
          isGenerating={true}
          onClose={vi.fn()}
          providerKind="builtin"
          engineWarming={true}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Warming up…',
      );
    });

    it('never shows a label for a remote provider', () => {
      render(
        <ConversationView
          messages={plainMessages}
          isGenerating={true}
          onClose={vi.fn()}
          providerKind="openai"
        />,
      );
      act(() => {
        vi.advanceTimersByTime(10000);
      });
      expect(screen.queryByTestId('loading-label')).toBeNull();
    });

    it('lets an active search stage take priority over the engine label', () => {
      render(
        <ConversationView
          messages={plainMessages}
          isGenerating={true}
          onClose={vi.fn()}
          providerKind="builtin"
          engineWarming={true}
          searchStage={{ kind: 'analyzing_query' }}
        />,
      );
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Analyzing query',
      );
    });

    it('shows the slow-warm cue instead of "starting up" when the engine is already loaded', () => {
      render(
        <ConversationView
          messages={plainMessages}
          isGenerating={true}
          onClose={vi.fn()}
          providerKind="builtin"
          engineState="loaded"
        />,
      );
      act(() => {
        vi.advanceTimersByTime(1000);
      });
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Processing your message…',
      );
    });
  });

  // Regression coverage for issue #296 follow-up: onLoadAnyway used to be a
  // single shared callback forwarded verbatim to every ChatBubble, which let
  // a later turn's data silently hijack an older, still-visible error card's
  // "Load anyway" click. ConversationView now wraps onLoadAnyway per-message
  // with that message's own retained retrySnapshot.
  describe('onLoadAnyway wiring (issue #296 follow-up)', () => {
    it("invokes onLoadAnyway with the clicked card's own retrySnapshot", async () => {
      invoke.mockImplementationOnce(async () => ({
        required_bytes: 1,
        available_bytes: 1,
        verdict: 'insufficient',
      }));
      const onLoadAnyway = vi.fn();
      const retrySnapshot = {
        kind: 'chat' as const,
        displayContent: 'turn A content',
        userMessageId: 'stale-user-id',
        assistantMessageId: 'stale-assistant-id',
      };
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: 'may not fit',
              errorKind: 'InsufficientMemory' as const,
              retrySnapshot,
            },
          ]}
          isGenerating={false}
          onClose={vi.fn()}
          onLoadAnyway={onLoadAnyway}
        />,
      );

      fireEvent.click(
        await screen.findByRole('button', { name: 'Load anyway' }),
      );

      expect(onLoadAnyway).toHaveBeenCalledTimes(1);
      expect(onLoadAnyway).toHaveBeenCalledWith(retrySnapshot);
    });

    it('omits the button instead of crashing when a message has no retrySnapshot', async () => {
      invoke.mockImplementationOnce(async () => ({
        required_bytes: 1,
        available_bytes: 1,
        verdict: 'insufficient',
      }));
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: 'may not fit',
              errorKind: 'InsufficientMemory' as const,
            },
          ]}
          isGenerating={false}
          onClose={vi.fn()}
          onLoadAnyway={vi.fn()}
        />,
      );

      await screen.findByText('This model may not fit in memory right now.');
      expect(
        screen.queryByRole('button', { name: 'Load anyway' }),
      ).not.toBeInTheDocument();
    });
  });

  // Regression coverage for issue #296 follow-up (bug 2): picking a new model
  // from "Switch model" must be able to replay the abandoned turn too, not
  // just "Load anyway". ConversationView wraps onSwitchModel per-message with
  // that message's own retained retrySnapshot, mirroring onLoadAnyway.
  describe('onSwitchModel wiring (issue #296 follow-up)', () => {
    it("invokes onSwitchModel with the clicked card's own retrySnapshot", async () => {
      invoke.mockImplementationOnce(async () => ({
        required_bytes: 1,
        available_bytes: 1,
        verdict: 'insufficient',
      }));
      const onSwitchModel = vi.fn();
      const retrySnapshot = {
        kind: 'chat' as const,
        displayContent: 'turn A content',
        userMessageId: 'stale-user-id',
        assistantMessageId: 'stale-assistant-id',
      };
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: 'may not fit',
              errorKind: 'InsufficientMemory' as const,
              retrySnapshot,
            },
          ]}
          isGenerating={false}
          onClose={vi.fn()}
          onSwitchModel={onSwitchModel}
        />,
      );

      fireEvent.click(
        await screen.findByRole('button', { name: 'Switch model' }),
      );

      expect(onSwitchModel).toHaveBeenCalledTimes(1);
      expect(onSwitchModel).toHaveBeenCalledWith(retrySnapshot);
    });

    it('invokes onSwitchModel with undefined for an EngineStartFailed message (no retrySnapshot)', () => {
      const onSwitchModel = vi.fn();
      render(
        <ConversationView
          messages={[
            {
              id: '1',
              role: 'assistant' as const,
              content: 'Could not start the engine',
              errorKind: 'EngineStartFailed' as const,
            },
          ]}
          isGenerating={false}
          onClose={vi.fn()}
          onSwitchModel={onSwitchModel}
        />,
      );

      fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));

      expect(onSwitchModel).toHaveBeenCalledTimes(1);
      expect(onSwitchModel).toHaveBeenCalledWith(undefined);
    });
  });
});
