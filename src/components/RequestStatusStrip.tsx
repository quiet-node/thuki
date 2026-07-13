import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ThreeDotMotion } from './ThreeDotMotion';

/**
 * Props for the unified post-submit status strip (engine load, search, think).
 */
export interface RequestStatusStripProps {
  /**
   * Optional stage label next to the three-dot motion. When null/undefined,
   * only the dots render (plus any accessory). Label text uses production
   * shimmer plus G-style tracking-settle when the string changes.
   */
  label?: string | null;
  /**
   * Optional node between the dots host and the title (child order: dots,
   * accessory, title). Use for expand chevrons on search/reasoning hosts.
   * Leave undefined for engine-load and other dots-only rows.
   */
  accessory?: ReactNode;
}

/** Outgoing tracking-settle duration (ms). */
const TRACK_OUT_MS = 400;
/** Incoming tracking-settle duration (ms). */
const TRACK_IN_MS = 480;

/**
 * Unified post-submit status strip: Y1 three-dot motion + optional shimmer label.
 *
 * Pixel-identical in every host (engine row, search progress, reasoning).
 * Layout (gap, type size, min-height) lives only on this component and its
 * CSS classes. Optional `accessory` sits between dots and title so chevrons
 * can share the strip gap; parents must not pass size variants or inject
 * label prefixes.
 *
 * Drivers stay outside: callers pass the current label only.
 */
export function RequestStatusStrip({
  label,
  accessory,
}: RequestStatusStripProps) {
  const [displayLabel, setDisplayLabel] = useState(label ?? '');
  const [phase, setPhase] = useState<'in' | 'out' | 'enter-from'>('in');
  /**
   * Last label value reconciled against, used only to detect prop changes
   * during render (see below). Mirrors what `displayRef` used to do as a
   * plain ref.
   */
  const [lastLabel, setLastLabel] = useState(label ?? '');
  /**
   * Non-null while an out→enter→in transition is pending or in flight; a
   * fresh object every time so the timer effect below always restarts even
   * if the target text repeats. Reset to null by the immediate (untimed)
   * branches so the effect only fires for real animated transitions, and so
   * internal `phase` churn (out → enter-from → in) never retriggers or
   * cancels it early.
   */
  const [pendingTransition, setPendingTransition] = useState<{
    target: string;
  } | null>(null);
  const timersRef = useRef<ReturnType<typeof setTimeout>[]>([]);

  /**
   * Clears pending tracking-settle timeouts (unmount or superseded change).
   */
  function clearTimers(): void {
    for (const id of timersRef.current) {
      clearTimeout(id);
    }
    timersRef.current = [];
  }

  // Derive display state from the `label` prop during render (React's
  // "adjusting state when a prop changes" pattern) rather than in an
  // effect, since these branches are pure reactions to the prop and need
  // no side effect of their own; only the animated out→enter→in sequence
  // below needs a real effect (it schedules timers).
  const next = label ?? '';
  if (next !== lastLabel) {
    setLastLabel(next);
    if (!next) {
      // Empty label: drop text immediately (dots-only mode).
      setDisplayLabel('');
      setPhase('in');
      setPendingTransition(null);
    } else if (!lastLabel) {
      // First paint with a label: no exit animation.
      setDisplayLabel(next);
      setPhase('in');
      setPendingTransition(null);
    } else {
      // Mid-flight change: flip to 'out' and hand the target to the timer
      // effect below, which owns the out → enter → in timing.
      setPhase('out');
      setPendingTransition({ target: next });
    }
  }

  useEffect(() => {
    if (!pendingTransition) return;
    const { target } = pendingTransition;
    // eslint-disable-next-line @eslint-react/web-api-no-leaked-timeout -- tracked in timersRef, cleared in cleanup below
    const tOut = setTimeout(() => {
      setDisplayLabel(target);
      setPhase('enter-from');
      // Double rAF so the browser applies enter-from before transitioning to in.
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          setPhase('in');
        });
      });
      // eslint-disable-next-line @eslint-react/web-api-no-leaked-timeout -- tracked in timersRef, cleared in cleanup below
      const tIn = setTimeout(() => {
        /* settle complete */
      }, TRACK_IN_MS);
      timersRef.current.push(tIn);
    }, TRACK_OUT_MS);
    timersRef.current.push(tOut);
    return () => {
      clearTimers();
    };
  }, [pendingTransition]);

  const showLabel = Boolean(displayLabel);

  return (
    <span className="request-status-strip" data-testid="request-status-strip">
      <span className="request-status-strip__dots shrink-0">
        <ThreeDotMotion />
      </span>
      {accessory ?? null}
      {showLabel ? (
        <span
          data-testid="loading-stage-title"
          className="request-status-strip__title"
        >
          <span
            data-testid="loading-label"
            // Logical stage is the prop (immediate); visible glyphs may lag
            // one tracking-settle cycle for the soft appear animation.
            data-label={label ?? displayLabel}
            className={`loading-label loading-label-track request-status-strip__label min-w-0 ${
              phase === 'out'
                ? 'loading-label-track-out'
                : phase === 'enter-from'
                  ? 'loading-label-track-enter'
                  : 'loading-label-track-in'
            }`}
          >
            {displayLabel}
          </span>
        </span>
      ) : null}
    </span>
  );
}
