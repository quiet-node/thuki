import { describe, expect, it } from 'vitest';
import { TIPS } from '../tips';

describe('TIPS', () => {
  it('is non-empty', () => {
    expect(TIPS.length).toBeGreaterThan(0);
  });

  it('all strings are under 110 chars', () => {
    for (const tip of TIPS) {
      expect(tip.length).toBeLessThanOrEqual(110);
    }
  });
});
