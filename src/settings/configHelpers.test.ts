import { describe, it, expect } from 'vitest';

import { configHelp } from './configHelpers';

describe('configHelp', () => {
  it('returns the doc-mirrored helper string for every section', () => {
    // One probe per section is enough to exercise the typed lookup branches.
    expect(configHelp('model', 'available')).toMatch(/list of Ollama models/);
    expect(configHelp('prompt', 'system')).toMatch(/custom personality/);
    expect(configHelp('window', 'overlay_width')).toMatch(/in pixels/);
    expect(configHelp('quote', 'max_display_lines')).toMatch(
      /lines of the quoted/,
    );
    expect(configHelp('search', 'searxng_url')).toMatch(/SearXNG/);
  });

  it('returns a non-empty string for every documented field', () => {
    const fields: Array<[Parameters<typeof configHelp>[0], string]> = [
      ['model', 'available'],
      ['model', 'ollama_url'],
      ['prompt', 'system'],
      ['window', 'overlay_width'],
      ['window', 'collapsed_height'],
      ['window', 'max_chat_height'],
      ['window', 'hide_commit_delay_ms'],
      ['quote', 'max_display_lines'],
      ['quote', 'max_display_chars'],
      ['quote', 'max_context_length'],
      ['search', 'searxng_url'],
      ['search', 'reader_url'],
      ['search', 'searxng_max_results'],
      ['search', 'max_iterations'],
      ['search', 'top_k_urls'],
      ['search', 'search_timeout_s'],
      ['search', 'reader_per_url_timeout_s'],
      ['search', 'reader_batch_timeout_s'],
      ['search', 'judge_timeout_s'],
      ['search', 'router_timeout_s'],
    ];
    for (const [section, key] of fields) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const value = configHelp(section as any, key as any);
      expect(typeof value).toBe('string');
      expect(value.length).toBeGreaterThan(20);
    }
  });
});
