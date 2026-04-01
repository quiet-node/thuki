import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { AskBarView } from '../AskBarView';

function makeRef(): React.RefObject<HTMLTextAreaElement | null> {
  return { current: null };
}

describe('AskBarView', () => {
  it('renders textarea with placeholder for input bar mode', () => {
    render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    expect(textarea).not.toBeNull();
  });

  it('renders textarea with chat mode placeholder', () => {
    render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Reply...');
    expect(textarea).not.toBeNull();
  });

  it('calls setQuery on textarea change', () => {
    const setQuery = vi.fn();
    render(
      <AskBarView
        query=""
        setQuery={setQuery}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    fireEvent.change(textarea, { target: { value: 'hello' } });
    expect(setQuery).toHaveBeenCalledWith('hello');
  });

  it('disables textarea during generation', () => {
    render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={true}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    expect((textarea as HTMLTextAreaElement).disabled).toBe(true);
  });

  it('calls onSubmit on Enter key', () => {
    const onSubmit = vi.fn();
    render(
      <AskBarView
        query="hello"
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it('does not submit on Shift+Enter', () => {
    const onSubmit = vi.fn();
    render(
      <AskBarView
        query="hello"
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const textarea = screen.getByPlaceholderText('Ask Thuki anything...');
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: true });
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('calls onSubmit on button click', () => {
    const onSubmit = vi.fn();
    render(
      <AskBarView
        query="hello"
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it('shows logo at 40px in input bar mode (w-10 h-10 rounded-xl classes)', () => {
    const { container } = render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const logo = container.querySelector('img[alt="Thuki"]');
    expect(logo).not.toBeNull();
    expect(logo?.classList.contains('w-10')).toBe(true);
    expect(logo?.classList.contains('h-10')).toBe(true);
    expect(logo?.classList.contains('rounded-xl')).toBe(true);
  });

  it('shows logo at 24px in chat mode (w-6 h-6 rounded-lg classes)', () => {
    const { container } = render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    const logo = container.querySelector('img[alt="Thuki"]');
    expect(logo).not.toBeNull();
    expect(logo?.classList.contains('w-6')).toBe(true);
    expect(logo?.classList.contains('h-6')).toBe(true);
    expect(logo?.classList.contains('rounded-lg')).toBe(true);
  });

  it('shows send button with accessible label', () => {
    render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Send message' }),
    ).toBeInTheDocument();
  });

  it('displays selectedText when provided', () => {
    render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
        selectedText="some highlighted text"
      />,
    );
    expect(screen.getByText(/some highlighted text/)).toBeInTheDocument();
  });

  it('hides context area when no selectedText', () => {
    const { container } = render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    expect(container.querySelector('.whitespace-pre-wrap')).toBeNull();
  });

  it('shows stop button with accessible label during generation', () => {
    render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={true}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Stop generating' }),
    ).toBeInTheDocument();
  });

  it('calls onCancel when stop button is clicked', () => {
    const onCancel = vi.fn();
    render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={true}
        onSubmit={vi.fn()}
        onCancel={onCancel}
        inputRef={makeRef()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Stop generating' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('does not call onSubmit when stop button is clicked during generation', () => {
    const onSubmit = vi.fn();
    render(
      <AskBarView
        query="hello"
        setQuery={vi.fn()}
        isChatMode={true}
        isGenerating={true}
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        inputRef={makeRef()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Stop generating' }));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('displays selectedText with whitespace-pre-wrap class', () => {
    const { container } = render(
      <AskBarView
        query=""
        setQuery={vi.fn()}
        isChatMode={false}
        isGenerating={false}
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        inputRef={makeRef()}
        selectedText="context text here"
      />,
    );
    const el = container.querySelector('.whitespace-pre-wrap');
    expect(el).not.toBeNull();
    expect(el?.textContent).toContain('context text here');
  });
});
