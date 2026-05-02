import { useState, useEffect, useRef } from 'react';
import { TIPS, type Tip } from '../config/tips';

/** Random pause before the first cycle and between cycles (seconds). */
const CYCLE_PAUSE_MIN_MS = 30_000;
const CYCLE_PAUSE_MAX_MS = 45_000;

/** Fixed display time for each tip in a cycle. */
const TIP_HOLD_MS = 20_000;

/** How many tips to show per cycle (1 or 2). */
const TIPS_PER_CYCLE_MIN = 1;
const TIPS_PER_CYCLE_MAX = 2;

function randBetween(min: number, max: number): number {
  return min + Math.floor(Math.random() * (max - min + 1));
}

/**
 * Fisher-Yates shuffle: returns a new random permutation of indices [0, n-1].
 * Used to fill the tip deck so every tip is seen once before any repeats.
 */
function shuffled(n: number): number[] {
  const arr = Array.from({ length: n }, (_, i) => i);
  for (let i = arr.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [arr[i], arr[j]] = [arr[j], arr[i]];
  }
  return arr;
}

export function useTips(active: boolean): {
  tip: Tip;
  tipKey: number;
  isVisible: boolean;
} {
  const [index, setIndex] = useState(0);
  const [tipKey, setTipKey] = useState(-1);
  const [isVisible, setIsVisible] = useState(false);

  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  // Persists across overlay open/close cycles for the lifetime of the app.
  const tipDeckRef = useRef<number[]>([]);

  useEffect(() => {
    if (!active) return;

    function nextTipIndex(): number {
      if (tipDeckRef.current.length === 0) {
        tipDeckRef.current = shuffled(TIPS.length);
      }
      return tipDeckRef.current.shift()!;
    }

    function startShowing() {
      const tipsThisCycle = randBetween(TIPS_PER_CYCLE_MIN, TIPS_PER_CYCLE_MAX);
      let shownCount = 0;

      setIndex(nextTipIndex());
      setTipKey((k) => k + 1);
      setIsVisible(true);

      intervalRef.current = setInterval(() => {
        shownCount++;
        if (shownCount >= tipsThisCycle) {
          clearInterval(intervalRef.current!);
          intervalRef.current = null;
          setIsVisible(false);

          timerRef.current = setTimeout(
            () => {
              timerRef.current = null;
              startShowing();
            },
            randBetween(CYCLE_PAUSE_MIN_MS, CYCLE_PAUSE_MAX_MS),
          );
        } else {
          setIndex(nextTipIndex());
          setTipKey((k) => k + 1);
        }
      }, TIP_HOLD_MS);
    }

    timerRef.current = setTimeout(
      () => {
        timerRef.current = null;
        startShowing();
      },
      randBetween(CYCLE_PAUSE_MIN_MS, CYCLE_PAUSE_MAX_MS),
    );

    return () => {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      if (intervalRef.current !== null) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
      setIsVisible(false);
    };
  }, [active]);

  return { tip: TIPS[index], tipKey, isVisible };
}
