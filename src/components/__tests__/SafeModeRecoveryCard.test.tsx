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

  it('does not paint a warm halo behind the primary button (the halo itself read as a ring)', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    const primaryButton = screen.getByRole('button', {
      name: 'Choose a different model',
    });
    expect(primaryButton.style.boxShadow).toBe('');
  });

  it('re-asserts focus onto the dialog container when the window regains focus while a card button holds it, since that means WebKit assigned focus unprompted', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    const dialog = screen.getByRole('dialog');
    const primaryButton = screen.getByRole('button', {
      name: 'Choose a different model',
    });
    // Simulate WebKit assigning focus to the first focusable element when
    // the NSPanel becomes key, independent of the mount-time rAF focus.
    primaryButton.focus();
    expect(document.activeElement).toBe(primaryButton);

    window.dispatchEvent(new Event('focus'));

    expect(document.activeElement).toBe(dialog);
  });

  it('leaves the dialog container focused (no-op) when the window regains focus and nothing has moved focus onto a button', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    const dialog = screen.getByRole('dialog');
    expect(document.activeElement).toBe(dialog);

    window.dispatchEvent(new Event('focus'));

    expect(document.activeElement).toBe(dialog);
  });

  it('does not steal focus back after a deliberate Tab out of the dialog container', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    const dialog = screen.getByRole('dialog');
    const primaryButton = screen.getByRole('button', {
      name: 'Choose a different model',
    });

    fireEvent.keyDown(dialog, { key: 'Tab' });
    primaryButton.focus();
    expect(document.activeElement).toBe(primaryButton);

    window.dispatchEvent(new Event('focus'));

    expect(document.activeElement).toBe(primaryButton);
  });

  it('ignores keys other than Tab/arrow on the container, so focus is still reclaimed afterward', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    const dialog = screen.getByRole('dialog');
    const primaryButton = screen.getByRole('button', {
      name: 'Choose a different model',
    });

    fireEvent.keyDown(dialog, { key: 'a' });
    primaryButton.focus();
    window.dispatchEvent(new Event('focus'));

    expect(document.activeElement).toBe(dialog);
  });

  it('treats an arrow keydown on the container as deliberate interaction too', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    const dialog = screen.getByRole('dialog');
    const secondaryButton = screen.getByRole('button', {
      name: 'Load last model anyway',
    });

    fireEvent.keyDown(dialog, { key: 'ArrowDown' });
    secondaryButton.focus();
    window.dispatchEvent(new Event('focus'));

    expect(document.activeElement).toBe(secondaryButton);
  });

  it('reclaims focus from the secondary button too, not just the primary one', () => {
    render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );
    const dialog = screen.getByRole('dialog');
    const secondaryButton = screen.getByRole('button', {
      name: 'Load last model anyway',
    });

    secondaryButton.focus();
    window.dispatchEvent(new Event('focus'));

    expect(document.activeElement).toBe(dialog);
  });

  it('cleans up the window focus listener and cancels the pending animation frame on unmount', () => {
    const addEventListenerSpy = vi.spyOn(window, 'addEventListener');
    const removeEventListenerSpy = vi.spyOn(window, 'removeEventListener');
    const cancelAnimationFrameSpy = vi.spyOn(
      globalThis,
      'cancelAnimationFrame',
    );

    const { unmount } = render(
      <SafeModeRecoveryCard
        modelName="Qwen3.5 9B"
        sizeGb="8.4"
        onChooseDifferentModel={vi.fn()}
        onLoadAnyway={vi.fn()}
      />,
    );

    const [, focusHandler] = addEventListenerSpy.mock.calls.find(
      ([eventName]) => eventName === 'focus',
    )!;

    unmount();

    expect(removeEventListenerSpy).toHaveBeenCalledWith('focus', focusHandler);
    expect(cancelAnimationFrameSpy).toHaveBeenCalled();

    addEventListenerSpy.mockRestore();
    removeEventListenerSpy.mockRestore();
    cancelAnimationFrameSpy.mockRestore();
  });
});
