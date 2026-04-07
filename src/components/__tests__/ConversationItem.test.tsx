import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ConversationItem } from '../ConversationItem';
import type { ConversationSummary } from '../../types/history';

const SUMMARY: ConversationSummary = {
  id: 'conv-1',
  title: 'How does React work?',
  model: 'gemma4:e2b',
  updated_at: Date.now(),
  message_count: 6,
};

describe('ConversationItem', () => {
  it('renders the conversation title', () => {
    render(
      <ConversationItem
        conversation={SUMMARY}
        onSelect={vi.fn()}
        onDelete={vi.fn()}
      />,
    );
    expect(screen.getByText('How does React work?')).toBeInTheDocument();
  });

  it('renders "Untitled" when title is null', () => {
    render(
      <ConversationItem
        conversation={{ ...SUMMARY, title: null }}
        onSelect={vi.fn()}
        onDelete={vi.fn()}
      />,
    );
    expect(screen.getByText('Untitled')).toBeInTheDocument();
  });

  it('renders relative timestamp', () => {
    render(
      <ConversationItem
        conversation={SUMMARY}
        onSelect={vi.fn()}
        onDelete={vi.fn()}
      />,
    );
    expect(screen.getByText('just now')).toBeInTheDocument();
  });

  it('calls onSelect with conversation id when clicked', () => {
    const onSelect = vi.fn();
    render(
      <ConversationItem
        conversation={SUMMARY}
        onSelect={onSelect}
        onDelete={vi.fn()}
      />,
    );
    fireEvent.click(
      screen.getByRole('button', { name: /how does react work/i }),
    );
    expect(onSelect).toHaveBeenCalledWith('conv-1');
  });

  it('calls onDelete with conversation id when delete button is clicked', () => {
    const onDelete = vi.fn();
    render(
      <ConversationItem
        conversation={SUMMARY}
        onSelect={vi.fn()}
        onDelete={onDelete}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /delete/i }));
    expect(onDelete).toHaveBeenCalledWith('conv-1');
  });

  it('does not call onSelect when delete button is clicked', () => {
    const onSelect = vi.fn();
    render(
      <ConversationItem
        conversation={SUMMARY}
        onSelect={onSelect}
        onDelete={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /delete/i }));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('applies active styling when isActive is true', () => {
    render(
      <ConversationItem
        conversation={SUMMARY}
        onSelect={vi.fn()}
        onDelete={vi.fn()}
        isActive
      />,
    );
    const button = screen.getByRole('button', {
      name: /how does react work/i,
    });
    expect(button).toHaveAttribute('aria-current', 'true');
    expect(button.className).toContain('bg-primary/10');
    expect(button.className).toContain('border-primary');
  });

  it('does not apply active styling when isActive is false', () => {
    render(
      <ConversationItem
        conversation={SUMMARY}
        onSelect={vi.fn()}
        onDelete={vi.fn()}
        isActive={false}
      />,
    );
    const button = screen.getByRole('button', {
      name: /how does react work/i,
    });
    expect(button).not.toHaveAttribute('aria-current');
    expect(button.className).not.toContain('bg-primary/10');
  });
});
