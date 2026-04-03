import { motion } from 'framer-motion';
import { useEffect, useRef, useState } from 'react';

/**
 * Spiral traversal order for the 3×3 dot grid.
 * Starts top-right, sweeps the outer ring, ends at center.
 * Coordinates are [row, col], zero-indexed from the top-left.
 */
const SPIRAL_PATH: readonly [number, number][] = [
  [0, 2],
  [0, 1],
  [0, 0],
  [1, 0],
  [2, 0],
  [2, 1],
  [2, 2],
  [1, 2],
  [1, 1],
] as const;

/** Milliseconds per dot advance along the spiral. */
const STEP_MS = 110;
/** Milliseconds the center dot stays lit before fading. */
const FADE_MS = 200;
/** Milliseconds of all-idle pause after center fades, before next cycle. */
const PAUSE_MS = 500;

/** Pre-computed reverse lookup: "row,col" → spiral path index. */
const PATH_INDEX: Readonly<Record<string, number>> = Object.fromEntries(
  SPIRAL_PATH.map(([r, c], i) => [`${r},${c}`, i]),
);

type Intensity = 'active' | 'trail1' | 'trail2' | 'idle';

const INTENSITY_CLASS: Record<Intensity, string> = {
  active: 'bg-primary',
  trail1: 'bg-primary/50',
  trail2: 'bg-primary/[0.22]',
  idle: 'bg-white/[0.18]',
};

/** Returns the brightness level of a dot given the current animation state. */
function getIntensity(
  row: number,
  col: number,
  step: number,
  dimmed: boolean,
): Intensity {
  if (dimmed) return 'idle';
  const idx = PATH_INDEX[`${row},${col}`];
  if (idx === step) return 'active';
  if (step >= 1 && idx === step - 1) return 'trail1';
  if (step >= 2 && idx === step - 2) return 'trail2';
  return 'idle';
}

/**
 * Nine-dot spiral loading indicator.
 *
 * Renders a 3×3 grid of dots. The brand color sweeps along the spiral path
 * (top-right → outer ring clockwise → center) at `STEP_MS` per dot.
 * On reaching the center the active dot holds for `FADE_MS`, then all dots
 * dim for `PAUSE_MS` before the next cycle begins.
 *
 * No shadows or glows — pure color transitions only.
 */
export function TypingIndicator() {
  const [step, setStep] = useState(0);
  const [dimmed, setDimmed] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let currentStep = 0;
    let cancelled = false;

    /** Schedule `fn` after `delay` ms; no-ops if cancelled. */
    function schedule(fn: () => void, delay: number) {
      timerRef.current = setTimeout(() => {
        /* v8 ignore start -- cancelled guard: only reachable when component unmounts mid-tick */
        if (!cancelled) fn();
        /* v8 ignore stop */
      }, delay);
    }

    function tick() {
      currentStep += 1;
      setStep(currentStep);

      if (currentStep === SPIRAL_PATH.length - 1) {
        // Reached center — hold, then dim, then pause before restart.
        schedule(() => {
          setDimmed(true);
          schedule(() => {
            currentStep = 0;
            setStep(0);
            setDimmed(false);
            schedule(tick, STEP_MS);
          }, PAUSE_MS);
        }, FADE_MS);
      } else {
        schedule(tick, STEP_MS);
      }
    }

    // First step fires after STEP_MS; initial render already shows step 0.
    schedule(tick, STEP_MS);

    return () => {
      cancelled = true;
      /* v8 ignore start -- timerRef null guard: ref is always set by the time cleanup runs */
      if (timerRef.current) clearTimeout(timerRef.current);
      /* v8 ignore stop */
    };
  }, []);

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      className="flex w-full justify-start py-1"
    >
      <div
        className="grid gap-[3px]"
        style={{ gridTemplateColumns: 'repeat(3, 3px)' }}
        role="status"
        aria-label="AI is thinking"
      >
        {([0, 1, 2] as const).flatMap((row) =>
          ([0, 1, 2] as const).map((col) => (
            <div
              key={`${row},${col}`}
              className={`w-[3px] h-[3px] rounded-full transition-colors duration-[100ms] ${INTENSITY_CLASS[getIntensity(row, col, step, dimmed)]}`}
            />
          )),
        )}
      </div>
    </motion.div>
  );
}
