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
});
