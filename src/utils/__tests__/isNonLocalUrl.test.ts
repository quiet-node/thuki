import { describe, it, expect } from 'vitest';

import { isNonLocalUrl } from '../isNonLocalUrl';

describe('isNonLocalUrl', () => {
  it('treats localhost and loopback as local', () => {
    expect(isNonLocalUrl('http://localhost:11434')).toBe(false);
    expect(isNonLocalUrl('http://127.0.0.1:11434')).toBe(false);
    expect(isNonLocalUrl('http://[::1]:11434')).toBe(false);
    expect(isNonLocalUrl('http://api.localhost:11434')).toBe(false);
  });

  it('treats RFC1918 and link-local ranges as local', () => {
    expect(isNonLocalUrl('http://192.168.1.50:11434')).toBe(false);
    expect(isNonLocalUrl('http://10.0.0.2:11434')).toBe(false);
    expect(isNonLocalUrl('http://172.16.0.5:11434')).toBe(false);
    expect(isNonLocalUrl('http://172.31.255.1:11434')).toBe(false);
    expect(isNonLocalUrl('http://169.254.1.1:11434')).toBe(false);
  });

  it('flags public hosts as non-local', () => {
    expect(isNonLocalUrl('http://example.com:11434')).toBe(true);
    expect(isNonLocalUrl('http://8.8.8.8:11434')).toBe(true);
    expect(isNonLocalUrl('https://ollama.my-server.net')).toBe(true);
    // 172.32 is outside the 172.16-31 private block.
    expect(isNonLocalUrl('http://172.32.0.1:11434')).toBe(true);
  });

  it('treats malformed or empty input as local (no warning)', () => {
    expect(isNonLocalUrl('')).toBe(false);
    expect(isNonLocalUrl('not a url')).toBe(false);
  });
});
