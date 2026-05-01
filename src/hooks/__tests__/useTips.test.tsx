import { renderHook, act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useTips } from '../useTips';
import { TIPS } from '../../config/tips';

const CYCLE_PAUSE = 30_000; // CYCLE_PAUSE_MIN_MS; with random=0 → 30000
const TIP_HOLD = 20_000; // TIP_HOLD_MS (fixed)

// With Math.random() = 0:
//   randBetween(CYCLE_PAUSE_MIN_MS, CYCLE_PAUSE_MAX_MS) → 30000
//   randBetween(TIPS_PER_CYCLE_MIN, TIPS_PER_CYCLE_MAX) → 1
//   shuffled(12) with all j=0 produces [1, 2, 3, ..., 11, 0]
//     so nextTipIndex() calls return 1, 2, 3, ... in order

describe('useTips', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('isVisible is false initially', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { result } = renderHook(() => useTips(true));
    expect(result.current.isVisible).toBe(false);
  });

  it('isVisible is false before cycle pause fires', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { result } = renderHook(() => useTips(true));
    act(() => {
      vi.advanceTimersByTime(29_000);
    });
    expect(result.current.isVisible).toBe(false);
  });

  it('isVisible becomes true after cycle pause fires', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { result } = renderHook(() => useTips(true));
    act(() => {
      vi.advanceTimersByTime(CYCLE_PAUSE);
    });
    expect(result.current.isVisible).toBe(true);
  });

  it('tip and tipKey update on each interval tick during showing', () => {
    // tipsThisCycle=2: first tick advances tip rather than ending the cycle
    // shuffled(12) with all 0s → [1,2,...,11,0]; first two pops: TIPS[1], TIPS[2]
    vi.spyOn(Math, 'random')
      .mockReturnValueOnce(0) // cycle pause → 30000
      .mockReturnValueOnce(0.5) // tipsThisCycle → 2
      .mockReturnValue(0); // shuffle deck + rest

    const { result } = renderHook(() => useTips(true));
    act(() => {
      vi.advanceTimersByTime(CYCLE_PAUSE);
    });
    expect(result.current.tip).toBe(TIPS[1]);
    expect(result.current.isVisible).toBe(true);
    const keyAfterStart = result.current.tipKey;

    act(() => {
      vi.advanceTimersByTime(TIP_HOLD);
    });
    expect(result.current.tip).toBe(TIPS[2]);
    expect(result.current.tipKey).toBe(keyAfterStart + 1);
    expect(result.current.isVisible).toBe(true);
  });

  it('isVisible returns false after exactly N=1 tip shown', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0); // tipsThisCycle=1
    const { result } = renderHook(() => useTips(true));
    act(() => {
      vi.advanceTimersByTime(CYCLE_PAUSE);
    });
    expect(result.current.isVisible).toBe(true);
    act(() => {
      vi.advanceTimersByTime(TIP_HOLD);
    });
    expect(result.current.isVisible).toBe(false);
  });

  it('isVisible returns true again after cycle pause following showing phase', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0); // tipsThisCycle=1 each cycle
    const { result } = renderHook(() => useTips(true));
    act(() => {
      vi.advanceTimersByTime(CYCLE_PAUSE + TIP_HOLD);
    });
    expect(result.current.isVisible).toBe(false);
    act(() => {
      vi.advanceTimersByTime(CYCLE_PAUSE);
    });
    expect(result.current.isVisible).toBe(true);
  });

  it('deactivating during waiting cancels timer, isVisible stays false', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { result, rerender } = renderHook(({ active }) => useTips(active), {
      initialProps: { active: true },
    });
    act(() => {
      vi.advanceTimersByTime(29_000);
    });
    rerender({ active: false });
    act(() => {
      vi.advanceTimersByTime(CYCLE_PAUSE);
    });
    expect(result.current.isVisible).toBe(false);
  });

  it('deactivating during showing sets isVisible to false immediately', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { result, rerender } = renderHook(({ active }) => useTips(active), {
      initialProps: { active: true },
    });
    act(() => {
      vi.advanceTimersByTime(CYCLE_PAUSE);
    });
    expect(result.current.isVisible).toBe(true);
    rerender({ active: false });
    expect(result.current.isVisible).toBe(false);
  });

  it('deactivating during resting keeps isVisible false and cancels rest timer', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { result, rerender } = renderHook(({ active }) => useTips(active), {
      initialProps: { active: true },
    });
    act(() => {
      vi.advanceTimersByTime(CYCLE_PAUSE + TIP_HOLD);
    });
    expect(result.current.isVisible).toBe(false);
    rerender({ active: false });
    act(() => {
      vi.advanceTimersByTime(CYCLE_PAUSE);
    });
    expect(result.current.isVisible).toBe(false);
  });

  it('re-activating starts a fresh cycle pause', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { result, rerender } = renderHook(({ active }) => useTips(active), {
      initialProps: { active: false },
    });
    rerender({ active: true });
    act(() => {
      vi.advanceTimersByTime(29_000);
    });
    expect(result.current.isVisible).toBe(false);
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(result.current.isVisible).toBe(true);
  });

  it('tipKey increments on each tip change', () => {
    // tipsThisCycle=2: both startShowing and the first tick each increment tipKey
    vi.spyOn(Math, 'random')
      .mockReturnValueOnce(0) // cycle pause → 30000
      .mockReturnValueOnce(0.5) // tipsThisCycle → 2
      .mockReturnValue(0); // shuffle deck + rest

    const { result } = renderHook(() => useTips(true));
    const initialKey = result.current.tipKey;

    act(() => {
      vi.advanceTimersByTime(CYCLE_PAUSE);
    });
    expect(result.current.tipKey).toBe(initialKey + 1);

    act(() => {
      vi.advanceTimersByTime(TIP_HOLD);
    });
    expect(result.current.tipKey).toBe(initialKey + 2);
  });

  it('tip deck refills after all tips have been shown once', () => {
    // With random=0: tipsThisCycle=1 per cycle, each full cycle = TIP_HOLD + CYCLE_PAUSE.
    // After initial CYCLE_PAUSE + TIPS.length full cycles the deck is exhausted and refilled,
    // and the next showing begins: isVisible should be true.
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { result } = renderHook(() => useTips(true));

    act(() => {
      vi.advanceTimersByTime(
        CYCLE_PAUSE + TIPS.length * (TIP_HOLD + CYCLE_PAUSE),
      );
    });
    expect(result.current.isVisible).toBe(true);
  });
});
