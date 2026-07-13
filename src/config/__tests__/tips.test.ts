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
    const imagesTip = TIPS.find((t) => {
      const s = tipText(t).toLowerCase();
      return s.includes('settings') && s.includes('image');
    });
    expect(imagesTip).toBeDefined();
  });

  it('includes a tip about replacing rewritten text back into the source app', () => {
    const replaceTip = TIPS.find((t) =>
      tipText(t).toLowerCase().includes('replace'),
    );
    expect(replaceTip).toBeDefined();
  });

  it('includes Auto search and /search tips for the current web pipeline', () => {
    const texts = TIPS.map(tipText).join('\n').toLowerCase();
    expect(texts).toContain('auto search');
    expect(texts).toContain('/search');
    expect(texts).not.toMatch(/agentic|searxng|docker/);
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
