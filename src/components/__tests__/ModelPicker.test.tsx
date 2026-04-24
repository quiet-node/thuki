import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ModelPicker } from '../ModelPicker';

function renderTrigger(
  overrides: Partial<React.ComponentProps<typeof ModelPicker>> = {},
) {
  const props: React.ComponentProps<typeof ModelPicker> = {
    onClick: vi.fn(),
    disabled: false,
    isOpen: false,
    ...overrides,
  };
  return { props, ...render(<ModelPicker {...props} />) };
}

describe('ModelPicker', () => {
  it('renders the Choose model trigger button with chip icon', () => {
    const { container } = renderTrigger();
    const trigger = screen.getByRole('button', { name: 'Choose model' });
    expect(trigger).toBeInTheDocument();
    expect(container.querySelector('svg')).not.toBeNull();
  });

  it('sets aria-expanded false when isOpen is false', () => {
    renderTrigger({ isOpen: false });
    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toHaveAttribute('aria-expanded', 'false');
  });

  it('sets aria-expanded true when isOpen is true', () => {
    renderTrigger({ isOpen: true });
    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toHaveAttribute('aria-expanded', 'true');
  });

  it('calls onClick when clicked', () => {
    const onClick = vi.fn();
    renderTrigger({ onClick });
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('is disabled and does not call onClick when disabled', () => {
    const onClick = vi.fn();
    renderTrigger({ disabled: true, onClick });
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(onClick).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Choose model' })).toBeDisabled();
  });
});
