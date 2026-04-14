import { describe, it, expect } from 'vitest';
import { COMMANDS, buildPrompt } from '../commands';
import type { Command } from '../commands';

describe('COMMANDS registry', () => {
  it('is non-empty', () => {
    expect(COMMANDS.length).toBeGreaterThan(0);
  });

  it('every entry has non-empty trigger, label, and description', () => {
    for (const cmd of COMMANDS) {
      expect(typeof cmd.trigger).toBe('string');
      expect(cmd.trigger.length).toBeGreaterThan(0);

      expect(typeof cmd.label).toBe('string');
      expect(cmd.label.length).toBeGreaterThan(0);

      expect(typeof cmd.description).toBe('string');
      expect(cmd.description.length).toBeGreaterThan(0);
    }
  });

  it('all triggers start with "/"', () => {
    for (const cmd of COMMANDS) {
      expect(cmd.trigger.startsWith('/')).toBe(true);
    }
  });

  it('no duplicate triggers', () => {
    const triggers = COMMANDS.map((c: Command) => c.trigger);
    const unique = new Set(triggers);
    expect(unique.size).toBe(triggers.length);
  });

  it('includes the /screen command', () => {
    const screen = COMMANDS.find((c: Command) => c.trigger === '/screen');
    expect(screen).toBeDefined();
    expect(screen?.label).toBe('/screen');
    expect(screen?.description.length).toBeGreaterThan(0);
  });

  it('includes the /think command', () => {
    const think = COMMANDS.find((c: Command) => c.trigger === '/think');
    expect(think).toBeDefined();
    expect(think?.label).toBe('/think');
    expect(think?.description.length).toBeGreaterThan(0);
  });

  it('includes the /translate command', () => {
    const cmd = COMMANDS.find((c: Command) => c.trigger === '/translate');
    expect(cmd).toBeDefined();
    expect(cmd?.label).toBe('/translate');
    expect(cmd?.description.length).toBeGreaterThan(0);
  });

  it('includes the /rewrite command', () => {
    const cmd = COMMANDS.find((c: Command) => c.trigger === '/rewrite');
    expect(cmd).toBeDefined();
    expect(cmd?.label).toBe('/rewrite');
    expect(cmd?.description.length).toBeGreaterThan(0);
  });

  it('includes the /tldr command', () => {
    const cmd = COMMANDS.find((c: Command) => c.trigger === '/tldr');
    expect(cmd).toBeDefined();
    expect(cmd?.label).toBe('/tldr');
    expect(cmd?.description.length).toBeGreaterThan(0);
  });

  it('includes the /refine command', () => {
    const cmd = COMMANDS.find((c: Command) => c.trigger === '/refine');
    expect(cmd).toBeDefined();
    expect(cmd?.label).toBe('/refine');
    expect(cmd?.description.length).toBeGreaterThan(0);
  });

  it('includes the /bullets command', () => {
    const cmd = COMMANDS.find((c: Command) => c.trigger === '/bullets');
    expect(cmd).toBeDefined();
    expect(cmd?.label).toBe('/bullets');
    expect(cmd?.description.length).toBeGreaterThan(0);
  });

  it('includes the /action command', () => {
    const cmd = COMMANDS.find((c: Command) => c.trigger === '/action');
    expect(cmd).toBeDefined();
    expect(cmd?.label).toBe('/action');
    expect(cmd?.description.length).toBeGreaterThan(0);
  });

  it('all commands with promptTemplate have $INPUT placeholder', () => {
    for (const cmd of COMMANDS) {
      if (cmd.promptTemplate) {
        expect(cmd.promptTemplate).toContain('$INPUT');
      }
    }
  });

  it('/translate command template contains $LANG placeholder', () => {
    const cmd = COMMANDS.find((c: Command) => c.trigger === '/translate');
    expect(cmd?.promptTemplate).toContain('$LANG');
  });

  it('/screen and /think have no promptTemplate', () => {
    const screen = COMMANDS.find((c: Command) => c.trigger === '/screen');
    const think = COMMANDS.find((c: Command) => c.trigger === '/think');
    expect(screen?.promptTemplate).toBeUndefined();
    expect(think?.promptTemplate).toBeUndefined();
  });
});

describe('buildPrompt', () => {
  it('returns null for commands without a promptTemplate', () => {
    expect(buildPrompt('/screen', 'hello')).toBeNull();
    expect(buildPrompt('/think', 'hello')).toBeNull();
  });

  it('returns null for unknown triggers', () => {
    expect(buildPrompt('/nonexistent', 'hello')).toBeNull();
  });

  it('uses typed text as $INPUT when no selected text', () => {
    const result = buildPrompt('/rewrite', 'fix this please');
    expect(result).toContain('fix this please');
    expect(result).not.toContain('$INPUT');
  });

  it('uses selected text as $INPUT when typed text is empty', () => {
    const result = buildPrompt('/rewrite', '', 'selected paragraph');
    expect(result).toContain('selected paragraph');
    expect(result).not.toContain('$INPUT');
  });

  it('uses selected text as $INPUT and appends typed text as instruction when both present', () => {
    const result = buildPrompt('/rewrite', 'make it shorter', 'selected paragraph');
    expect(result).toContain('selected paragraph');
    expect(result).toContain('make it shorter');
  });

  it('returns null when both typed text and selected text are empty', () => {
    expect(buildPrompt('/rewrite', '')).toBeNull();
    expect(buildPrompt('/rewrite', '', '')).toBeNull();
    expect(buildPrompt('/rewrite', '  ', '  ')).toBeNull();
  });

  it('/translate parses full language name from typed text', () => {
    const result = buildPrompt('/translate', 'Vietnamese hello world');
    expect(result).toContain('Target language: Vietnamese');
    expect(result).toContain('Text: hello world');
  });

  it('/translate parses short code from typed text', () => {
    const result = buildPrompt('/translate', 'jpn this is a test');
    expect(result).toContain('Target language: jpn');
    expect(result).toContain('Text: this is a test');
  });

  it('/translate with only language code and selected text uses selected text as input', () => {
    const result = buildPrompt('/translate', 'vie', 'selected text here');
    expect(result).toContain('Target language: vie');
    expect(result).toContain('Text: selected text here');
  });

  it('/translate with no language and selected text leaves $LANG empty', () => {
    const result = buildPrompt('/translate', '', 'translate me');
    expect(result).toContain('Target language: ');
    expect(result).toContain('Text: translate me');
    expect(result).not.toContain('$LANG');
  });

  it('/translate with only a language code and no selected text returns null', () => {
    expect(buildPrompt('/translate', 'vie')).toBeNull();
  });

  it('/tldr populates template correctly', () => {
    const result = buildPrompt('/tldr', 'a very long text here');
    expect(result).toContain('Summarize the following text');
    expect(result).toContain('Text: a very long text here');
  });

  it('/refine populates template correctly', () => {
    const result = buildPrompt('/refine', 'she dont goes there');
    expect(result).toContain('Refine the following text');
    expect(result).toContain('Text: she dont goes there');
  });

  it('/bullets populates template correctly', () => {
    const result = buildPrompt('/bullets', 'point one and point two');
    expect(result).toContain('Extract the key points');
    expect(result).toContain('Text: point one and point two');
  });

  it('/action populates template correctly', () => {
    const result = buildPrompt('/action', 'John should fix the bug by Friday');
    expect(result).toContain('Extract every action item');
    expect(result).toContain('Text: John should fix the bug by Friday');
  });
});
