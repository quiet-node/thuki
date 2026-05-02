import { describe, expect, it } from 'vitest';
import { TIPS } from '../tips';

function tipText(tip: (typeof TIPS)[number]): string {
  return typeof tip === 'string' ? tip : tip.text;
}

describe('TIPS', () => {
  it('is non-empty', () => {
    expect(TIPS.length).toBeGreaterThan(0);
  });

  it('all tip strings are under 110 chars', () => {
    for (const tip of TIPS) {
      expect(tipText(tip).length).toBeLessThanOrEqual(110);
    }
  });

  it('includes an images tip pointing to Settings', () => {
    const imagesTip = TIPS.find((t) => tipText(t).includes('Settings'));
    expect(imagesTip).toBeDefined();
    expect(tipText(imagesTip!).toLowerCase()).toContain('image');
  });

  it('linked tips carry an https URL', () => {
    const linked = TIPS.filter((t) => typeof t !== 'string');
    expect(linked.length).toBeGreaterThan(0);
    for (const tip of linked) {
      const linkTip = tip as { text: string; url: string };
      expect(linkTip.url).toMatch(/^https:\/\//);
      expect(linkTip.text.length).toBeGreaterThan(0);
    }
  });
});
