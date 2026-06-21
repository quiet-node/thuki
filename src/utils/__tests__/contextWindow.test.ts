import { describe, expect, it } from 'vitest';

import { formatContextWindow } from '../contextWindow';

describe('formatContextWindow', () => {
  it('formats the common power-of-two windows as round K labels', () => {
    expect(formatContextWindow(32_768)).toBe('32K');
    expect(formatContextWindow(131_072)).toBe('128K');
    expect(formatContextWindow(262_144)).toBe('256K');
  });

  it('rounds an odd token count to the nearest K', () => {
    expect(formatContextWindow(40_000)).toBe('39K');
    expect(formatContextWindow(8_000)).toBe('8K');
  });

  it('switches to M at a mebitoken and trims a whole-number decimal', () => {
    expect(formatContextWindow(1_048_576)).toBe('1M');
    expect(formatContextWindow(1_572_864)).toBe('1.5M');
  });

  it('renders a sub-1K count raw, with no unit', () => {
    expect(formatContextWindow(512)).toBe('512');
  });

  it('returns an empty string for non-positive or non-finite input so the pill can be skipped', () => {
    expect(formatContextWindow(0)).toBe('');
    expect(formatContextWindow(-1)).toBe('');
    expect(formatContextWindow(Number.NaN)).toBe('');
    expect(formatContextWindow(Number.POSITIVE_INFINITY)).toBe('');
  });
});
