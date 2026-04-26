import type { ModelCapabilities } from '../types/model';

/**
 * Compose-state inputs the gate inspects. `imageCount` covers manually
 * attached + pasted + dragged images. `hasScreenCommand` covers the
 * `/screen` slash command (which produces an image after capture and so
 * has the same vision-required constraint as a non-zero imageCount).
 * `hasThinkCommand` covers the `/think` slash command, which requires a
 * model that emits reasoning tokens for the ThinkingBlock UI to render
 * anything meaningful.
 */
export interface ComposeCapabilityState {
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
 * Copy used when Ollama is reachable but the user has no models installed.
 * Exported so tests can match it without duplicating the prose, and so
 * App.tsx can route through one symbol per state.
 */
export const NO_MODELS_INSTALLED_MESSAGE =
  "Thuki couldn't find any local LLM models. Pull one from Ollama with `ollama pull <model>`, then come back.";

/**
 * Copy used when the local Ollama daemon cannot be reached (connection
 * refused, timeout, port closed). The recovery action is "start Ollama",
 * not "pull a model": telling the user to pull when the daemon is down
 * sends them down the wrong rabbit hole.
 */
export const OLLAMA_UNREACHABLE_MESSAGE =
  "Ollama isn't running. Start Ollama and try again.";

/**
 * Picks the right environment-state message to render in
 * `CapabilityMismatchStrip`, or returns `null` when the environment is
 * healthy enough that a per-message capability gate should run instead.
 *
 * Three states are distinguished so the strip never tells the user to
 * "pull a model" when the actual problem is that Ollama is down:
 *
 * - S1: Ollama unreachable. Returns the unreachable copy regardless of
 *   `installedCount` or `activeModel` because we cannot trust either.
 * - S2: Ollama reachable, zero models installed. Returns the no-models copy.
 * - S3: Ollama reachable, models installed, none active. Returns the
 *   pick-a-model copy. This state is rare post-Phase-A because the backend
 *   auto-picks on first launch, but the strip handles it defensively.
 *
 * Returns `null` once a model is actually active so callers fall through
 * to the per-message capability check.
 */
export function getEnvironmentMessage(
  ollamaReachable: boolean,
  installedCount: number,
  activeModel: string | null | undefined,
): string | null {
  if (!ollamaReachable) return OLLAMA_UNREACHABLE_MESSAGE;
  if (installedCount === 0) return NO_MODELS_INSTALLED_MESSAGE;
  if (!activeModel) {
    return 'Pick a model from the chip above to start chatting.';
  }
  return null;
}

/**
 * Returns a single human-readable reason why the active model cannot
 * send the current compose state, or `null` if the message is sendable.
 *
 * The strip and the submit-time toast both render the returned string
 * verbatim so the wording lives in exactly one place.
 *
 * This helper is only meaningful once a model is actually active.
 * Empty / null / undefined `modelName` short-circuits to `null` so the
 * caller can fall back to {@link getEnvironmentMessage} for the right
 * "Ollama is down / pull a model / pick a model" copy. Capabilities-aware
 * checks below only run once a model is actually selected.
 *
 * For a selected model with unknown capabilities (not yet fetched, or
 * fetch failed) the gate is permissive and returns `null` so the user is
 * never blocked by missing metadata. The backend surfaces a real error
 * if the model truly cannot accept the payload.
 */
export function getCapabilityConflict(
  modelName: string | undefined | null,
  capabilities: ModelCapabilities | undefined | null,
  state: ComposeCapabilityState,
): string | null {
  if (!modelName) {
    // Environment-state messaging lives in `getEnvironmentMessage`. This
    // helper has no insight into Ollama reachability or installed count,
    // so the safe behavior is to defer rather than emit a stale copy.
    return null;
  }
  const needsVision = state.imageCount > 0 || state.hasScreenCommand;
  const needsThinking = state.hasThinkCommand;
  if (!needsVision && !needsThinking) return null;
  if (!capabilities) return null;
  const name = modelName;

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
