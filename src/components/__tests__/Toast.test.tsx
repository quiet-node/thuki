import { render, screen, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { Toast } from '../Toast';

describe('Toast', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders nothing when message is null', () => {
    render(<Toast message={null} onDismiss={() => {}} />);
    expect(screen.queryByTestId('toast')).toBeNull();
  });

  it('renders the message when provided', () => {
    render(<Toast message="hello" onDismiss={() => {}} />);
    expect(screen.getByTestId('toast')).toHaveTextContent('hello');
  });

  it('auto-dismisses after the default 3000ms', () => {
    const onDismiss = vi.fn();
    render(<Toast message="bye" onDismiss={onDismiss} />);
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('honors a custom durationMs', () => {
    const onDismiss = vi.fn();
    render(<Toast message="bye" onDismiss={onDismiss} durationMs={500} />);
    act(() => {
      vi.advanceTimersByTime(499);
    });
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('clears the timer when message changes to null before timeout', () => {
    const onDismiss = vi.fn();
    const { rerender } = render(
      <Toast message="first" onDismiss={onDismiss} durationMs={1000} />,
    );
    act(() => {
      vi.advanceTimersByTime(500);
    });
    rerender(<Toast message={null} onDismiss={onDismiss} durationMs={1000} />);
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it('resets the timer when message changes to a new value', () => {
    const onDismiss = vi.fn();
    const { rerender } = render(
      <Toast message="first" onDismiss={onDismiss} durationMs={1000} />,
    );
    act(() => {
      vi.advanceTimersByTime(900);
    });
    rerender(
      <Toast message="second" onDismiss={onDismiss} durationMs={1000} />,
    );
    act(() => {
      vi.advanceTimersByTime(900);
    });
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(200);
    });
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
