import { describe, it, expect } from 'vitest';
import { getCapabilityConflict } from '../capabilityConflicts';
import type { ModelCapabilities } from '../../types/model';
import type { ComposeCapabilityState } from '../capabilityConflicts';

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

  it('falls back to a generic name when model name is empty', () => {
    const result = getCapabilityConflict('', TEXT_ONLY, {
      ...EMPTY,
      imageCount: 1,
    });
    expect(result).toBe(
      'this model reads text only. Try a vision model for images.',
    );
  });

  it('falls back to a generic name when model name is null', () => {
    const result = getCapabilityConflict(null, TEXT_ONLY, {
      ...EMPTY,
      imageCount: 1,
    });
    expect(result?.startsWith('this model')).toBe(true);
  });

  it('falls back to a generic name when model name is undefined', () => {
    const result = getCapabilityConflict(undefined, TEXT_ONLY, {
      ...EMPTY,
      imageCount: 1,
    });
    expect(result?.startsWith('this model')).toBe(true);
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

  it('falls back to a generic name when /think mismatches and name is empty', () => {
    const result = getCapabilityConflict('', TEXT_ONLY, {
      ...EMPTY,
      hasThinkCommand: true,
    });
    expect(result).toBe(
      "this model doesn't show reasoning. Try a thinking model for /think.",
    );
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
});
