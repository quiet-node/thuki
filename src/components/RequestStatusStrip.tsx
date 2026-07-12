import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ThreeDotMotion } from './ThreeDotMotion';

/**
 * Props for the unified post-submit status strip (engine load, search, think).
 */
export interface RequestStatusStripProps {
  /**
   * Optional stage label next to the three-dot motion. When null/undefined,
   * only the dots render. Label text uses production shimmer plus G-style
   * tracking-settle when the string changes.
   */
  label?: string | null;
  /**
   * Compact typography for nested chrome (search progress header, etc.).
   */
  compact?: boolean;
  /**
   * Optional prefix before the label (e.g. reasoning chevron spacer).
   */
  labelPrefix?: ReactNode;
}

/** Outgoing tracking-settle duration (ms). */
const TRACK_OUT_MS = 400;
/** Incoming tracking-settle duration (ms). */
const TRACK_IN_MS = 480;

/**
 * Unified post-submit status strip: Y1 three-dot motion + optional shimmer label.
 *
 * Replaces the old 9-dot `TypingIndicator` + plain `LoadingStage` pair so
 * engine cold-start, web search stages, and `/think` share one visual language
 * (locked design: warm cream outer dots, brand middle, trails, App.css
 * shimmer, G tracking-settle on copy change).
 *
 * Drivers stay outside: callers pass the current label; this component only
 * owns presentation and label-change motion.
 */
export function RequestStatusStrip({
  label,
  compact = false,
  labelPrefix,
}: RequestStatusStripProps) {
  const [displayLabel, setDisplayLabel] = useState(label ?? '');
  const [phase, setPhase] = useState<'in' | 'out' | 'enter-from'>('in');
  /** Mirrors displayed text so the effect can key only on `label`. */
  const displayRef = useRef(label ?? '');
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

  useEffect(() => {
    const next = label ?? '';
    if (next === displayRef.current) {
      return;
    }
    // Empty label: drop text immediately (dots-only mode).
    if (!next) {
      clearTimers();
      displayRef.current = '';
      setDisplayLabel('');
      setPhase('in');
      return;
    }
    // First paint with a label: no exit animation.
    if (!displayRef.current) {
      displayRef.current = next;
      setDisplayLabel(next);
      setPhase('in');
      return;
    }
    // Prior transition timers are cleared by this effect's cleanup when
    // `label` changes mid-flight, then we start a fresh out → enter → in.
    setPhase('out');
    const tOut = setTimeout(() => {
      displayRef.current = next;
      setDisplayLabel(next);
      setPhase('enter-from');
      // Double rAF so the browser applies enter-from before transitioning to in.
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          setPhase('in');
        });
      });
      const tIn = setTimeout(() => {
        /* settle complete */
      }, TRACK_IN_MS);
      timersRef.current.push(tIn);
    }, TRACK_OUT_MS);
    timersRef.current.push(tOut);
    return () => {
      clearTimers();
    };
  }, [label]);

  const showLabel = Boolean(displayLabel);

  return (
    <span
      className="inline-flex items-center gap-2"
      data-testid="request-status-strip"
    >
      <span className="shrink-0">
        <ThreeDotMotion />
      </span>
      {showLabel ? (
        <span
          data-testid="loading-stage-title"
          className={`inline-flex min-w-0 items-center ${compact ? 'gap-1 text-[11px] leading-none' : 'gap-1.5 text-xs'}`}
        >
          {labelPrefix ? (
            <span
              data-testid="loading-label-prefix"
              className="inline-flex shrink-0 items-center"
            >
              {labelPrefix}
            </span>
          ) : null}
          <span
            data-testid="loading-label"
            // Logical stage is the prop (immediate); visible glyphs may lag
            // one tracking-settle cycle for the soft appear animation.
            data-label={label ?? displayLabel}
            className={`loading-label loading-label-track min-w-0 ${
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
