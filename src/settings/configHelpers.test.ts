import { describe, it, expect } from 'vitest';

import { configHelp } from './configHelpers';

describe('configHelp', () => {
  it('returns the doc-mirrored helper string for every section', () => {
    // One probe per section is enough to exercise the typed lookup branches.
    expect(configHelp('inference', 'ollama_base_url')).toMatch(/Ollama server/);
    expect(configHelp('prompt', 'system')).toMatch(/custom personality/);
    expect(configHelp('window', 'overlay_width')).toMatch(/in pixels/);
    expect(configHelp('quote', 'max_display_lines')).toMatch(
      /lines of the quoted/,
    );
    expect(configHelp('behavior', 'auto_replace')).toMatch(/rewrite/);
    expect(configHelp('debug', 'trace_enabled')).toMatch(/JSONL trace/);
  });

  it('returns a non-empty string for every documented field', () => {
    const fields: Array<[Parameters<typeof configHelp>[0], string]> = [
      ['inference', 'ollama_base_url'],
      ['prompt', 'system'],
      ['window', 'overlay_width'],
      ['window', 'max_chat_height'],
      ['window', 'max_images'],
      ['quote', 'max_display_lines'],
      ['quote', 'max_display_chars'],
      ['quote', 'max_context_length'],
      ['behavior', 'auto_replace'],
      ['behavior', 'auto_close'],
      ['debug', 'trace_enabled'],
    ];
    for (const [section, key] of fields) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const value = configHelp(section as any, key as any);
      expect(typeof value).toBe('string');
      expect(value.length).toBeGreaterThan(20);
    }
  });
});
