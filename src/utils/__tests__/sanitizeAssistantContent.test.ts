import { describe, it, expect } from 'vitest';
import { cleanForRender, STRIP_PATTERNS } from '../sanitizeAssistantContent';

describe('cleanForRender', () => {
  it('returns clean input unchanged', () => {
    const input = 'Hello **world**\n\n```ts\nconst x = 1;\n```\nDone.';
    expect(cleanForRender(input)).toBe(input);
  });

  it('returns empty string unchanged', () => {
    expect(cleanForRender('')).toBe('');
  });

  it('strips every known pattern', () => {
    for (const pattern of STRIP_PATTERNS) {
      expect(cleanForRender(`before${pattern}after`)).toBe('beforeafter');
    }
  });

  it('strips multiple occurrences in a single string', () => {
    expect(cleanForRender('<|im_start|>a<|im_start|>b<|im_end|>c')).toBe('abc');
  });

  it('preserves unicode and emoji', () => {
    const input = 'héllo 世界 🚀';
    expect(cleanForRender(input)).toBe(input);
  });

  it('strips a leaked thinking tag wrapper from legacy assistant content', () => {
    // Pre-Phase-B reasoning models occasionally emitted `<think>...</think>`
    // into `content` instead of the structured `thinking` field. The
    // render-time scrub keeps that legacy text visually clean.
    const dirty = 'answer<think>internal reasoning</think> shipped.';
    expect(cleanForRender(dirty)).toBe('answerinternal reasoning shipped.');
  });
});
