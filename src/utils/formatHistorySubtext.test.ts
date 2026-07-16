import { describe, expect, it } from 'vitest';

import { formatHistorySubtext } from './formatHistorySubtext';

describe('formatHistorySubtext', () => {
  it('returns empty-state copy when count is 0', () => {
    expect(formatHistorySubtext(0, 0)).toBe('No saved chats yet');
  });

  it('uses singular chat for count 1', () => {
    expect(formatHistorySubtext(1, 512)).toBe('1 chat · 512 B on disk');
  });

  it('uses plural chats for count > 1', () => {
    expect(formatHistorySubtext(3, 900)).toBe('3 chats · 900 B on disk');
  });

  it('formats whole kilobytes without a trailing .0', () => {
    expect(formatHistorySubtext(2, 1024)).toBe('2 chats · 1 KB on disk');
  });

  it('formats fractional kilobytes to one decimal', () => {
    expect(formatHistorySubtext(2, 1536)).toBe('2 chats · 1.5 KB on disk');
  });

  it('formats megabytes', () => {
    expect(formatHistorySubtext(5, 1024 * 1024)).toBe('5 chats · 1 MB on disk');
  });

  it('formats fractional megabytes like the traces helper', () => {
    expect(formatHistorySubtext(12, 4404019)).toBe('12 chats · 4.2 MB on disk');
  });

  it('formats gigabytes', () => {
    expect(formatHistorySubtext(1, 1024 * 1024 * 1024)).toBe(
      '1 chat · 1 GB on disk',
    );
    expect(formatHistorySubtext(9, Math.round(2.5 * 1024 * 1024 * 1024))).toBe(
      '9 chats · 2.5 GB on disk',
    );
  });
});
