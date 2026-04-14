import { describe, it, expect } from 'vitest';
import { COMMANDS } from '../commands';
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
