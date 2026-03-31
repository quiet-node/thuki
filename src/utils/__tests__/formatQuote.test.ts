import { describe, it, expect } from 'vitest';
import { formatQuotedText } from '../formatQuote';

describe('formatQuotedText', () => {
  it('returns a single line unchanged', () => {
    expect(formatQuotedText('hello world')).toBe('hello world');
  });

  it('preserves line breaks for multi-line text', () => {
    const input = 'line one\nline two\nline three';
    expect(formatQuotedText(input)).toBe('line one\nline two\nline three');
  });

  it('caps at 4 lines by default and appends ...', () => {
    const input = 'one\ntwo\nthree\nfour\nfive\nsix';
    expect(formatQuotedText(input)).toBe('one\ntwo\nthree\nfour\n...');
  });

  it('respects custom maxLines parameter', () => {
    const input = 'a\nb\nc\nd';
    expect(formatQuotedText(input, 2)).toBe('a\nb\n...');
  });

  it('truncates a long line at maxChars boundary', () => {
    const input = 'a'.repeat(400);
    const result = formatQuotedText(input);
    expect(result).toBe('a'.repeat(300) + '...');
  });

  it('truncates mid-line when cumulative chars exceed maxChars', () => {
    // First line takes 100 chars, second would push over 150 limit
    const input = 'a'.repeat(100) + '\n' + 'b'.repeat(100);
    const result = formatQuotedText(input, 4, 150);
    expect(result).toBe('a'.repeat(100) + '\n' + 'b'.repeat(50) + '...');
  });

  it('skips empty lines without counting them against line limit', () => {
    const input = 'one\n\n\ntwo\n\nthree\nfour\nfive';
    const result = formatQuotedText(input);
    expect(result).toBe('one\ntwo\nthree\nfour\n...');
  });

  it('trims whitespace from each line', () => {
    const input = '  hello  \n  world  ';
    expect(formatQuotedText(input)).toBe('hello\nworld');
  });

  it('handles text that exactly hits the line limit without ...', () => {
    const input = 'one\ntwo\nthree\nfour';
    expect(formatQuotedText(input)).toBe('one\ntwo\nthree\nfour');
  });

  it('handles text that exactly hits the char limit without ...', () => {
    const input = 'a'.repeat(300);
    expect(formatQuotedText(input)).toBe('a'.repeat(300));
  });

  it('returns empty string for all-empty-lines input', () => {
    expect(formatQuotedText('\n\n\n')).toBe('');
  });

  it('returns empty string for whitespace-only input', () => {
    expect(formatQuotedText('   \n   \n   ')).toBe('');
  });
});
