import { describe, expect, test } from 'vitest';

import {
  type Assertion,
  evaluateAll,
  evaluateResponse,
  extractJsonObject,
} from './assertions';

describe('extractJsonObject', () => {
  test('parses a bare JSON object', () => {
    expect(extractJsonObject('{"ok": true}')).toEqual({ ok: true });
  });

  test('parses a JSON object wrapped in a fenced code block', () => {
    expect(extractJsonObject('```json\n{"ok": true}\n```')).toEqual({
      ok: true,
    });
  });

  test('ignores prose before and after the object', () => {
    expect(extractJsonObject('Sure! Here: {"ok": true} hope that helps')).toEqual(
      { ok: true },
    );
  });

  test('matches the balanced closing brace with nested objects', () => {
    expect(extractJsonObject('{"a": {"b": 1}}')).toEqual({ a: { b: 1 } });
  });

  test('does not treat a brace inside a string as the closing brace', () => {
    expect(extractJsonObject('{"msg": "a } b"}')).toEqual({ msg: 'a } b' });
  });

  test('returns null when there is no object', () => {
    expect(extractJsonObject('Paris')).toBeNull();
  });

  test('returns null when the braced span is not valid JSON', () => {
    expect(extractJsonObject('{not json}')).toBeNull();
  });
});

describe('evaluateResponse', () => {
  test('contains matches case-insensitively anywhere in the response', () => {
    const assertion = {
      id: 'capital',
      prompt: 'x',
      kind: 'contains' as const,
      needle: 'Paris',
    };
    expect(evaluateResponse(assertion, 'The capital is paris.')).toBe(true);
    expect(evaluateResponse(assertion, 'It is London.')).toBe(false);
  });

  test('jsonEquals matches the embedded object regardless of key order', () => {
    const assertion = {
      id: 'json',
      prompt: 'x',
      kind: 'jsonEquals' as const,
      expected: { ok: true, n: 1 },
    };
    expect(evaluateResponse(assertion, '```json\n{"n": 1, "ok": true}\n```')).toBe(
      true,
    );
  });

  test('jsonEquals fails on extra keys or a missing object', () => {
    const assertion = {
      id: 'json',
      prompt: 'x',
      kind: 'jsonEquals' as const,
      expected: { ok: true },
    };
    expect(evaluateResponse(assertion, '{"ok": true, "extra": 1}')).toBe(false);
    expect(evaluateResponse(assertion, 'no json here')).toBe(false);
  });
});

describe('evaluateAll', () => {
  const assertions: Assertion[] = [
    { id: 'capital', prompt: 'x', kind: 'contains', needle: 'Paris' },
    { id: 'arithmetic', prompt: 'x', kind: 'contains', needle: '42' },
  ];

  test('passes with no failures when every response satisfies its assertion', () => {
    const result = evaluateAll(assertions, {
      capital: 'Paris',
      arithmetic: 'The answer is 42.',
    });
    expect(result.pass).toBe(true);
    expect(result.failures).toEqual([]);
  });

  test('reports the ids of failing assertions', () => {
    const result = evaluateAll(assertions, {
      capital: 'Paris',
      arithmetic: 'The answer is 41.',
    });
    expect(result.pass).toBe(false);
    expect(result.failures).toEqual(['arithmetic']);
  });

  test('treats a missing response as a failure', () => {
    const result = evaluateAll(assertions, { capital: 'Paris' });
    expect(result.pass).toBe(false);
    expect(result.failures).toEqual(['arithmetic']);
  });
});
