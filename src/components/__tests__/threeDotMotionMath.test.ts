import { describe, it, expect } from 'vitest';
import {
  TRAIL_LAG_MS,
  TRAIL_MAX_AGE_MS,
  arcPose,
  buildY1Segments,
  easeInOut,
  easeInOutSoft,
  figure8Pose,
  firstPoseOf,
  flattenY1Timeline,
  lastPoseOf,
  lerp,
  lerpPose,
  linePose,
  poseAtElapsed,
  pushTrailHistory,
  sampleTrail,
  snappySpinPose,
  trianglePose,
} from '../threeDotMotionMath';

describe('threeDotMotionMath', () => {
  it('linePose places three dots with middle at center', () => {
    const p = linePose(0, 0, 0);
    expect(p[1].x).toBeGreaterThan(p[0].x);
    expect(p[2].x).toBeGreaterThan(p[1].x);
    expect(p[0].y).toBe(p[1].y);
  });

  it('arcPose raises or lowers the middle relative to outsides', () => {
    const up = arcPose(4, 1);
    expect(up[1].y).toBeLessThan(up[0].y);
    const down = arcPose(4, -1);
    expect(down[1].y).toBeGreaterThan(down[0].y);
  });

  it('trianglePose returns three distinct points', () => {
    const t = trianglePose(0, 6);
    const xs = new Set(t.map((p) => p.x.toFixed(3)));
    expect(xs.size).toBeGreaterThan(1);
  });

  it('lerp and lerpPose interpolate at midpoints', () => {
    expect(lerp(0, 10, 0.5)).toBe(5);
    const a = linePose(0, 0, 0);
    const b = linePose(2, 2, 2);
    const m = lerpPose(a, b, 0.5);
    expect(m[0].y).toBe((a[0].y + b[0].y) / 2);
  });

  it('ease helpers map 0→0 and 1→1', () => {
    expect(easeInOutSoft(0)).toBe(0);
    expect(easeInOutSoft(1)).toBe(1);
    expect(easeInOut(0)).toBe(0);
    expect(easeInOut(1)).toBe(1);
  });

  it('snappySpinPose and figure8Pose stay finite', () => {
    const s = snappySpinPose(0.5);
    const f = figure8Pose(0.25);
    for (const p of [...s, ...f]) {
      expect(Number.isFinite(p.x)).toBe(true);
      expect(Number.isFinite(p.y)).toBe(true);
    }
  });

  it('buildY1Segments returns the locked six acts', () => {
    const segs = buildY1Segments();
    expect(segs.map((s) => s.name)).toEqual([
      'wave',
      'morph steps',
      'triangle spin',
      'figure-eight',
      'orbit ladder',
      'settle',
    ]);
  });

  it('firstPoseOf / lastPoseOf handle keys and snappySpin', () => {
    const segs = buildY1Segments();
    const wave = segs[0];
    const spin = segs[2];
    expect(firstPoseOf(wave)).toEqual(
      wave.kind === 'keys' ? wave.keys[0] : linePose(),
    );
    expect(lastPoseOf(wave)).toEqual(
      wave.kind === 'keys' ? wave.keys[wave.keys.length - 1] : linePose(),
    );
    expect(firstPoseOf(spin)[0].x).toBeTypeOf('number');
    expect(lastPoseOf(spin)[0].x).toBeTypeOf('number');
  });

  it('flattenY1Timeline produces a positive total and looping steps', () => {
    const { steps, totalMs } = flattenY1Timeline();
    expect(steps.length).toBeGreaterThan(10);
    expect(totalMs).toBeGreaterThan(1000);
  });

  it('poseAtElapsed returns a pose for start, mid, and past total', () => {
    const tl = flattenY1Timeline();
    const a = poseAtElapsed(0, tl);
    const b = poseAtElapsed(tl.totalMs / 2, tl);
    const c = poseAtElapsed(tl.totalMs + 50, tl);
    expect(a.pose).toHaveLength(3);
    expect(b.phase.length).toBeGreaterThan(0);
    expect(c.pose[0].x).toBeTypeOf('number');
  });

  it('poseAtElapsed handles empty timeline and negative elapsed', () => {
    const empty = poseAtElapsed(10, { steps: [], totalMs: 0 });
    expect(empty.phase).toBe('wave');
    const tl = flattenY1Timeline();
    const neg = poseAtElapsed(-5, tl);
    expect(neg.pose).toHaveLength(3);
  });

  it('pushTrailHistory bounds age and sampleTrail returns samples', () => {
    const histories: { x: number; y: number; t: number }[][] = [[], [], []];
    const pose = linePose(0, 0, 0);
    pushTrailHistory(histories, pose, 1000, TRAIL_MAX_AGE_MS);
    pushTrailHistory(
      histories,
      pose,
      1000 + TRAIL_MAX_AGE_MS + 50,
      TRAIL_MAX_AGE_MS,
    );
    expect(histories[0].length).toBeGreaterThan(0);
    const sample = sampleTrail(
      histories[0],
      1000 + TRAIL_MAX_AGE_MS + 50,
      TRAIL_LAG_MS[0],
    );
    expect(sample).not.toBeNull();
    expect(sampleTrail([], 0, 10)).toBeNull();
  });

  it('snappySpinPose handles zero and negative turns', () => {
    const z = snappySpinPose(0.3, 0, 5);
    const n = snappySpinPose(0.3, -1.2, 5);
    expect(z).toHaveLength(3);
    expect(n).toHaveLength(3);
  });

  it('poseAtElapsed falls through when totalMs exceeds the step sum', () => {
    // totalMs larger than sum(ms) forces the post-loop fallback paths.
    const from = linePose(0, 0, 0);
    const to = linePose(1, 1, 1);
    const morphOnly = {
      steps: [
        {
          kind: 'morph' as const,
          from,
          to,
          ms: 100,
          phase: 'wave',
          ease: (t: number) => t,
        },
      ],
      totalMs: 1000,
    };
    expect(poseAtElapsed(500, morphOnly).pose[0].y).toBe(to[0].y);

    const spinOnly = {
      steps: [
        {
          kind: 'snappySpin' as const,
          ms: 100,
          turns: 1,
          baseR: 5,
          phase: 'triangle spin',
        },
      ],
      totalMs: 1000,
    };
    expect(poseAtElapsed(500, spinOnly).phase).toBe('triangle spin');
  });
});
