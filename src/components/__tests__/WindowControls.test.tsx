import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { WindowControls } from '../WindowControls';

describe('WindowControls', () => {
  it('close button calls onClose when clicked', () => {
    const onClose = vi.fn();
    render(<WindowControls onClose={onClose} />);
    fireEvent.click(screen.getByRole('button', { name: 'Close window' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('close button has correct styling (bg-[#FF5F57])', () => {
    const { container } = render(<WindowControls onClose={vi.fn()} />);
    const closeBtn = container.querySelector('.bg-\\[\\#FF5F57\\]');
    expect(closeBtn).not.toBeNull();
  });

  it('renders decorative minimize and zoom dots (aria-hidden elements)', () => {
    const { container } = render(<WindowControls onClose={vi.fn()} />);
    const hiddenDots = container.querySelectorAll('[aria-hidden="true"]');
    // The two decorative divs (minimize + zoom) plus SVG inside close button = 3
    // but we only care that at least 2 non-button aria-hidden elements exist
    const decorativeDivs = Array.from(hiddenDots).filter(
      (el) => el.tagName.toLowerCase() === 'div',
    );
    expect(decorativeDivs).toHaveLength(2);
  });

  it('renders divider separator (bg-surface-border)', () => {
    const { container } = render(<WindowControls onClose={vi.fn()} />);
    expect(container.querySelector('.bg-surface-border')).not.toBeNull();
  });

  it('close button has x icon svg', () => {
    render(<WindowControls onClose={vi.fn()} />);
    const closeBtn = screen.getByRole('button', { name: 'Close window' });
    const svg = closeBtn.querySelector('svg');
    expect(svg).not.toBeNull();
  });

  it('save button shows "Save conversation" aria-label when not saved', () => {
    render(
      <WindowControls
        onClose={vi.fn()}
        onSave={vi.fn()}
        canSave
        isSaved={false}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Save conversation' }),
    ).toBeInTheDocument();
  });

  it('save button shows "Remove from history" aria-label when saved', () => {
    render(
      <WindowControls onClose={vi.fn()} onSave={vi.fn()} canSave isSaved />,
    );
    expect(
      screen.getByRole('button', { name: 'Remove from history' }),
    ).toBeInTheDocument();
  });

  it('save button calls onSave when clicked while saved', () => {
    const onSave = vi.fn();
    render(
      <WindowControls onClose={vi.fn()} onSave={onSave} canSave isSaved />,
    );
    fireEvent.click(
      screen.getByRole('button', { name: 'Remove from history' }),
    );
    expect(onSave).toHaveBeenCalledTimes(1);
  });
});
