import { describe, it, expect } from 'vitest';
import { escapeCurrencyDollars } from '../escapeCurrencyDollars';

describe('escapeCurrencyDollars', () => {
  describe('passthrough / no-op fast paths', () => {
    it('returns empty string unchanged', () => {
      expect(escapeCurrencyDollars('')).toBe('');
    });

    it('returns content with no dollar sign unchanged', () => {
      const input = 'Hello **world**\n\n```ts\nconst x = 1;\n```\nDone.';
      expect(escapeCurrencyDollars(input)).toBe(input);
    });

    it('leaves a lone dollar not followed by a digit untouched', () => {
      expect(escapeCurrencyDollars('it costs 5$ total')).toBe(
        'it costs 5$ total',
      );
      expect(escapeCurrencyDollars('a $ b')).toBe('a $ b');
    });
  });

  describe('currency escaping (the bug)', () => {
    it('escapes a single currency amount', () => {
      expect(escapeCurrencyDollars('raise $1M soon')).toBe('raise \\$1M soon');
    });

    it('escapes a currency amount at the start of the string', () => {
      expect(escapeCurrencyDollars('$5 each')).toBe('\\$5 each');
    });

    it('escapes every currency amount so no inline math span forms', () => {
      const input =
        'generate $1M in 18 months ... $1M in 18 months and accepted';
      expect(escapeCurrencyDollars(input)).toBe(
        'generate \\$1M in 18 months ... \\$1M in 18 months and accepted',
      );
    });

    it('escapes decimal and grouped currency forms', () => {
      expect(escapeCurrencyDollars('$10.50 and $1,000 and $1B')).toBe(
        '\\$10.50 and \\$1,000 and \\$1B',
      );
    });

    it('escapes adjacent currency values', () => {
      expect(escapeCurrencyDollars('from $5 to $9')).toBe('from \\$5 to \\$9');
    });
  });

  describe('genuine math is preserved', () => {
    it('leaves letter-led inline math untouched', () => {
      const input = 'Energy is $E = mc^2$ at rest';
      expect(escapeCurrencyDollars(input)).toBe(input);
    });

    it('leaves backslash-led inline math untouched', () => {
      const input = 'angle $\\alpha$ here';
      expect(escapeCurrencyDollars(input)).toBe(input);
    });

    it('leaves $$ block math untouched even when it opens with a digit', () => {
      const input = '$$1 + 1 = 2$$';
      expect(escapeCurrencyDollars(input)).toBe(input);
    });

    it('leaves a multi-line $$ block untouched', () => {
      const input = '$$\n1 + 1\n$$';
      expect(escapeCurrencyDollars(input)).toBe(input);
    });
  });

  describe('already-escaped dollars are not double-escaped', () => {
    it('does not touch a backslash-escaped currency dollar', () => {
      expect(escapeCurrencyDollars('cost is \\$5 only')).toBe(
        'cost is \\$5 only',
      );
    });
  });

  describe('documented trade-off: digit-led inline math', () => {
    it('escapes digit-led math expressions (rendered as literal text)', () => {
      // Known, documented limitation: "$2x$" is indistinguishable from
      // currency at the syntax level, so it is treated as currency.
      expect(escapeCurrencyDollars('the term $2x$ vanishes')).toBe(
        'the term \\$2x$ vanishes',
      );
    });
  });

  describe('code regions are never modified', () => {
    it('does not escape inside an inline code span', () => {
      expect(escapeCurrencyDollars('run `echo $5` now')).toBe(
        'run `echo $5` now',
      );
    });

    it('escapes outside but not inside inline code on the same line', () => {
      expect(escapeCurrencyDollars('pay $5 then `echo $9` done')).toBe(
        'pay \\$5 then `echo $9` done',
      );
    });

    it('handles multi-backtick inline code spans', () => {
      expect(escapeCurrencyDollars('a ``$5 `nested` $5`` b $7')).toBe(
        'a ``$5 `nested` $5`` b \\$7',
      );
    });

    it('treats an unclosed inline backtick run as literal text', () => {
      expect(escapeCurrencyDollars('weird ` $5 tail')).toBe(
        'weird ` \\$5 tail',
      );
    });

    it('does not escape inside a fenced code block', () => {
      const input = 'before $5\n```sh\necho $9\nprice=$10\n```\nafter $5';
      expect(escapeCurrencyDollars(input)).toBe(
        'before \\$5\n```sh\necho $9\nprice=$10\n```\nafter \\$5',
      );
    });

    it('does not escape inside a tilde fenced code block', () => {
      const input = '~~~\ncost $9\n~~~\nout $5';
      expect(escapeCurrencyDollars(input)).toBe('~~~\ncost $9\n~~~\nout \\$5');
    });

    it('keeps content fenced when a fence is never closed (streaming)', () => {
      const input = 'lead $5\n```\necho $9\nstill $9';
      expect(escapeCurrencyDollars(input)).toBe(
        'lead \\$5\n```\necho $9\nstill $9',
      );
    });

    it('does not treat a shorter marker run as a fence close', () => {
      // Opened with ````; a ``` line inside is content, not a close.
      const input = '````\n$9\n```\n$9\n````\nout $5';
      expect(escapeCurrencyDollars(input)).toBe(
        '````\n$9\n```\n$9\n````\nout \\$5',
      );
    });

    it('does not treat a different fence char as a close', () => {
      const input = '```\n$9\n~~~\n$9\n```\nout $5';
      expect(escapeCurrencyDollars(input)).toBe(
        '```\n$9\n~~~\n$9\n```\nout \\$5',
      );
    });

    it('does not close a fence on a marker line that has a trailing info string', () => {
      const input = '```\n$9\n``` not-a-close\n$9\n```\nout $5';
      expect(escapeCurrencyDollars(input)).toBe(
        '```\n$9\n``` not-a-close\n$9\n```\nout \\$5',
      );
    });
  });
});
