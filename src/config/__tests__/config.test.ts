import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// We need to test config with different env values, so we re-import after mocking.
// Vitest's module cache must be reset between tests.

describe('config', () => {
  const originalEnv = { ...import.meta.env };

  afterEach(() => {
    // Restore original env
    Object.keys(import.meta.env).forEach((key) => {
      if (!(key in originalEnv)) {
        delete (import.meta.env as Record<string, string | undefined>)[key];
      }
    });
    Object.assign(import.meta.env, originalEnv);
    vi.resetModules();
  });

  describe('quote defaults (no env override)', () => {
    it('uses default maxDisplayLines of 4', async () => {
      const { quote } = await import('..');
      expect(quote.maxDisplayLines).toBe(4);
    });

    it('uses default maxDisplayChars of 300', async () => {
      const { quote } = await import('..');
      expect(quote.maxDisplayChars).toBe(300);
    });

    it('uses default maxContextLength of 4096', async () => {
      const { quote } = await import('..');
      expect(quote.maxContextLength).toBe(4096);
    });
  });

  describe('quote with env overrides', () => {
    beforeEach(() => {
      vi.resetModules();
    });

    it('reads VITE_QUOTE_MAX_DISPLAY_LINES from env', async () => {
      (import.meta.env as Record<string, string>).VITE_QUOTE_MAX_DISPLAY_LINES =
        '6';
      const { quote } = await import('..');
      expect(quote.maxDisplayLines).toBe(6);
    });

    it('reads VITE_QUOTE_MAX_DISPLAY_CHARS from env', async () => {
      (import.meta.env as Record<string, string>).VITE_QUOTE_MAX_DISPLAY_CHARS =
        '500';
      const { quote } = await import('..');
      expect(quote.maxDisplayChars).toBe(500);
    });

    it('reads VITE_QUOTE_MAX_CONTEXT_LENGTH from env', async () => {
      (
        import.meta.env as Record<string, string>
      ).VITE_QUOTE_MAX_CONTEXT_LENGTH = '8192';
      const { quote } = await import('..');
      expect(quote.maxContextLength).toBe(8192);
    });
  });

  describe('envInt edge cases', () => {
    beforeEach(() => {
      vi.resetModules();
    });

    it('falls back to default for empty string', async () => {
      (import.meta.env as Record<string, string>).VITE_QUOTE_MAX_DISPLAY_LINES =
        '';
      const { quote } = await import('..');
      expect(quote.maxDisplayLines).toBe(4);
    });

    it('falls back to default for non-numeric string', async () => {
      (import.meta.env as Record<string, string>).VITE_QUOTE_MAX_DISPLAY_LINES =
        'abc';
      const { quote } = await import('..');
      expect(quote.maxDisplayLines).toBe(4);
    });

    it('falls back to default for negative number', async () => {
      (import.meta.env as Record<string, string>).VITE_QUOTE_MAX_DISPLAY_LINES =
        '-5';
      const { quote } = await import('..');
      expect(quote.maxDisplayLines).toBe(4);
    });

    it('falls back to default for zero', async () => {
      (import.meta.env as Record<string, string>).VITE_QUOTE_MAX_DISPLAY_LINES =
        '0';
      const { quote } = await import('..');
      expect(quote.maxDisplayLines).toBe(4);
    });

    it('floors decimal values', async () => {
      (import.meta.env as Record<string, string>).VITE_QUOTE_MAX_DISPLAY_LINES =
        '5.7';
      const { quote } = await import('..');
      expect(quote.maxDisplayLines).toBe(5);
    });

    it('falls back to default for Infinity', async () => {
      (import.meta.env as Record<string, string>).VITE_QUOTE_MAX_DISPLAY_LINES =
        'Infinity';
      const { quote } = await import('..');
      expect(quote.maxDisplayLines).toBe(4);
    });
  });
});
