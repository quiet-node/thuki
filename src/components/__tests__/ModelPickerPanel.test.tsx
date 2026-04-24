import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ModelPickerPanel } from '../ModelPickerPanel';

const MODELS = ['gemma4:e2b', 'qwen2.5:7b', 'llama3.2:3b'];

function renderPanel(
  overrides: Partial<React.ComponentProps<typeof ModelPickerPanel>> = {},
) {
  const props: React.ComponentProps<typeof ModelPickerPanel> = {
    models: MODELS,
    activeModel: 'gemma4:e2b',
    onSelect: vi.fn(),
    ...overrides,
  };
  return { props, ...render(<ModelPickerPanel {...props} />) };
}

describe('ModelPickerPanel', () => {
  it('renders filter input', () => {
    renderPanel();
    expect(screen.getByPlaceholderText(/filter models/i)).toBeInTheDocument();
  });

  it('shows all models on first render', () => {
    renderPanel();
    for (const model of MODELS) {
      expect(screen.getByRole('option', { name: model })).toBeInTheDocument();
    }
  });

  it('marks active model with aria-selected true, others false', () => {
    renderPanel({ activeModel: 'qwen2.5:7b' });
    expect(screen.getByRole('option', { name: 'qwen2.5:7b' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.getByRole('option', { name: 'gemma4:e2b' })).toHaveAttribute(
      'aria-selected',
      'false',
    );
    expect(screen.getByRole('option', { name: 'llama3.2:3b' })).toHaveAttribute(
      'aria-selected',
      'false',
    );
  });

  it('shows visible checkmark on active model, hidden on others', () => {
    renderPanel({ activeModel: 'gemma4:e2b' });
    const activeItem = screen.getByRole('option', { name: 'gemma4:e2b' });
    const inactiveItem = screen.getByRole('option', { name: 'qwen2.5:7b' });
    const activeCheck = activeItem.querySelector('svg')!;
    const inactiveCheck = inactiveItem.querySelector('svg')!;
    expect((activeCheck as SVGElement).style.opacity).toBe('1');
    expect((inactiveCheck as SVGElement).style.opacity).toBe('0');
  });

  it('calls onSelect with slug when row clicked', () => {
    const onSelect = vi.fn();
    renderPanel({ onSelect });
    fireEvent.click(screen.getByRole('option', { name: 'qwen2.5:7b' }));
    expect(onSelect).toHaveBeenCalledWith('qwen2.5:7b');
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('filters models as user types', () => {
    renderPanel();
    fireEvent.change(screen.getByPlaceholderText(/filter models/i), {
      target: { value: 'qwen' },
    });
    expect(
      screen.getByRole('option', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();
    expect(screen.queryByRole('option', { name: 'gemma4:e2b' })).toBeNull();
    expect(screen.queryByRole('option', { name: 'llama3.2:3b' })).toBeNull();
  });

  it('shows no-models-found message when filter matches nothing', () => {
    renderPanel();
    fireEvent.change(screen.getByPlaceholderText(/filter models/i), {
      target: { value: 'zzz' },
    });
    expect(screen.getByText(/no models found/i)).toBeInTheDocument();
    expect(screen.queryByRole('option')).toBeNull();
  });

  it('restores full list when filter is cleared', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.change(input, { target: { value: 'qwen' } });
    fireEvent.change(input, { target: { value: '' } });
    for (const model of MODELS) {
      expect(screen.getByRole('option', { name: model })).toBeInTheDocument();
    }
  });

  it('shows no-models-available message when models list is empty', () => {
    renderPanel({ models: [] });
    expect(screen.getByText(/no models available/i)).toBeInTheDocument();
    expect(screen.queryByRole('option')).toBeNull();
  });

  it('marks the filter input as an aria-activedescendant combobox', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    expect(input).toHaveAttribute('role', 'combobox');
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining('option-0'),
    );
  });

  it('ArrowDown advances the highlighted descendant', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining('option-1'),
    );
  });

  it('ArrowUp wraps to the last row from the first', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'ArrowUp' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining(`option-${MODELS.length - 1}`),
    );
  });

  it('Home/End jump to the first and last rows', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'End' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining(`option-${MODELS.length - 1}`),
    );
    fireEvent.keyDown(input, { key: 'Home' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining('option-0'),
    );
  });

  it('Enter commits the highlighted row via onSelect', () => {
    const onSelect = vi.fn();
    renderPanel({ onSelect });
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSelect).toHaveBeenCalledWith('qwen2.5:7b');
  });

  it('Escape fires onClose when provided', () => {
    const onClose = vi.fn();
    renderPanel({ onClose });
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Escape without onClose is a safe no-op', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    // Asserting no throw by calling keyDown; onClose is undefined here.
    fireEvent.keyDown(input, { key: 'Escape' });
  });

  it('keyboard nav on empty filter result is a safe no-op', () => {
    const onSelect = vi.fn();
    renderPanel({ onSelect });
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.change(input, { target: { value: 'zzz' } });
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'ArrowUp' });
    fireEvent.keyDown(input, { key: 'Home' });
    fireEvent.keyDown(input, { key: 'End' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSelect).not.toHaveBeenCalled();
    expect(input).not.toHaveAttribute('aria-activedescendant');
  });

  it('clamps highlighted index when the filtered list shrinks', () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'End' });
    // Narrow the visible set to one row; the activedescendant must clamp to 0.
    fireEvent.change(input, { target: { value: 'qwen' } });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining('option-0'),
    );
  });

  it('mouse-over updates the highlighted descendant', () => {
    renderPanel();
    fireEvent.mouseEnter(screen.getByRole('option', { name: 'llama3.2:3b' }));
    const input = screen.getByPlaceholderText(/filter models/i);
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      expect.stringContaining('option-2'),
    );
  });

  it('ignores other keys without preventing default or firing handlers', () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    renderPanel({ onSelect, onClose });
    const input = screen.getByPlaceholderText(/filter models/i);
    fireEvent.keyDown(input, { key: 'a' });
    expect(onSelect).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });
});
