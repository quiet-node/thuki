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

  it("reserves the chevron's width while pending via an invisible strip accessory, so the label does not shift once thinking starts", () => {
    render(
      <ReasoningBlock
        isThinking={false}
        isPending
        pendingLabel="Starting up the model…"
      />,
    );
    const strip = screen.getByTestId('request-status-strip');
    const chevron = screen.getByTestId('reasoning-chevron');
    expect(chevron).toBeInTheDocument();
    expect(chevron.textContent).toBe('▲');
    expect(chevron.className).toContain('opacity-0');
    // DOM order: dots → accessory chevron → title.
    const stripChildren = Array.from(strip.children);
    expect(stripChildren[0].className).toContain('request-status-strip__dots');
    expect(stripChildren[1]).toBe(chevron);
    expect(stripChildren[2]).toHaveAttribute(
      'data-testid',
      'loading-stage-title',
    );
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
    // Live thinking: dots → chevron accessory → label inside the strip.
    const strip = screen.getByTestId('request-status-strip');
    const chevron = screen.getByTestId('reasoning-chevron');
    const stripChildren = Array.from(strip.children);
    expect(stripChildren[0].className).toContain('request-status-strip__dots');
    expect(stripChildren[1]).toBe(chevron);
    expect(stripChildren[2]).toHaveAttribute(
      'data-testid',
      'loading-stage-title',
    );
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
    // Done (no live dots): chevron left of static title only.
    const button = screen.getByRole('button', {
      name: 'Toggle reasoning details',
    });
    const chevron = screen.getByTestId('reasoning-chevron');
    expect(button.firstElementChild).toBe(chevron);
    expect(screen.queryByTestId('request-status-strip')).toBeNull();
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

  it('renders a sources chip under the summary when searchSources is non-empty', () => {
    render(
      <ReasoningBlock
        thinkingContent="Using the web"
        isThinking
        searchSources={[{ title: 'ESPN', url: 'https://espn.com/score' }]}
      />,
    );
    expect(screen.getByTestId('reasoning-sources-chip')).toBeInTheDocument();
    expect(
      screen.getByTestId('reasoning-sources-chip-label'),
    ).toHaveTextContent('1 source');
    expect(screen.getByLabelText('1 source')).toBeInTheDocument();
  });

  it('uses plural chip label for multiple sources while thinking', () => {
    render(
      <ReasoningBlock
        thinkingContent="Using the web"
        isThinking
        searchSources={[
          { title: 'A', url: 'https://a.example/x' },
          { title: 'B', url: 'https://b.example/y' },
        ]}
      />,
    );
    expect(
      screen.getByTestId('reasoning-sources-chip-label'),
    ).toHaveTextContent('2 sources');
  });

  it('hides the sources chip once reasoning is done (footer owns sources)', () => {
    render(
      <ReasoningBlock
        thinkingContent="Using the web"
        isThinking={false}
        searchSources={[{ title: 'A', url: 'https://a.example/x' }]}
      />,
    );
    expect(screen.queryByTestId('reasoning-sources-chip')).toBeNull();
  });

  it('omits the sources chip when searchSources is empty or missing', () => {
    const { rerender } = render(
      <ReasoningBlock thinkingContent="No web" isThinking />,
    );
    expect(screen.queryByTestId('reasoning-sources-chip')).toBeNull();

    rerender(
      <ReasoningBlock thinkingContent="No web" isThinking searchSources={[]} />,
    );
    expect(screen.queryByTestId('reasoning-sources-chip')).toBeNull();
  });

  it('shows the sources chip while pending when sources are already known', () => {
    render(
      <ReasoningBlock
        isThinking={false}
        isPending
        pendingLabel="Starting…"
        searchSources={[{ title: 'A', url: 'https://example.com/a' }]}
      />,
    );
    expect(screen.getByTestId('reasoning-pending')).toBeInTheDocument();
    expect(
      screen.getByTestId('reasoning-sources-chip-label'),
    ).toHaveTextContent('1 source');
  });
});
