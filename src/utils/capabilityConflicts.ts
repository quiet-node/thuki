import type { ModelCapabilities } from '../types/model';

/**
 * Compose-state inputs the gate inspects. `hasImages` covers manually
 * attached + pasted + dragged images. `hasScreenCommand` covers the
 * `/screen` slash command (which produces an image after capture and so
 * has the same vision-required constraint).
 */
export interface ComposeCapabilityState {
  /** True if the user has at least one image attached or queued. */
  hasImages: boolean;
  /** True if the message contains the `/screen` slash command. */
  hasScreenCommand: boolean;
}

/**
 * Returns a single human-readable reason why the active model cannot
 * send the current compose state, or `null` if the message is sendable.
 *
 * The strip and the submit-time toast both render the returned string
 * verbatim so the wording lives in exactly one place.
 *
 * Defaults to permissive: an unknown active model (capabilities not yet
 * fetched, or fetch failed) returns `null` so the user is never blocked
 * by missing metadata. The backend is the final authority and will
 * surface a real error if the model truly cannot accept the payload.
 */
export function getCapabilityConflict(
  modelName: string | undefined | null,
  capabilities: ModelCapabilities | undefined | null,
  state: ComposeCapabilityState,
): string | null {
  const needsVision = state.hasImages || state.hasScreenCommand;
  if (!needsVision) return null;
  if (!capabilities) return null;
  if (capabilities.vision) return null;
  const name = modelName && modelName.length > 0 ? modelName : 'this model';
  return `${name} reads text only. Try a vision model for images.`;
}
