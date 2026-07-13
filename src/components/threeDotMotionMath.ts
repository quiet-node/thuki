/**
 * Pure geometry and Y1 Full-suite timeline for {@link ThreeDotMotion}.
 * Kept free of React so pose math can be unit-tested without timers.
 */

/** 2D point inside the motion host (CSS px from host top-left). */
export type DotPoint = { x: number; y: number };

/** Three-leader pose: left, middle (brand), right. */
export type DotPose = readonly [DotPoint, DotPoint, DotPoint];

/** Host width used by the compact strip (matches CSS `.tdm-host`). */
export const TDM_HOST_W = 24;
/** Host height used by the compact strip. */
export const TDM_HOST_H = 22;
/** Horizontal center of the host. */
export const TDM_CX = TDM_HOST_W / 2;
/** Vertical center of the host. */
export const TDM_CY = TDM_HOST_H / 2;
/** Half-gap between outer dots on a flat line (compact). */
export const TDM_SPREAD = 6.2;

/**
 * Horizontal line pose with optional per-dot vertical offsets (px).
 */
export function linePose(y0 = 0, y1 = 0, y2 = 0, spread = TDM_SPREAD): DotPose {
  return [
    { x: TDM_CX - spread, y: TDM_CY + y0 },
    { x: TDM_CX, y: TDM_CY + y1 },
    { x: TDM_CX + spread, y: TDM_CY + y2 },
  ];
}

/**
 * Arc pose (middle raised or lowered). `flip` -1 inverts the arc.
 */
export function arcPose(amp: number, flip = 1, spread = TDM_SPREAD): DotPose {
  return [
    { x: TDM_CX - spread, y: TDM_CY + amp * 0.55 * flip },
    { x: TDM_CX, y: TDM_CY - amp * flip },
    { x: TDM_CX + spread, y: TDM_CY + amp * 0.55 * flip },
  ];
}

/**
 * Equilateral triangle pose rotated by `angleDeg` degrees around the host center.
 */
export function trianglePose(angleDeg: number, r = 6.2): DotPose {
  const a = (angleDeg * Math.PI) / 180;
  return [0, 120, 240].map((d) => {
    const t = a + (d * Math.PI) / 180;
    return { x: TDM_CX + Math.cos(t) * r, y: TDM_CY + Math.sin(t) * r };
  }) as unknown as DotPose;
}

/**
 * Linear interpolation between two numbers.
 */
export function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

/**
 * Linear interpolation between two poses.
 */
export function lerpPose(a: DotPose, b: DotPose, t: number): DotPose {
  return [
    { x: lerp(a[0].x, b[0].x, t), y: lerp(a[0].y, b[0].y, t) },
    { x: lerp(a[1].x, b[1].x, t), y: lerp(a[1].y, b[1].y, t) },
    { x: lerp(a[2].x, b[2].x, t), y: lerp(a[2].y, b[2].y, t) },
  ];
}

/**
 * Smoothstep-ish ease used for morph segments (soft start/stop).
 */
export function easeInOutSoft(t: number): number {
  return t * t * t * (t * (t * 6 - 15) + 10);
}

/**
 * Standard ease-in-out used by the snappy triangle spin (matches design mock C).
 */
export function easeInOut(t: number): number {
  return t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2;
}

/**
 * C-style snappy triangle spin pose at progress `p` in [0, 1].
 */
export function snappySpinPose(p: number, turns = 1.35, baseR = 5.8): DotPose {
  const mag = Math.abs(turns) || 1;
  const sign = turns < 0 ? -1 : 1;
  const angle = sign * (easeInOut(p) * 360 * mag + p * 40);
  const r = baseR + Math.sin(p * Math.PI * 4 * mag) * 1.2;
  return trianglePose(angle, r);
}

/**
 * Samples a figure-8 path for three dots with phase offsets.
 */
export function figure8Pose(t: number, r = 6.5): DotPose {
  return [0, 0.33, 0.66].map((off) => {
    const u = (t + off) * Math.PI * 2;
    return {
      x: TDM_CX + Math.sin(u) * r,
      y: TDM_CY + Math.sin(u) * Math.cos(u) * r * 0.95,
    };
  }) as unknown as DotPose;
}

/** Named timeline segment for the Y1 suite. */
export type Y1Segment =
  | {
      name: string;
      kind: 'keys';
      keys: DotPose[];
      stepMs: number;
      bridgeMs: number;
    }
  | {
      name: string;
      kind: 'snappySpin';
      durationMs: number;
      turns: number;
      baseR: number;
      bridgeMs: number;
    };

/**
 * Builds the locked Y1 Full-suite segment list:
 * wave → morph steps → triangle spin → figure-eight → orbit ladder → settle.
 */
export function buildY1Segments(): Y1Segment[] {
  const waveKeys: DotPose[] = [
    linePose(0, 0, 0),
    linePose(1.5, -3.2, 1.5),
    linePose(-1.5, 3.2, -1.5),
    arcPose(4),
    arcPose(4, -1),
    linePose(0, 0, 0),
  ];
  const morphKeys: DotPose[] = [
    linePose(0, 0, 0),
    arcPose(4),
    trianglePose(200, 6.2),
    trianglePose(110, 6.2),
    trianglePose(30, 6.2),
    linePose(0, 0, 0),
  ];
  const figKeys: DotPose[] = [];
  for (let i = 0; i <= 8; i++) {
    figKeys.push(figure8Pose(i / 8, 6.5));
  }
  const orbitKeys: DotPose[] = [];
  for (let a = 0; a <= 360; a += 36) {
    orbitKeys.push(trianglePose(a, 6.2));
  }
  const settleKeys: DotPose[] = [
    trianglePose(40, 5.8),
    arcPose(2.8),
    linePose(0, 0, 0),
    linePose(0, 0, 0),
  ];
  return [
    { name: 'wave', kind: 'keys', keys: waveKeys, stepMs: 420, bridgeMs: 160 },
    {
      name: 'morph steps',
      kind: 'keys',
      keys: morphKeys,
      stepMs: 480,
      bridgeMs: 320,
    },
    {
      name: 'triangle spin',
      kind: 'snappySpin',
      durationMs: 1900,
      turns: 1.35,
      baseR: 5.8,
      bridgeMs: 420,
    },
    {
      name: 'figure-eight',
      kind: 'keys',
      keys: figKeys,
      stepMs: 360,
      bridgeMs: 360,
    },
    {
      name: 'orbit ladder',
      kind: 'keys',
      keys: orbitKeys,
      stepMs: 300,
      bridgeMs: 300,
    },
    {
      name: 'settle',
      kind: 'keys',
      keys: settleKeys,
      stepMs: 440,
      bridgeMs: 440,
    },
  ];
}

/** One morph step on the flattened Y1 timeline. */
export type Y1Step =
  | {
      kind: 'morph';
      from: DotPose;
      to: DotPose;
      ms: number;
      phase: string;
      ease: (t: number) => number;
    }
  | {
      kind: 'snappySpin';
      ms: number;
      turns: number;
      baseR: number;
      phase: string;
    };

/**
 * Returns the first pose of a Y1 segment (for bridges).
 */
export function firstPoseOf(seg: Y1Segment): DotPose {
  if (seg.kind === 'snappySpin') {
    return snappySpinPose(0, seg.turns, seg.baseR);
  }
  return seg.keys[0];
}

/**
 * Returns the last pose of a Y1 segment (for bridges).
 */
export function lastPoseOf(seg: Y1Segment): DotPose {
  if (seg.kind === 'snappySpin') {
    return snappySpinPose(1, seg.turns, seg.baseR);
  }
  return seg.keys[seg.keys.length - 1];
}

/**
 * Flattens Y1 segments into a looping step timeline with bridges.
 */
export function flattenY1Timeline(segments: Y1Segment[] = buildY1Segments()): {
  steps: Y1Step[];
  totalMs: number;
} {
  const steps: Y1Step[] = [];
  for (let s = 0; s < segments.length; s++) {
    const seg = segments[s];
    const next = segments[(s + 1) % segments.length];
    if (seg.kind === 'snappySpin') {
      steps.push({
        kind: 'snappySpin',
        ms: seg.durationMs,
        turns: seg.turns,
        baseR: seg.baseR,
        phase: seg.name,
      });
      steps.push({
        kind: 'morph',
        from: lastPoseOf(seg),
        to: firstPoseOf(next),
        ms: seg.bridgeMs,
        phase: `${seg.name} → ${next.name}`,
        ease: easeInOutSoft,
      });
      continue;
    }
    for (let i = 0; i < seg.keys.length - 1; i++) {
      steps.push({
        kind: 'morph',
        from: seg.keys[i],
        to: seg.keys[i + 1],
        ms: seg.stepMs,
        phase: seg.name,
        ease: easeInOutSoft,
      });
    }
    steps.push({
      kind: 'morph',
      from: lastPoseOf(seg),
      to: firstPoseOf(next),
      ms: seg.bridgeMs,
      phase: `${seg.name} → ${next.name}`,
      ease: easeInOutSoft,
    });
  }
  const totalMs = steps.reduce((a, st) => a + st.ms, 0);
  return { steps, totalMs };
}

/**
 * Samples the Y1 timeline at elapsed milliseconds since cycle start.
 */
export function poseAtElapsed(
  elapsedMs: number,
  timeline: { steps: Y1Step[]; totalMs: number } = flattenY1Timeline(),
): { pose: DotPose; phase: string } {
  const { steps, totalMs } = timeline;
  if (totalMs <= 0 || steps.length === 0) {
    return { pose: linePose(), phase: 'wave' };
  }
  let u = elapsedMs % totalMs;
  if (u < 0) u += totalMs;
  let acc = 0;
  for (const step of steps) {
    if (u < acc + step.ms) {
      const p = (u - acc) / step.ms;
      if (step.kind === 'snappySpin') {
        return {
          pose: snappySpinPose(p, step.turns, step.baseR),
          phase: step.phase,
        };
      }
      return {
        pose: lerpPose(step.from, step.to, step.ease(p)),
        phase: step.phase,
      };
    }
    acc += step.ms;
  }
  const last = steps[steps.length - 1];
  if (last.kind === 'snappySpin') {
    return {
      pose: snappySpinPose(1, last.turns, last.baseR),
      phase: last.phase,
    };
  }
  return { pose: last.to, phase: last.phase };
}

/** Trail lag offsets (ms) for the six ghost samples per leader. */
export const TRAIL_LAG_MS = [28, 56, 88, 124, 165, 210] as const;
/** How long history is retained for trail sampling (ms). */
export const TRAIL_MAX_AGE_MS = 280;
/** Opacity taper for trail ghosts (index 0 near leader). */
export const TRAIL_OPACITY = [0.72, 0.55, 0.4, 0.28, 0.16, 0.08] as const;
/** Scale taper for trail ghosts. */
export const TRAIL_SCALE = [0.92, 0.78, 0.64, 0.5, 0.38, 0.26] as const;

/** One sample in a leader's motion history. */
export type TrailSample = { x: number; y: number; t: number };

/**
 * Appends a pose to per-leader histories and drops samples older than max age.
 * Mutates `histories` in place; each history array is bounded by age.
 */
export function pushTrailHistory(
  histories: TrailSample[][],
  pose: DotPose,
  now: number,
  maxAgeMs = TRAIL_MAX_AGE_MS,
): void {
  for (let i = 0; i < 3; i++) {
    const h = histories[i];
    h.push({ x: pose[i].x, y: pose[i].y, t: now });
    while (h.length > 0 && now - h[0].t > maxAgeMs) {
      h.shift();
    }
  }
}

/**
 * Samples a leader history at `now - lagMs` (nearest older sample).
 */
export function sampleTrail(
  history: TrailSample[],
  now: number,
  lagMs: number,
): TrailSample | null {
  if (history.length === 0) return null;
  const targetT = now - lagMs;
  let sample = history[0];
  for (let k = 0; k < history.length; k++) {
    if (history[k].t <= targetT) sample = history[k];
    else break;
  }
  return sample;
}
