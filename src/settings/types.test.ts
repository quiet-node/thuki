import { describe, it, expect } from 'vitest';

import { describeConfigError } from './types';

describe('describeConfigError', () => {
  it('returns generic message for non-object input', () => {
    expect(describeConfigError(null)).toMatch(/try again/);
    expect(describeConfigError('boom')).toMatch(/try again/);
    expect(describeConfigError(42)).toMatch(/try again/);
    expect(describeConfigError(undefined)).toMatch(/try again/);
  });

  it('formats io_error with source string', () => {
    expect(
      describeConfigError({
        kind: 'io_error',
        path: '/x',
        source: 'permission denied',
      }),
    ).toMatch(/permission denied/);
  });

  it('falls back to generic I/O wording when io_error lacks source', () => {
    expect(describeConfigError({ kind: 'io_error', path: '/x' })).toMatch(
      /I\/O error/,
    );
  });

  it('formats unknown_section', () => {
    expect(
      describeConfigError({ kind: 'unknown_section', section: 'bogus' }),
    ).toMatch(/bogus/);
  });

  it('formats unknown_field with section.key', () => {
    expect(
      describeConfigError({
        kind: 'unknown_field',
        section: 'model',
        key: 'secret',
      }),
    ).toMatch(/model\.secret/);
  });

  it('returns explicit type_mismatch message when present', () => {
    expect(
      describeConfigError({
        kind: 'type_mismatch',
        section: 'window',
        key: 'overlay_width',
        message: 'expected integer',
      }),
    ).toBe('expected integer');
  });

  it('falls back to default wording when type_mismatch lacks message', () => {
    expect(
      describeConfigError({
        kind: 'type_mismatch',
        section: 'window',
        key: 'overlay_width',
      }),
    ).toMatch(/Wrong type/);
  });

  it('returns parse copy for parse error', () => {
    expect(
      describeConfigError({
        kind: 'parse',
        path: '/x',
        message: 'unexpected EOF',
      }),
    ).toMatch(/syntax error/);
  });

  it('formats seed_failed with source string', () => {
    expect(
      describeConfigError({
        kind: 'seed_failed',
        path: '/x',
        source: 'disk full',
      }),
    ).toMatch(/disk full/);
  });

  it('seed_failed without source still produces a usable message', () => {
    expect(describeConfigError({ kind: 'seed_failed', path: '/x' })).toMatch(
      /Couldn’t write defaults/,
    );
  });

  it('falls through to message for unknown kinds', () => {
    expect(describeConfigError({ message: 'arbitrary' })).toBe('arbitrary');
    expect(describeConfigError({ foo: 'bar' })).toMatch(/Couldn’t save\./);
  });
});
