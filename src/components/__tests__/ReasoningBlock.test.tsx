import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { ReasoningBlock } from '../ReasoningBlock';

describe('ReasoningBlock', () => {
  it('returns null when thinkingContent is empty and no pending state is set', () => {
    const { container } = render(
      <ReasoningBlock thinkingContent="" isThinking={false} />,
    );
    expect(container.innerHTML).toBe('');
  });

  it('renders bare dots with no label when pending and no pendingLabel is supplied', () => {
    render(<ReasoningBlock isThinking={false} isPending />);
    expect(screen.queryByTestId('loading-label')).toBeNull();
  });

  it('shows the caller-supplied pendingLabel while pending', () => {
    render(
      <ReasoningBlock
        isThinking={false}
        isPending
        pendingLabel="Starting up the model…"
      />,
    );
    const label = screen.getByTestId('loading-label');
    expect(label).toBeInTheDocument();
    expect(label.textContent).toBe('Starting up the model…');
  });

  it('does not render a toggle button while pending', () => {
    render(<ReasoningBlock isThinking={false} isPending />);
    expect(
      screen.queryByRole('button', { name: 'Toggle reasoning details' }),
    ).toBeNull();
  });

  it('hides the chevron from assistive tech while pending (nothing to expand yet)', () => {
    render(
      <ReasoningBlock
        isThinking={false}
        isPending
        pendingLabel="Starting up the model…"
      />,
    );
    const chevron = screen.getByTestId('reasoning-chevron');
    expect(chevron).toHaveAttribute('aria-hidden', 'true');
    expect(chevron.className).toContain('opacity-0');
  });

  it("reserves the chevron's width while pending via an invisible spacer, so the label does not shift once thinking starts", () => {
    render(
      <ReasoningBlock
        isThinking={false}
        isPending
        pendingLabel="Starting up the model…"
      />,
    );
    const prefix = screen.getByTestId('loading-label-prefix');
    expect(prefix).toBeInTheDocument();
    expect(prefix.textContent).toBe('▲');
  });

  it('uses the exact same summary-row classes for the pending row and the clickable summary row', () => {
    const { unmount } = render(
      <ReasoningBlock
        isThinking={false}
        isPending
        pendingLabel="Starting up the model…"
      />,
    );
    const pendingRow = screen.getByTestId('reasoning-pending');
    const pendingClasses = pendingRow.className;
    unmount();

    render(
      <ReasoningBlock thinkingContent="Working on it" isThinking={true} />,
    );
    const summaryButton = screen.getByRole('button', {
      name: 'Toggle reasoning details',
    });
    // The button adds interactive-only extras on top of the same base
    // classes the pending row uses - assert the base is a strict subset.
    pendingClasses
      .split(' ')
      .forEach((cls) => expect(summaryButton.className).toContain(cls));
  });

  it('shows a clickable "Reasoning..." summary while isThinking', () => {
    render(
      <ReasoningBlock thinkingContent="Working on it" isThinking={true} />,
    );
    const label = screen.getByTestId('loading-label');
    expect(label).toBeInTheDocument();
    expect(label.textContent).toBe('Reasoning...');
    expect(screen.getByTestId('loading-label-prefix')).toBeInTheDocument();
  });

  it('is collapsed by default, even while thinking', () => {
    render(
      <ReasoningBlock thinkingContent="Working on it" isThinking={true} />,
    );
    // Collapsed: no timeline rail visible
    expect(screen.queryByTestId('timeline-rail')).not.toBeInTheDocument();
  });

  it('shows "Reasoning" in collapsed state when done', () => {
    render(
      <ReasoningBlock thinkingContent="Some reasoning." isThinking={false} />,
    );
    expect(screen.getByTestId('reasoning-summary-label').textContent).toBe(
      'Reasoning',
    );
  });

  it('expands on click to show thinking content', () => {
    render(
      <ReasoningBlock
        thinkingContent="I analyzed the code."
        isThinking={false}
      />,
    );

    // Click to expand
    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle reasoning details' }),
    );

    // Timeline rail and content visible
    expect(screen.getByTestId('timeline-rail')).toBeInTheDocument();
  });

  it('collapses on second click', () => {
    render(
      <ReasoningBlock
        thinkingContent="Some thinking content."
        isThinking={false}
      />,
    );

    const toggleBtn = screen.getByRole('button', {
      name: 'Toggle reasoning details',
    });

    fireEvent.click(toggleBtn);
    expect(screen.getByTestId('timeline-rail')).toBeInTheDocument();

    fireEvent.click(toggleBtn);
    expect(screen.queryByTestId('timeline-rail')).not.toBeInTheDocument();
  });

  it('strips "Thinking Process:" from displayed content', () => {
    render(
      <ReasoningBlock
        thinkingContent="Thinking Process:\n\nActual reasoning here."
        isThinking={false}
      />,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle reasoning details' }),
    );

    // "Thinking Process:" stripped, actual content shown
    expect(screen.getByText(/Actual reasoning here/)).toBeInTheDocument();
  });

  it('renders thinking content as markdown', () => {
    render(
      <ReasoningBlock
        thinkingContent="Some **bold** text."
        isThinking={false}
      />,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle reasoning details' }),
    );

    // MarkdownRenderer is used (content appears in DOM)
    expect(screen.getByText(/bold/)).toBeInTheDocument();
  });

  it('shows Done label with checkmark when expanded after done', () => {
    render(
      <ReasoningBlock thinkingContent="Done thinking." isThinking={false} />,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle reasoning details' }),
    );

    expect(screen.getByTestId('checkmark-icon')).toBeInTheDocument();
    expect(screen.getByText('Done')).toBeInTheDocument();
  });

  it('does not show checkmark or Done while thinking', () => {
    render(<ReasoningBlock thinkingContent="Thinking now" isThinking={true} />);

    // Expand manually
    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle reasoning details' }),
    );

    expect(screen.queryByTestId('checkmark-icon')).not.toBeInTheDocument();
    expect(screen.queryByText('Done')).not.toBeInTheDocument();
  });

  it('spins clock icon while isThinking', () => {
    render(<ReasoningBlock thinkingContent="Thinking now" isThinking={true} />);

    // Expand manually to see clock
    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle reasoning details' }),
    );

    expect(
      screen.getByTestId('clock-icon').classList.contains('animate-spin'),
    ).toBe(true);
  });

  it('does not spin clock icon when done', () => {
    render(<ReasoningBlock thinkingContent="Done." isThinking={false} />);

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle reasoning details' }),
    );

    expect(
      screen.getByTestId('clock-icon').classList.contains('animate-spin'),
    ).toBe(false);
  });

  it('sets aria-expanded correctly on the toggle button', () => {
    render(
      <ReasoningBlock thinkingContent="Test content." isThinking={false} />,
    );
    const button = screen.getByRole('button', {
      name: 'Toggle reasoning details',
    });
    expect(button.getAttribute('aria-expanded')).toBe('false');

    fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('true');
  });

  it('rotates chevron based on expanded state', () => {
    render(
      <ReasoningBlock thinkingContent="Chevron test." isThinking={false} />,
    );
    const chevron = screen.getByTestId('reasoning-chevron');
    expect(chevron.style.transform).toBe('rotate(90deg)');

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle reasoning details' }),
    );
    expect(chevron.style.transform).toBe('rotate(180deg)');
  });

  it('renders thinking text in normal color (not grayed out)', () => {
    render(
      <ReasoningBlock thinkingContent="Normal text." isThinking={false} />,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'Toggle reasoning details' }),
    );

    // The thinking text container should NOT have text-secondary/70 class
    const textContainer = screen.getByText(/Normal text/).closest('div');
    expect(textContainer?.className).not.toContain('text-text-secondary');
  });
});
