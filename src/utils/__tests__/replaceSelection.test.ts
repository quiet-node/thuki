import { describe, it, expect, beforeEach } from 'vitest';
import { replaceSelection, shouldAutoReplace } from '../replaceSelection';
import { invoke } from '../../testUtils/mocks/tauri';

describe('replaceSelection', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('invokes the backend with the text and returns true on "replaced"', async () => {
    invoke.mockResolvedValueOnce('replaced');
    const ok = await replaceSelection('hello');
    expect(ok).toBe(true);
    expect(invoke).toHaveBeenCalledWith('replace_selection', { text: 'hello' });
  });

  it('returns false when the backend reports "skipped"', async () => {
    invoke.mockResolvedValueOnce('skipped');
    expect(await replaceSelection('x')).toBe(false);
  });

  it('returns false when the IPC call rejects', async () => {
    invoke.mockRejectedValueOnce(new Error('no access'));
    expect(await replaceSelection('x')).toBe(false);
  });
});

describe('shouldAutoReplace', () => {
  const assistant = { replaceCommand: '/rewrite', content: 'rewritten' };
  const user = { quotedText: 'original' };

  it('is true when the setting is on, the command is replaceable, there is content, and a selection', () => {
    expect(shouldAutoReplace(true, assistant, user)).toBe(true);
  });

  it('is false when the setting is off', () => {
    expect(shouldAutoReplace(false, assistant, user)).toBe(false);
  });

  it('is false when the turn was not a replaceable command', () => {
    expect(shouldAutoReplace(true, { content: 'rewritten' }, user)).toBe(false);
  });

  it('is false when there is no content', () => {
    expect(
      shouldAutoReplace(
        true,
        { replaceCommand: '/rewrite', content: '' },
        user,
      ),
    ).toBe(false);
  });

  it('is false when there was no selection to replace', () => {
    expect(shouldAutoReplace(true, assistant, {})).toBe(false);
  });
});
