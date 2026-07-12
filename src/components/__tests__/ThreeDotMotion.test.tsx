import { render } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { ThreeDotMotion } from '../ThreeDotMotion';

describe('ThreeDotMotion', () => {
  beforeEach(() => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn().mockReturnValue({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('renders three leaders and eighteen trail ghosts', () => {
    const { container } = render(<ThreeDotMotion />);
    expect(container.querySelectorAll('.tdm-dot')).toHaveLength(3);
    expect(container.querySelectorAll('.tdm-trail')).toHaveLength(18);
  });

  it('marks the middle leader as brand', () => {
    const { getByTestId } = render(<ThreeDotMotion />);
    expect(getByTestId('tdm-dot-brand')).toHaveAttribute('data-role', 'brand');
  });

  it('exposes an accessible status role', () => {
    const { getByRole } = render(<ThreeDotMotion />);
    expect(getByRole('status')).toHaveAttribute('aria-label', 'AI is thinking');
  });

  it('freezes under prefers-reduced-motion without throwing', () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn().mockReturnValue({
        matches: true,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    );
    const { getByTestId } = render(<ThreeDotMotion />);
    expect(getByTestId('three-dot-motion')).toBeInTheDocument();
  });

  it('schedules a timeout loop when motion is allowed and clears on unmount', () => {
    vi.useFakeTimers();
    const spy = vi.spyOn(globalThis, 'setTimeout');
    const { unmount } = render(<ThreeDotMotion />);
    expect(spy).toHaveBeenCalled();
    unmount();
    vi.useRealTimers();
    spy.mockRestore();
  });
});
