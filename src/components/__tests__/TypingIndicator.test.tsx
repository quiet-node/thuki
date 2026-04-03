import { render, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { TypingIndicator } from '../TypingIndicator';

/** Step duration kept in sync with the component constant (110ms). */
const STEP_MS = 110;
/** Hold duration before center dims (200ms). */
const FADE_MS = 200;

/**
 * Grid-flat index of a [row, col] cell rendered in row-major order.
 * [0,0]=0, [0,1]=1, [0,2]=2, [1,0]=3, [1,1]=4, [1,2]=5,
 * [2,0]=6, [2,1]=7, [2,2]=8
 */
function gridIdx(row: number, col: number) {
  return row * 3 + col;
}

describe('TypingIndicator', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('renders exactly nine dots in a 3×3 grid', () => {
    const { container } = render(<TypingIndicator />);
    expect(container.querySelectorAll('.rounded-full')).toHaveLength(9);
  });

  it('renders without AI bubble wrapper (no chat-bubble-ai class)', () => {
    const { container } = render(<TypingIndicator />);
    expect(container.querySelector('.chat-bubble-ai')).toBeNull();
  });

  it('is left-aligned (justify-start on outer wrapper)', () => {
    const { container } = render(<TypingIndicator />);
    expect(
      container.firstElementChild?.classList.contains('justify-start'),
    ).toBe(true);
  });

  it('has accessible status role and label', () => {
    const { container } = render(<TypingIndicator />);
    const grid = container.querySelector('[role="status"]');
    expect(grid).not.toBeNull();
    expect(grid?.getAttribute('aria-label')).toBe('AI is thinking');
  });

  it('lights up top-right dot (spiral start) on mount', () => {
    const { container } = render(<TypingIndicator />);
    const dots = container.querySelectorAll('.rounded-full');
    // Spiral index 0 → grid [0,2] → flat index 2
    expect(dots[gridIdx(0, 2)]?.classList.contains('bg-primary')).toBe(true);
  });

  it('advances active dot to top-middle after one STEP_MS tick', () => {
    const { container } = render(<TypingIndicator />);
    act(() => {
      vi.advanceTimersByTime(STEP_MS);
    });
    const dots = container.querySelectorAll('.rounded-full');
    // Step 1 → [0,1] active; step 0 → [0,2] becomes trail-1
    expect(dots[gridIdx(0, 1)]?.classList.contains('bg-primary')).toBe(true);
    expect(dots[gridIdx(0, 2)]?.classList.contains('bg-primary/50')).toBe(true);
  });

  it('lights up center dot after 8 ticks', () => {
    const { container } = render(<TypingIndicator />);
    act(() => {
      vi.advanceTimersByTime(STEP_MS * 8);
    });
    const dots = container.querySelectorAll('.rounded-full');
    // Spiral index 8 → grid [1,1] → flat index 4
    expect(dots[gridIdx(1, 1)]?.classList.contains('bg-primary')).toBe(true);
  });

  it('all dots go idle after center hold (FADE_MS after reaching center)', () => {
    const { container } = render(<TypingIndicator />);
    act(() => {
      vi.advanceTimersByTime(STEP_MS * 8 + FADE_MS);
    });
    const dots = container.querySelectorAll('.rounded-full');
    Array.from(dots).forEach((dot) => {
      expect(dot.classList.contains('bg-primary')).toBe(false);
    });
  });

  it('restarts cycle: top-right dot is active again after full pause', () => {
    const { container } = render(<TypingIndicator />);
    // Advance through all 8 steps + center hold (FADE_MS) + full pause (500ms).
    // At this exact moment setStep(0) and setDimmed(false) have fired but the
    // next tick has not yet run — spiral index 0 ([0,2]) is active again.
    act(() => {
      vi.advanceTimersByTime(STEP_MS * 8 + FADE_MS + 500);
    });
    const dots = container.querySelectorAll('.rounded-full');
    // Spiral index 0 → grid [0,2] → flat index 2 — back to active
    expect(dots[gridIdx(0, 2)]?.classList.contains('bg-primary')).toBe(true);
  });
});
