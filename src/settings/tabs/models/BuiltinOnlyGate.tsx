/**
 * Built-in-only gate for the Library and Discover panes.
 *
 * Library and Discover manage models for Thuki's bundled engine. While another
 * provider (Ollama, OpenAI) is active they do not apply, but hiding them would
 * bury the built-in engine and leave on-device models undiscoverable. Instead
 * the real pane stays mounted behind a dimmed, inert layer (so the user sees
 * what is waiting) beneath a centered card that activates the built-in engine
 * in one click. When ungated the children render untouched.
 */

import type { ReactNode } from 'react';

import styles from './BuiltinOnlyGate.module.css';

interface BuiltinOnlyGateProps {
  /** True when a non-built-in provider is active, so the surface is gated. */
  gated: boolean;
  /** The active provider's label, named in the explanation copy. */
  activeLabel: string;
  /** Activate the built-in engine. */
  onSwitch: () => void;
  /** The real pane: rendered directly when ungated, behind glass when gated. */
  children: ReactNode;
}

export function BuiltinOnlyGate({
  gated,
  activeLabel,
  onSwitch,
  children,
}: BuiltinOnlyGateProps) {
  if (!gated) return <>{children}</>;

  return (
    <div className={styles.wrap}>
      {/* `inert` removes the dimmed pane from tab order, hit-testing, and the
          accessibility tree so gated controls cannot be reached by keyboard or
          pointer; aria-hidden is kept as a belt-and-suspenders for older AT. */}
      <div className={styles.faint} aria-hidden="true" inert>
        {children}
      </div>
      <div className={styles.overlay}>
        <div className={styles.card} role="status">
          <p className={styles.title}>Your built-in models live here</p>
          <p className={styles.body}>
            Switch to the built-in engine to use and manage them. You're using{' '}
            {activeLabel} now.
          </p>
          <button type="button" className={styles.switch} onClick={onSwitch}>
            Switch to built-in
          </button>
        </div>
      </div>
    </div>
  );
}
