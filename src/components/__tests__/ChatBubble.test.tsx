import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { ChatBubble } from '../ChatBubble';
import { invoke } from '../../testUtils/mocks/tauri';
beforeEach(() => {
  invoke.mockClear();
});

function openSources(container: HTMLElement) {
  const trigger = container.querySelector(
    'button.sources-trigger',
  ) as HTMLButtonElement;
  fireEvent.click(trigger);
}

describe('ChatBubble', () => {
  describe('User messages', () => {
    it('renders user message content as plain text', () => {
      render(<ChatBubble role="user" content="Hello there" index={0} />);
      expect(screen.getByText('Hello there')).toBeInTheDocument();
    });

    it('applies user styling (chat-bubble-user class)', () => {
      const { container } = render(
        <ChatBubble role="user" content="Hi" index={0} />,
      );
      expect(container.querySelector('.chat-bubble-user')).not.toBeNull();
    });

    it('does not render markdown for user messages (** shows as text, no <strong>)', () => {
      const { container } = render(
        <ChatBubble role="user" content="**bold**" index={0} />,
      );
      // User content is rendered as plain text inside a <span>, not parsed as markdown
      expect(container.querySelector('strong')).toBeNull();
      expect(screen.getByText('**bold**')).toBeInTheDocument();
    });

    it('shows copy button for user messages', () => {
      render(<ChatBubble role="user" content="copy me" index={0} />);
      expect(
        screen.getByRole('button', { name: 'Copy message' }),
      ).toBeInTheDocument();
    });

    it('right-aligns user messages (justify-end class)', () => {
      const { container } = render(
        <ChatBubble role="user" content="Hi" index={0} />,
      );
      // The outer motion.div wrapper carries justify-end for user messages
      const outerDiv = container.firstElementChild;
      expect(outerDiv?.classList.contains('justify-end')).toBe(true);
    });
  });

  describe('Assistant messages', () => {
    it('renders assistant content via MarkdownRenderer (** becomes bold)', () => {
      const { container } = render(
        <ChatBubble role="assistant" content="**bold**" index={0} />,
      );
      // Streamdown renders bold as span with data-streamdown="strong"
      const bold = container.querySelector('[data-streamdown="strong"]');
      expect(bold).not.toBeNull();
      expect(bold!.textContent).toBe('bold');
    });

    it('renders as plain text without a bubble wrapper (no chat-bubble-ai class)', () => {
      const { container } = render(
        <ChatBubble role="assistant" content="Hello" index={0} />,
      );
      expect(container.querySelector('.chat-bubble-ai')).toBeNull();
    });

    it('is not width-constrained (no max-w-[80%] on wrapper)', () => {
      const { container } = render(
        <ChatBubble role="assistant" content="Hello" index={0} />,
      );
      // AI messages span full width - no max-width cap like user bubbles
      expect(container.querySelector('.group')).toBeNull();
    });

    it('shows copy button for assistant messages', () => {
      render(<ChatBubble role="assistant" content="response" index={0} />);
      expect(
        screen.getByRole('button', { name: 'Copy message' }),
      ).toBeInTheDocument();
    });

    it('renders the Replace button when onReplace is provided', () => {
      render(
        <ChatBubble
          role="assistant"
          content="rewritten"
          index={0}
          onReplace={vi.fn().mockResolvedValue(true)}
        />,
      );
      expect(
        screen.getByRole('button', {
          name: 'Replace selection in source app',
        }),
      ).toBeInTheDocument();
    });

    it('omits the Replace button when onReplace is absent', () => {
      render(<ChatBubble role="assistant" content="response" index={0} />);
      expect(
        screen.queryByRole('button', {
          name: 'Replace selection in source app',
        }),
      ).toBeNull();
    });

    it('left-aligns assistant messages (justify-start class)', () => {
      const { container } = render(
        <ChatBubble role="assistant" content="Hi" index={0} />,
      );
      const outerDiv = container.firstElementChild;
      expect(outerDiv?.classList.contains('justify-start')).toBe(true);
    });
  });

  describe('Quoted text', () => {
    it('renders quote block when quotedText is provided for user messages', () => {
      const { container } = render(
        <ChatBubble
          role="user"
          content="explain this"
          index={0}
          quotedText="some code"
        />,
      );
      const quote = container.querySelector('.border-l-2');
      expect(quote).not.toBeNull();
      expect(quote?.textContent).toContain('some code');
    });

    it('does not render quote block when quotedText is not provided', () => {
      const { container } = render(
        <ChatBubble role="user" content="hello" index={0} />,
      );
      expect(container.querySelector('.border-l-2')).toBeNull();
    });

    it('does not render quote block for assistant messages even if quotedText is passed', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="response"
          index={0}
          quotedText="ignored"
        />,
      );
      expect(container.querySelector('.border-l-2')).toBeNull();
    });

    it('preserves line breaks in quoted text via whitespace-pre-wrap', () => {
      const { container } = render(
        <ChatBubble
          role="user"
          content="explain"
          index={0}
          quotedText="line one\nline two"
        />,
      );
      const quote = container.querySelector('.whitespace-pre-wrap');
      expect(quote).not.toBeNull();
    });
  });

  describe('Image attachments', () => {
    it('renders ImageThumbnails when imagePaths and onImagePreview are provided', () => {
      render(
        <ChatBubble
          role="user"
          content="look at this"
          index={0}
          imagePaths={['/tmp/img1.jpg', '/tmp/img2.jpg']}
          onImagePreview={vi.fn()}
        />,
      );
      expect(
        screen.getByRole('list', { name: /attached images/i }),
      ).toBeInTheDocument();
      expect(screen.getAllByRole('listitem')).toHaveLength(2);
    });

    it('does not render ImageThumbnails when imagePaths is not provided', () => {
      render(
        <ChatBubble
          role="user"
          content="no images"
          index={0}
          onImagePreview={vi.fn()}
        />,
      );
      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('does not render ImageThumbnails when imagePaths is empty', () => {
      render(
        <ChatBubble
          role="user"
          content="empty images"
          index={0}
          imagePaths={[]}
          onImagePreview={vi.fn()}
        />,
      );
      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('does not render ImageThumbnails when onImagePreview is not provided', () => {
      render(
        <ChatBubble
          role="user"
          content="no preview handler"
          index={0}
          imagePaths={['/tmp/img1.jpg']}
        />,
      );
      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('renders images but no text span when content is empty', () => {
      const { container } = render(
        <ChatBubble
          role="user"
          content=""
          index={0}
          imagePaths={['/tmp/img1.jpg']}
          onImagePreview={vi.fn()}
        />,
      );
      expect(
        screen.getByRole('list', { name: /attached images/i }),
      ).toBeInTheDocument();
      // The content span should not be rendered when content is empty
      expect(container.querySelector('.text-white\\/95')).toBeNull();
      // CopyButton should also be hidden - nothing to copy
      expect(screen.queryByRole('button', { name: /copy/i })).toBeNull();
    });
  });

  describe('User message text formatting', () => {
    it('preserves newlines in user message content via whitespace-pre-wrap', () => {
      const { container } = render(
        <ChatBubble
          role="user"
          content={'line one\nline two\nline three'}
          index={0}
        />,
      );
      const contentSpan = container.querySelector('.text-white\\/95');
      expect(contentSpan).not.toBeNull();
      expect(contentSpan?.classList.contains('whitespace-pre-wrap')).toBe(true);
    });

    it('preserves indentation in user message content via whitespace-pre-wrap', () => {
      const { container } = render(
        <ChatBubble
          role="user"
          content={'  indented\n    more indented'}
          index={0}
        />,
      );
      const contentSpan = container.querySelector('.text-white\\/95');
      expect(contentSpan?.classList.contains('whitespace-pre-wrap')).toBe(true);
    });
  });

  describe('Slash command styling', () => {
    it('styles a leading /screen command', () => {
      const { container } = render(
        <ChatBubble role="user" content="/screen explain this" index={0} />,
      );
      const styled = container.querySelector(
        '.font-semibold.text-\\[\\#7C2D12\\]',
      );
      expect(styled).not.toBeNull();
      expect(styled!.textContent).toBe('/screen');
    });

    it('styles a leading /think command', () => {
      const { container } = render(
        <ChatBubble
          role="user"
          content="/think why is the sky blue?"
          index={0}
        />,
      );
      const styled = container.querySelector(
        '.font-semibold.text-\\[\\#7C2D12\\]',
      );
      expect(styled).not.toBeNull();
      expect(styled!.textContent).toBe('/think');
    });

    it('styles multiple commands anywhere in the text', () => {
      const { container } = render(
        <ChatBubble
          role="user"
          content="/screen /think explain this"
          index={0}
        />,
      );
      const styled = container.querySelectorAll(
        '.font-semibold.text-\\[\\#7C2D12\\]',
      );
      expect(styled).toHaveLength(2);
      expect(styled[0].textContent).toBe('/screen');
      expect(styled[1].textContent).toBe('/think');
    });

    it('styles a command in the middle of text', () => {
      const { container } = render(
        <ChatBubble role="user" content="please /think about this" index={0} />,
      );
      const styled = container.querySelector(
        '.font-semibold.text-\\[\\#7C2D12\\]',
      );
      expect(styled).not.toBeNull();
      expect(styled!.textContent).toBe('/think');
    });

    it('does not style partial matches like /screensaver', () => {
      const { container } = render(
        <ChatBubble role="user" content="/screensaver is nice" index={0} />,
      );
      const styled = container.querySelector(
        '.font-semibold.text-\\[\\#7C2D12\\]',
      );
      expect(styled).toBeNull();
    });

    it('renders plain text when no commands are present', () => {
      render(
        <ChatBubble role="user" content="just a normal message" index={0} />,
      );
      expect(screen.getByText('just a normal message')).toBeInTheDocument();
    });

    it('handles a command at the end of text', () => {
      const { container } = render(
        <ChatBubble role="user" content="do /think" index={0} />,
      );
      const styled = container.querySelector(
        '.font-semibold.text-\\[\\#7C2D12\\]',
      );
      expect(styled).not.toBeNull();
      expect(styled!.textContent).toBe('/think');
    });
  });

  describe('Layout', () => {
    it('has max-width constraint (max-w-[80%])', () => {
      const { container } = render(
        <ChatBubble role="user" content="test" index={0} />,
      );
      expect(container.querySelector('.max-w-\\[80\\%\\]')).not.toBeNull();
    });
  });

  describe('ReasoningBlock rendering', () => {
    it('renders ReasoningBlock for AI message with thinkingContent', () => {
      render(
        <ChatBubble
          role="assistant"
          content="The answer is 42."
          index={0}
          thinkingContent="Let me reason about this..."
        />,
      );
      expect(screen.getByTestId('reasoning-block')).toBeInTheDocument();
    });

    it('does not render ReasoningBlock for AI message without thinkingContent', () => {
      render(<ChatBubble role="assistant" content="Hello" index={0} />);
      expect(screen.queryByTestId('reasoning-block')).toBeNull();
    });

    it('renders the pending /think placeholder before thinking tokens arrive', () => {
      render(
        <ChatBubble
          role="assistant"
          content=""
          index={0}
          isThinkingPending={true}
          pendingLabel="Starting up the model…"
        />,
      );
      expect(screen.getByTestId('reasoning-block')).toBeInTheDocument();
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Starting up the model…',
      );
    });

    it('renders bare dots for the pending /think placeholder before the engine label threshold elapses', () => {
      render(
        <ChatBubble
          role="assistant"
          content=""
          index={0}
          isThinkingPending={true}
        />,
      );
      expect(screen.getByTestId('reasoning-block')).toBeInTheDocument();
      expect(screen.queryByTestId('loading-label')).toBeNull();
    });

    it('does not render ReasoningBlock for user message even with thinkingContent', () => {
      render(
        <ChatBubble
          role="user"
          content="Hello"
          index={0}
          thinkingContent="Should not appear"
        />,
      );
      expect(screen.queryByTestId('reasoning-block')).toBeNull();
    });

    it('shows "Reasoning..." state when isThinking is true', () => {
      render(
        <ChatBubble
          role="assistant"
          content=""
          index={0}
          thinkingContent="Reasoning in progress..."
          isThinking={true}
        />,
      );
      expect(screen.getByTestId('reasoning-block')).toBeInTheDocument();
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Reasoning...',
      );
      expect(screen.getByTestId('loading-label-prefix')).toBeInTheDocument();
    });
  });

  describe('Error messages (errorKind)', () => {
    it('renders ErrorCard instead of MarkdownRenderer when errorKind is set', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content={"Ollama isn't running\nStart Ollama and try again."}
          index={0}
          errorKind="EngineUnreachable"
        />,
      );
      expect(container.querySelector('[data-error-bar]')).not.toBeNull();
    });

    it('does not render MarkdownRenderer when errorKind is set', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content={"Ollama isn't running\nStart Ollama and try again."}
          index={0}
          errorKind="EngineUnreachable"
        />,
      );
      // MarkdownRenderer would produce a <p> or streamdown elements; ErrorCard does not
      expect(container.querySelector('[data-streamdown]')).toBeNull();
    });

    it('renders MarkdownRenderer when errorKind is absent', () => {
      const { container } = render(
        <ChatBubble role="assistant" content="**bold**" index={0} />,
      );
      expect(
        container.querySelector('[data-streamdown="strong"]'),
      ).not.toBeNull();
    });
  });

  // InsufficientMemory (issue #296): ChatBubble fetches the exact figures via
  // `estimate_model_fit` and forwards them plus `onLoadAnyway` to ErrorCard.
  describe('InsufficientMemory error card', () => {
    const FALLBACK_MESSAGE =
      'This model may not fit in memory\nClose some apps, pick a smaller model, or load it anyway.';

    it('fetches model fit info and renders the displayNames label', async () => {
      invoke.mockImplementationOnce(async () => ({
        required_bytes: 8 * 1024 ** 3,
        available_bytes: 4 * 1024 ** 3,
        verdict: 'insufficient',
      }));
      render(
        <ChatBubble
          role="assistant"
          content={FALLBACK_MESSAGE}
          index={0}
          errorKind="InsufficientMemory"
          modelName="repo:file.gguf"
          displayNames={{ 'repo:file.gguf': 'Qwen3.5 9B' }}
        />,
      );
      expect(
        await screen.findByText('Qwen3.5 9B may not fit in memory right now.'),
      ).toBeInTheDocument();
      expect(
        screen.getByText(
          'Estimated need: ~8.0 GB. Currently available: ~4.0 GB.',
        ),
      ).toBeInTheDocument();
      expect(invoke).toHaveBeenCalledWith('estimate_model_fit', {
        modelId: 'repo:file.gguf',
      });
    });

    it("stays pinned to this message's own model even after the active model changes elsewhere", async () => {
      invoke.mockImplementation(async () => ({
        required_bytes: 13.3 * 1024 ** 3,
        available_bytes: 4 * 1024 ** 3,
        verdict: 'insufficient',
      }));
      const { rerender } = render(
        <ChatBubble
          role="assistant"
          content={FALLBACK_MESSAGE}
          index={0}
          errorKind="InsufficientMemory"
          modelName="gpt-oss-20b-slug"
          displayNames={{ 'gpt-oss-20b-slug': 'gpt-oss 20B' }}
        />,
      );
      expect(
        await screen.findByText('gpt-oss 20B may not fit in memory right now.'),
      ).toBeInTheDocument();
      expect(invoke).toHaveBeenLastCalledWith('estimate_model_fit', {
        modelId: 'gpt-oss-20b-slug',
      });

      // Simulate a model switch elsewhere triggering a `displayNames` refresh
      // (new object reference) for this same, still-unretried message. The
      // effect refires, but must still target THIS message's own model, not
      // whatever the app switched the active model to.
      rerender(
        <ChatBubble
          role="assistant"
          content={FALLBACK_MESSAGE}
          index={0}
          errorKind="InsufficientMemory"
          modelName="gpt-oss-20b-slug"
          displayNames={{
            'gpt-oss-20b-slug': 'gpt-oss 20B',
            'llama-3.2-3b-slug': 'Llama 3.2 3B',
          }}
        />,
      );
      expect(invoke).toHaveBeenLastCalledWith('estimate_model_fit', {
        modelId: 'gpt-oss-20b-slug',
      });
    });

    it('re-attributes to model B atomically from carried figures on an in-place switch, without refetching (issue #296)', async () => {
      // Initial failure on model A: the effect fetches and populates A's
      // figures into the bubble's local state.
      invoke.mockImplementationOnce(async () => ({
        required_bytes: 13.3 * 1024 ** 3,
        available_bytes: 4 * 1024 ** 3,
        verdict: 'insufficient',
      }));
      const displayNames = {
        'model-a-slug': 'Model A',
        'model-b-slug': 'Model B',
      };
      const { rerender } = render(
        <ChatBubble
          role="assistant"
          content={FALLBACK_MESSAGE}
          index={0}
          errorKind="InsufficientMemory"
          modelName="model-a-slug"
          displayNames={displayNames}
        />,
      );
      expect(
        await screen.findByText('Model A may not fit in memory right now.'),
      ).toBeInTheDocument();
      expect(
        screen.getByText(
          'Estimated need: ~13.3 GB. Currently available: ~4.0 GB.',
        ),
      ).toBeInTheDocument();

      // The user picks model B, which the gate ALSO blocks: the app swaps
      // modelName to B and carries B's figures in the same commit (memoryFit).
      invoke.mockClear();
      rerender(
        <ChatBubble
          role="assistant"
          content={FALLBACK_MESSAGE}
          index={0}
          errorKind="InsufficientMemory"
          modelName="model-b-slug"
          displayNames={displayNames}
          memoryFit={{
            requiredBytes: 6 * 1024 ** 3,
            availableBytes: 4 * 1024 ** 3,
          }}
        />,
      );

      // B's name and figures render immediately - never A's stale local state.
      expect(
        screen.getByText('Model B may not fit in memory right now.'),
      ).toBeInTheDocument();
      expect(
        screen.getByText(
          'Estimated need: ~6.0 GB. Currently available: ~4.0 GB.',
        ),
      ).toBeInTheDocument();
      expect(screen.queryByText(/Model A may not fit/)).toBeNull();
      // No async refetch fires on the switch, so there is no window in which a
      // stale (or a rejected) estimate could ever render A's data.
      expect(invoke).not.toHaveBeenCalledWith(
        'estimate_model_fit',
        expect.anything(),
      );
    });

    it('falls back to the raw model id when displayNames has no entry', async () => {
      invoke.mockImplementationOnce(async () => ({
        required_bytes: 1,
        available_bytes: 1,
        verdict: 'insufficient',
      }));
      render(
        <ChatBubble
          role="assistant"
          content={FALLBACK_MESSAGE}
          index={0}
          errorKind="InsufficientMemory"
          modelName="raw-slug"
        />,
      );
      expect(
        await screen.findByText('raw-slug may not fit in memory right now.'),
      ).toBeInTheDocument();
    });

    it("falls back to 'This model' when modelName is absent", async () => {
      invoke.mockImplementationOnce(async () => ({
        required_bytes: 1,
        available_bytes: 1,
        verdict: 'insufficient',
      }));
      render(
        <ChatBubble
          role="assistant"
          content={FALLBACK_MESSAGE}
          index={0}
          errorKind="InsufficientMemory"
        />,
      );
      expect(
        await screen.findByText('This model may not fit in memory right now.'),
      ).toBeInTheDocument();
    });

    it('forwards onLoadAnyway and fires it on click', async () => {
      invoke.mockImplementationOnce(async () => ({
        required_bytes: 1,
        available_bytes: 1,
        verdict: 'insufficient',
      }));
      const onLoadAnyway = vi.fn();
      render(
        <ChatBubble
          role="assistant"
          content={FALLBACK_MESSAGE}
          index={0}
          errorKind="InsufficientMemory"
          onLoadAnyway={onLoadAnyway}
        />,
      );
      fireEvent.click(
        await screen.findByRole('button', { name: 'Load anyway' }),
      );
      expect(onLoadAnyway).toHaveBeenCalledTimes(1);
    });

    it('falls back to the generic message render when estimate_model_fit rejects', async () => {
      invoke.mockImplementationOnce(async () => {
        throw new Error('boom');
      });
      render(
        <ChatBubble
          role="assistant"
          content={FALLBACK_MESSAGE}
          index={0}
          errorKind="InsufficientMemory"
        />,
      );
      // The generic fallback (message split on the first newline) is what
      // renders synchronously; assert it survives the rejection settling.
      expect(
        await screen.findByText('This model may not fit in memory'),
      ).toBeInTheDocument();
      expect(
        screen.queryByText(/may not fit in memory right now\./),
      ).toBeNull();
    });

    it('ignores a stale estimate_model_fit response after unmount', async () => {
      let resolveFit!: (value: unknown) => void;
      invoke.mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFit = resolve;
          }),
      );
      const { unmount } = render(
        <ChatBubble
          role="assistant"
          content={FALLBACK_MESSAGE}
          index={0}
          errorKind="InsufficientMemory"
        />,
      );
      unmount();
      resolveFit({
        required_bytes: 1,
        available_bytes: 1,
        verdict: 'insufficient',
      });
      // Flush the now-ignored resolution; nothing should throw.
      await Promise.resolve();
      await Promise.resolve();
    });
  });

  describe('search sources footer', () => {
    it('does not render the sources list by default; trigger button shows count', () => {
      render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={[
            { title: 'A', url: 'https://a' },
            { title: 'B', url: 'https://b' },
          ]}
        />,
      );
      expect(screen.queryByTestId('search-sources')).toBeNull();
      const trigger = screen.getByRole('button', { name: /2 sources/ });
      expect(trigger.getAttribute('aria-expanded')).toBe('false');
    });

    it('toggling the trigger expands and collapses the sources list', () => {
      render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={[{ title: 'A', url: 'https://a' }]}
        />,
      );
      const trigger = screen.getByRole('button', { name: /1 source/ });
      fireEvent.click(trigger);
      expect(screen.getByTestId('search-sources')).toBeInTheDocument();
      expect(trigger.getAttribute('aria-expanded')).toBe('true');
      fireEvent.click(trigger);
      expect(screen.queryByTestId('search-sources')).toBeNull();
    });

    it('renders source rows with numbered position, title, domain, and title-only tooltip', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={[
            { title: 'Rust Docs', url: 'https://doc.rust-lang.org' },
            { title: 'Tokio', url: 'https://tokio.rs' },
          ]}
        />,
      );
      openSources(container);
      const el = screen.getByTestId('search-sources');
      const buttons = el.querySelectorAll('button');
      expect(buttons).toHaveLength(2);
      // Row numbers [1.] [2.]
      expect(buttons[0].textContent).toContain('1.');
      expect(buttons[1].textContent).toContain('2.');
      expect(buttons[0].textContent).toContain('Rust Docs');
      expect(buttons[1].textContent).toContain('Tokio');
      // Hover tooltip via `title` attribute shows title only (not domain)
      expect(buttons[0].title).toBe('Rust Docs');
      expect(buttons[1].title).toBe('Tokio');
      // data-citation wired up for two-way hover linking
      expect(buttons[0].getAttribute('data-citation')).toBe('1');
      expect(buttons[0].getAttribute('data-url')).toBe(
        'https://doc.rust-lang.org',
      );
    });

    it('falls back to URL text when title is empty', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={[{ title: '', url: 'https://example.com' }]}
        />,
      );
      openSources(container);
      expect(screen.getByTestId('search-sources').textContent).toContain(
        'https://example.com',
      );
    });

    it('invokes open_url with the source URL when a row is clicked', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={[{ title: 'Docs', url: 'https://docs.rs' }]}
        />,
      );
      openSources(container);
      screen.getByTitle('Docs').click();
      expect(invoke).toHaveBeenCalledWith('open_url', {
        url: 'https://docs.rs',
      });
    });

    it('does not render sources section when searchSources is absent', () => {
      render(<ChatBubble role="assistant" content="plain answer" index={0} />);
      expect(screen.queryByTestId('search-sources')).toBeNull();
      expect(screen.queryByRole('button', { name: /sources/ })).toBeNull();
    });

    it('does not render sources section when searchSources is empty', () => {
      render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={[]}
        />,
      );
      expect(screen.queryByTestId('search-sources')).toBeNull();
      expect(screen.queryByRole('button', { name: /sources/ })).toBeNull();
    });

    it('strips www. prefix from domain in row display', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={[
            { title: 'Example', url: 'https://www.example.com/page' },
          ]}
        />,
      );
      openSources(container);
      const row = screen.getByTestId('search-sources').querySelector('button')!;
      const domainSpan = row.querySelector('.source-row-domain')!;
      expect(domainSpan.textContent).toBe('example.com');
    });

    it('falls back to raw URL for unparseable source URLs in row display', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={[{ title: 'Weird', url: 'not a valid url' }]}
        />,
      );
      openSources(container);
      const row = screen.getByTestId('search-sources').querySelector('button')!;
      const domainSpan = row.querySelector('.source-row-domain')!;
      expect(domainSpan.textContent).toBe('not a valid url');
    });

    it('does not render sources during streaming', () => {
      render(
        <ChatBubble
          role="assistant"
          content="partial"
          index={0}
          isStreaming={true}
          searchSources={[{ title: 'X', url: 'https://x.com' }]}
        />,
      );
      expect(screen.queryByTestId('search-sources')).toBeNull();
      expect(screen.queryByRole('button', { name: /sources/ })).toBeNull();
    });

    it('renders up to three letter avatars in the trigger', () => {
      const sources = [
        { title: 'Wiki', url: 'https://wikipedia.org' },
        { title: 'Forbes', url: 'https://forbes.com' },
        { title: 'CNN', url: 'https://cnn.com' },
        { title: 'NYT', url: 'https://nytimes.com' },
      ];
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={sources}
        />,
      );
      const trigger = container.querySelector('button.sources-trigger')!;
      const avatars = trigger.querySelectorAll('span.rounded-full');
      // At most 3 avatars are shown even when there are more sources.
      expect(avatars).toHaveLength(3);
      expect(avatars[0].textContent).toBe('W');
      expect(avatars[1].textContent).toBe('F');
      expect(avatars[2].textContent).toBe('C');
    });

    it('shows the correct singular/plural count in the trigger label', () => {
      const { rerender, container } = render(
        <ChatBubble
          role="assistant"
          content="x"
          index={0}
          searchSources={[{ title: 'A', url: 'https://a.com' }]}
        />,
      );
      expect(
        (container.querySelector('button.sources-trigger') as HTMLElement)
          .textContent,
      ).toContain('1 source');
      rerender(
        <ChatBubble
          role="assistant"
          content="x"
          index={0}
          searchSources={[
            { title: 'A', url: 'https://a.com' },
            { title: 'B', url: 'https://b.com' },
          ]}
        />,
      );
      expect(
        (container.querySelector('button.sources-trigger') as HTMLElement)
          .textContent,
      ).toContain('2 sources');
    });

    it('generates a deterministic avatar color from the source domain', () => {
      const sources = [
        { title: 'Wiki', url: 'https://wikipedia.org' },
        { title: 'Forbes', url: 'https://forbes.com' },
      ];
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="x"
          index={0}
          searchSources={sources}
        />,
      );
      const avatars = Array.from(
        container.querySelectorAll('button.sources-trigger span.rounded-full'),
      ) as HTMLElement[];
      // Different domains produce different colors.
      expect(avatars[0].style.background).not.toBe(avatars[1].style.background);
      // Color is a linear-gradient string.
      expect(avatars[0].style.background).toContain('linear-gradient');
    });

    it('renders first letter uppercased for each avatar', () => {
      const sources = [{ title: 'Test', url: 'https://a.com' }];
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="x"
          index={0}
          searchSources={sources}
        />,
      );
      const avatar = container.querySelector(
        'button.sources-trigger span.rounded-full',
      )!;
      // The domain "a.com" first character is "a", uppercased to "A".
      expect(avatar.textContent).toBe('A');
    });

    it('verifies avatar letter uses either domain first char or fallback', () => {
      // The expression (domain[0] ?? '?').toUpperCase() ensures we always get a character.
      // If domain is not empty, domain[0] is truthy.
      // If domain[0] is undefined (e.g., domain === ''), the fallback '?' is used.
      const sources = [
        { title: 'A', url: 'https://a.com' },
        { title: 'B', url: 'https://b.com' },
      ];
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="x"
          index={0}
          searchSources={sources}
        />,
      );
      const avatars = container.querySelectorAll(
        'button.sources-trigger span.rounded-full',
      );
      // Both should render valid characters.
      expect(avatars[0].textContent).toBe('A');
      expect(avatars[1].textContent).toBe('B');
      // Verify they're both truthy single characters.
      expect(avatars[0].textContent?.length).toBe(1);
      expect(avatars[1].textContent?.length).toBe(1);
    });
  });

  describe('search warning icon', () => {
    it('renders the warning icon beside Sources when message has searchWarnings', () => {
      render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={[{ title: 'A', url: 'https://a.com' }]}
          searchWarnings={['reader_unavailable']}
        />,
      );
      expect(screen.getByRole('img', { name: /warning/i })).toBeInTheDocument();
    });

    it('renders the warning icon even when there are no sources', () => {
      render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchWarnings={['no_results_initial']}
        />,
      );
      expect(screen.getByRole('img', { name: /error/i })).toBeInTheDocument();
    });

    it('does not render the warning icon when searchWarnings is absent', () => {
      render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchSources={[{ title: 'A', url: 'https://a.com' }]}
        />,
      );
      expect(screen.queryByRole('img', { name: /warning/i })).toBeNull();
    });

    it('does not render the warning icon when searchWarnings is empty', () => {
      render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchWarnings={[]}
        />,
      );
      expect(screen.queryByRole('img')).toBeNull();
    });

    it('applies search-bubble--error class when any warning is error-severity', () => {
      render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchWarnings={['router_failure']}
        />,
      );
      const bubble = screen.getByTestId('chat-bubble');
      expect(bubble.className).toContain('search-bubble--error');
    });

    it('does not apply search-bubble--error class for warn-severity warnings', () => {
      render(
        <ChatBubble
          role="assistant"
          content="answer"
          index={0}
          searchWarnings={['reader_unavailable']}
        />,
      );
      const bubble = screen.getByTestId('chat-bubble');
      expect(bubble.className).not.toContain('search-bubble--error');
    });

    it('does not apply search-bubble--error class when no warnings', () => {
      render(<ChatBubble role="assistant" content="answer" index={0} />);
      const bubble = screen.getByTestId('chat-bubble');
      expect(bubble.className).not.toContain('search-bubble--error');
    });
  });

  describe('inline citation wrapping and hover linking', () => {
    const SOURCES = [
      { title: 'Rust Docs', url: 'https://doc.rust-lang.org' },
      { title: 'Tokio', url: 'https://tokio.rs' },
    ];

    it('wraps plain-text [N] citations in anchor elements with data-url', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="Rust [1] is fast and Tokio [2] is async [2]."
          index={0}
          searchSources={SOURCES}
        />,
      );
      const anchors = container.querySelectorAll('a.citation-link');
      // Three citations ([1], [2], [2]) should all be wrapped.
      expect(anchors).toHaveLength(3);
      expect(anchors[0].getAttribute('data-citation')).toBe('1');
      expect(anchors[0].getAttribute('data-url')).toBe(
        'https://doc.rust-lang.org',
      );
      expect(anchors[1].getAttribute('data-citation')).toBe('2');
      expect(anchors[2].getAttribute('data-citation')).toBe('2');
      expect(anchors[0].textContent).toBe('[1]');
    });

    it('skips [N] markers that reference a source index past the end of the array', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="Only [1] is real; [9] is orphan."
          index={0}
          searchSources={[SOURCES[0]]}
        />,
      );
      const anchors = container.querySelectorAll('a.citation-link');
      expect(anchors).toHaveLength(1);
      expect(anchors[0].getAttribute('data-citation')).toBe('1');
    });

    it('does not re-wrap on re-render (idempotent)', () => {
      const { container, rerender } = render(
        <ChatBubble
          role="assistant"
          content="One [1] citation."
          index={0}
          searchSources={SOURCES}
        />,
      );
      expect(container.querySelectorAll('a.citation-link')).toHaveLength(1);
      rerender(
        <ChatBubble
          role="assistant"
          content="One [1] citation."
          index={0}
          searchSources={SOURCES}
        />,
      );
      expect(container.querySelectorAll('a.citation-link')).toHaveLength(1);
    });

    it('leaves content untouched when searchSources is absent', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="Mentions [1] but no sources."
          index={0}
        />,
      );
      expect(container.querySelectorAll('a.citation-link')).toHaveLength(0);
    });

    it('leaves content untouched when searchSources is empty', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="Mentions [1] but empty."
          index={0}
          searchSources={[]}
        />,
      );
      expect(container.querySelectorAll('a.citation-link')).toHaveLength(0);
    });

    it('toggles data-active-citation on the bubble when a citation is hovered', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="Answer [1] here."
          index={0}
          searchSources={SOURCES}
        />,
      );
      const bubble = container.querySelector('.search-bubble') as HTMLElement;
      const anchor = container.querySelector('a.citation-link') as HTMLElement;

      fireEvent.mouseOver(anchor);
      expect(bubble.getAttribute('data-active-citation')).toBe('1');

      fireEvent.mouseOut(anchor);
      expect(bubble.hasAttribute('data-active-citation')).toBe(false);
    });

    it('toggles data-active-citation when a source row is hovered', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="Answer [2] here."
          index={0}
          searchSources={SOURCES}
        />,
      );
      openSources(container);
      const bubble = container.querySelector('.search-bubble') as HTMLElement;
      const rows = container.querySelectorAll(
        '[data-testid="search-sources"] button',
      );
      const secondRow = rows[1] as HTMLElement;

      fireEvent.mouseEnter(secondRow);
      expect(bubble.getAttribute('data-active-citation')).toBe('2');

      fireEvent.mouseLeave(secondRow);
      expect(bubble.hasAttribute('data-active-citation')).toBe(false);
    });

    it('opens the URL via open_url when a citation is clicked', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="See [1] for details."
          index={0}
          searchSources={SOURCES}
        />,
      );
      const anchor = container.querySelector('a.citation-link') as HTMLElement;
      fireEvent.click(anchor);
      expect(invoke).toHaveBeenCalledWith('open_url', {
        url: 'https://doc.rust-lang.org',
      });
    });

    it('wraps citation at the very start of text and handles text ending in citation', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="[1] opens the answer and it ends with a cite [2]"
          index={0}
          searchSources={SOURCES}
        />,
      );
      const anchors = container.querySelectorAll('a.citation-link');
      expect(anchors).toHaveLength(2);
      expect(anchors[0].textContent).toBe('[1]');
    });

    it('does not wrap citations that point past the source array', () => {
      // Every [N] in the content references a non-existent source. The walker
      // collects the text node, but no anchors are inserted - lastIndex stays 0.
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="[5] and [9] have nowhere to go."
          index={0}
          searchSources={[SOURCES[0]]}
        />,
      );
      expect(container.querySelectorAll('a.citation-link')).toHaveLength(0);
    });

    it('falls back to URL for citation title when source has empty title', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="See [1]."
          index={0}
          searchSources={[{ title: '', url: 'https://bare.com' }]}
        />,
      );
      const anchor = container.querySelector('a.citation-link')!;
      expect(anchor.getAttribute('title')).toBe('https://bare.com');
    });

    it('mouse events on non-citation targets do not toggle the active attribute', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="No citations here."
          index={0}
          searchSources={SOURCES}
        />,
      );
      const bubble = container.querySelector('.search-bubble') as HTMLElement;
      fireEvent.mouseOver(bubble);
      fireEvent.mouseOut(bubble);
      fireEvent.click(bubble);
      expect(bubble.hasAttribute('data-active-citation')).toBe(false);
      // A click with no citation target does NOT call open_url for the bubble.
      expect(invoke).not.toHaveBeenCalled();
    });
  });

  describe('search progress', () => {
    it('does not render SearchProgressBlock when not searching', () => {
      render(<ChatBubble role="assistant" content="answer" index={0} />);
      expect(
        screen.queryByTestId('search-progress-block'),
      ).not.toBeInTheDocument();
    });

    it('renders SearchProgressBlock when isSearching', () => {
      render(
        <ChatBubble
          role="assistant"
          content=""
          index={0}
          isSearching
          searchStage={{ kind: 'searching' }}
        />,
      );
      expect(screen.getByTestId('search-progress-block')).toBeInTheDocument();
    });

    it('renders SearchProgressBlock above the thinking block while searching', () => {
      render(
        <ChatBubble
          role="assistant"
          content=""
          index={0}
          isSearching
          searchStage={{ kind: 'searching' }}
          thinkingContent="thoughts"
          isThinking={false}
        />,
      );
      const progress = screen.getByTestId('search-progress-block');
      const reasoningBlock = screen.getByTestId('reasoning-block');
      expect(
        progress.compareDocumentPosition(reasoningBlock) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    });
  });

  describe('model attribution', () => {
    it('renders the attribution chip when modelName is provided on assistant messages', () => {
      render(
        <ChatBubble
          role="assistant"
          content="Hello there"
          index={0}
          modelName="gemma4:e2b"
        />,
      );
      const chip = screen.getByTestId('model-attribution');
      expect(chip).toBeInTheDocument();
      expect(chip).toHaveTextContent('gemma4:e2b');
    });

    it('renders the friendly display name in the chip when the model id has one', () => {
      // Built-in model ids are raw "repo:file.gguf" slugs; the chip must show
      // the elegant label, matching the model picker and titlebar pill.
      render(
        <ChatBubble
          role="assistant"
          content="Hello there"
          index={0}
          modelName="unsloth/Qwen3.5:Qwen3.5-9B-Q4_K_M.gguf"
          displayNames={{
            'unsloth/Qwen3.5:Qwen3.5-9B-Q4_K_M.gguf': 'Qwen3.5 9B',
          }}
        />,
      );
      const chip = screen.getByTestId('model-attribution');
      expect(chip).toHaveTextContent('Qwen3.5 9B');
      expect(chip).not.toHaveTextContent(
        'unsloth/Qwen3.5:Qwen3.5-9B-Q4_K_M.gguf',
      );
    });

    it('does not render the attribution chip when modelName is absent', () => {
      render(<ChatBubble role="assistant" content="Hello" index={0} />);
      expect(screen.queryByTestId('model-attribution')).toBeNull();
    });

    it('does not render the attribution chip on user messages even with modelName', () => {
      render(
        <ChatBubble
          role="user"
          content="Hello"
          index={0}
          modelName="gemma4:e2b"
        />,
      );
      expect(screen.queryByTestId('model-attribution')).toBeNull();
    });

    it('does not render the attribution chip when the message is an error callout', () => {
      render(
        <ChatBubble
          role="assistant"
          content="Something went wrong"
          index={0}
          modelName="gemma4:e2b"
          errorKind="Other"
        />,
      );
      expect(screen.queryByTestId('model-attribution')).toBeNull();
    });

    it('does not render the attribution chip while the assistant is still streaming', () => {
      render(
        <ChatBubble
          role="assistant"
          content="partial..."
          index={0}
          isStreaming
          modelName="gemma4:e2b"
        />,
      );
      // Footer row including the attribution is hidden during streaming.
      expect(screen.queryByTestId('model-attribution')).toBeNull();
    });
  });

  describe('Render-time legacy artifact scrub', () => {
    it('hides leaked special tokens from assistant markdown output', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content="<|im_start|>assistant\nHello there<|im_end|>"
          index={0}
        />,
      );
      // The markdown body should render the visible text but never expose
      // the raw special tokens, regardless of where they appeared.
      expect(container.textContent).toContain('Hello there');
      expect(container.textContent).not.toContain('<|im_start|>');
      expect(container.textContent).not.toContain('<|im_end|>');
    });

    it('does not scrub user content (markers passed through verbatim)', () => {
      // User input never contains real model markers; if it ever does
      // (paste, debugging) we render it verbatim so the user sees what
      // they sent.
      const { container } = render(
        <ChatBubble role="user" content="<|im_start|>" index={0} />,
      );
      expect(container.textContent).toContain('<|im_start|>');
    });
  });
});
