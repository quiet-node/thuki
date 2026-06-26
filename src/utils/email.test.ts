import { describe, it, expect } from 'vitest';
import { isValidEmail } from './email';

describe('isValidEmail', () => {
  it('accepts well-formed addresses', () => {
    expect(isValidEmail('founder@thuki.app')).toBe(true);
    expect(isValidEmail('a.b+tag@sub.example.co.uk')).toBe(true);
  });

  it('trims surrounding whitespace before checking', () => {
    expect(isValidEmail('  founder@thuki.app  ')).toBe(true);
  });

  it('rejects malformed addresses', () => {
    expect(isValidEmail('')).toBe(false);
    expect(isValidEmail('plainaddress')).toBe(false);
    expect(isValidEmail('no-at-sign.com')).toBe(false);
    expect(isValidEmail('@nolocal.com')).toBe(false);
    expect(isValidEmail('nodomain@')).toBe(false);
    expect(isValidEmail('user@nodot')).toBe(false);
    expect(isValidEmail('has space@thuki.app')).toBe(false);
  });
});
