import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { SwitchConfirmation } from '../SwitchConfirmation';

describe('SwitchConfirmation', () => {
  it('renders the confirmation prompt text', () => {
    render(
      <SwitchConfirmation
        onSaveAndSwitch={vi.fn()}
        onJustSwitch={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(screen.getByText(/switch conversations/i)).toBeInTheDocument();
  });

  it('renders Save & Switch button', () => {
    render(
      <SwitchConfirmation
        onSaveAndSwitch={vi.fn()}
        onJustSwitch={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(
      screen.getByRole('button', { name: /save & switch/i }),
    ).toBeInTheDocument();
  });

  it('renders Just Switch button', () => {
    render(
      <SwitchConfirmation
        onSaveAndSwitch={vi.fn()}
        onJustSwitch={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(
      screen.getByRole('button', { name: /just switch/i }),
    ).toBeInTheDocument();
  });

  it('calls onSaveAndSwitch when Save & Switch is clicked', () => {
    const onSaveAndSwitch = vi.fn();
    render(
      <SwitchConfirmation
        onSaveAndSwitch={onSaveAndSwitch}
        onJustSwitch={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /save & switch/i }));
    expect(onSaveAndSwitch).toHaveBeenCalledOnce();
  });

  it('calls onJustSwitch when Just Switch is clicked', () => {
    const onJustSwitch = vi.fn();
    render(
      <SwitchConfirmation
        onSaveAndSwitch={vi.fn()}
        onJustSwitch={onJustSwitch}
        onCancel={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /just switch/i }));
    expect(onJustSwitch).toHaveBeenCalledOnce();
  });

  it('calls onCancel when cancel/back is clicked', () => {
    const onCancel = vi.fn();
    render(
      <SwitchConfirmation
        onSaveAndSwitch={vi.fn()}
        onJustSwitch={vi.fn()}
        onCancel={onCancel}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /cancel/i }));
    expect(onCancel).toHaveBeenCalledOnce();
  });
});
