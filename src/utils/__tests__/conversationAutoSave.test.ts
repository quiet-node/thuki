import { describe, it, expect, vi } from 'vitest';
import type { Message } from '../../hooks/useModel';
import {
  createConversationOnSubmit,
  messagesForCreateSave,
} from '../conversationAutoSave';

const user = (id: string, content = 'hi'): Message => ({
  id,
  role: 'user',
  content,
});

const assistant = (id: string, overrides: Partial<Message> = {}): Message => ({
  id,
  role: 'assistant',
  content: '',
  ...overrides,
});

describe('messagesForCreateSave', () => {
  it('keeps users and drops empty streaming assistants', () => {
    const msgs = [user('u1'), assistant('a1')];
    expect(messagesForCreateSave(msgs).map((m) => m.id)).toEqual(['u1']);
  });

  it('keeps assistants with content, thinking, or errorKind', () => {
    const msgs = [
      user('u1'),
      assistant('a1', { content: 'done' }),
      user('u2'),
      assistant('a2', { thinkingContent: 'reason' }),
      user('u3'),
      assistant('a3', { errorKind: 'Other', content: '' }),
      assistant('a4'),
    ];
    expect(messagesForCreateSave(msgs).map((m) => m.id)).toEqual([
      'u1',
      'a1',
      'u2',
      'a2',
      'u3',
      'a3',
    ]);
  });
});

describe('createConversationOnSubmit', () => {
  function makeDeps(
    overrides: Partial<Parameters<typeof createConversationOnSubmit>[0]> = {},
  ) {
    return {
      getConversationId: vi.fn(() => null as string | null),
      isAutoSaveOn: vi.fn(() => true),
      getModel: vi.fn(() => 'gemma4:e2b' as string | null),
      messages: [user('u1', 'question')],
      // Default save is a no-op; tests that need post-create identity override
      // `save` + `getConversationId` together.
      save: vi.fn(async () => {}),
      onUserPersisted: vi.fn(),
      onShowNotice: vi.fn(),
      isNoticeAcked: vi.fn(() => false),
      ...overrides,
    };
  }

  it('no-ops when conversation already has an id', async () => {
    const deps = makeDeps({ getConversationId: vi.fn(() => 'existing') });
    await createConversationOnSubmit(deps);
    expect(deps.save).not.toHaveBeenCalled();
  });

  it('no-ops when auto-save is off', async () => {
    const deps = makeDeps({ isAutoSaveOn: vi.fn(() => false) });
    await createConversationOnSubmit(deps);
    expect(deps.save).not.toHaveBeenCalled();
  });

  it('no-ops when model is null', async () => {
    const deps = makeDeps({ getModel: vi.fn(() => null) });
    await createConversationOnSubmit(deps);
    expect(deps.save).not.toHaveBeenCalled();
  });

  it('no-ops when messages lack a user row', async () => {
    const deps = makeDeps({
      messages: [assistant('a1', { content: 'orphan' })],
    });
    await createConversationOnSubmit(deps);
    expect(deps.save).not.toHaveBeenCalled();
  });

  it('saves without title, records user ids, and shows notice when unacked', async () => {
    let id: string | null = null;
    const deps = makeDeps({
      getConversationId: vi.fn(() => id),
      messages: [user('u1'), assistant('a0', { content: 'prior' }), user('u2')],
      save: vi.fn(async () => {
        id = 'conv-new';
      }),
    });
    await createConversationOnSubmit(deps);
    expect(deps.save).toHaveBeenCalledWith(deps.messages, 'gemma4:e2b', {
      generateTitle: false,
    });
    expect(deps.onUserPersisted).toHaveBeenCalledWith('u1');
    expect(deps.onUserPersisted).toHaveBeenCalledWith('u2');
    expect(deps.onUserPersisted).toHaveBeenCalledTimes(2);
    expect(deps.onShowNotice).toHaveBeenCalledOnce();
  });

  it('skips notice when already acknowledged', async () => {
    let id: string | null = null;
    const deps = makeDeps({
      getConversationId: vi.fn(() => id),
      isNoticeAcked: vi.fn(() => true),
      save: vi.fn(async () => {
        id = 'conv-acked';
      }),
    });
    await createConversationOnSubmit(deps);
    expect(deps.onShowNotice).not.toHaveBeenCalled();
  });

  it('skips onUserPersisted and notice when save leaves identity unset', async () => {
    const deps = makeDeps({
      save: vi.fn(async () => {
        // save no-op: id stays null
      }),
    });
    await createConversationOnSubmit(deps);
    expect(deps.onUserPersisted).not.toHaveBeenCalled();
    expect(deps.onShowNotice).not.toHaveBeenCalled();
  });

  it('propagates save errors to the caller', async () => {
    const deps = makeDeps({
      save: vi.fn(async () => {
        throw new Error('disk full');
      }),
    });
    await expect(createConversationOnSubmit(deps)).rejects.toThrow(/disk full/);
    expect(deps.onUserPersisted).not.toHaveBeenCalled();
    expect(deps.onShowNotice).not.toHaveBeenCalled();
  });
});
