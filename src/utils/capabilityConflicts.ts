import type { ModelCapabilities } from '../types/model';

/**
 * Compose-state inputs the gate inspects. `hasImages` covers manually
 * attached + pasted + dragged images. `hasScreenCommand` covers the
 * `/screen` slash command (which produces an image after capture and so
 * has the same vision-required constraint). `hasThinkCommand` covers the
 * `/think` slash command, which requires a model that emits reasoning
 * tokens for the ThinkingBlock UI to render anything meaningful.
 */
export interface ComposeCapabilityState {
  /** True if the user has at least one image attached or queued. */
  hasImages: boolean;
  /** True if the message contains the `/screen` slash command. */
  hasScreenCommand: boolean;
  /** True if the message contains the `/think` slash command. */
  hasThinkCommand: boolean;
  /**
   * Number of images attached to the compose state. Used by the
   * max-images gate to refuse multi-image submits to single-image
   * vision models (e.g. llama3.2-vision). The `/screen` command adds
   * exactly one image at capture time so callers should fold it into
   * this count when both are true.
   */
  imageCount: number;
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
  const needsThinking = state.hasThinkCommand;
  if (!needsVision && !needsThinking) return null;
  if (!capabilities) return null;
  const name = modelName && modelName.length > 0 ? modelName : 'this model';

  // Vision is checked first when both apply because it is the more
  // fundamental constraint: a text-only model cannot consume the image
  // payload at all, while /think on a non-thinking model just degrades
  // to a normal answer. Picking the vision message keeps the user
  // pointed at the action that unblocks the most.
  if (needsVision) {
    if (!capabilities.vision) {
      return `${name} reads text only. Try a vision model for images.`;
    }
    // Vision model, but it may cap the number of images per request
    // (today: mllama-family models such as llama3.2-vision are 1-image
    // only). Fold the /screen command into the effective count so a
    // queued capture counts toward the cap exactly like an attached
    // image.
    const max = capabilities.maxImages;
    if (max != null && max >= 1) {
      const effective = state.imageCount + (state.hasScreenCommand ? 1 : 0);
      if (effective > max) {
        const noun = max === 1 ? 'one image' : `${max} images`;
        return `${name} accepts ${noun} at a time. Remove the extras to send.`;
      }
    }
  }

  // /think requires a model that emits reasoning tokens; otherwise the
  // command is silently ignored and the user gets a normal answer with
  // no ThinkingBlock, which feels broken. Surface the mismatch instead.
  if (needsThinking && !capabilities.thinking) {
    return `${name} doesn't show reasoning. Try a thinking model for /think.`;
  }

  return null;
}
