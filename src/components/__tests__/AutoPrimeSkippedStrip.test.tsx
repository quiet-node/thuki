import { fireEvent, render, screen } from '@testing-library/react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import { AutoPrimeSkippedStrip } from '../AutoPrimeSkippedStrip';
import { INSUFFICIENT_MEMORY_CONSEQUENCE } from '../ErrorCard';
import { mockReducedMotion } from '../../testUtils/mocks/framer-motion';

describe('AutoPrimeSkippedStrip', () => {
  afterEach(() => {
    mockReducedMotion.current = false;
  });

  it('shows the model name and need-vs-available GB figures', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={8 * 1024 ** 3}
        availableBytes={4 * 1024 ** 3}
        ceilingFraction={0.8}
        canRemember={true}
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
        canRemember={true}
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
        canRemember={true}
        onSwitchModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    expect(screen.getByTestId('auto-prime-skipped-strip')).toHaveAttribute(
      'role',
      'status',
    );
  });

  it('shows amber status dot and row actions under primary text (no top bar)', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        canRemember={true}
        onSwitchModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    const root = screen.getByTestId('auto-prime-skipped-strip');
    expect(screen.getByTestId('auto-prime-skipped-dot')).toHaveStyle({
      background: '#f59e0b',
    });
    // No full-width top amber track.
    expect(root.querySelector(':scope > span > span')).toBeNull();
    expect(root.querySelector('.flex-wrap')).toBeTruthy();
  });

  it('shows both actions in stage 1', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        canRemember={true}
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
        canRemember={true}
        onSwitchModel={onSwitchModel}
        onLoadAnyway={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));
    expect(onSwitchModel).toHaveBeenCalledTimes(1);
  });

  it('advances to stage 2 on "Load anyway" without loading, showing the consequence and split actions (mild band)', () => {
    const onLoadAnyway = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        canRemember={true}
        onSwitchModel={vi.fn()}
        onLoadAnyway={onLoadAnyway}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
    // Fit line stays; consequence appears muted under it; load has NOT fired.
    expect(
      screen.getByText(
        'Qwen3.5 9B may not fit in memory (~0.0 GB needed, ~0.0 GB available, over the 80% safe limit)',
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByTestId('auto-prime-skipped-consequence'),
    ).toHaveTextContent(INSUFFICIENT_MEMORY_CONSEQUENCE);
    expect(onLoadAnyway).not.toHaveBeenCalled();
    // Stage-1 "Load anyway" is gone; the mild-band split now shows.
    expect(
      screen.queryByRole('button', { name: 'Load anyway' }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Load once' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Always allow this model' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Switch model' }),
    ).toBeInTheDocument();
  });

  it('mild band stage 2: "Load once" fires onLoadAnyway(false)', () => {
    const onLoadAnyway = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        canRemember={true}
        onSwitchModel={vi.fn()}
        onLoadAnyway={onLoadAnyway}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
    fireEvent.click(screen.getByRole('button', { name: 'Load once' }));
    expect(onLoadAnyway).toHaveBeenCalledTimes(1);
    expect(onLoadAnyway).toHaveBeenCalledWith(false);
  });

  it('mild band stage 2: "Always allow this model" fires onLoadAnyway(true)', () => {
    const onLoadAnyway = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        canRemember={true}
        onSwitchModel={vi.fn()}
        onLoadAnyway={onLoadAnyway}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
    fireEvent.click(
      screen.getByRole('button', { name: 'Always allow this model' }),
    );
    expect(onLoadAnyway).toHaveBeenCalledTimes(1);
    expect(onLoadAnyway).toHaveBeenCalledWith(true);
  });

  it('freeze band states the danger up front: chip, title, note and both actions with no click', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={8 * 1024 ** 3}
        availableBytes={4 * 1024 ** 3}
        ceilingFraction={0.8}
        canRemember={false}
        onSwitchModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    // Everything is stated up front: no "Load anyway" advance step first.
    const chip = screen.getByTestId('memory-critical-chip') as HTMLElement;
    expect(chip).toHaveTextContent('Memory critically low');
    expect(chip.style.color).toBe('rgb(248, 113, 113)');
    expect(chip.style.background).toBe('rgba(248, 113, 113, 0.12)');
    expect(chip.style.border).toBe('1px solid rgba(248, 113, 113, 0.35)');
    expect(chip.style.borderRadius).toBe('5px');
    // The tag carries no dot, and it replaces the strip's amber status dot, so
    // this band shows exactly one indicator.
    expect(chip.querySelector('span')).toBeNull();
    expect(screen.queryByTestId('auto-prime-skipped-dot')).toBeNull();
    expect(
      screen.getByText('Only ~4.0 GB free. Qwen3.5 9B needs ~8.0 GB.'),
    ).toBeInTheDocument();
    expect(screen.getByTestId('memory-freeze-note')).toHaveTextContent(
      'That is far too tight to load on its own. Thuki always asks at this level, because loading can slow your Mac badly or freeze it.',
    );
    // The old staged copy is gone in this band.
    expect(screen.queryByTestId('auto-prime-skipped-consequence')).toBeNull();
    expect(screen.queryByText(/may not fit in memory \(/)).toBeNull();
    // Exactly Load anyway + Switch model; no Acknowledge, no Always allow.
    expect(screen.getAllByRole('button')).toHaveLength(2);
    expect(
      screen.queryByRole('button', { name: 'Always allow this model' }),
    ).toBeNull();
    expect(screen.queryByRole('button', { name: 'Acknowledge' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Load once' })).toBeNull();
  });

  it('freeze band "Load anyway" only advances, so the riskiest load is never one click', () => {
    const onLoadAnyway = vi.fn();
    const onSwitchModel = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={8 * 1024 ** 3}
        availableBytes={4 * 1024 ** 3}
        ceilingFraction={0.8}
        canRemember={false}
        onSwitchModel={onSwitchModel}
        onLoadAnyway={onLoadAnyway}
      />,
    );
    // First click must NOT load: a stray click on an unprompted strip cannot
    // wire memory the machine does not have.
    fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
    expect(onLoadAnyway).not.toHaveBeenCalled();
    // Stage 2 offers the one-time force only: no remember at this ratio.
    expect(screen.queryByRole('button', { name: 'Load anyway' })).toBeNull();
    expect(
      screen.queryByRole('button', { name: 'Always allow this model' }),
    ).toBeNull();
    // The severity copy stays put across both stages.
    expect(screen.getByTestId('memory-critical-chip')).toBeInTheDocument();
    expect(screen.getByTestId('memory-freeze-note')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Load once' }));
    expect(onLoadAnyway).toHaveBeenCalledTimes(1);
    expect(onLoadAnyway).toHaveBeenCalledWith(false);
    fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));
    expect(onSwitchModel).toHaveBeenCalledTimes(1);
  });

  it('mild band renders no chip and no freeze note in either stage', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        canRemember={true}
        onSwitchModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    expect(screen.queryByTestId('memory-freeze-note')).toBeNull();
    expect(screen.queryByTestId('memory-critical-chip')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
    expect(
      screen.getByTestId('auto-prime-skipped-consequence'),
    ).toBeInTheDocument();
    expect(screen.queryByTestId('memory-freeze-note')).toBeNull();
    expect(screen.queryByTestId('memory-critical-chip')).toBeNull();
  });

  it('calls onSwitchModel from the stage-2 "Switch model" action', () => {
    const onSwitchModel = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        canRemember={true}
        onSwitchModel={onSwitchModel}
        onLoadAnyway={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
    fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));
    expect(onSwitchModel).toHaveBeenCalledTimes(1);
  });

  it('still shows the consequence copy on confirm under prefers-reduced-motion', () => {
    mockReducedMotion.current = true;
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        ceilingFraction={0.8}
        canRemember={true}
        onSwitchModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Load anyway' }));
    expect(
      screen.getByTestId('auto-prime-skipped-consequence'),
    ).toHaveTextContent(INSUFFICIENT_MEMORY_CONSEQUENCE);
  });
});
