import { fireEvent, render, screen } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { AutoPrimeSkippedStrip } from '../AutoPrimeSkippedStrip';

describe('AutoPrimeSkippedStrip', () => {
  it('shows the model name and need-vs-available GB figures', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={8 * 1024 ** 3}
        availableBytes={4 * 1024 ** 3}
        onSwitchModel={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );
    expect(
      screen.getByText(
        'Qwen3.5 9B may not fit in memory (~8.0 GB needed, ~4.0 GB available)',
      ),
    ).toBeInTheDocument();
  });

  it('exposes role=status for assistive tech', () => {
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        onSwitchModel={vi.fn()}
        onDismiss={vi.fn()}
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
        onSwitchModel={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );
    const edgeFill = container.querySelector(
      '[data-testid="auto-prime-skipped-strip"] > span > span',
    );
    expect(edgeFill).toHaveStyle({ background: '#f59e0b' });
  });

  it('calls onSwitchModel when "Switch model" is clicked', () => {
    const onSwitchModel = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        onSwitchModel={onSwitchModel}
        onDismiss={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Switch model' }));
    expect(onSwitchModel).toHaveBeenCalledTimes(1);
  });

  it('calls onDismiss when "Dismiss" is clicked', () => {
    const onDismiss = vi.fn();
    render(
      <AutoPrimeSkippedStrip
        modelName="Qwen3.5 9B"
        requiredBytes={1}
        availableBytes={1}
        onSwitchModel={vi.fn()}
        onDismiss={onDismiss}
      />,
    );
    fireEvent.click(
      screen.getByRole('button', { name: 'Dismiss memory warning' }),
    );
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
