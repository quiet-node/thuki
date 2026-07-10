import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { SafeModeRecoveryCard } from '../SafeModeRecoveryCard';

describe('SafeModeRecoveryCard', () => {
  it('renders the locked headline', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    expect(screen.getByText('Recovered in Safe Mode')).toBeInTheDocument();
  });

  it('interpolates the model name and size into the locked body copy', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    expect(
      screen.getByText(
        'Qwen3.5 9B (8.4 GB) was loading when the last session ended unexpectedly, possibly because it needed more memory than was available.',
      ),
    ).toBeInTheDocument();
  });

  it('renders the primary and secondary buttons', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Choose a different model' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Load last model anyway' }),
    ).toBeInTheDocument();
  });

  it('calls onChooseDifferentModel when the primary button is clicked', () => {
    const onChooseDifferentModel = vi.fn();
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={onChooseDifferentModel}
        onLoadAnyway={vi.fn()}
      />,
    );
    fireEvent.click(
      screen.getByRole('button', { name: 'Choose a different model' }),
    );
    expect(onChooseDifferentModel).toHaveBeenCalledOnce();
  });

  it('calls onLoadAnyway when the secondary button is clicked', () => {
    const onLoadAnyway = vi.fn();
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={onLoadAnyway}
      />,
    );
    fireEvent.click(
      screen.getByRole('button', { name: 'Load last model anyway' }),
    );
    expect(onLoadAnyway).toHaveBeenCalledOnce();
  });

  it('focuses the dialog container on mount, not either button, so no unprompted focus ring paints on a button', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    expect(document.activeElement).toBe(screen.getByRole('dialog'));
    expect(document.activeElement).not.toBe(
      screen.getByRole('button', { name: 'Choose a different model' }),
    );
    expect(document.activeElement).not.toBe(
      screen.getByRole('button', { name: 'Load last model anyway' }),
    );
  });

  it('exposes aria-modal and labels the dialog by its heading', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    const heading = screen.getByText('Recovered in Safe Mode');
    expect(dialog.getAttribute('aria-labelledby')).toBe(heading.id);
  });
});
