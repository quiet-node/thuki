import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { ThinkingBlock } from '../ThinkingBlock';

describe('ThinkingBlock', () => {
  it('returns null when thinkingContent is empty', () => {
    const { container } = render(
      <ThinkingBlock thinkingContent="" isThinking={false} />,
    );
    expect(container.innerHTML).toBe('');
  });

  it('shows animated "Thinking..." label while isThinking', () => {
    render(<ThinkingBlock thinkingContent="Working on it" isThinking={true} />);
    expect(screen.getByTestId('thinking-label')).toBeInTheDocument();
    expect(screen.getByTestId('thinking-label').textContent).toBe(
      'Thinking...',
    );
  });

  it('is collapsed by default, even while thinking', () => {
    render(<ThinkingBlock thinkingContent="Working on it" isThinking={true} />);
    // Collapsed: no timeline rail visible
    expect(screen.queryByTestId('timeline-rail')).not.toBeInTheDocument();
  });

  it('shows "Thought for X seconds" in collapsed state', () => {
    render(
      <ThinkingBlock
        thinkingContent="Some reasoning."
        isThinking={false}
        durationMs={3500}
      />,
    );
    expect(screen.getByText('Thought for 4 seconds')).toBeInTheDocument();
  });

  it('expands on click to show thinking content', () => {
    render(
      <ThinkingBlock
        thinkingContent="I analyzed the code."
        isThinking={false}
        durationMs={2000}
      />,
    );

    // Click to expand
    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );

    // Timeline rail and content visible
    expect(screen.getByTestId('timeline-rail')).toBeInTheDocument();
  });

  it('collapses on second click', () => {
    render(
      <ThinkingBlock
        thinkingContent="Some thinking content."
        isThinking={false}
        durationMs={5000}
      />,
    );

    const toggleBtn = screen.getByRole('button', {
      name: 'Toggle thinking details',
    });

    fireEvent.click(toggleBtn);
    expect(screen.getByTestId('timeline-rail')).toBeInTheDocument();

    fireEvent.click(toggleBtn);
    expect(screen.queryByTestId('timeline-rail')).not.toBeInTheDocument();
  });

  it('strips "Thinking Process:" from displayed content', () => {
    render(
      <ThinkingBlock
        thinkingContent="Thinking Process:\n\nActual reasoning here."
        isThinking={false}
        durationMs={4000}
      />,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );

    // "Thinking Process:" stripped, actual content shown
    expect(screen.getByText(/Actual reasoning here/)).toBeInTheDocument();
  });

  it('renders thinking content as markdown', () => {
    render(
      <ThinkingBlock
        thinkingContent="Some **bold** text."
        isThinking={false}
        durationMs={2000}
      />,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );

    // MarkdownRenderer is used (content appears in DOM)
    expect(screen.getByText(/bold/)).toBeInTheDocument();
  });

  it('shows Done label with checkmark when expanded after done', () => {
    render(
      <ThinkingBlock
        thinkingContent="Done thinking."
        isThinking={false}
        durationMs={2000}
      />,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );

    expect(screen.getByTestId('checkmark-icon')).toBeInTheDocument();
    expect(screen.getByText('Done')).toBeInTheDocument();
  });

  it('does not show checkmark or Done while thinking', () => {
    render(<ThinkingBlock thinkingContent="Thinking now" isThinking={true} />);

    // Expand manually
    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );

    expect(screen.queryByTestId('checkmark-icon')).not.toBeInTheDocument();
    expect(screen.queryByText('Done')).not.toBeInTheDocument();
  });

  it('spins clock icon while isThinking', () => {
    render(<ThinkingBlock thinkingContent="Thinking now" isThinking={true} />);

    // Expand manually to see clock
    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );

    expect(
      screen.getByTestId('clock-icon').classList.contains('animate-spin'),
    ).toBe(true);
  });

  it('does not spin clock icon when done', () => {
    render(
      <ThinkingBlock
        thinkingContent="Done."
        isThinking={false}
        durationMs={2000}
      />,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );

    expect(
      screen.getByTestId('clock-icon').classList.contains('animate-spin'),
    ).toBe(false);
  });

  it('formats sub-second durations as "less than a second"', () => {
    render(
      <ThinkingBlock
        thinkingContent="Quick thought."
        isThinking={false}
        durationMs={500}
      />,
    );
    expect(
      screen.getByText('Thought for less than a second'),
    ).toBeInTheDocument();
  });

  it('formats singular second correctly', () => {
    render(
      <ThinkingBlock
        thinkingContent="Brief thought."
        isThinking={false}
        durationMs={1000}
      />,
    );
    expect(screen.getByText('Thought for 1 second')).toBeInTheDocument();
  });

  it('shows animated label instead of duration while still thinking', () => {
    render(
      <ThinkingBlock
        thinkingContent="Still going"
        isThinking={true}
        durationMs={3000}
      />,
    );
    expect(screen.queryByText(/Thought for/)).not.toBeInTheDocument();
    expect(screen.getByTestId('thinking-label')).toBeInTheDocument();
  });

  it('sets aria-expanded correctly on the toggle button', () => {
    render(
      <ThinkingBlock
        thinkingContent="Test content."
        isThinking={false}
        durationMs={2000}
      />,
    );
    const button = screen.getByRole('button', {
      name: 'Toggle thinking details',
    });
    expect(button.getAttribute('aria-expanded')).toBe('false');

    fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('true');
  });

  it('rotates chevron based on expanded state', () => {
    render(
      <ThinkingBlock
        thinkingContent="Chevron test."
        isThinking={false}
        durationMs={1000}
      />,
    );
    const chevron = screen.getByTestId('thinking-chevron');
    expect(chevron.style.transform).toBe('rotate(90deg)');

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );
    expect(chevron.style.transform).toBe('rotate(180deg)');
  });

  it('renders thinking text in normal color (not grayed out)', () => {
    render(
      <ThinkingBlock
        thinkingContent="Normal text."
        isThinking={false}
        durationMs={1000}
      />,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );

    // The thinking text container should NOT have text-secondary/70 class
    const textContainer = screen.getByText(/Normal text/).closest('div');
    expect(textContainer?.className).not.toContain('text-text-secondary');
  });
});
