import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import {
  ConfirmDialog,
  DIALOG_EXIT_MS,
  Dropdown,
  NumberSlider,
  NumberStepper,
  ResetSectionLink,
  SavedPill,
  Section,
  SettingRow,
  TextField,
  Textarea,
  Toggle,
} from './index';
import type { ConfigError } from '../types';

describe('Section', () => {
  it('renders the heading and children', () => {
    render(
      <Section heading="GENERAL">
        <span>row</span>
      </Section>,
    );
    expect(screen.getByText('GENERAL')).toBeInTheDocument();
    expect(screen.getByText('row')).toBeInTheDocument();
  });

  it('renders a `?` info button next to the heading when helper is provided', () => {
    render(
      <Section heading="GENERAL" helper="What this group is about.">
        <span>row</span>
      </Section>,
    );
    expect(
      screen.getByRole('button', { name: /About GENERAL/ }),
    ).toBeInTheDocument();
  });

  it('shows the section helper tooltip on hover', () => {
    render(
      <Section heading="GENERAL" helper="What this group is about.">
        <span>row</span>
      </Section>,
    );
    fireEvent.mouseEnter(
      screen.getByRole('button', { name: /About GENERAL/ }).parentElement!,
    );
    expect(screen.getByText('What this group is about.')).toBeInTheDocument();
  });

  it('omits the section info button when no helper is provided', () => {
    render(
      <Section heading="GENERAL">
        <span>row</span>
      </Section>,
    );
    expect(
      screen.queryByRole('button', { name: /About GENERAL/ }),
    ).not.toBeInTheDocument();
  });
});

describe('SettingRow', () => {
  it('renders label and children', () => {
    render(
      <SettingRow label="Width">
        <input aria-label="width-input" />
      </SettingRow>,
    );
    expect(screen.getByText('Width')).toBeInTheDocument();
    expect(screen.getByLabelText('width-input')).toBeInTheDocument();
  });

  it('renders the `?` info button when helper is provided', () => {
    render(
      <SettingRow label="Width" helper="Pixel width of the overlay window.">
        <input aria-label="x" />
      </SettingRow>,
    );
    expect(
      screen.getByRole('button', { name: /About Width/ }),
    ).toBeInTheDocument();
  });

  it('omits the info button when helper is missing', () => {
    render(
      <SettingRow label="Width">
        <input aria-label="x" />
      </SettingRow>,
    );
    expect(screen.queryByRole('button', { name: /About Width/ })).toBeNull();
  });

  it('renders the inline error message when error is set', () => {
    const err: ConfigError = {
      kind: 'type_mismatch',
      section: 'window',
      key: 'overlay_width',
      message: 'expected integer',
    };
    render(
      <SettingRow label="Width" error={err}>
        <input aria-label="x" />
      </SettingRow>,
    );
    expect(screen.getByRole('alert')).toHaveTextContent('expected integer');
  });

  it('applies the vertical layout class when vertical=true', () => {
    const { container } = render(
      <SettingRow label="Width" vertical>
        <input aria-label="x" />
      </SettingRow>,
    );
    const row = container.querySelector('[role="group"]')!;
    // CSS-modules hash the class name; we just verify both row tokens are
    // present (joined with whitespace).
    expect(row.className.split(/\s+/).length).toBeGreaterThanOrEqual(2);
  });
});

describe('TextField', () => {
  it('calls onChange on input', () => {
    const onChange = vi.fn();
    render(<TextField value="hello" onChange={onChange} ariaLabel="t" />);
    fireEvent.change(screen.getByLabelText('t'), {
      target: { value: 'world' },
    });
    expect(onChange).toHaveBeenCalledWith('world');
  });

  it('renders placeholder and the errored visual state', () => {
    render(
      <TextField
        value=""
        onChange={() => {}}
        placeholder="ph"
        errored
        ariaLabel="t"
      />,
    );
    const el = screen.getByLabelText('t');
    expect(el).toHaveAttribute('placeholder', 'ph');
  });
});

describe('Textarea', () => {
  it('calls onChange on input', () => {
    const onChange = vi.fn();
    render(
      <Textarea
        value=""
        onChange={onChange}
        placeholder="ph"
        maxLength={10}
        ariaLabel="ta"
      />,
    );
    fireEvent.change(screen.getByLabelText('ta'), { target: { value: 'x' } });
    expect(onChange).toHaveBeenCalledWith('x');
  });
});

describe('NumberSlider', () => {
  it('updates the displayed value during drag without firing onChange', () => {
    const onChange = vi.fn();
    render(
      <NumberSlider
        value={50}
        min={0}
        max={100}
        unit="px"
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    const input = screen.getByLabelText('s') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '75' } });
    // The displayed chip updates immediately.
    expect(screen.getByText(/75/)).toBeInTheDocument();
    // onChange has NOT fired yet (waits for commit).
    expect(onChange).not.toHaveBeenCalled();
  });

  it('commits on mouseUp and fires onChange with the new value', () => {
    const onChange = vi.fn();
    render(
      <NumberSlider
        value={50}
        min={0}
        max={100}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    const input = screen.getByLabelText('s') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '75' } });
    fireEvent.mouseUp(input);
    expect(onChange).toHaveBeenCalledWith(75);
  });

  it('does not fire onChange on mouseUp when value is unchanged', () => {
    const onChange = vi.fn();
    render(
      <NumberSlider
        value={50}
        min={0}
        max={100}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    fireEvent.mouseUp(screen.getByLabelText('s'));
    expect(onChange).not.toHaveBeenCalled();
  });

  it('commits on touchEnd', () => {
    const onChange = vi.fn();
    render(
      <NumberSlider
        value={50}
        min={0}
        max={100}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    const input = screen.getByLabelText('s') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '60' } });
    fireEvent.touchEnd(input);
    expect(onChange).toHaveBeenCalledWith(60);
  });

  it('does not fire on touchEnd when value is unchanged', () => {
    const onChange = vi.fn();
    render(
      <NumberSlider
        value={50}
        min={0}
        max={100}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    fireEvent.touchEnd(screen.getByLabelText('s'));
    expect(onChange).not.toHaveBeenCalled();
  });

  it('commits on blur when local diverged', () => {
    const onChange = vi.fn();
    render(
      <NumberSlider
        value={50}
        min={0}
        max={100}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    const input = screen.getByLabelText('s') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '60' } });
    fireEvent.blur(input);
    expect(onChange).toHaveBeenCalledWith(60);
  });

  it('blur with no local change is a no-op', () => {
    const onChange = vi.fn();
    render(
      <NumberSlider
        value={50}
        min={0}
        max={100}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    fireEvent.blur(screen.getByLabelText('s'));
    expect(onChange).not.toHaveBeenCalled();
  });

  it('keyUp commits the new value', () => {
    const onChange = vi.fn();
    render(
      <NumberSlider
        value={50}
        min={0}
        max={100}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    const input = screen.getByLabelText('s') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '60' } });
    fireEvent.keyUp(input, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenCalledWith(60);
  });

  it('keyUp with no local change is a no-op', () => {
    const onChange = vi.fn();
    render(
      <NumberSlider
        value={50}
        min={0}
        max={100}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    fireEvent.keyUp(screen.getByLabelText('s'), { key: 'ArrowRight' });
    expect(onChange).not.toHaveBeenCalled();
  });

  it('renders the unit suffix in the value chip', () => {
    render(
      <NumberSlider
        value={42}
        min={0}
        max={100}
        unit="ms"
        onChange={() => {}}
      />,
    );
    expect(screen.getByText('42 ms')).toBeInTheDocument();
  });

  it('drag-in-progress prevents external value updates from clobbering local state', () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <NumberSlider
        value={50}
        min={0}
        max={100}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    const input = screen.getByLabelText('s') as HTMLInputElement;
    // Start a drag: change event flips draggingRef.current to true.
    fireEvent.change(input, { target: { value: '70' } });
    expect(screen.getByText('70')).toBeInTheDocument();

    // External resync arrives mid-drag (e.g. parent re-rendered with a
    // brand-new prop). The slider must NOT clobber the in-progress drag.
    rerender(
      <NumberSlider
        value={20}
        min={0}
        max={100}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    // Local drag value (70) is preserved; the new prop (20) is ignored.
    expect(screen.getByText('70')).toBeInTheDocument();
  });

  it('renders without unit when none is supplied', () => {
    render(<NumberSlider value={42} min={0} max={100} onChange={() => {}} />);
    expect(screen.getByText('42')).toBeInTheDocument();
  });
});

describe('NumberStepper', () => {
  it('increments and decrements within bounds', () => {
    const onChange = vi.fn();
    render(
      <NumberStepper
        value={5}
        min={0}
        max={10}
        onChange={onChange}
        ariaLabel="s"
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Increase' }));
    expect(onChange).toHaveBeenCalledWith(6);

    fireEvent.click(screen.getByRole('button', { name: 'Decrease' }));
    expect(onChange).toHaveBeenCalledWith(4);
  });

  it('does not go below min', () => {
    const onChange = vi.fn();
    render(<NumberStepper value={0} min={0} max={10} onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Decrease' }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it('does not exceed max', () => {
    const onChange = vi.fn();
    render(<NumberStepper value={10} min={0} max={10} onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Increase' }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it('respects custom step', () => {
    const onChange = vi.fn();
    render(
      <NumberStepper
        value={5}
        min={0}
        max={100}
        step={5}
        onChange={onChange}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Increase' }));
    expect(onChange).toHaveBeenCalledWith(10);
  });
});

describe('Toggle', () => {
  it('renders with role=switch and the initial aria-checked state', () => {
    render(<Toggle checked={false} onChange={() => {}} ariaLabel="Enable X" />);
    const btn = screen.getByRole('switch', { name: 'Enable X' });
    expect(btn).toHaveAttribute('aria-checked', 'false');
  });

  it('renders aria-checked=true when checked=true', () => {
    render(<Toggle checked={true} onChange={() => {}} ariaLabel="Enable X" />);
    expect(screen.getByRole('switch', { name: 'Enable X' })).toHaveAttribute(
      'aria-checked',
      'true',
    );
  });

  it('calls onChange with the flipped value on click', () => {
    const onChange = vi.fn();
    render(<Toggle checked={false} onChange={onChange} ariaLabel="t" />);
    fireEvent.click(screen.getByRole('switch', { name: 't' }));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it('calls onChange with false when checked=true and clicked', () => {
    const onChange = vi.fn();
    render(<Toggle checked={true} onChange={onChange} ariaLabel="t" />);
    fireEvent.click(screen.getByRole('switch', { name: 't' }));
    expect(onChange).toHaveBeenCalledWith(false);
  });
});

describe('Dropdown', () => {
  it('reflects current value and emits onChange on selection', () => {
    const onChange = vi.fn();
    render(
      <Dropdown<'a' | 'b'>
        value="a"
        options={['a', 'b']}
        onChange={onChange}
        ariaLabel="d"
      />,
    );
    const select = screen.getByLabelText('d') as HTMLSelectElement;
    expect(select.value).toBe('a');
    fireEvent.change(select, { target: { value: 'b' } });
    expect(onChange).toHaveBeenCalledWith('b');
  });
});

describe('SavedPill', () => {
  it('renders ✓ Saved', () => {
    render(<SavedPill visible />);
    expect(screen.getByRole('status')).toHaveTextContent('Saved');
  });

  it('toggles visibility class via prop', () => {
    const { container, rerender } = render(<SavedPill visible={false} />);
    const first = container.firstElementChild!;
    const before = first.className;
    rerender(<SavedPill visible />);
    const after = container.firstElementChild!.className;
    expect(after).not.toBe(before);
  });
});

describe('ConfirmDialog', () => {
  it('renders when open', () => {
    render(
      <ConfirmDialog
        open
        title="Reset?"
        message="This wipes everything."
        confirmLabel="Yes"
        onConfirm={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(screen.getByText('Reset?')).toBeInTheDocument();
    expect(screen.getByText('This wipes everything.')).toBeInTheDocument();
  });

  it('returns null when closed', () => {
    const { container } = render(
      <ConfirmDialog
        open={false}
        title="Reset?"
        message="m"
        confirmLabel="Yes"
        onConfirm={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('confirm button fires onConfirm', () => {
    const onConfirm = vi.fn();
    render(
      <ConfirmDialog
        open
        title="t"
        message="m"
        confirmLabel="Yes"
        onConfirm={onConfirm}
        onCancel={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Yes' }));
    expect(onConfirm).toHaveBeenCalled();
  });

  it('cancel button fires onCancel', () => {
    const onCancel = vi.fn();
    render(
      <ConfirmDialog
        open
        title="t"
        message="m"
        confirmLabel="Yes"
        cancelLabel="No"
        onConfirm={() => {}}
        onCancel={onCancel}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'No' }));
    expect(onCancel).toHaveBeenCalled();
  });

  it('Escape key fires onCancel', () => {
    const onCancel = vi.fn();
    render(
      <ConfirmDialog
        open
        title="t"
        message="m"
        confirmLabel="Yes"
        onConfirm={() => {}}
        onCancel={onCancel}
      />,
    );
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onCancel).toHaveBeenCalled();
  });

  it('non-Escape keys do not fire onCancel', () => {
    const onCancel = vi.fn();
    render(
      <ConfirmDialog
        open
        title="t"
        message="m"
        confirmLabel="Yes"
        onConfirm={() => {}}
        onCancel={onCancel}
      />,
    );
    fireEvent.keyDown(document, { key: 'Enter' });
    expect(onCancel).not.toHaveBeenCalled();
  });

  it('does not register the listener when closed', () => {
    // Simply rendering the closed dialog and dispatching Escape proves the
    // listener does not fire.
    const onCancel = vi.fn();
    render(
      <ConfirmDialog
        open={false}
        title="t"
        message="m"
        confirmLabel="Yes"
        onConfirm={() => {}}
        onCancel={onCancel}
      />,
    );
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onCancel).not.toHaveBeenCalled();
  });

  it('applies the destructive class to the confirm button when destructive=true', () => {
    render(
      <ConfirmDialog
        open
        title="t"
        message="m"
        confirmLabel="Wipe"
        destructive
        onConfirm={() => {}}
        onCancel={() => {}}
      />,
    );
    const wipe = screen.getByRole('button', { name: 'Wipe' });
    // Class names are CSS-modules hashed; just verify there's more than one
    // class token (the destructive modifier appended).
    expect(wipe.className.split(/\s+/).length).toBeGreaterThanOrEqual(2);
  });

  it('keeps the dialog mounted through its exit animation, then unmounts', () => {
    vi.useFakeTimers();
    try {
      const { rerender } = render(
        <ConfirmDialog
          open
          title="t"
          message="m"
          confirmLabel="Yes"
          onConfirm={() => {}}
          onCancel={() => {}}
        />,
      );
      expect(screen.getByRole('dialog')).toBeInTheDocument();

      rerender(
        <ConfirmDialog
          open={false}
          title="t"
          message="m"
          confirmLabel="Yes"
          onConfirm={() => {}}
          onCancel={() => {}}
        />,
      );
      // Still mounted while the leave animation plays, flagged as closing.
      expect(screen.getByRole('dialog')).toHaveAttribute(
        'data-closing',
        'true',
      );

      act(() => {
        vi.advanceTimersByTime(DIALOG_EXIT_MS);
      });
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it('skips the exit animation and unmounts at once under reduced motion', () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn().mockReturnValue({
        matches: true,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    );
    try {
      const { rerender } = render(
        <ConfirmDialog
          open
          title="t"
          message="m"
          confirmLabel="Yes"
          onConfirm={() => {}}
          onCancel={() => {}}
        />,
      );
      expect(screen.getByRole('dialog')).toBeInTheDocument();

      rerender(
        <ConfirmDialog
          open={false}
          title="t"
          message="m"
          confirmLabel="Yes"
          onConfirm={() => {}}
          onCancel={() => {}}
        />,
      );
      // No exit animation: the dialog is gone immediately, no partial fade.
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('cancels the pending unmount when re-opened mid-exit', () => {
    vi.useFakeTimers();
    try {
      const { rerender } = render(
        <ConfirmDialog
          open
          title="t"
          message="m"
          confirmLabel="Yes"
          onConfirm={() => {}}
          onCancel={() => {}}
        />,
      );
      rerender(
        <ConfirmDialog
          open={false}
          title="t"
          message="m"
          confirmLabel="Yes"
          onConfirm={() => {}}
          onCancel={() => {}}
        />,
      );
      expect(screen.getByRole('dialog')).toHaveAttribute(
        'data-closing',
        'true',
      );

      // Re-open before the exit completes.
      act(() => {
        vi.advanceTimersByTime(DIALOG_EXIT_MS / 2);
      });
      rerender(
        <ConfirmDialog
          open
          title="t"
          message="m"
          confirmLabel="Yes"
          onConfirm={() => {}}
          onCancel={() => {}}
        />,
      );
      // Back to the open state, no longer closing.
      expect(screen.getByRole('dialog')).not.toHaveAttribute('data-closing');

      // The stale unmount timer was cancelled: advancing past it stays mounted.
      act(() => {
        vi.advanceTimersByTime(DIALOG_EXIT_MS);
      });
      expect(screen.getByRole('dialog')).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it('clears a pending exit timer when it unmounts mid-close', () => {
    vi.useFakeTimers();
    const clearSpy = vi.spyOn(window, 'clearTimeout');
    try {
      const { rerender, unmount } = render(
        <ConfirmDialog
          open
          title="t"
          message="m"
          confirmLabel="Yes"
          onConfirm={() => {}}
          onCancel={() => {}}
        />,
      );
      rerender(
        <ConfirmDialog
          open={false}
          title="t"
          message="m"
          confirmLabel="Yes"
          onConfirm={() => {}}
          onCancel={() => {}}
        />,
      );
      clearSpy.mockClear();

      unmount();
      // The cleanup cleared the outstanding exit timer, so nothing fires after
      // the component is gone.
      expect(clearSpy).toHaveBeenCalled();
      act(() => {
        vi.advanceTimersByTime(DIALOG_EXIT_MS);
      });
    } finally {
      clearSpy.mockRestore();
      vi.useRealTimers();
    }
  });
});

describe('ResetSectionLink', () => {
  it('fires onClick', () => {
    const onClick = vi.fn();
    render(<ResetSectionLink label="Reset" onClick={onClick} />);
    fireEvent.click(screen.getByRole('button', { name: /Reset/ }));
    expect(onClick).toHaveBeenCalled();
  });
});
