import { describe, expect, it } from 'vitest';
import {
  AVATAR_PALETTE,
  avatarColor,
  domainHue,
  domainOf,
} from '../domainAvatar';

describe('domainOf', () => {
  it('strips a leading www. prefix', () => {
    expect(domainOf('https://www.britannica.com/x')).toBe('britannica.com');
  });

  it('keeps a hostname that has no www. prefix', () => {
    expect(domainOf('https://en.wikipedia.org/wiki/X')).toBe(
      'en.wikipedia.org',
    );
  });

  it('falls back to the raw input when URL parsing fails', () => {
    expect(domainOf('not-a-url')).toBe('not-a-url');
  });

  it('returns Punycode (xn--) for IDN hosts so homographs cannot look latin', () => {
    expect(domainOf('https://münchen.example/path')).toBe(
      'xn--mnchen-3ya.example',
    );
    expect(domainOf('https://xn--mnchen-3ya.example/')).toBe(
      'xn--mnchen-3ya.example',
    );
    const cyrillic = domainOf('https://аррle.com/x');
    expect(cyrillic.startsWith('xn--')).toBe(true);
    expect(cyrillic).not.toBe('apple.com');
  });
});

describe('domainHue', () => {
  it('is deterministic and within 0–359', () => {
    const hue = domainHue('example.com');
    expect(hue).toBe(domainHue('example.com'));
    expect(hue).toBeGreaterThanOrEqual(0);
    expect(hue).toBeLessThan(360);
  });

  it('returns 0 for an empty domain', () => {
    expect(domainHue('')).toBe(0);
  });
});

describe('avatarColor', () => {
  it('returns a palette entry keyed deterministically by domain', () => {
    const color = avatarColor('example.com');
    expect(AVATAR_PALETTE).toContain(color);
    expect(color).toBe(avatarColor('example.com'));
  });
});
