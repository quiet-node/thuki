import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ModelPicker } from '../ModelPicker';

describe('ModelPicker', () => {
  it('opens a slug-only popup and highlights the active row', () => {
    render(
      <ModelPicker
        activeModel="gemma4:e2b"
        models={['gemma4:e2b', 'qwen2.5:7b']}
        disabled={false}
        onSelect={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));

    expect(screen.getByRole('button', { name: 'gemma4:e2b' })).toHaveClass(
      'bg-primary/10',
    );
    expect(
      screen.getByRole('button', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();
    expect(screen.queryByText(/fast|vision|recent/i)).toBeNull();
  });

  it('calls onSelect and closes after choosing a new model', () => {
    const onSelect = vi.fn();
    render(
      <ModelPicker
        activeModel="gemma4:e2b"
        models={['gemma4:e2b', 'qwen2.5:7b']}
        disabled={false}
        onSelect={onSelect}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    fireEvent.click(screen.getByRole('button', { name: 'qwen2.5:7b' }));

    expect(onSelect).toHaveBeenCalledWith('qwen2.5:7b');
    expect(screen.queryByRole('button', { name: 'gemma4:e2b' })).toBeNull();
  });

  it('returns null when models list is empty', () => {
    const { container } = render(
      <ModelPicker
        activeModel=""
        models={[]}
        disabled={false}
        onSelect={vi.fn()}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('closes when clicking outside', () => {
    render(
      <ModelPicker
        activeModel="gemma4:e2b"
        models={['gemma4:e2b', 'qwen2.5:7b']}
        disabled={false}
        onSelect={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(
      screen.getByRole('button', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();

    fireEvent.mouseDown(document.body);

    expect(screen.queryByRole('button', { name: 'qwen2.5:7b' })).toBeNull();
  });

  it('toggles closed when the trigger is clicked twice', () => {
    render(
      <ModelPicker
        activeModel="gemma4:e2b"
        models={['gemma4:e2b', 'qwen2.5:7b']}
        disabled={false}
        onSelect={vi.fn()}
      />,
    );

    const trigger = screen.getByRole('button', { name: 'Choose model' });
    fireEvent.click(trigger);
    expect(
      screen.getByRole('button', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();

    fireEvent.click(trigger);
    expect(screen.queryByRole('button', { name: 'qwen2.5:7b' })).toBeNull();
  });

  it('ignores clicks when disabled', () => {
    render(
      <ModelPicker
        activeModel="gemma4:e2b"
        models={['gemma4:e2b', 'qwen2.5:7b']}
        disabled={true}
        onSelect={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(screen.queryByRole('button', { name: 'qwen2.5:7b' })).toBeNull();
  });

  it('keeps mousedown inside the picker from closing the popup', () => {
    render(
      <ModelPicker
        activeModel="gemma4:e2b"
        models={['gemma4:e2b', 'qwen2.5:7b']}
        disabled={false}
        onSelect={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    const row = screen.getByRole('button', { name: 'qwen2.5:7b' });
    fireEvent.mouseDown(row);
    expect(
      screen.getByRole('button', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();
  });
});
