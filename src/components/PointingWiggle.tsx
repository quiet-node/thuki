/**
 * Reusable “point at this control” attention underline (design D).
 *
 * Hand-drawn primary squiggle: soft draw → settle → three mild breaths → fade.
 * Use whenever a click navigates the user to a specific UI target (Settings
 * deep-links, in-app “look here” affordances). Do not invent a second pointer
 * style.
 */

import type { ReactNode } from 'react';
import styles from './PointingWiggle.module.css';

/**
 * Full highlight timeline length in ms. Matches `pointingWiggleLife` in CSS.
 * Callers that clear `active` after a timeout should use this constant.
 */
export const POINTING_WIGGLE_MS = 7200;

/**
 * Organic squiggle path for viewBox `0 0 100 10`. `pathLength` on the SVG
 * path makes stroke-dash animation independent of geometric length.
 */
const WIGGLE_PATH_D =
  'M1.5 6.2 C 8 3.8, 12 7.5, 18 5.5 S 28 2.8, 34 5.8 S 44 8.2, 50 5.2 S 60 2.5, 68 5.9 S 80 8.5, 88 4.8 S 95 6.5, 98.5 5.2';

export interface PointingWiggleProps {
  /**
   * When true, mounts the SVG and runs the full draw/breathe/fade once.
   * When false, renders nothing.
   */
  active: boolean;
  /** Optional test id; default `pointing-wiggle`. */
  testId?: string;
}

/**
 * Animated squiggle drawn under a label. Parent must be
 * `position: relative` (or wrap with {@link PointingLabel}).
 *
 * @param active Whether the one-shot animation is running.
 * @param testId Override for testing.
 */
export function PointingWiggle({
  active,
  testId = 'pointing-wiggle',
}: PointingWiggleProps) {
  if (!active) return null;
  return (
    <svg
      className={styles.wiggle}
      viewBox="0 0 100 10"
      preserveAspectRatio="none"
      aria-hidden="true"
      data-testid={testId}
    >
      <path
        className={styles.path}
        pathLength={1}
        d={WIGGLE_PATH_D}
      />
    </svg>
  );
}

export interface PointingLabelProps {
  /** Label text or nodes the squiggle sits under. */
  children: ReactNode;
  /** When true, show the wiggle under the children. */
  active?: boolean;
  /** Optional test id for the SVG. */
  testId?: string;
  /** Optional className on the wrapper span. */
  className?: string;
}

/**
 * Inline label wrapper: positions children and optional {@link PointingWiggle}.
 *
 * @param children Visible label content.
 * @param active When true, plays the pointing animation under the label.
 * @param testId Forwarded to the wiggle SVG.
 * @param className Extra class on the wrap span.
 */
export function PointingLabel({
  children,
  active = false,
  testId,
  className,
}: PointingLabelProps) {
  return (
    <span
      className={
        className ? `${styles.labelWrap} ${className}` : styles.labelWrap
      }
    >
      {children}
      <PointingWiggle active={active} testId={testId} />
    </span>
  );
}
