import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ModelPickerList, ModelPickerTrigger } from '../ModelPicker';

describe('ModelPickerTrigger', () => {
  it('exposes a Choose model button with aria-expanded reflecting open state', () => {
    const { rerender } = render(
      <ModelPickerTrigger isOpen={false} disabled={false} onToggle={vi.fn()} />,
    );

    const trigger = screen.getByRole('button', { name: 'Choose model' });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');

    rerender(
      <ModelPickerTrigger isOpen={true} disabled={false} onToggle={vi.fn()} />,
    );
    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toHaveAttribute('aria-expanded', 'true');
  });

  it('fires onToggle when clicked', () => {
    const onToggle = vi.fn();
    render(
      <ModelPickerTrigger
        isOpen={false}
        disabled={false}
        onToggle={onToggle}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it('does not fire onToggle when disabled', () => {
    const onToggle = vi.fn();
    render(
      <ModelPickerTrigger isOpen={false} disabled={true} onToggle={onToggle} />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(onToggle).not.toHaveBeenCalled();
  });
});

describe('ModelPickerList', () => {
  it('renders nothing when closed', () => {
    const { container } = render(
      <ModelPickerList
        activeModel="gemma4:e2b"
        models={['gemma4:e2b', 'qwen2.5:7b']}
        isOpen={false}
        onSelect={vi.fn()}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('renders nothing when models list is empty', () => {
    const { container } = render(
      <ModelPickerList
        activeModel=""
        models={[]}
        isOpen={true}
        onSelect={vi.fn()}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('renders a slug-only row per model when open and highlights the active row', () => {
    render(
      <ModelPickerList
        activeModel="gemma4:e2b"
        models={['gemma4:e2b', 'qwen2.5:7b']}
        isOpen={true}
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByRole('button', { name: 'gemma4:e2b' })).toHaveClass(
      'bg-primary/10',
    );
    expect(
      screen.getByRole('button', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();
    expect(screen.queryByText(/fast|vision|recent/i)).toBeNull();
  });

  it('marks the active row with aria-current', () => {
    render(
      <ModelPickerList
        activeModel="gemma4:e2b"
        models={['gemma4:e2b', 'qwen2.5:7b']}
        isOpen={true}
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByRole('button', { name: 'gemma4:e2b' })).toHaveAttribute(
      'aria-current',
      'true',
    );
    expect(
      screen.getByRole('button', { name: 'qwen2.5:7b' }),
    ).not.toHaveAttribute('aria-current');
  });

  it('calls onSelect with the chosen slug when a row is clicked', () => {
    const onSelect = vi.fn();
    render(
      <ModelPickerList
        activeModel="gemma4:e2b"
        models={['gemma4:e2b', 'qwen2.5:7b']}
        isOpen={true}
        onSelect={onSelect}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'qwen2.5:7b' }));
    expect(onSelect).toHaveBeenCalledWith('qwen2.5:7b');
  });
});
