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

  it('renders streaming text when isThinking=true', () => {
    render(
      <ThinkingBlock
        thinkingContent="Let me analyze this problem"
        isThinking={true}
      />,
    );
    // Auto-expanded while thinking, rendered as markdown
    expect(screen.getByText('Let me analyze this problem')).toBeInTheDocument();
  });

  it('shows "Thinking..." as summary while isThinking', () => {
    render(<ThinkingBlock thinkingContent="Working on it" isThinking={true} />);
    expect(screen.getByText('Thinking...')).toBeInTheDocument();
  });

  it('renders collapsed with duration when isThinking=false', () => {
    render(
      <ThinkingBlock
        thinkingContent="I analyzed the code. It looks correct."
        isThinking={false}
        durationMs={3500}
      />,
    );
    // Summary visible
    expect(screen.getByText('I analyzed the code')).toBeInTheDocument();
    // Duration visible
    expect(screen.getByText('Thought for 4 seconds')).toBeInTheDocument();
  });

  it('expands on click and renders thinking content', () => {
    render(
      <ThinkingBlock
        thinkingContent="I analyzed the code. It looks correct."
        isThinking={false}
        durationMs={2000}
      />,
    );

    // Click to expand
    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );

    // Content rendered inside the expanded section (summary + expanded both match)
    const matches = screen.getAllByText(/I analyzed the code/);
    expect(matches.length).toBeGreaterThanOrEqual(2); // summary + expanded content
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

    // Click to expand
    fireEvent.click(toggleBtn);
    expect(screen.getByTestId('timeline-rail')).toBeInTheDocument();

    // Click again to collapse
    fireEvent.click(toggleBtn);
    expect(screen.queryByTestId('timeline-rail')).not.toBeInTheDocument();
  });

  it('uses first sentence as summary', () => {
    render(
      <ThinkingBlock
        thinkingContent="First sentence here! Second sentence follows."
        isThinking={false}
        durationMs={1000}
      />,
    );
    expect(screen.getByText('First sentence here')).toBeInTheDocument();
  });

  it('uses first line as summary when newline is the delimiter', () => {
    render(
      <ThinkingBlock
        thinkingContent={'First line\nSecond line'}
        isThinking={false}
        durationMs={1000}
      />,
    );
    expect(screen.getByText('First line')).toBeInTheDocument();
  });

  it('uses question mark as summary delimiter', () => {
    render(
      <ThinkingBlock
        thinkingContent="What is the answer? Let me think."
        isThinking={false}
        durationMs={1000}
      />,
    );
    expect(screen.getByText('What is the answer')).toBeInTheDocument();
  });

  it('uses full content as summary when no sentence-ending punctuation', () => {
    render(
      <ThinkingBlock
        thinkingContent="No punctuation here"
        isThinking={false}
        durationMs={1000}
      />,
    );
    expect(screen.getByText('No punctuation here')).toBeInTheDocument();
  });

  it('skips "Thinking Process:" label in summary extraction', () => {
    render(
      <ThinkingBlock
        thinkingContent="Thinking Process:\n\n1. **Analyze the Request:** The user asked about algorithms."
        isThinking={false}
        durationMs={4000}
      />,
    );
    // Should skip "Thinking Process:" and extract meaningful summary
    expect(
      screen.getByText(/Analyze the Request.*user asked about algorithms/),
    ).toBeInTheDocument();
  });

  it('strips markdown bold markers from summary', () => {
    render(
      <ThinkingBlock
        thinkingContent="**Bold summary.** More text."
        isThinking={false}
        durationMs={1000}
      />,
    );
    expect(screen.getByText('Bold summary')).toBeInTheDocument();
  });

  it('shows timeline rail with checkmark and Done label when expanded after done', () => {
    render(
      <ThinkingBlock
        thinkingContent="Done thinking."
        isThinking={false}
        durationMs={2000}
      />,
    );

    // Expand
    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );

    expect(screen.getByTestId('timeline-rail')).toBeInTheDocument();
    expect(screen.getByTestId('checkmark-icon')).toBeInTheDocument();
    expect(screen.getByTestId('clock-icon')).toBeInTheDocument();
    // "Done" label next to checkmark
    expect(screen.getByText('Done')).toBeInTheDocument();
    // Clock should NOT be spinning
    expect(
      screen.getByTestId('clock-icon').classList.contains('animate-spin'),
    ).toBe(false);
  });

  it('spins clock icon while isThinking', () => {
    render(<ThinkingBlock thinkingContent="Thinking now" isThinking={true} />);
    expect(
      screen.getByTestId('clock-icon').classList.contains('animate-spin'),
    ).toBe(true);
    // No checkmark while thinking
    expect(screen.queryByTestId('checkmark-icon')).not.toBeInTheDocument();
    // No Done label while thinking
    expect(screen.queryByText('Done')).not.toBeInTheDocument();
  });

  it('auto-collapses when isThinking transitions true to false', () => {
    const { rerender } = render(
      <ThinkingBlock
        thinkingContent="Working on it"
        isThinking={true}
        durationMs={1500}
      />,
    );
    // Auto-expanded while thinking
    expect(screen.getByTestId('timeline-rail')).toBeInTheDocument();

    // Transition to done
    rerender(
      <ThinkingBlock
        thinkingContent="Working on it. Done now."
        isThinking={false}
        durationMs={1500}
      />,
    );

    // Should auto-collapse: timeline rail no longer visible
    expect(screen.queryByTestId('timeline-rail')).not.toBeInTheDocument();
    // Duration now shown
    expect(screen.getByText('Thought for 2 seconds')).toBeInTheDocument();
  });

  it('auto-expands when isThinking transitions false to true', () => {
    const { rerender } = render(
      <ThinkingBlock
        thinkingContent="Previous thought."
        isThinking={false}
        durationMs={1000}
      />,
    );
    // Collapsed
    expect(screen.queryByTestId('timeline-rail')).not.toBeInTheDocument();

    // Transition to thinking
    rerender(
      <ThinkingBlock
        thinkingContent="New thought in progress"
        isThinking={true}
      />,
    );
    // Auto-expanded
    expect(screen.getByTestId('timeline-rail')).toBeInTheDocument();
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

  it('does not show duration when durationMs is undefined', () => {
    render(
      <ThinkingBlock thinkingContent="No duration info." isThinking={false} />,
    );
    // Summary visible but no duration text
    expect(screen.getByText('No duration info')).toBeInTheDocument();
    expect(screen.queryByText(/Thought for/)).not.toBeInTheDocument();
  });

  it('does not show duration while still thinking', () => {
    render(
      <ThinkingBlock
        thinkingContent="Still going"
        isThinking={true}
        durationMs={3000}
      />,
    );
    expect(screen.queryByText(/Thought for/)).not.toBeInTheDocument();
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
    // Collapsed: rotated 90deg
    expect(chevron.style.transform).toBe('rotate(90deg)');

    // Expand
    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle thinking details' }),
    );
    expect(chevron.style.transform).toBe('rotate(180deg)');
  });

  it('truncates long summaries to 80 characters', () => {
    const longContent =
      'This is a very long first sentence that goes on and on and on without any punctuation to break it up naturally';
    render(
      <ThinkingBlock
        thinkingContent={longContent}
        isThinking={false}
        durationMs={1000}
      />,
    );
    const summary = screen.getByText(/This is a very long/);
    expect(summary.textContent!.length).toBeLessThanOrEqual(83); // 80 + "..."
  });
});
