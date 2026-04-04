import { describe, it, expect } from 'vitest';
import { formatRelativeTime } from '../formatRelativeTime';

const NOW = 1_700_000_000_000; // milliseconds

describe('formatRelativeTime', () => {
  it('returns "just now" for timestamps less than 60 seconds ago', () => {
    expect(formatRelativeTime(NOW, NOW)).toBe('just now');
    expect(formatRelativeTime(NOW - 30_000, NOW)).toBe('just now');
    expect(formatRelativeTime(NOW - 59_000, NOW)).toBe('just now');
  });

  it('returns minutes for timestamps less than 1 hour ago', () => {
    expect(formatRelativeTime(NOW - 60_000, NOW)).toBe('1m ago');
    expect(formatRelativeTime(NOW - 120_000, NOW)).toBe('2m ago');
    expect(formatRelativeTime(NOW - 3_599_000, NOW)).toBe('59m ago');
  });

  it('returns hours for timestamps less than 24 hours ago', () => {
    expect(formatRelativeTime(NOW - 3_600_000, NOW)).toBe('1h ago');
    expect(formatRelativeTime(NOW - 7_200_000, NOW)).toBe('2h ago');
    expect(formatRelativeTime(NOW - 86_399_000, NOW)).toBe('23h ago');
  });

  it('returns days for timestamps less than 14 days ago', () => {
    expect(formatRelativeTime(NOW - 86_400_000, NOW)).toBe('1d ago');
    expect(formatRelativeTime(NOW - 86_400_000 * 7, NOW)).toBe('7d ago');
    expect(formatRelativeTime(NOW - 86_400_000 * 13, NOW)).toBe('13d ago');
  });

  it('returns weeks for timestamps 14+ days ago', () => {
    expect(formatRelativeTime(NOW - 86_400_000 * 14, NOW)).toBe('2w ago');
    expect(formatRelativeTime(NOW - 86_400_000 * 21, NOW)).toBe('3w ago');
    expect(formatRelativeTime(NOW - 86_400_000 * 60, NOW)).toBe('8w ago');
  });

  it('clamps negative diffs to "just now"', () => {
    expect(formatRelativeTime(NOW + 100_000, NOW)).toBe('just now');
  });

  it('defaults to Date.now() when nowMillis is omitted', () => {
    const recent = Date.now() - 5_000;
    expect(formatRelativeTime(recent)).toBe('just now');
  });
});
