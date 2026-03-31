import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { CopyButton } from '../CopyButton';

describe('CopyButton', () => {
  beforeEach(() => {
    vi.mocked(navigator.clipboard.writeText).mockClear();
    vi.mocked(navigator.clipboard.writeText).mockResolvedValue(undefined);
  });

  it('calls clipboard.writeText with correct content on click', () => {
    render(<CopyButton content="Hello world" align="right" />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('Hello world');
  });

  it('shows checkmark after successful copy (aria-label changes to "Copied")', async () => {
    render(<CopyButton content="test content" align="right" />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
    });
    expect(screen.getByRole('button', { name: 'Copied' })).toBeTruthy();
  });

  it('reverts to copy icon after 1.5 seconds', async () => {
    vi.useFakeTimers();
    render(<CopyButton content="test" align="right" />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
    });
    expect(screen.getByRole('button', { name: 'Copied' })).toBeTruthy();
    act(() => {
      vi.advanceTimersByTime(1500);
    });
    expect(screen.getByRole('button', { name: 'Copy message' })).toBeTruthy();
    vi.useRealTimers();
  });

  it('handles clipboard rejection gracefully (no error thrown)', async () => {
    vi.mocked(navigator.clipboard.writeText).mockRejectedValue(
      new Error('Clipboard denied'),
    );
    render(<CopyButton content="test" align="right" />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
    });
    // Button should remain in un-copied state after failure
    expect(screen.getByRole('button', { name: 'Copy message' })).toBeTruthy();
  });

  it('handles multiple rapid clicks', async () => {
    render(<CopyButton content="rapid" align="left" />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copied' }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copied' }));
    });
    expect(navigator.clipboard.writeText).toHaveBeenCalledTimes(3);
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('rapid');
  });

  it('renders with align="left" positioning (justify-start class)', () => {
    const { container } = render(<CopyButton content="test" align="left" />);
    const wrapper = container.firstElementChild;
    expect(wrapper?.classList.contains('justify-start')).toBe(true);
  });

  it('renders with align="right" positioning (justify-end class)', () => {
    const { container } = render(<CopyButton content="test" align="right" />);
    const wrapper = container.firstElementChild;
    expect(wrapper?.classList.contains('justify-end')).toBe(true);
  });

  it('has accessible button role and label', () => {
    render(<CopyButton content="accessible" align="left" />);
    const button = screen.getByRole('button', { name: 'Copy message' });
    expect(button).toBeTruthy();
  });
});
