import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
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
      expect(screen.getByRole('button', { name: 'Copy message' })).toBeInTheDocument();
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
    it('renders assistant content via MarkdownRenderer (** becomes <strong>)', () => {
      const { container } = render(
        <ChatBubble role="assistant" content="**bold**" index={0} />,
      );
      expect(container.querySelector('strong')).not.toBeNull();
      expect(container.querySelector('strong')!.textContent).toBe('bold');
    });

    it('applies assistant styling (chat-bubble-ai class)', () => {
      const { container } = render(
        <ChatBubble role="assistant" content="Hello" index={0} />,
      );
      expect(container.querySelector('.chat-bubble-ai')).not.toBeNull();
    });

    it('shows copy button for assistant messages', () => {
      render(<ChatBubble role="assistant" content="response" index={0} />);
      expect(screen.getByRole('button', { name: 'Copy message' })).toBeInTheDocument();
    });

    it('left-aligns assistant messages (justify-start class)', () => {
      const { container } = render(
        <ChatBubble role="assistant" content="Hi" index={0} />,
      );
      const outerDiv = container.firstElementChild;
      expect(outerDiv?.classList.contains('justify-start')).toBe(true);
    });
  });

  describe('Layout', () => {
    it('has max-width constraint (max-w-[80%])', () => {
      const { container } = render(
        <ChatBubble role="user" content="test" index={0} />,
      );
      // The inner group wrapper has the max-width class
      const groupDiv = container.querySelector('.group');
      expect(groupDiv?.classList.contains('max-w-[80%]')).toBe(true);
    });
  });
});
