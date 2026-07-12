import type { ReactNode } from 'react';
import { RequestStatusStrip } from './RequestStatusStrip';

interface LoadingStageProps {
  /**
   * Optional label shown next to the three-dot motion. When `null` or
   * `undefined`, only the dots render (no label text). The label uses the
   * shared shimmer sweep plus tracking-settle on change.
   */
  label?: string | null;
  /**
   * Compact layout used in secondary UI surfaces like the search progress
   * header, where the stage label should stay supportive rather than dominate
   * the response body.
   */
  compact?: boolean;
  /**
   * Optional inline prefix rendered immediately before the label text. Used by
   * disclosure-style headers where the chevron should read as part of the
   * title rather than as a separate trailing control.
   */
  labelPrefix?: ReactNode;
}

/**
 * Unified loading row for engine cold-start, web search stages, and `/think`.
 *
 * Thin wrapper around {@link RequestStatusStrip} so existing call sites keep a
 * stable import path. Presentation lives entirely in RequestStatusStrip
 * (Y1 three-dot motion + shimmer label).
 */
export function LoadingStage({
  label,
  compact = false,
  labelPrefix,
}: LoadingStageProps) {
  return (
    <RequestStatusStrip
      label={label}
      compact={compact}
      labelPrefix={labelPrefix}
    />
  );
}
