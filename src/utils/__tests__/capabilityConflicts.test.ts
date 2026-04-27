import { describe, it, expect } from 'vitest';
import {
  getCapabilityConflict,
  getEnvironmentMessage,
  isComposeCapabilityConflict,
  NO_MODELS_INSTALLED_MESSAGE,
  OLLAMA_UNREACHABLE_MESSAGE,
} from '../capabilityConflicts';
import type { ModelCapabilities } from '../../types/model';
import type {
  ComposeCapabilityState,
  HistoryCapabilityState,
} from '../capabilityConflicts';

const VISION: ModelCapabilities = {
  vision: true,
  thinking: false,
  maxImages: null,
};
const VISION_SINGLE_IMAGE: ModelCapabilities = {
  vision: true,
  thinking: false,
  maxImages: 1,
};
const VISION_TWO_IMAGES: ModelCapabilities = {
  vision: true,
  thinking: false,
  maxImages: 2,
};
const TEXT_ONLY: ModelCapabilities = {
  vision: false,
  thinking: false,
  maxImages: null,
};
const THINKING_ONLY: ModelCapabilities = {
  vision: false,
  thinking: true,
  maxImages: null,
};
const VISION_AND_THINKING: ModelCapabilities = {
  vision: true,
  thinking: true,
  maxImages: null,
};

const EMPTY: ComposeCapabilityState = {
  hasScreenCommand: false,
  hasThinkCommand: false,
  imageCount: 0,
};

describe('getCapabilityConflict', () => {
  it('returns null when nothing is queued', () => {
    expect(getCapabilityConflict('llama3', TEXT_ONLY, EMPTY)).toBeNull();
  });

  it('returns null when capabilities are unknown (defaults permissive)', () => {
    const result = getCapabilityConflict('llama3', undefined, {
      ...EMPTY,
      imageCount: 1,
    });
    expect(result).toBeNull();
  });

  it('returns null when capabilities is null', () => {
    const result = getCapabilityConflict('llama3', null, {
      ...EMPTY,
      imageCount: 1,
    });
    expect(result).toBeNull();
  });

  it('returns null when active model can see images and has no max-images cap', () => {
    const result = getCapabilityConflict('llava', VISION, {
      ...EMPTY,
      hasScreenCommand: true,
      imageCount: 3,
    });
    expect(result).toBeNull();
  });

  it('returns conflict when images attached and model is text-only', () => {
    const result = getCapabilityConflict('llama3', TEXT_ONLY, {
      ...EMPTY,
      imageCount: 1,
    });
    expect(result).toBe(
      'llama3 reads text only. Try a vision model for images.',
    );
  });

  it('returns conflict when /screen is queued and model is text-only', () => {
    const result = getCapabilityConflict('llama3', TEXT_ONLY, {
      ...EMPTY,
      hasScreenCommand: true,
    });
    expect(result).toContain('reads text only');
  });

  it('returns null when modelName is empty so the env-state helper can take over', () => {
    // Environment-state messaging now lives in `getEnvironmentMessage`.
    // The capability helper defers rather than emit a stale "pick a model"
    // copy that would not know whether Ollama is reachable.
    const result = getCapabilityConflict('', TEXT_ONLY, {
      ...EMPTY,
      imageCount: 1,
    });
    expect(result).toBeNull();
  });

  it('returns null when modelName is null', () => {
    const result = getCapabilityConflict(null, TEXT_ONLY, {
      ...EMPTY,
      imageCount: 1,
    });
    expect(result).toBeNull();
  });

  it('returns null when modelName is undefined', () => {
    const result = getCapabilityConflict(undefined, TEXT_ONLY, {
      ...EMPTY,
      imageCount: 1,
    });
    expect(result).toBeNull();
  });

  // ── max-images gate ───────────────────────────────────────────────────────

  it('returns null when single-image vision model has exactly one image', () => {
    const result = getCapabilityConflict(
      'llama3.2-vision',
      VISION_SINGLE_IMAGE,
      { ...EMPTY, imageCount: 1 },
    );
    expect(result).toBeNull();
  });

  it('refuses two attached images on a single-image vision model', () => {
    const result = getCapabilityConflict(
      'llama3.2-vision',
      VISION_SINGLE_IMAGE,
      { ...EMPTY, imageCount: 2 },
    );
    expect(result).toBe(
      'llama3.2-vision accepts one image at a time. Remove the extras to send.',
    );
  });

  it('counts /screen as one image toward the cap', () => {
    // Single-image vision model + one attached image + /screen queued =
    // effective count of 2, exceeds the cap of 1.
    const result = getCapabilityConflict(
      'llama3.2-vision',
      VISION_SINGLE_IMAGE,
      { ...EMPTY, hasScreenCommand: true, imageCount: 1 },
    );
    expect(result).toBe(
      'llama3.2-vision accepts one image at a time. Remove the extras to send.',
    );
  });

  it('allows /screen alone on a single-image vision model', () => {
    const result = getCapabilityConflict(
      'llama3.2-vision',
      VISION_SINGLE_IMAGE,
      { ...EMPTY, hasScreenCommand: true },
    );
    expect(result).toBeNull();
  });

  it('pluralizes the noun for a multi-image cap', () => {
    const result = getCapabilityConflict('multi-cap', VISION_TWO_IMAGES, {
      ...EMPTY,
      imageCount: 5,
    });
    expect(result).toBe(
      'multi-cap accepts 2 images at a time. Remove the extras to send.',
    );
  });

  it('allows submits at the cap exactly', () => {
    const result = getCapabilityConflict('multi-cap', VISION_TWO_IMAGES, {
      ...EMPTY,
      imageCount: 2,
    });
    expect(result).toBeNull();
  });

  it('ignores a max-images cap below 1 (defensive)', () => {
    const odd: ModelCapabilities = {
      vision: true,
      thinking: false,
      maxImages: 0,
    };
    const result = getCapabilityConflict('odd', odd, {
      ...EMPTY,
      imageCount: 3,
    });
    expect(result).toBeNull();
  });

  // ── /think gate ───────────────────────────────────────────────────────────

  it('refuses /think on a non-thinking model', () => {
    const result = getCapabilityConflict('llama3', TEXT_ONLY, {
      ...EMPTY,
      hasThinkCommand: true,
    });
    expect(result).toBe(
      "llama3 doesn't show reasoning. Try a thinking model for /think.",
    );
  });

  it('allows /think on a thinking-capable model', () => {
    const result = getCapabilityConflict('reasoner', THINKING_ONLY, {
      ...EMPTY,
      hasThinkCommand: true,
    });
    expect(result).toBeNull();
  });

  it('returns null when name is empty even with /think queued', () => {
    // Empty name still short-circuits to null so the env-state helper
    // owns the messaging. The /think mismatch copy is meaningless without
    // a real model anyway.
    const result = getCapabilityConflict('', TEXT_ONLY, {
      ...EMPTY,
      hasThinkCommand: true,
    });
    expect(result).toBeNull();
  });

  it('prefers the vision message when /think and images both mismatch', () => {
    // Vision is the more fundamental constraint and recovery from it
    // (switching to a vision model) is also more likely to satisfy the
    // /think requirement than the other way around.
    const result = getCapabilityConflict('llama3', TEXT_ONLY, {
      ...EMPTY,
      imageCount: 1,
      hasThinkCommand: true,
    });
    expect(result).toBe(
      'llama3 reads text only. Try a vision model for images.',
    );
  });

  it('still fires the /think gate when vision is satisfied but thinking is not', () => {
    const result = getCapabilityConflict('llava', VISION, {
      ...EMPTY,
      imageCount: 1,
      hasThinkCommand: true,
    });
    expect(result).toBe(
      "llava doesn't show reasoning. Try a thinking model for /think.",
    );
  });

  it('returns null when both vision and thinking are satisfied', () => {
    const result = getCapabilityConflict('omnimodel', VISION_AND_THINKING, {
      ...EMPTY,
      imageCount: 1,
      hasThinkCommand: true,
    });
    expect(result).toBeNull();
  });

  // ── history-state gates (Phase B) ─────────────────────────────────────────

  const HISTORY_HAS_IMAGES: HistoryCapabilityState = {
    historyHasImages: true,
    historyHasThinking: false,
    historyMaxImagesPerMessage: 1,
  };
  const HISTORY_HAS_THINKING: HistoryCapabilityState = {
    historyHasImages: false,
    historyHasThinking: true,
    historyMaxImagesPerMessage: 0,
  };
  const HISTORY_HAS_BOTH: HistoryCapabilityState = {
    historyHasImages: true,
    historyHasThinking: true,
    historyMaxImagesPerMessage: 1,
  };
  const HISTORY_HAS_TWO_IMAGES: HistoryCapabilityState = {
    historyHasImages: true,
    historyHasThinking: false,
    historyMaxImagesPerMessage: 2,
  };

  it('warns when history has images but active model is text-only', () => {
    const result = getCapabilityConflict(
      'llama3',
      TEXT_ONLY,
      EMPTY,
      HISTORY_HAS_IMAGES,
    );
    expect(result).toBe(
      'Images from earlier turns are hidden from llama3 because it reads text only. Switch to a vision model to keep them.',
    );
  });

  it('warns when history has thinking but active model does not emit it', () => {
    const result = getCapabilityConflict(
      'llama3',
      TEXT_ONLY,
      EMPTY,
      HISTORY_HAS_THINKING,
    );
    expect(result).toBe(
      'Reasoning from earlier turns is hidden from llama3 because it does not emit thinking tokens. Switch to a thinking model to keep it.',
    );
  });

  it('returns null when history has images and model is vision-capable', () => {
    const result = getCapabilityConflict(
      'llava',
      VISION,
      EMPTY,
      HISTORY_HAS_IMAGES,
    );
    expect(result).toBeNull();
  });

  it('returns null when history has thinking and model is thinking-capable', () => {
    const result = getCapabilityConflict(
      'reasoner',
      THINKING_ONLY,
      EMPTY,
      HISTORY_HAS_THINKING,
    );
    expect(result).toBeNull();
  });

  it('prefers history-images warning over history-thinking when both apply', () => {
    // Vision is the more fundamental loss; surface it first.
    const result = getCapabilityConflict(
      'llama3',
      TEXT_ONLY,
      EMPTY,
      HISTORY_HAS_BOTH,
    );
    expect(result).toContain('Images from earlier turns are hidden');
  });

  it('compose conflict wins over history conflict when both apply', () => {
    // Compose is actionable now; history is passive. The user can fix
    // compose before submit, so surface it first.
    const result = getCapabilityConflict(
      'llama3',
      TEXT_ONLY,
      { ...EMPTY, imageCount: 1 },
      HISTORY_HAS_IMAGES,
    );
    expect(result).toBe(
      'llama3 reads text only. Try a vision model for images.',
    );
  });

  it('defers history check when capabilities are unknown', () => {
    const result = getCapabilityConflict(
      'unknown',
      undefined,
      EMPTY,
      HISTORY_HAS_IMAGES,
    );
    expect(result).toBeNull();
  });

  it('defaults history-state to empty when caller omits the argument', () => {
    // Existing callers (and earlier tests in this file) pass only 3 args.
    // The 4th argument defaults to a no-history shape so the legacy
    // contract is preserved.
    const result = getCapabilityConflict('llama3', TEXT_ONLY, EMPTY);
    expect(result).toBeNull();
  });

  it('still warns from history when compose is empty', () => {
    const result = getCapabilityConflict(
      'llama3',
      TEXT_ONLY,
      EMPTY,
      HISTORY_HAS_IMAGES,
    );
    expect(result).toContain('hidden from llama3');
  });

  it('warns when history has more images per message than vision model accepts', () => {
    const result = getCapabilityConflict(
      'llama3.2-vision',
      VISION_SINGLE_IMAGE,
      EMPTY,
      HISTORY_HAS_TWO_IMAGES,
    );
    expect(result).toBe(
      'llama3.2-vision accepts one image per message. Extra images from earlier turns are hidden. Switch to a multi-image vision model to keep them.',
    );
  });

  it('pluralizes per-message history cap warning for max>1', () => {
    const result = getCapabilityConflict(
      'multi-cap',
      VISION_TWO_IMAGES,
      EMPTY,
      {
        historyHasImages: true,
        historyHasThinking: false,
        historyMaxImagesPerMessage: 5,
      },
    );
    expect(result).toBe(
      'multi-cap accepts 2 images per message. Extra images from earlier turns are hidden. Switch to a multi-image vision model to keep them.',
    );
  });

  it('does not warn when history image count fits within the cap', () => {
    const result = getCapabilityConflict(
      'llama3.2-vision',
      VISION_SINGLE_IMAGE,
      EMPTY,
      HISTORY_HAS_IMAGES,
    );
    expect(result).toBeNull();
  });

  it('does not fire history-cap warning for non-vision models (text-only branch wins)', () => {
    // Even though historyMaxImagesPerMessage > 1, the model is text-only
    // so the more fundamental "reads text only" copy takes priority.
    const result = getCapabilityConflict(
      'llama3',
      TEXT_ONLY,
      EMPTY,
      HISTORY_HAS_TWO_IMAGES,
    );
    expect(result).toContain('reads text only');
  });

  it('ignores history-cap warning when vision model has no max-images cap', () => {
    const result = getCapabilityConflict(
      'omnimodel',
      VISION,
      EMPTY,
      HISTORY_HAS_TWO_IMAGES,
    );
    expect(result).toBeNull();
  });

  it('ignores history-cap warning for defensive max=0 caps', () => {
    const odd: ModelCapabilities = {
      vision: true,
      thinking: false,
      maxImages: 0,
    };
    const result = getCapabilityConflict(
      'odd',
      odd,
      EMPTY,
      HISTORY_HAS_TWO_IMAGES,
    );
    expect(result).toBeNull();
  });
});

describe('isComposeCapabilityConflict', () => {
  it('returns false when capabilities are unknown', () => {
    expect(
      isComposeCapabilityConflict(undefined, {
        hasScreenCommand: false,
        hasThinkCommand: false,
        imageCount: 1,
      }),
    ).toBe(false);
    expect(
      isComposeCapabilityConflict(null, {
        hasScreenCommand: false,
        hasThinkCommand: false,
        imageCount: 1,
      }),
    ).toBe(false);
  });

  it('returns true when image attached to text-only model', () => {
    expect(
      isComposeCapabilityConflict(TEXT_ONLY, {
        hasScreenCommand: false,
        hasThinkCommand: false,
        imageCount: 1,
      }),
    ).toBe(true);
  });

  it('returns true when /screen on text-only model', () => {
    expect(
      isComposeCapabilityConflict(TEXT_ONLY, {
        hasScreenCommand: true,
        hasThinkCommand: false,
        imageCount: 0,
      }),
    ).toBe(true);
  });

  it('returns true when image count exceeds vision model max', () => {
    expect(
      isComposeCapabilityConflict(VISION_SINGLE_IMAGE, {
        hasScreenCommand: false,
        hasThinkCommand: false,
        imageCount: 2,
      }),
    ).toBe(true);
  });

  it('returns true when /screen pushes count over the cap', () => {
    expect(
      isComposeCapabilityConflict(VISION_SINGLE_IMAGE, {
        hasScreenCommand: true,
        hasThinkCommand: false,
        imageCount: 1,
      }),
    ).toBe(true);
  });

  it('returns false when image count is under the cap', () => {
    expect(
      isComposeCapabilityConflict(VISION_TWO_IMAGES, {
        hasScreenCommand: false,
        hasThinkCommand: false,
        imageCount: 1,
      }),
    ).toBe(false);
  });

  it('returns true when /think on a non-thinking model', () => {
    expect(
      isComposeCapabilityConflict(TEXT_ONLY, {
        hasScreenCommand: false,
        hasThinkCommand: true,
        imageCount: 0,
      }),
    ).toBe(true);
  });

  it('returns false when /think on a thinking-capable model', () => {
    expect(
      isComposeCapabilityConflict(THINKING_ONLY, {
        hasScreenCommand: false,
        hasThinkCommand: true,
        imageCount: 0,
      }),
    ).toBe(false);
  });

  it('returns false when nothing is queued', () => {
    expect(
      isComposeCapabilityConflict(TEXT_ONLY, {
        hasScreenCommand: false,
        hasThinkCommand: false,
        imageCount: 0,
      }),
    ).toBe(false);
  });

  it('returns false when vision-capable model has no max-images cap', () => {
    expect(
      isComposeCapabilityConflict(VISION, {
        hasScreenCommand: false,
        hasThinkCommand: false,
        imageCount: 5,
      }),
    ).toBe(false);
  });

  it('ignores a max-images cap below 1 (defensive)', () => {
    const odd: ModelCapabilities = {
      vision: true,
      thinking: false,
      maxImages: 0,
    };
    expect(
      isComposeCapabilityConflict(odd, {
        hasScreenCommand: false,
        hasThinkCommand: false,
        imageCount: 5,
      }),
    ).toBe(false);
  });
});

describe('getEnvironmentMessage', () => {
  it('returns the unreachable copy when Ollama cannot be reached (S1)', () => {
    // S1: connection refused / timeout / DNS failure. Even if the
    // installedCount and activeModel happen to be non-empty (stale state
    // from a prior fetch), reachability is the dominant constraint.
    expect(getEnvironmentMessage(false, 0, null)).toBe(
      OLLAMA_UNREACHABLE_MESSAGE,
    );
  });

  it('returns the unreachable copy even with stale active/installed values', () => {
    expect(getEnvironmentMessage(false, 3, 'gemma4:e4b')).toBe(
      OLLAMA_UNREACHABLE_MESSAGE,
    );
  });

  it('returns the no-models copy when reachable but installed list is empty (S2)', () => {
    expect(getEnvironmentMessage(true, 0, null)).toBe(
      NO_MODELS_INSTALLED_MESSAGE,
    );
  });

  it('returns the pick-a-model copy when reachable, models present, none active (S3)', () => {
    // S3 is the rare post-Phase-A defensive state. Backend auto-picks the
    // first installed model on launch, but if a payload drift ever lands
    // here we still surface a clear recovery cue instead of falling
    // through to the capability helper with a null model.
    const result = getEnvironmentMessage(true, 2, null);
    expect(result).toBe('Pick a model from the chip above to start chatting.');
  });

  it('returns null when an active model is set so per-message gates can run (S4)', () => {
    expect(getEnvironmentMessage(true, 2, 'gemma4:e4b')).toBeNull();
  });

  it('returns the pick-a-model copy when activeModel is the empty string', () => {
    // Empty string is treated as "no active model" so the strip surfaces
    // the recovery cue rather than letting the capability helper pretend
    // the empty slug is a real selection.
    expect(getEnvironmentMessage(true, 1, '')).toBe(
      'Pick a model from the chip above to start chatting.',
    );
  });
});
