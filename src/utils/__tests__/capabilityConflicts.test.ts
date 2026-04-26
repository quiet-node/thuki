import { describe, it, expect } from 'vitest';
import { getCapabilityConflict } from '../capabilityConflicts';
import type { ModelCapabilities } from '../../types/model';

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

describe('getCapabilityConflict', () => {
  it('returns null when no images and no /screen', () => {
    const result = getCapabilityConflict('llama3', TEXT_ONLY, {
      hasImages: false,
      hasScreenCommand: false,
      imageCount: 0,
    });
    expect(result).toBeNull();
  });

  it('returns null when capabilities are unknown (defaults permissive)', () => {
    const result = getCapabilityConflict('llama3', undefined, {
      hasImages: true,
      hasScreenCommand: false,
      imageCount: 1,
    });
    expect(result).toBeNull();
  });

  it('returns null when capabilities is null', () => {
    const result = getCapabilityConflict('llama3', null, {
      hasImages: true,
      hasScreenCommand: false,
      imageCount: 1,
    });
    expect(result).toBeNull();
  });

  it('returns null when active model can see images and has no max-images cap', () => {
    const result = getCapabilityConflict('llava', VISION, {
      hasImages: true,
      hasScreenCommand: true,
      imageCount: 3,
    });
    expect(result).toBeNull();
  });

  it('returns conflict when images attached and model is text-only', () => {
    const result = getCapabilityConflict('llama3', TEXT_ONLY, {
      hasImages: true,
      hasScreenCommand: false,
      imageCount: 1,
    });
    expect(result).toBe(
      'llama3 reads text only. Try a vision model for images.',
    );
  });

  it('returns conflict when /screen is queued and model is text-only', () => {
    const result = getCapabilityConflict('llama3', TEXT_ONLY, {
      hasImages: false,
      hasScreenCommand: true,
      imageCount: 0,
    });
    expect(result).toContain('reads text only');
  });

  it('falls back to a generic name when model name is empty', () => {
    const result = getCapabilityConflict('', TEXT_ONLY, {
      hasImages: true,
      hasScreenCommand: false,
      imageCount: 1,
    });
    expect(result).toBe(
      'this model reads text only. Try a vision model for images.',
    );
  });

  it('falls back to a generic name when model name is null', () => {
    const result = getCapabilityConflict(null, TEXT_ONLY, {
      hasImages: true,
      hasScreenCommand: false,
      imageCount: 1,
    });
    expect(result?.startsWith('this model')).toBe(true);
  });

  it('falls back to a generic name when model name is undefined', () => {
    const result = getCapabilityConflict(undefined, TEXT_ONLY, {
      hasImages: true,
      hasScreenCommand: false,
      imageCount: 1,
    });
    expect(result?.startsWith('this model')).toBe(true);
  });

  // ── max-images gate ───────────────────────────────────────────────────────

  it('returns null when single-image vision model has exactly one image', () => {
    const result = getCapabilityConflict(
      'llama3.2-vision',
      VISION_SINGLE_IMAGE,
      {
        hasImages: true,
        hasScreenCommand: false,
        imageCount: 1,
      },
    );
    expect(result).toBeNull();
  });

  it('refuses two attached images on a single-image vision model', () => {
    const result = getCapabilityConflict(
      'llama3.2-vision',
      VISION_SINGLE_IMAGE,
      {
        hasImages: true,
        hasScreenCommand: false,
        imageCount: 2,
      },
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
      {
        hasImages: true,
        hasScreenCommand: true,
        imageCount: 1,
      },
    );
    expect(result).toBe(
      'llama3.2-vision accepts one image at a time. Remove the extras to send.',
    );
  });

  it('allows /screen alone on a single-image vision model', () => {
    const result = getCapabilityConflict(
      'llama3.2-vision',
      VISION_SINGLE_IMAGE,
      {
        hasImages: false,
        hasScreenCommand: true,
        imageCount: 0,
      },
    );
    expect(result).toBeNull();
  });

  it('pluralizes the noun for a multi-image cap', () => {
    const result = getCapabilityConflict('multi-cap', VISION_TWO_IMAGES, {
      hasImages: true,
      hasScreenCommand: false,
      imageCount: 5,
    });
    expect(result).toBe(
      'multi-cap accepts 2 images at a time. Remove the extras to send.',
    );
  });

  it('allows submits at the cap exactly', () => {
    const result = getCapabilityConflict('multi-cap', VISION_TWO_IMAGES, {
      hasImages: true,
      hasScreenCommand: false,
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
      hasImages: true,
      hasScreenCommand: false,
      imageCount: 3,
    });
    expect(result).toBeNull();
  });
});
