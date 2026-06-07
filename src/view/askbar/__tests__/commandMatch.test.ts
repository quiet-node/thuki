import { describe, it, expect } from 'vitest';
import { getCommandMatch } from '../commandMatch';

describe('getCommandMatch', () => {
  it('returns null for an empty string', () => {
    expect(getCommandMatch('')).toBeNull();
  });

  it('returns null when no known trigger is present', () => {
    expect(getCommandMatch('hello world')).toBeNull();
  });

  it('matches a trigger at the start of the string', () => {
    expect(getCommandMatch('/search')).toEqual({ start: 0, end: 7 });
  });

  it('matches a trigger preceded by whitespace', () => {
    // "hi /think" -> "/think" starts at index 3, length 6.
    expect(getCommandMatch('hi /think')).toEqual({ start: 3, end: 9 });
  });

  it('matches a trigger followed by whitespace', () => {
    expect(getCommandMatch('/search foo')).toEqual({ start: 0, end: 7 });
  });

  it('rejects a trigger not preceded by a boundary', () => {
    expect(getCommandMatch('x/search')).toBeNull();
  });

  it('rejects a trigger not followed by a boundary', () => {
    // "/searching" is not the command "/search".
    expect(getCommandMatch('/searching')).toBeNull();
  });

  it('skips a boundary-invalid occurrence and matches a later valid one', () => {
    // First "/search" (index 1) is glued to "x"; the second (index 9) is valid.
    expect(getCommandMatch('x/search /search')).toEqual({ start: 9, end: 16 });
  });

  it('replaces the best match when a later-listed trigger sits earlier', () => {
    // "/search" (listed first) matches at index 7; "/think" (listed later)
    // matches at index 0, so the smaller start replaces the initial best.
    expect(getCommandMatch('/think /search')).toEqual({ start: 0, end: 6 });
  });

  it('keeps the best match when a later-listed trigger sits further right', () => {
    // "/search" matches at index 0 first; "/think" at index 8 does not displace
    // it because 8 is not smaller than 0.
    expect(getCommandMatch('/search /think')).toEqual({ start: 0, end: 7 });
  });
});
