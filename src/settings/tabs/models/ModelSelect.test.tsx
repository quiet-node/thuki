import { render, screen, fireEvent, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  ModelSelect,
  computePlacement,
  type ModelSelectItem,
} from './ModelSelect';

const RICH: ModelSelectItem[] = [
  {
    id: 'a',
    label: 'Alpha',
    sub: '6.6 GB · 128K · Org · Q4_K_M',
    vision: true,
    thinking: false,
    fit: 'fits',
  },
  {
    id: 'b',
    label: 'Beta',
    sub: '7.3 GB · 256K · Org · Q4_K_M',
    vision: false,
    thinking: true,
    fit: 'tight',
  },
  {
    id: 'c',
    label: 'Gamma',
    sub: '2.0 GB · 32K · Org · F16',
    vision: false,
    thinking: false,
    fit: 'too_big',
  },
];

const SLUGS: ModelSelectItem[] = [
  { id: 'llama3.2:3b', label: 'llama3.2:3b' },
  { id: 'qwen2.5:14b', label: 'qwen2.5:14b' },
];

function open(ariaLabel = 'Built-in model') {
  fireEvent.click(screen.getByRole('button', { name: ariaLabel }));
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('computePlacement', () => {
  it('drops below the trigger when there is room', () => {
    const p = computePlacement(
      { top: 60, bottom: 100, left: 24, width: 260 },
      1000,
      1440,
    );
    expect(p).toEqual({ top: 106, left: 24, width: 260 });
  });

  it('flips above the trigger when the space below cannot hold it', () => {
    const p = computePlacement(
      { top: 700, bottom: 740, left: 10, width: 200 },
      768,
      1440,
      320,
      6,
    );
    expect(p.top).toBe(374);
    expect(p.left).toBe(10);
  });

  it('stays below when space is cramped both ways but the trigger sits high', () => {
    const p = computePlacement(
      { top: 5, bottom: 45, left: 0, width: 100 },
      60,
      1440,
      320,
      6,
    );
    expect(p.top).toBe(51);
  });

  it('clamps the left edge so the popover stays within the viewport', () => {
    // Trigger near the right edge: left pulls back to keep it on-screen.
    const right = computePlacement(
      { top: 60, bottom: 100, left: 1300, width: 300 },
      1000,
      1440,
    );
    expect(right.left).toBe(1132);
    // Trigger past the left edge: left clamps to the minimum margin.
    const left = computePlacement(
      { top: 60, bottom: 100, left: 2, width: 300 },
      1000,
      1440,
    );
    expect(left.left).toBe(8);
  });
});

describe('ModelSelect', () => {
  it('shows the selected label and a placeholder when nothing matches', () => {
    const { rerender } = render(
      <ModelSelect
        value="b"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
        placeholder="Choose a model"
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Built-in model' }),
    ).toHaveTextContent('Beta');

    rerender(
      <ModelSelect
        value="missing"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
        placeholder="Choose a model"
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Built-in model' }),
    ).toHaveTextContent('Choose a model');
  });

  it('toggles the popover open and closed from the trigger', () => {
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    open();
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    open();
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });

  it('does not open when the trigger cannot be measured', () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(
      undefined as unknown as DOMRect,
    );
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    open();
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });

  it('renders capability pills, sub-line, and a RAM-fit badge per item', () => {
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    open();
    const alpha = screen.getByRole('option', { name: /Alpha/ });
    expect(within(alpha).getByText('Text')).toBeInTheDocument();
    expect(within(alpha).getByText('Vision')).toBeInTheDocument();
    expect(within(alpha).queryByText('Thinking')).not.toBeInTheDocument();
    // The truncated name and sub-line carry the full text as a native tooltip.
    expect(within(alpha).getByText('Alpha')).toHaveAttribute('title', 'Alpha');
    const sub = within(alpha).getByText('6.6 GB · 128K · Org · Q4_K_M');
    expect(sub).toBeInTheDocument();
    expect(sub).toHaveAttribute('title', '6.6 GB · 128K · Org · Q4_K_M');
    expect(within(alpha).getByText('Comfortable')).toHaveAttribute(
      'title',
      'Fits comfortably.',
    );

    const beta = screen.getByRole('option', { name: /Beta/ });
    expect(within(beta).getByText('Thinking')).toBeInTheDocument();
    expect(within(beta).getByText('Tight')).toBeInTheDocument();

    const gamma = screen.getByRole('option', { name: /Gamma/ });
    expect(within(gamma).getByText('Text')).toBeInTheDocument();
    expect(within(gamma).queryByText('Vision')).not.toBeInTheDocument();
    expect(within(gamma).queryByText('Thinking')).not.toBeInTheDocument();
    expect(within(gamma).getByText('Heavy')).toBeInTheDocument();
  });

  it('marks the active row selected', () => {
    render(
      <ModelSelect
        value="b"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    open();
    expect(screen.getByRole('option', { name: /Beta/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.getByRole('option', { name: /Alpha/ })).toHaveAttribute(
      'aria-selected',
      'false',
    );
  });

  it('renders Ollama slugs without pills, sub-line, or fit', () => {
    render(
      <ModelSelect
        value="llama3.2:3b"
        items={SLUGS}
        onChange={() => {}}
        ariaLabel="Active Ollama model"
      />,
    );
    open('Active Ollama model');
    const row = screen.getByRole('option', { name: 'llama3.2:3b' });
    expect(within(row).queryByText('Text')).not.toBeInTheDocument();
    expect(within(row).queryByText('Comfortable')).not.toBeInTheDocument();
  });

  it('filters the list and shows an empty message when nothing matches', () => {
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    open();
    const input = screen.getByRole('combobox');
    fireEvent.change(input, { target: { value: 'al' } });
    expect(screen.getByRole('option', { name: /Alpha/ })).toBeInTheDocument();
    expect(
      screen.queryByRole('option', { name: /Beta/ }),
    ).not.toBeInTheDocument();

    fireEvent.change(input, { target: { value: 'zzz' } });
    expect(screen.getByText('No models found.')).toBeInTheDocument();
  });

  it('commits a clicked option and closes', () => {
    const onChange = vi.fn();
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={onChange}
        ariaLabel="Built-in model"
      />,
    );
    open();
    fireEvent.click(screen.getByRole('option', { name: /Beta/ }));
    expect(onChange).toHaveBeenCalledWith('b');
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });

  it('navigates with the keyboard and commits with Enter', () => {
    const onChange = vi.fn();
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={onChange}
        ariaLabel="Built-in model"
      />,
    );
    open();
    const input = screen.getByRole('combobox');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onChange).toHaveBeenCalledWith('b');
  });

  it('wraps to the last item with ArrowUp and reaches ends with Home/End', () => {
    const onChange = vi.fn();
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={onChange}
        ariaLabel="Built-in model"
      />,
    );
    open();
    const input = screen.getByRole('combobox');
    fireEvent.keyDown(input, { key: 'ArrowUp' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      'thuki-model-select-listbox-option-2',
    );
    fireEvent.keyDown(input, { key: 'Home' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      'thuki-model-select-listbox-option-0',
    );
    fireEvent.keyDown(input, { key: 'End' });
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      'thuki-model-select-listbox-option-2',
    );
  });

  it('highlights the row under the cursor', () => {
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    open();
    fireEvent.mouseEnter(screen.getByRole('option', { name: /Gamma/ }));
    expect(screen.getByRole('combobox')).toHaveAttribute(
      'aria-activedescendant',
      'thuki-model-select-listbox-option-2',
    );
  });

  it('ignores arrows and Enter while the filter matches nothing', () => {
    const onChange = vi.fn();
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={onChange}
        ariaLabel="Built-in model"
      />,
    );
    open();
    const input = screen.getByRole('combobox');
    fireEvent.change(input, { target: { value: 'zzz' } });
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'ArrowUp' });
    fireEvent.keyDown(input, { key: 'Home' });
    fireEvent.keyDown(input, { key: 'End' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(input).not.toHaveAttribute('aria-activedescendant');
    expect(onChange).not.toHaveBeenCalled();
  });

  it('ignores keys it does not handle', () => {
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    open();
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'a' });
    expect(screen.getByRole('listbox')).toBeInTheDocument();
  });

  it('closes on Tab so it is not left open behind the next control', () => {
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    open();
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Tab' });
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });

  it('closes when the page scrolls or the window resizes', () => {
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    open();
    fireEvent.scroll(window);
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
    open();
    fireEvent.resize(window);
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });

  it('returns focus to the trigger on Escape and after a selection', () => {
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    const trigger = screen.getByRole('button', { name: 'Built-in model' });
    open();
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Escape' });
    expect(trigger).toHaveFocus();
    open();
    fireEvent.click(screen.getByRole('option', { name: /Beta/ }));
    expect(trigger).toHaveFocus();
  });

  it('pre-highlights the active row so Enter commits it, else the first row', () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <ModelSelect
        value="b"
        items={RICH}
        onChange={onChange}
        ariaLabel="Built-in model"
      />,
    );
    open();
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Enter' });
    expect(onChange).toHaveBeenLastCalledWith('b');

    rerender(
      <ModelSelect
        value="missing"
        items={RICH}
        onChange={onChange}
        ariaLabel="Built-in model"
        placeholder="Choose a model"
      />,
    );
    open();
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Enter' });
    expect(onChange).toHaveBeenLastCalledWith('a');
  });

  it('closes on Escape', () => {
    render(
      <ModelSelect
        value="a"
        items={RICH}
        onChange={() => {}}
        ariaLabel="Built-in model"
      />,
    );
    open();
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Escape' });
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });

  it('closes on an outside press but not on a press inside the popover', () => {
    render(
      <div>
        <span data-testid="outside">outside</span>
        <ModelSelect
          value="a"
          items={RICH}
          onChange={() => {}}
          ariaLabel="Built-in model"
        />
      </div>,
    );
    open();
    fireEvent.mouseDown(screen.getByRole('combobox'));
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    fireEvent.mouseDown(screen.getByTestId('outside'));
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });
});
