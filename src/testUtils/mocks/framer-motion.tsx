import React from 'react';

/**
 * Lightweight framer-motion stub for the test environment.
 *
 * Replacing the real library avoids the rAF-loop / animation-batcher
 * infinite-recursion that occurs when Vitest's synchronous
 * requestAnimationFrame shim interacts with motion-dom internals.
 * Each motion.* element is swapped for its plain HTML equivalent so
 * tests can assert on real DOM structure and class names.
 *
 * Framer-motion-specific props (animate, initial, exit, transition,
 * variants, whileHover, whileTap, layout, etc.) are stripped so they
 * don't leak onto DOM elements and trigger React warnings.
 */

const MOTION_PROPS = new Set([
  'animate',
  'initial',
  'exit',
  'transition',
  'variants',
  'whileHover',
  'whileTap',
  'whileFocus',
  'whileDrag',
  'whileInView',
  'layout',
  'layoutId',
  'onAnimationStart',
  'onAnimationComplete',
  'dragConstraints',
  'dragElastic',
]);

function stripMotionProps<T extends Record<string, unknown>>(
  props: T,
): Record<string, unknown> {
  const clean: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(props)) {
    if (!MOTION_PROPS.has(key)) {
      clean[key] = value;
    }
  }
  return clean;
}

export const motion = {
  div: ({
    children,
    className,
    ...props
  }: React.HTMLAttributes<HTMLDivElement> & Record<string, unknown>) => (
    <div className={className} {...stripMotionProps(props)}>
      {children}
    </div>
  ),
  span: ({
    children,
    className,
    ...props
  }: React.HTMLAttributes<HTMLSpanElement> & Record<string, unknown>) => (
    <span className={className} {...stripMotionProps(props)}>
      {children}
    </span>
  ),
  button: ({
    children,
    className,
    onClick,
    disabled,
    'aria-label': ariaLabel,
    ...props
  }: React.ButtonHTMLAttributes<HTMLButtonElement> &
    Record<string, unknown>) => (
    <button
      className={className}
      onClick={onClick}
      disabled={disabled}
      aria-label={ariaLabel}
      {...stripMotionProps(props)}
    >
      {children}
    </button>
  ),
};

export const AnimatePresence = ({
  children,
}: {
  children: React.ReactNode;
}) => <>{children}</>;

/**
 * Stub for `useMotionValue` — returns a minimal object with get/set methods.
 * No animation in tests; the DOM renders with static values.
 */
// eslint-disable-next-line @eslint-react/no-unnecessary-use-prefix
export function useMotionValue(initial: number) {
  let value = initial;
  return {
    get: () => value,
    set: (v: number) => {
      value = v;
    },
  };
}

/**
 * Stub for `useSpring` — passthrough, no spring physics in tests.
 */
// eslint-disable-next-line @eslint-react/no-unnecessary-use-prefix
export function useSpring(motionValue: ReturnType<typeof useMotionValue>) {
  return motionValue;
}
