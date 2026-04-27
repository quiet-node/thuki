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
 * History-state inputs the gate inspects so the strip can warn when the
 * active model lacks a capability earlier turns relied on. Stored history
 * is never mutated; the backend per-request filter strips incompatible
 * content from the snapshot so the conversation keeps working. The strip
 * exists to tell the user what is happening, not to block them.
 */
export interface HistoryCapabilityState {
  /** True if any prior message in the conversation carried images. */
  historyHasImages: boolean;
  /** True if any prior assistant message produced thinking content. */
  historyHasThinking: boolean;
  /**
   * Maximum images attached to any single prior message. Used to surface
   * a heads-up when the active vision model has a per-message image cap
   * (e.g. llama3.2-vision = 1) and earlier turns carried more than that.
   * Zero when no prior message carried images.
   */
  historyMaxImagesPerMessage: number;
}

/** Empty history-state literal used as the default when no history is
 *  passed in. Internal: the public default lives on the function
 *  parameter signature, this constant exists so we never drift. */
const EMPTY_HISTORY_STATE: HistoryCapabilityState = {
  historyHasImages: false,
  historyHasThinking: false,
  historyMaxImagesPerMessage: 0,
};

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
  history: HistoryCapabilityState = EMPTY_HISTORY_STATE,
): string | null {
  if (!modelName) {
    // Environment-state messaging lives in `getEnvironmentMessage`. This
    // helper has no insight into Ollama reachability or installed count,
    // so the safe behavior is to defer rather than emit a stale copy.
    return null;
  }
  if (!capabilities) {
    // Capabilities unknown (not yet fetched, or fetch failed). The gate
    // is permissive: never block the user on missing metadata, regardless
    // of compose or history state. Backend surfaces a real error if the
    // model truly cannot accept the payload.
    return null;
  }
  const name = modelName;
  const needsVision = state.imageCount > 0 || state.hasScreenCommand;
  const needsThinking = state.hasThinkCommand;

  // Compose-state checks fire before history checks because compose
  // conflicts are the actionable kind: the user is mid-edit and can fix
  // the conflict by removing content or switching models before send.
  // History conflicts are passive (the conversation already exists, the
  // user is continuing it) and only inform.
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

  // History-state checks (Phase B). The backend already strips images
  // and thinking artifacts from the per-request snapshot when the active
  // model lacks the capability; the strip is purely informational so the
  // user knows why earlier content is missing from the model's view of
  // the thread, and how to recover (switch back to a capable model).
  if (history.historyHasImages && !capabilities.vision) {
    return `Images from earlier turns are hidden from ${name} because it reads text only. Switch to a vision model to keep them.`;
  }
  // Vision-capable but with a per-message image cap that earlier turns
  // exceed. The backend filter trims to `maxImages` keeping the first
  // image per message; the user should know the rest is dropped from the
  // model's view.
  if (
    capabilities.vision &&
    capabilities.maxImages != null &&
    capabilities.maxImages >= 1 &&
    history.historyMaxImagesPerMessage > capabilities.maxImages
  ) {
    const noun =
      capabilities.maxImages === 1
        ? 'one image'
        : `${capabilities.maxImages} images`;
    return `${name} accepts ${noun} per message. Extra images from earlier turns are hidden. Switch to a multi-image vision model to keep them.`;
  }
  if (history.historyHasThinking && !capabilities.thinking) {
    return `Reasoning from earlier turns is hidden from ${name} because it does not emit thinking tokens. Switch to a thinking model to keep it.`;
  }

  return null;
}

/**
 * True when the conflict message returned by {@link getCapabilityConflict}
 * is rooted in compose state (image attached to text-only model, /think on
 * a non-thinking model, multi-image overflow). False when the conflict is
 * history-only or there is no conflict.
 *
 * The submit-time gate uses this to decide whether to shake the ask bar.
 * Compose conflicts shake (the user is about to send something the model
 * cannot take); history conflicts do not (the user is just continuing the
 * conversation; the backend filter handles the payload, and shaking would
 * trap the user every turn until they switched models).
 */
export function isComposeCapabilityConflict(
  capabilities: ModelCapabilities | undefined | null,
  state: ComposeCapabilityState,
): boolean {
  if (!capabilities) return false;
  const needsVision = state.imageCount > 0 || state.hasScreenCommand;
  const needsThinking = state.hasThinkCommand;
  if (needsVision && !capabilities.vision) return true;
  if (needsVision && capabilities.vision) {
    const max = capabilities.maxImages;
    if (max != null && max >= 1) {
      const effective = state.imageCount + (state.hasScreenCommand ? 1 : 0);
      if (effective > max) return true;
    }
  }
  if (needsThinking && !capabilities.thinking) return true;
  return false;
}
