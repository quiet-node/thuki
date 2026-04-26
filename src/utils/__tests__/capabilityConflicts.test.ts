import { describe, it, expect } from 'vitest';
import { getCapabilityConflict } from '../capabilityConflicts';
import type { ModelCapabilities } from '../../types/model';

const VISION: ModelCapabilities = {
  vision: true,
  thinking: false,
};
const TEXT_ONLY: ModelCapabilities = {
  vision: false,
  thinking: false,
};

describe('getCapabilityConflict', () => {
  it('returns null when no images and no /screen', () => {
    const result = getCapabilityConflict('llama3', TEXT_ONLY, {
      hasImages: false,
      hasScreenCommand: false,
    });
    expect(result).toBeNull();
  });

  it('returns null when capabilities are unknown (defaults permissive)', () => {
    const result = getCapabilityConflict('llama3', undefined, {
      hasImages: true,
      hasScreenCommand: false,
    });
    expect(result).toBeNull();
  });

  it('returns null when capabilities is null', () => {
    const result = getCapabilityConflict('llama3', null, {
      hasImages: true,
      hasScreenCommand: false,
    });
    expect(result).toBeNull();
  });

  it('returns null when active model can see images', () => {
    const result = getCapabilityConflict('llava', VISION, {
      hasImages: true,
      hasScreenCommand: true,
    });
    expect(result).toBeNull();
  });

  it('returns conflict when images attached and model is text-only', () => {
    const result = getCapabilityConflict('llama3', TEXT_ONLY, {
      hasImages: true,
      hasScreenCommand: false,
    });
    expect(result).toBe(
      "llama3 can't see images. Switch to a vision model to send.",
    );
  });

  it('returns conflict when /screen is queued and model is text-only', () => {
    const result = getCapabilityConflict('llama3', TEXT_ONLY, {
      hasImages: false,
      hasScreenCommand: true,
    });
    expect(result).toContain("can't see images");
  });

  it('falls back to a generic name when model name is empty', () => {
    const result = getCapabilityConflict('', TEXT_ONLY, {
      hasImages: true,
      hasScreenCommand: false,
    });
    expect(result).toBe(
      "this model can't see images. Switch to a vision model to send.",
    );
  });

  it('falls back to a generic name when model name is null', () => {
    const result = getCapabilityConflict(null, TEXT_ONLY, {
      hasImages: true,
      hasScreenCommand: false,
    });
    expect(result?.startsWith('this model')).toBe(true);
  });

  it('falls back to a generic name when model name is undefined', () => {
    const result = getCapabilityConflict(undefined, TEXT_ONLY, {
      hasImages: true,
      hasScreenCommand: false,
    });
    expect(result?.startsWith('this model')).toBe(true);
  });
});
