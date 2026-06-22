/**
 * The capability badge row shown next to a model name in Discover: a constant
 * `Text` pill plus optional `Vision` and `Thinking` pills. Shared by both the
 * Staff-picks and Browse-all panes so the two surfaces label capability
 * identically. Capability is a per-model property (every quant of a repo shares
 * it), so the pills live on the model/repo row, never the per-quant list.
 */

import styles from './CapabilityPills.module.css';

interface CapabilityPillsProps {
  /** The model accepts image input (an mmproj vision companion is present). */
  vision: boolean;
  /** The model emits reasoning tokens. */
  thinking: boolean;
}

export function CapabilityPills({ vision, thinking }: CapabilityPillsProps) {
  return (
    <span className={styles.pills}>
      <span className={`${styles.pill} ${styles.pillText}`}>Text</span>
      {vision ? (
        <span className={`${styles.pill} ${styles.pillVision}`}>Vision</span>
      ) : null}
      {thinking ? (
        <span className={`${styles.pill} ${styles.pillThinking}`}>
          Reasoning
        </span>
      ) : null}
    </span>
  );
}
