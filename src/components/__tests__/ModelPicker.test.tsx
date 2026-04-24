import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ModelPicker } from '../ModelPicker';

/** Renders a ModelPicker with a small default model list. */
function renderPicker(
  overrides: Partial<React.ComponentProps<typeof ModelPicker>> = {},
) {
  const props: React.ComponentProps<typeof ModelPicker> = {
    activeModel: 'gemma4:e2b',
    models: ['gemma4:e2b', 'qwen2.5:7b'],
    disabled: false,
    onSelect: vi.fn(),
    ...overrides,
  };
  return { props, ...render(<ModelPicker {...props} />) };
}

describe('ModelPicker', () => {
  it('does not render when models list is empty', () => {
    const { container } = renderPicker({ models: [], activeModel: '' });
    expect(container.firstChild).toBeNull();
    expect(screen.queryByRole('button', { name: 'Choose model' })).toBeNull();
  });

  it('renders the Choose model trigger with chip icon', () => {
    const { container } = renderPicker();
    const trigger = screen.getByRole('button', { name: 'Choose model' });
    expect(trigger).toBeInTheDocument();
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
    // The chip icon is rendered inside the trigger.
    expect(container.querySelector('svg')).not.toBeNull();
  });

  it('opens menu on click with aria-expanded true', () => {
    renderPicker();
    const trigger = screen.getByRole('button', { name: 'Choose model' });
    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('menu')).toBeInTheDocument();
  });

  it('portals the menu to document.body (not inside trigger DOM)', () => {
    const { container } = renderPicker();
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    const menu = screen.getByRole('menu');
    // Menu is NOT inside the component container.
    expect(container.contains(menu)).toBe(false);
    // Menu IS a descendant of document.body.
    expect(document.body.contains(menu)).toBe(true);
  });

  it('lists each model slug LEFT of the check icon', () => {
    renderPicker();
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    const firstRow = screen.getByRole('menuitem', { name: 'gemma4:e2b' });
    const slug = firstRow.querySelector('span');
    const check = firstRow.querySelector('svg');
    expect(slug).not.toBeNull();
    expect(check).not.toBeNull();
    expect(slug!.textContent).toBe('gemma4:e2b');
    // DOM order: slug precedes the check svg.
    const children = Array.from(firstRow.children);
    expect(children.indexOf(slug!)).toBeLessThan(children.indexOf(check!));
  });

  it('marks only the active row with visible check (opacity 1 via inline style)', () => {
    renderPicker();
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    const activeRow = screen.getByRole('menuitem', { name: 'gemma4:e2b' });
    const inactiveRow = screen.getByRole('menuitem', { name: 'qwen2.5:7b' });
    expect(activeRow).toHaveAttribute('aria-current', 'true');
    expect(inactiveRow).not.toHaveAttribute('aria-current');
    const activeCheck = activeRow.querySelector('svg')!;
    const inactiveCheck = inactiveRow.querySelector('svg')!;
    expect((activeCheck as SVGElement).style.opacity).toBe('1');
    expect((inactiveCheck as SVGElement).style.opacity).toBe('0');
  });

  it('calls onSelect and closes when row clicked', () => {
    const onSelect = vi.fn();
    renderPicker({ onSelect });
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'qwen2.5:7b' }));
    expect(onSelect).toHaveBeenCalledWith('qwen2.5:7b');
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('closes on outside click (document mousedown)', () => {
    renderPicker();
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(screen.getByRole('menu')).toBeInTheDocument();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('keeps the menu open on mousedown inside the menu', () => {
    renderPicker();
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    const row = screen.getByRole('menuitem', { name: 'qwen2.5:7b' });
    fireEvent.mouseDown(row);
    expect(screen.getByRole('menu')).toBeInTheDocument();
  });

  it('keeps the menu open on mousedown on the trigger itself', () => {
    renderPicker();
    const trigger = screen.getByRole('button', { name: 'Choose model' });
    fireEvent.click(trigger);
    fireEvent.mouseDown(trigger);
    expect(screen.getByRole('menu')).toBeInTheDocument();
  });

  it('closes on Escape key', () => {
    renderPicker();
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(screen.getByRole('menu')).toBeInTheDocument();
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('ignores non-Escape document keydown events', () => {
    renderPicker();
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    fireEvent.keyDown(document, { key: 'ArrowDown' });
    expect(screen.getByRole('menu')).toBeInTheDocument();
  });

  it('closes when disabled flips true mid-open', () => {
    const { rerender, props } = renderPicker();
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(screen.getByRole('menu')).toBeInTheDocument();
    rerender(<ModelPicker {...props} disabled={true} />);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('does not fire onSelect when trigger is disabled (click ignored)', () => {
    const onSelect = vi.fn();
    renderPicker({ disabled: true, onSelect });
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(screen.queryByRole('menu')).toBeNull();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('repositions on window resize without throwing', () => {
    renderPicker();
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(() => {
      act(() => {
        window.dispatchEvent(new Event('resize'));
      });
    }).not.toThrow();
    expect(screen.getByRole('menu')).toBeInTheDocument();
  });

  it('repositions on window scroll without throwing', () => {
    renderPicker();
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(() => {
      act(() => {
        window.dispatchEvent(new Event('scroll'));
      });
    }).not.toThrow();
    expect(screen.getByRole('menu')).toBeInTheDocument();
  });

  it('clicking the trigger while open toggles closed', () => {
    renderPicker();
    const trigger = screen.getByRole('button', { name: 'Choose model' });
    fireEvent.click(trigger);
    expect(screen.getByRole('menu')).toBeInTheDocument();
    fireEvent.click(trigger);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('opens below the trigger when there is no room above', () => {
    // Force the trigger rect to read as being at the very top of the viewport
    // so the above-trigger math would go negative and the menu must flip below.
    const originalGetRect = Element.prototype.getBoundingClientRect;
    Element.prototype.getBoundingClientRect = function () {
      return {
        top: 0,
        left: 100,
        right: 128,
        bottom: 28,
        width: 28,
        height: 28,
        x: 100,
        y: 0,
        toJSON() {
          return {};
        },
      } as DOMRect;
    };
    try {
      renderPicker();
      fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
      const menu = screen.getByRole('menu');
      // Top coordinate should be non-negative since we flipped below.
      const topPx = parseFloat((menu as HTMLElement).style.top);
      expect(topPx).toBeGreaterThanOrEqual(0);
    } finally {
      Element.prototype.getBoundingClientRect = originalGetRect;
    }
  });

  it('clamps left to the 8px edge when the trigger is near the left edge', () => {
    // Trigger far to the left so right-align math would produce a negative left.
    const originalGetRect = Element.prototype.getBoundingClientRect;
    Element.prototype.getBoundingClientRect = function () {
      return {
        top: 500,
        left: 0,
        right: 28,
        bottom: 528,
        width: 28,
        height: 28,
        x: 0,
        y: 500,
        toJSON() {
          return {};
        },
      } as DOMRect;
    };
    try {
      renderPicker();
      fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
      const menu = screen.getByRole('menu');
      const leftPx = parseFloat((menu as HTMLElement).style.left);
      expect(leftPx).toBe(8);
    } finally {
      Element.prototype.getBoundingClientRect = originalGetRect;
    }
  });
});
