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
    const closeDot = container.querySelector('.bg-\\[\\#FF5F57\\]');
    expect(closeDot).not.toBeNull();
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

  it('close button blurs itself on programmatic focus (no relatedTarget)', () => {
    render(<WindowControls onClose={vi.fn()} />);
    const btn = screen.getByRole('button', { name: 'Close window' });
    const blurSpy = vi.spyOn(btn, 'blur');
    fireEvent.focus(btn, { relatedTarget: null });
    expect(blurSpy).toHaveBeenCalledTimes(1);
  });

  it('close button keeps focus when focused via keyboard tab (relatedTarget present)', () => {
    render(<WindowControls onClose={vi.fn()} />);
    const btn = screen.getByRole('button', { name: 'Close window' });
    const blurSpy = vi.spyOn(btn, 'blur');
    fireEvent.focus(btn, { relatedTarget: document.body });
    expect(blurSpy).not.toHaveBeenCalled();
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

  it('attaches bookmarkButtonRef and saveButtonTestId to the save button', () => {
    const bookmarkButtonRef = { current: null as HTMLButtonElement | null };
    render(
      <WindowControls
        onClose={vi.fn()}
        onSave={vi.fn()}
        canSave
        isSaved
        bookmarkButtonRef={bookmarkButtonRef}
        saveButtonTestId="auto-save-bookmark"
      />,
    );
    const btn = screen.getByTestId('auto-save-bookmark');
    expect(btn).toBe(bookmarkButtonRef.current);
    expect(btn).toHaveAttribute('aria-label', 'Remove from history');
  });

  it('renders active model pill when activeModel and onModelPickerToggle provided', () => {
    render(
      <WindowControls
        onClose={vi.fn()}
        activeModel="gemma4:e2b"
        onModelPickerToggle={vi.fn()}
        isModelPickerOpen={false}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toBeInTheDocument();
    expect(screen.getByText('gemma4:e2b')).toBeInTheDocument();
  });

  it('renders the friendly display name when the active model id has one', () => {
    // Built-in model ids are raw "repo:file.gguf" slugs; the pill must show
    // the elegant label, matching the model picker.
    render(
      <WindowControls
        onClose={vi.fn()}
        activeModel="unsloth/Qwen3.5:Qwen3.5-9B-Q4_K_M.gguf"
        displayNames={{
          'unsloth/Qwen3.5:Qwen3.5-9B-Q4_K_M.gguf': 'Qwen3.5 9B',
        }}
        onModelPickerToggle={vi.fn()}
      />,
    );
    expect(screen.getByText('Qwen3.5 9B')).toBeInTheDocument();
    expect(
      screen.queryByText('unsloth/Qwen3.5:Qwen3.5-9B-Q4_K_M.gguf'),
    ).toBeNull();
  });

  it('renders the picker chip with a "Pick a model" placeholder when activeModel is null', () => {
    // The chip is the recovery affordance for the no-model state, so it
    // must stay visible (and clickable) even when activeModel is null.
    // Without this, the user has no path back to the picker.
    render(
      <WindowControls
        onClose={vi.fn()}
        activeModel={null}
        onModelPickerToggle={vi.fn()}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toBeInTheDocument();
    expect(screen.getByText('Pick a model')).toBeInTheDocument();
  });

  it('renders the picker chip with a "Pick a model" placeholder when activeModel is omitted', () => {
    render(<WindowControls onClose={vi.fn()} onModelPickerToggle={vi.fn()} />);
    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toBeInTheDocument();
    expect(screen.getByText('Pick a model')).toBeInTheDocument();
  });

  it('renders the picker chip with a "Pick a model" placeholder when activeModel is empty string', () => {
    render(
      <WindowControls
        onClose={vi.fn()}
        activeModel=""
        onModelPickerToggle={vi.fn()}
      />,
    );
    expect(screen.getByText('Pick a model')).toBeInTheDocument();
  });

  it('hides model pill when onModelPickerToggle is not provided', () => {
    render(<WindowControls onClose={vi.fn()} activeModel="gemma4:e2b" />);
    expect(screen.queryByRole('button', { name: 'Choose model' })).toBeNull();
  });

  it('calls onModelPickerToggle when pill is clicked', () => {
    const onModelPickerToggle = vi.fn();
    render(
      <WindowControls
        onClose={vi.fn()}
        activeModel="gemma4:e2b"
        onModelPickerToggle={onModelPickerToggle}
        isModelPickerOpen={false}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    expect(onModelPickerToggle).toHaveBeenCalledTimes(1);
  });

  it('sets aria-expanded false on pill when isModelPickerOpen is false', () => {
    render(
      <WindowControls
        onClose={vi.fn()}
        activeModel="gemma4:e2b"
        onModelPickerToggle={vi.fn()}
        isModelPickerOpen={false}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toHaveAttribute('aria-expanded', 'false');
  });

  it('sets aria-expanded true on pill when isModelPickerOpen is true', () => {
    render(
      <WindowControls
        onClose={vi.fn()}
        activeModel="gemma4:e2b"
        onModelPickerToggle={vi.fn()}
        isModelPickerOpen={true}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toHaveAttribute('aria-expanded', 'true');
  });

  it('pill renders before save button in DOM order', () => {
    render(
      <WindowControls
        onClose={vi.fn()}
        activeModel="gemma4:e2b"
        onModelPickerToggle={vi.fn()}
        isModelPickerOpen={false}
        onSave={vi.fn()}
        canSave
      />,
    );
    const pill = screen.getByRole('button', { name: 'Choose model' });
    const save = screen.getByRole('button', { name: 'Save conversation' });
    const relation = pill.compareDocumentPosition(save);
    expect(relation & Node.DOCUMENT_POSITION_FOLLOWING).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );
  });
});

describe('WindowControls minimize affordance', () => {
  it('renders a Minimize button and calls onMinimize when provided', () => {
    const onMinimize = vi.fn();
    render(<WindowControls onClose={vi.fn()} onMinimize={onMinimize} />);
    const btn = screen.getByRole('button', { name: /minimize/i });
    fireEvent.click(btn);
    expect(onMinimize).toHaveBeenCalledTimes(1);
  });

  it('renders an inert minimize dot (no button) when onMinimize is omitted', () => {
    render(<WindowControls onClose={vi.fn()} />);
    expect(
      screen.queryByRole('button', { name: /minimize/i }),
    ).not.toBeInTheDocument();
  });

  it('minimize button blurs itself on programmatic focus (no relatedTarget)', () => {
    const onMinimize = vi.fn();
    render(<WindowControls onClose={vi.fn()} onMinimize={onMinimize} />);
    const btn = screen.getByRole('button', { name: /minimize/i });
    const blurSpy = vi.spyOn(btn, 'blur');
    fireEvent.focus(btn, { relatedTarget: null });
    expect(blurSpy).toHaveBeenCalledTimes(1);
  });

  it('minimize button keeps focus when focused via keyboard tab (relatedTarget present)', () => {
    const onMinimize = vi.fn();
    render(<WindowControls onClose={vi.fn()} onMinimize={onMinimize} />);
    const btn = screen.getByRole('button', { name: /minimize/i });
    const blurSpy = vi.spyOn(btn, 'blur');
    fireEvent.focus(btn, { relatedTarget: document.body });
    expect(blurSpy).not.toHaveBeenCalled();
  });
});
