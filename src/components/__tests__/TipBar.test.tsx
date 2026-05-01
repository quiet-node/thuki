import { render, screen, act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TipBar } from '../TipBar';

describe('TipBar', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.spyOn(Math, 'random').mockReturnValue(0);
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
});
