import { describe, it, expect } from 'vitest';
import { formatRelative } from '../relativeTime';

describe('formatRelative', () => {
  it.each([
    [10, 'just now'],
    [120, '2 minutes ago'],
    [7200, '2 hours ago'],
    [172800, '2 days ago'],
  ])('formats %i seconds ago as "%s"', (sec, expected) => {
    const unix = Math.floor(Date.now() / 1000) - sec;
    expect(formatRelative(unix)).toBe(expected);
  });

  it('returns "just now" for 0 seconds ago', () => {
    const unix = Math.floor(Date.now() / 1000);
    expect(formatRelative(unix)).toBe('just now');
  });

  it('returns "just now" for 59 seconds ago', () => {
    const unix = Math.floor(Date.now() / 1000) - 59;
    expect(formatRelative(unix)).toBe('just now');
  });

  it('returns "1 minutes ago" for 60 seconds ago', () => {
    const unix = Math.floor(Date.now() / 1000) - 60;
    expect(formatRelative(unix)).toBe('1 minutes ago');
  });

  it('returns "1 hours ago" for 3600 seconds ago', () => {
    const unix = Math.floor(Date.now() / 1000) - 3600;
    expect(formatRelative(unix)).toBe('1 hours ago');
  });

  it('returns "1 days ago" for 86400 seconds ago', () => {
    const unix = Math.floor(Date.now() / 1000) - 86400;
    expect(formatRelative(unix)).toBe('1 days ago');
  });
});
