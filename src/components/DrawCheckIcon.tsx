import styles from '../styles/settings.module.css';

/**
 * Two-stage success animation: draws a green ring around a 16x16 frame
 * (~550 ms) then stamps a checkmark inside (~300 ms after a small delay).
 *
 * Used by settings actions that may run long enough to need feedback:
 * the AI tab "Unload now" pill while VRAM eviction is in flight, and the
 * About tab "Check for updates" button while the manifest poll resolves.
 * The CSS keyframes live in `settings.module.css`
 * (`keepWarmCircleAnim` + `keepWarmCheckAnim`).
 */
export function DrawCheckIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      width="11"
      height="11"
      fill="none"
      aria-hidden="true"
    >
      <circle
        cx="8"
        cy="8"
        r="7"
        stroke="#5ec98a"
        strokeWidth="1.6"
        className={styles.keepWarmCircleAnim}
        transform="rotate(-90 8 8)"
      />
      <path
        d="M4.5 8.5L7 11L12 5.5"
        stroke="#5ec98a"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
        className={styles.keepWarmCheckAnim}
      />
    </svg>
  );
}
