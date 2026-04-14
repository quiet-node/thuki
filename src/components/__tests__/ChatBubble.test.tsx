import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ChatBubble } from '../ChatBubble';

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
      // AI messages span full width — no max-width cap like user bubbles
      expect(container.querySelector('.group')).toBeNull();
    });

    it('shows copy button for assistant messages', () => {
      render(<ChatBubble role="assistant" content="response" index={0} />);
      expect(
        screen.getByRole('button', { name: 'Copy message' }),
      ).toBeInTheDocument();
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
      // CopyButton should also be hidden — nothing to copy
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

  describe('Layout', () => {
    it('has max-width constraint (max-w-[80%])', () => {
      const { container } = render(
        <ChatBubble role="user" content="test" index={0} />,
      );
      expect(container.querySelector('.max-w-\\[80\\%\\]')).not.toBeNull();
    });
  });

  describe('ThinkingBlock rendering', () => {
    it('renders ThinkingBlock for AI message with thinkingContent', () => {
      render(
        <ChatBubble
          role="assistant"
          content="The answer is 42."
          index={0}
          thinkingContent="Let me reason about this..."
        />,
      );
      expect(screen.getByTestId('thinking-block')).toBeInTheDocument();
    });

    it('does not render ThinkingBlock for AI message without thinkingContent', () => {
      render(<ChatBubble role="assistant" content="Hello" index={0} />);
      expect(screen.queryByTestId('thinking-block')).toBeNull();
    });

    it('does not render ThinkingBlock for user message even with thinkingContent', () => {
      render(
        <ChatBubble
          role="user"
          content="Hello"
          index={0}
          thinkingContent="Should not appear"
        />,
      );
      expect(screen.queryByTestId('thinking-block')).toBeNull();
    });

    it('shows "Thinking..." state when isThinking is true', () => {
      render(
        <ChatBubble
          role="assistant"
          content=""
          index={0}
          thinkingContent="Reasoning in progress..."
          isThinking={true}
        />,
      );
      expect(screen.getByTestId('thinking-block')).toBeInTheDocument();
      expect(screen.getByText('Thinking...')).toBeInTheDocument();
    });
  });

  describe('Error messages (errorKind)', () => {
    it('renders ErrorCard instead of MarkdownRenderer when errorKind is set', () => {
      const { container } = render(
        <ChatBubble
          role="assistant"
          content={"Ollama isn't running\nStart Ollama and try again."}
          index={0}
          errorKind="NotRunning"
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
          errorKind="NotRunning"
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
});
