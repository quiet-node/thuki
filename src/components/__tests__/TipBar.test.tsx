import { fireEvent, render, screen, act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { TipBar } from '../TipBar';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe('TipBar', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.spyOn(Math, 'random').mockReturnValue(0);
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('renders the TIP badge', () => {
    render(<TipBar tip="Hello world" tipKey={0} />);
    expect(screen.getByText('TIP')).toBeInTheDocument();
  });

  it('renders the tip-text span', () => {
    render(<TipBar tip="Hello world" tipKey={0} />);
    expect(screen.getByTestId('tip-text')).toBeInTheDocument();
  });

  it('renders the strip container', () => {
    render(<TipBar tip="Test" tipKey={0} />);
    expect(screen.getByTestId('tip-bar')).toBeInTheDocument();
  });

  it('reveals full tip text after animation completes (tipKey=0)', () => {
    render(<TipBar tip="Hi" tipKey={0} />);
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.getByTestId('tip-text').textContent).toBe('Hi');
  });

  it('handles space characters instantly without flicker', () => {
    render(<TipBar tip="a b" tipKey={0} />);
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.getByTestId('tip-text').textContent).toBe('a b');
  });

  it('re-animates and shows new tip after tipKey increments', () => {
    const { rerender } = render(<TipBar tip="Hello" tipKey={0} />);
    act(() => vi.advanceTimersByTime(5000));
    rerender(<TipBar tip="World" tipKey={1} />);
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.getByTestId('tip-text').textContent).toBe('World');
  });

  it('cleans up timers on unmount without throwing', () => {
    const { unmount } = render(<TipBar tip="Hello" tipKey={0} />);
    expect(() => unmount()).not.toThrow();
  });

  it('renders as a non-clickable div when the tip is a plain string', () => {
    render(<TipBar tip="Plain tip without any link" tipKey={0} />);
    const bar = screen.getByTestId('tip-bar');
    expect(bar.tagName).toBe('DIV');
  });

  it('renders as a button and opens the URL via open_url when tip carries a url field', () => {
    render(
      <TipBar
        tip={{
          text: 'Click here to learn how to tune Context Window',
          url: 'https://github.com/quiet-node/thuki/blob/main/docs/tuning-context-window.md',
        }}
        tipKey={0}
      />,
    );
    const bar = screen.getByTestId('tip-bar');
    expect(bar.tagName).toBe('BUTTON');
    fireEvent.click(bar);
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/blob/main/docs/tuning-context-window.md',
    });
  });

  it('renders only the friendly text from a linked tip (no raw URL)', () => {
    render(
      <TipBar
        tip={{
          text: 'Click here to learn how to tune Context Window',
          url: 'https://github.com/quiet-node/thuki',
        }}
        tipKey={0}
      />,
    );
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.getByTestId('tip-text').textContent).toBe(
      'Click here to learn how to tune Context Window',
    );
  });

  it('renders nothing when suppressed', () => {
    const { container } = render(<TipBar tip="hello" tipKey={0} suppressed />);
    expect(container.firstChild).toBeNull();
  });
});
