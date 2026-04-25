/**
 * Shared building blocks for onboarding steps.
 *
 * Extracted from PermissionsStep so ModelCheckStep (and any future
 * onboarding screen) can reuse the same active / done / waiting visual
 * language. The token values here are the source of truth for the
 * onboarding visual system; do not duplicate them inline in step
 * components.
 */

import type React from 'react';

export interface StepCardProps {
  /** Orange-glow treatment indicating the user must act on this step now. */
  active: boolean;
  /** Green-tinted "done" treatment with a thin success border. */
  done: boolean;
  children: React.ReactNode;
}

/**
 * Container that applies the onboarding step visual treatment.
 *
 * Three mutually exclusive states:
 *   - done: green border + green-tint background, no glow.
 *   - active && !done: warm orange border + orange-tint background +
 *     soft outer glow + 1px inner top highlight.
 *   - !active && !done: subtle white border + faint white-tint
 *     background, no glow. Used for "waiting" steps that the user
 *     cannot act on yet.
 */
export function StepCard({ active, done, children }: StepCardProps) {
  const borderColor = done
    ? 'rgba(34,197,94,0.2)'
    : active
      ? 'rgba(255,141,92,0.4)'
      : 'rgba(255,255,255,0.06)';

  const background = done
    ? 'rgba(34,197,94,0.05)'
    : active
      ? 'rgba(255,141,92,0.07)'
      : 'rgba(255,255,255,0.03)';

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '14px 16px',
        borderRadius: 16,
        border: `1px solid ${borderColor}`,
        background,
        boxShadow:
          active && !done
            ? '0 0 20px rgba(255,141,92,0.08), inset 0 1px 0 rgba(255,141,92,0.1)'
            : 'none',
      }}
    >
      {children}
    </div>
  );
}

export interface BadgeProps {
  color: 'green';
  children: React.ReactNode;
}

/**
 * Inline status pill rendered to the right of a done step's title.
 *
 * Single-color today (`green` for the success / connected state). Add
 * new colors as discrete variants rather than accepting arbitrary CSS,
 * which keeps the badge palette under one rule.
 */
export function Badge({ color, children }: BadgeProps) {
  const styles: Record<string, React.CSSProperties> = {
    green: {
      color: '#22c55e',
      background: 'rgba(34,197,94,0.1)',
      border: '1px solid rgba(34,197,94,0.2)',
    },
  };

  return (
    <span
      style={{
        fontSize: 11,
        fontWeight: 600,
        padding: '3px 9px',
        borderRadius: 20,
        ...styles[color],
      }}
    >
      {children}
    </span>
  );
}
