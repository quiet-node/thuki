import { fireEvent, render, screen } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { AutoPrimeSkippedStrip } from '../AutoPrimeSkippedStrip';
import { INSUFFICIENT_MEMORY_CONSEQUENCE } from '../ErrorCard';

describe('AutoPrimeSkippedStrip', () => {
  it('shows the model name and need-vs-available GB figures', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={8 * 1024 ** 3}
        availableBytes={4 * 1024 ** 3}
        ceilingFraction={0.8}
        onSwitchModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    expect(
      screen.getByText(
        'Qwen3.5 9B may not fit in memory (~8.0 GB needed, ~4.0 GB available, over the 80% safe limit)',
      ),
    ).toBeInTheDocument();
  });

  it('derives the percent from ceilingFraction, not a hardcoded 80', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={8 * 1024 ** 3}
        availableBytes={4 * 1024 ** 3}
        ceilingFraction={0.6}
        onSwitchModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    expect(
      screen.getByText(
        'Qwen3.5 9B may not fit in memory (~8.0 GB needed, ~4.0 GB available, over the 60% safe limit)',
      ),
    ).toBeInTheDocument();
  });

  it('exposes role=status for assistive tech', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        onSwitchModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    expect(screen.getByTestId('auto-prime-skipped-strip')).toHaveAttribute(
      'role',
      'status',
    );
  });

  it("renders the amber accent edge, matching ErrorCard's InsufficientMemory color", () => {
    const { container } = render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        onSwitchModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    const edgeFill = container.querySelector(
      '[data-testid="auto-prime-skipped-strip"] > span > span',
    );
    expect(edgeFill).toHaveStyle({ background: '#f59e0b' });
  });

  it('shows both actions in stage 1', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        onSwitchModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Switch model' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Load anyway' }),
    ).toBeInTheDocument();
  });

  it('calls onSwitchModel when stage-1 "Switch model" is clicked', () => {
    const onSwitchModel = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        onSwitchModel={onSwitchModel}
        onLoadAnyway={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));
    expect(onSwitchModel).toHaveBeenCalledTimes(1);
  });

  it('advances to the consequence stage on the first "Load anyway" click without loading', () => {
    const onLoadAnyway = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        onSwitchModel={vi.fn()}
        onLoadAnyway={onLoadAnyway}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
    // Stage 2: the consequence copy is shown and the load has NOT fired yet.
    expect(
      screen.getByText(INSUFFICIENT_MEMORY_CONSEQUENCE),
    ).toBeInTheDocument();
    expect(onLoadAnyway).not.toHaveBeenCalled();
    // Both actions are still present, with roles swapped: the confirm button
    // is now labelled "Acknowledge", and stage 1's "Load anyway" is gone.
    expect(
      screen.queryByRole('button', { name: 'Load anyway' }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Acknowledge' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Switch model' }),
    ).toBeInTheDocument();
  });

  it('force-loads on the stage-2 "Acknowledge" click', () => {
    const onLoadAnyway = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        onSwitchModel={vi.fn()}
        onLoadAnyway={onLoadAnyway}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
    fireEvent.click(screen.getByRole('button', { name: 'Acknowledge' }));
    expect(onLoadAnyway).toHaveBeenCalledTimes(1);
  });

  it('calls onSwitchModel from the stage-2 "Switch model" action', () => {
    const onSwitchModel = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        onSwitchModel={onSwitchModel}
        onLoadAnyway={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
    fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));
    expect(onSwitchModel).toHaveBeenCalledTimes(1);
  });
});
