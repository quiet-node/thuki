import { describe, expect, it } from 'vitest';

import { formatTracesSubtext } from './formatTracesSubtext';

describe('formatTracesSubtext', () => {
  it('returns the honest empty-state string when there are no traces', () => {
    expect(formatTracesSubtext(0, 0)).toBe('No traces recorded yet');
  });

  it('uses the singular noun for exactly one trace', () => {
    expect(formatTracesSubtext(1, 512)).toBe('1 trace · 512 B on disk');
  });

  it('pluralizes and renders bytes under a kilobyte', () => {
    expect(formatTracesSubtext(3, 900)).toBe('3 traces · 900 B on disk');
  });

  it('formats the exact kilobyte boundary without a trailing decimal', () => {
    expect(formatTracesSubtext(2, 1024)).toBe('2 traces · 1 KB on disk');
  });

  it('formats kilobytes with one decimal place', () => {
    expect(formatTracesSubtext(2, 1536)).toBe('2 traces · 1.5 KB on disk');
  });

  it('formats the exact megabyte boundary without a trailing decimal', () => {
    expect(formatTracesSubtext(5, 1024 * 1024)).toBe('5 traces · 1 MB on disk');
  });

  it('formats megabytes with one decimal place', () => {
    expect(formatTracesSubtext(12, 4404019)).toBe('12 traces · 4.2 MB on disk');
  });

  it('formats gigabytes at and above the boundary', () => {
    expect(formatTracesSubtext(1, 1024 * 1024 * 1024)).toBe(
      '1 trace · 1 GB on disk',
    );
    expect(formatTracesSubtext(9, Math.round(2.5 * 1024 * 1024 * 1024))).toBe(
      '9 traces · 2.5 GB on disk',
    );
  });
});
