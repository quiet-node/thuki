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

/**
 * Synchronous stand-in for a Framer Motion `motion.div`. In addition to
 * stripping motion props, it reproduces `onAnimationComplete`: the real
 * library fires it once the `animate` target settles, so here we fire it on
 * mount and again whenever the serialized `animate` prop changes. Tests that
 * drive the minimize/restore morph sequencing (which keys off
 * `onAnimationComplete`) can then flush it with a plain
 * `await act(async () => {})` instead of needing fake timers. Fires after
 * paint via `useEffect`, matching Framer's "after the animation" ordering
 * closely enough for assertions. Defined as a PascalCase component so the
 * Hook call satisfies the rules-of-hooks lint.
 */
function MotionDiv({
  children,
  className,
  ref,
  animate,
  onAnimationComplete,
  ...props
}: React.HTMLAttributes<HTMLDivElement> &
  Record<string, unknown> & { ref?: React.Ref<HTMLDivElement> }) {
  const animateKey = JSON.stringify(animate);
  React.useEffect(() => {
    if (typeof onAnimationComplete === 'function') {
      onAnimationComplete(animate);
    }
    // Only `animateKey` is a dep: the serialized animate target is the
    // trigger so the callback fires exactly once per distinct target
    // (mirrors Framer's settle semantics). `animate`/`onAnimationComplete`
    // are deliberately excluded.
    // eslint-disable-next-line @eslint-react/exhaustive-deps
  }, [animateKey]);
  return (
    <div ref={ref} className={className} {...stripMotionProps(props)}>
      {children}
    </div>
  );
}

export const motion = {
  div: MotionDiv,
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
  img: ({
    className,
    src,
    alt,
    ...props
  }: React.ImgHTMLAttributes<HTMLImageElement> & Record<string, unknown>) => (
    <img
      className={className}
      src={src}
      alt={alt}
      {...stripMotionProps(props)}
    />
  ),
};

/**
 * Synchronous AnimatePresence stand-in. Does not retain exiting children
 * (unlike real Framer), but fires `onExitComplete` when children go from
 * present → empty so handoff tests can assert the exit-complete path
 * without fake timers.
 */
export const AnimatePresence = ({
  children,
  onExitComplete,
}: {
  children: React.ReactNode;
  onExitComplete?: () => void;
  initial?: boolean;
  mode?: string;
}) => {
  const hadChildrenRef = React.useRef(false);
  // Conditionals pass `null` / `false` when empty; truthy node = present.
  const hasChildren = Boolean(children);

  React.useEffect(() => {
    if (hadChildrenRef.current && !hasChildren && onExitComplete) {
      onExitComplete();
    }
    hadChildrenRef.current = hasChildren;
  }, [hasChildren, onExitComplete]);

  return <>{children}</>;
};

/**
 * Tests default to motion on. Mutate `.current` in a test to cover
 * reduced-motion duration branches, then restore to false.
 */
export const mockReducedMotion = { current: false };

/**
 * Framer `useReducedMotion` stand-in; reads {@link mockReducedMotion}.
 * Name must match the real export for alias resolution.
 */
// eslint-disable-next-line @eslint-react/no-unnecessary-use-prefix -- mirrors framer-motion API
export function useReducedMotion(): boolean {
  return mockReducedMotion.current;
}
