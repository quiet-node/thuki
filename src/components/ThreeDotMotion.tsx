import { useEffect, useMemo, useRef } from 'react';
import {
  TRAIL_LAG_MS,
  TRAIL_OPACITY,
  TRAIL_SCALE,
  flattenY1Timeline,
  linePose,
  poseAtElapsed,
  pushTrailHistory,
  sampleTrail,
  type DotPose,
  type TrailSample,
} from './threeDotMotionMath';

/**
 * Brand coral for the middle leader (Thuki primary).
 */
const DOT_BRAND = '#ff8d5c';
/**
 * Soft warm cream for the outer leaders (locked design swatch).
 */
const DOT_SOFT = '#f0d0c0';
const DOT_BRAND_GLOW = 'rgba(255, 141, 92, 0.55)';
const DOT_SOFT_GLOW = 'rgba(240, 208, 192, 0.4)';

/**
 * Compact three-dot Y1 Full-suite motion for {@link RequestStatusStrip}.
 *
 * Middle leader is brand orange; outer leaders are warm cream. Each leader
 * leaves a tapered trail of six ghosts. The loop is wave → morph → snappy
 * triangle spin → figure-eight → orbit ladder → settle.
 *
 * Uses a single requestAnimationFrame loop; trail histories are age-capped
 * so memory cannot grow unbounded. Honors `prefers-reduced-motion` by
 * freezing on a flat line pose.
 */
export function ThreeDotMotion() {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const leadersRef = useRef<(HTMLSpanElement | null)[]>([]);
  const trailsRef = useRef<(HTMLSpanElement | null)[]>([]);
  const timeline = useMemo(() => flattenY1Timeline(), []);

  useEffect(() => {
    const reduced =
      typeof window !== 'undefined' &&
      typeof window.matchMedia === 'function' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches;

    const histories: TrailSample[][] = [[], [], []];
    const t0 = performance.now();

    /**
     * Writes a pose onto leader DOM nodes (left / middle / right).
     */
    /**
     * Writes a pose onto leader DOM nodes (left / middle / right).
     * Refs are always set before the effect paints leaders in production;
     * null checks are defensive for teardown races.
     */
    function placeLeaders(pose: DotPose): void {
      for (let i = 0; i < 3; i++) {
        const el = leadersRef.current[i];
        /* v8 ignore start -- ref null only during unmount race */
        if (!el) continue;
        /* v8 ignore stop */
        el.style.left = `${pose[i].x}px`;
        el.style.top = `${pose[i].y}px`;
      }
    }

    /**
     * Updates trail ghosts from age-capped per-leader history.
     */
    function placeTrails(now: number): void {
      let idx = 0;
      for (let i = 0; i < 3; i++) {
        for (let t = 0; t < TRAIL_LAG_MS.length; t++) {
          const el = trailsRef.current[idx++];
          /* v8 ignore start -- ref null only during unmount race */
          if (!el) continue;
          /* v8 ignore stop */
          const sample = sampleTrail(histories[i], now, TRAIL_LAG_MS[t]);
          if (!sample) {
            el.style.opacity = '0';
            continue;
          }
          el.style.left = `${sample.x}px`;
          el.style.top = `${sample.y}px`;
          el.style.opacity = String(TRAIL_OPACITY[t]);
          el.style.transform = `scale(${TRAIL_SCALE[t]})`;
        }
      }
    }

    if (reduced) {
      const pose = linePose();
      placeLeaders(pose);
      // No history yet: ghosts stay hidden under reduced motion.
      placeTrails(performance.now());
      return;
    }

    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    /**
     * One animation sample: Y1 pose, leaders, age-capped trails.
     * Loops via setTimeout(~60fps) so unit-test sync rAF stubs cannot
     * recurse infinitely (repo setup invokes rAF callbacks immediately).
     */
    function tick(): void {
      /* v8 ignore start -- cancelled only after unmount clears the timer */
      if (cancelled) return;
      /* v8 ignore stop */
      const now = performance.now();
      const { pose } = poseAtElapsed(now - t0, timeline);
      placeLeaders(pose);
      pushTrailHistory(histories, pose, now);
      placeTrails(now);
      timer = setTimeout(tick, 16);
    }

    tick();
    return () => {
      cancelled = true;
      // tick() always schedules before paint returns; timer is non-null here.
      clearTimeout(timer as ReturnType<typeof setTimeout>);
    };
  }, [timeline]);

  return (
    <div
      ref={hostRef}
      className="tdm-host"
      data-testid="three-dot-motion"
      role="status"
      aria-label="AI is thinking"
    >
      {([0, 1, 2] as const).flatMap((i) =>
        TRAIL_LAG_MS.map((lagMs, t) => (
          <span
            key={`trail-${i}-lag-${lagMs}`}
            ref={(el) => {
              trailsRef.current[i * TRAIL_LAG_MS.length + t] = el;
            }}
            className="tdm-trail"
            data-leader={i}
            data-trail={t}
            style={{
              background: i === 1 ? DOT_BRAND : DOT_SOFT,
              boxShadow:
                i === 1
                  ? `0 0 6px ${DOT_BRAND_GLOW}`
                  : `0 0 5px ${DOT_SOFT_GLOW}`,
            }}
          />
        )),
      )}
      {([0, 1, 2] as const).map((i) => (
        <span
          key={`leader-${i}`}
          ref={(el) => {
            leadersRef.current[i] = el;
          }}
          className="tdm-dot"
          data-role={i === 1 ? 'brand' : 'soft'}
          data-testid={i === 1 ? 'tdm-dot-brand' : `tdm-dot-soft-${i}`}
          style={{
            background: i === 1 ? DOT_BRAND : DOT_SOFT,
            boxShadow:
              i === 1
                ? `0 0 9px ${DOT_BRAND_GLOW}`
                : `0 0 7px ${DOT_SOFT_GLOW}`,
          }}
        />
      ))}
    </div>
  );
}
