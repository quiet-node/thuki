import React from 'react';

/**
 * Lightweight framer-motion stub for the test environment.
 *
 * Replacing the real library avoids the rAF-loop / animation-batcher
 * infinite-recursion that occurs when Vitest's synchronous
 * requestAnimationFrame shim interacts with motion-dom internals.
 * Each motion.* element is swapped for its plain HTML equivalent so
 * tests can assert on real DOM structure and class names.
 */

export const motion = {
  div: ({ children, className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
    <div className={className} {...props}>
      {children}
    </div>
  ),
  span: ({ children, className, ...props }: React.HTMLAttributes<HTMLSpanElement>) => (
    <span className={className} {...props}>
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
  }: React.ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button className={className} onClick={onClick} disabled={disabled} aria-label={ariaLabel} {...props}>
      {children}
    </button>
  ),
};

export const AnimatePresence = ({ children }: { children: React.ReactNode }) => <>{children}</>;
