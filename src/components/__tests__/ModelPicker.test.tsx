import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ModelPickerList, ModelPickerTrigger } from '../ModelPicker';

// ─── ModelPickerTrigger ─────────────────────────────────────────────────────

describe('ModelPickerTrigger', () => {
  it('renders the Choose model button with aria-expanded=false when closed', () => {
    render(
      <ModelPickerTrigger isOpen={false} disabled={false} onToggle={vi.fn()} />,
    );
    const trigger = screen.getByRole('button', { name: 'Choose model' });
    expect(trigger).toBeInTheDocument();
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
  });

  it('renders with aria-expanded=true when open', () => {
    render(
      <ModelPickerTrigger isOpen={true} disabled={false} onToggle={vi.fn()} />,
    );
    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toHaveAttribute('aria-expanded', 'true');
  });

  it('fires onToggle on click', () => {
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

// ─── ModelPickerList ────────────────────────────────────────────────────────

describe('ModelPickerList', () => {
  const DEFAULT_MODELS = ['gemma4:e2b', 'qwen2.5:7b'];

  it('renders nothing when closed', () => {
    render(
      <ModelPickerList
        activeModel="gemma4:e2b"
        models={DEFAULT_MODELS}
        isOpen={false}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('renders nothing when models list is empty', () => {
    render(
      <ModelPickerList
        activeModel=""
        models={[]}
        isOpen={true}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('renders rows with slug on the left and check on the right when open', () => {
    render(
      <ModelPickerList
        activeModel="gemma4:e2b"
        models={DEFAULT_MODELS}
        isOpen={true}
        onSelect={vi.fn()}
      />,
    );
    const firstRow = screen.getByRole('menuitem', { name: 'gemma4:e2b' });
    const slug = firstRow.querySelector('span');
    const check = firstRow.querySelector('svg');
    expect(slug).not.toBeNull();
    expect(check).not.toBeNull();
    expect(slug!.textContent).toBe('gemma4:e2b');
    const children = Array.from(firstRow.children);
    expect(children.indexOf(slug!)).toBeLessThan(children.indexOf(check!));
  });

  it('marks only the active row with visible check (opacity 1)', () => {
    render(
      <ModelPickerList
        activeModel="gemma4:e2b"
        models={DEFAULT_MODELS}
        isOpen={true}
        onSelect={vi.fn()}
      />,
    );
    const activeRow = screen.getByRole('menuitem', { name: 'gemma4:e2b' });
    const inactiveRow = screen.getByRole('menuitem', { name: 'qwen2.5:7b' });
    expect(activeRow).toHaveAttribute('aria-current', 'true');
    expect(inactiveRow).not.toHaveAttribute('aria-current');
    const activeCheck = activeRow.querySelector('svg') as SVGElement;
    const inactiveCheck = inactiveRow.querySelector('svg') as SVGElement;
    expect(activeCheck.style.opacity).toBe('1');
    expect(inactiveCheck.style.opacity).toBe('0');
  });

  it('fires onSelect with the chosen slug when a row is clicked', () => {
    const onSelect = vi.fn();
    render(
      <ModelPickerList
        activeModel="gemma4:e2b"
        models={DEFAULT_MODELS}
        isOpen={true}
        onSelect={onSelect}
      />,
    );
    fireEvent.click(screen.getByRole('menuitem', { name: 'qwen2.5:7b' }));
    expect(onSelect).toHaveBeenCalledWith('qwen2.5:7b');
  });
});
