import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { useConversationHistory } from '../useConversationHistory';
import { invoke } from '../../testUtils/mocks/tauri';
import type { Message } from '../useModel';

const MODEL = 'gemma4:e2b';

const MESSAGES: Message[] = [
  { id: 'u1', role: 'user', content: 'Hello', quotedText: undefined },
  { id: 'a1', role: 'assistant', content: 'Hi there' },
];

describe('useConversationHistory', () => {
  beforeEach(() => {
    invoke.mockReset();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('starts with isSaved false and conversationId null', () => {
    const { result } = renderHook(() => useConversationHistory());
    expect(result.current.isSaved).toBe(false);
    expect(result.current.conversationId).toBeNull();
  });

  it('save() invokes save_conversation with correct payload', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined); // generate_title

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    expect(invoke).toHaveBeenCalledWith('save_conversation', {
      messages: [
        {
          role: 'user',
          content: 'Hello',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          model_name: null,
        },
        {
          role: 'assistant',
          content: 'Hi there',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          model_name: null,
        },
      ],
    });
  });

  it('save() rejects when save_conversation returns no conversation id', async () => {
    invoke.mockResolvedValueOnce(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await expect(
      act(async () => {
        await result.current.save(MESSAGES, MODEL);
      }),
    ).rejects.toThrow(/no conversation id/);
    expect(result.current.isSaved).toBe(false);
  });

  it('save() sets isSaved to true and stores conversationId', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    expect(result.current.isSaved).toBe(true);
    expect(result.current.conversationId).toBe('conv-123');
  });

  it('save() fires generate_title as fire-and-forget after saving', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    expect(invoke).toHaveBeenCalledWith('generate_title', {
      conversationId: 'conv-123',
      messages: [
        {
          role: 'user',
          content: 'Hello',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          model_name: null,
        },
        {
          role: 'assistant',
          content: 'Hi there',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          model_name: null,
        },
      ],
      model: MODEL,
    });
  });

  it('save({ generateTitle: false }) skips generate_title', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-no-title' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL, { generateTitle: false });
    });

    expect(result.current.isSaved).toBe(true);
    expect(result.current.conversationId).toBe('conv-no-title');
    expect(invoke).toHaveBeenCalledWith('save_conversation', expect.anything());
    expect(invoke).not.toHaveBeenCalledWith(
      'generate_title',
      expect.anything(),
    );
  });

  it('requestTitle fires generate_title once then no-ops', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-req' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL, { generateTitle: false });
    });
    invoke.mockClear();
    invoke.mockResolvedValue(undefined);

    act(() => {
      result.current.requestTitle(MESSAGES, MODEL);
    });
    expect(invoke).toHaveBeenCalledWith('generate_title', {
      conversationId: 'conv-req',
      messages: expect.any(Array),
      model: MODEL,
    });

    invoke.mockClear();
    act(() => {
      result.current.requestTitle(MESSAGES, MODEL);
    });
    expect(invoke).not.toHaveBeenCalled();
  });

  it('requestTitle no-ops without identity or model', async () => {
    const { result } = renderHook(() => useConversationHistory());

    act(() => {
      result.current.requestTitle(MESSAGES, MODEL);
    });
    expect(invoke).not.toHaveBeenCalled();

    invoke.mockResolvedValueOnce({ conversation_id: 'conv-x' });
    invoke.mockResolvedValue(undefined);
    await act(async () => {
      await result.current.save(MESSAGES, MODEL, { generateTitle: false });
    });
    invoke.mockClear();

    act(() => {
      result.current.requestTitle(MESSAGES, null);
    });
    expect(invoke).not.toHaveBeenCalled();
  });

  it('save() is a no-op when already saved', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    invoke.mockClear();

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    expect(invoke).not.toHaveBeenCalled();
  });

  it('save() is a no-op when active model is null', async () => {
    // Ollama's /api/tags is the single source of truth: when no model is
    // selected we cannot honestly attribute a conversation, so save() must
    // short-circuit before any IPC call. Mirrors the backend save_conversation
    // contract which also rejects on a null active model.
    invoke.mockResolvedValue(undefined);
    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, null);
    });

    expect(invoke).not.toHaveBeenCalled();
  });

  it('persistTurn() is a no-op when not saved', async () => {
    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.persistTurn(MESSAGES[0], MESSAGES[1]);
    });

    expect(invoke).not.toHaveBeenCalled();
  });

  it('persistAssistant() is a no-op when not saved', async () => {
    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.persistAssistant(MESSAGES[1]);
    });

    expect(invoke).not.toHaveBeenCalled();
  });

  it('persistAssistant() invokes persist_message for assistant only when saved', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save([MESSAGES[0]], MODEL);
    });

    invoke.mockClear();

    const assistantMsg: Message = {
      id: 'a1',
      role: 'assistant',
      content: 'Hi there',
      thinkingContent: 'reason',
      // omit modelName → null on the wire
    };

    await act(async () => {
      await result.current.persistAssistant(assistantMsg);
    });

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('persist_message', {
      conversationId: 'conv-123',
      role: 'assistant',
      content: 'Hi there',
      quotedText: null,
      imagePaths: null,
      thinkingContent: 'reason',
      searchSources: null,
      modelName: null,
    });
  });

  it('persistTurn() invokes persist_message for both messages when saved', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    invoke.mockClear();

    const userMsg: Message = {
      id: 'u2',
      role: 'user',
      content: 'Follow up',
      quotedText: 'ctx',
    };
    const assistantMsg: Message = {
      id: 'a2',
      role: 'assistant',
      content: 'Reply',
    };

    await act(async () => {
      await result.current.persistTurn(userMsg, assistantMsg);
    });

    expect(invoke).toHaveBeenCalledWith('persist_message', {
      conversationId: 'conv-123',
      role: 'user',
      content: 'Follow up',
      quotedText: 'ctx',
      imagePaths: null,
      thinkingContent: null,
      searchSources: null,
      modelName: null,
    });
    expect(invoke).toHaveBeenCalledWith('persist_message', {
      conversationId: 'conv-123',
      role: 'assistant',
      content: 'Reply',
      quotedText: null,
      imagePaths: null,
      thinkingContent: null,
      searchSources: null,
      modelName: null,
    });
  });

  it('save then persistTurn in one chain both hit IPC via conversationIdRef', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-chain' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    const userMsg: Message = {
      id: 'u2',
      role: 'user',
      content: 'Follow up',
    };
    const assistantMsg: Message = {
      id: 'a2',
      role: 'assistant',
      content: 'Reply',
    };

    // Same async chain: after await save, persistTurn must not wait on React state.
    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
      await result.current.persistTurn(userMsg, assistantMsg);
    });

    expect(invoke).toHaveBeenCalledWith(
      'save_conversation',
      expect.objectContaining({ messages: expect.any(Array) }),
    );
    expect(invoke).toHaveBeenCalledWith(
      'persist_message',
      expect.objectContaining({
        conversationId: 'conv-chain',
        role: 'user',
        content: 'Follow up',
      }),
    );
    expect(invoke).toHaveBeenCalledWith(
      'persist_message',
      expect.objectContaining({
        conversationId: 'conv-chain',
        role: 'assistant',
        content: 'Reply',
      }),
    );
    expect(result.current.conversationIdRef.current).toBe('conv-chain');
  });

  it('persistTurn failure keeps conversationId; next turn still persists', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-retry' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    expect(result.current.conversationId).toBe('conv-retry');
    expect(result.current.conversationIdRef.current).toBe('conv-retry');

    invoke.mockClear();
    invoke.mockRejectedValueOnce(new Error('db busy'));

    const userFail: Message = {
      id: 'u-fail',
      role: 'user',
      content: 'lost?',
    };
    const asstFail: Message = {
      id: 'a-fail',
      role: 'assistant',
      content: 'maybe',
    };

    await expect(
      act(async () => {
        await result.current.persistTurn(userFail, asstFail);
      }),
    ).rejects.toThrow(/db busy/);

    // Identity must survive so later turns still attempt persist_message.
    expect(result.current.conversationId).toBe('conv-retry');
    expect(result.current.conversationIdRef.current).toBe('conv-retry');
    expect(result.current.isSaved).toBe(true);

    invoke.mockReset();
    invoke.mockResolvedValue(undefined);

    const userOk: Message = { id: 'u-ok', role: 'user', content: 'retry' };
    const asstOk: Message = {
      id: 'a-ok',
      role: 'assistant',
      content: 'ok',
    };

    await act(async () => {
      await result.current.persistTurn(userOk, asstOk);
    });

    expect(invoke).toHaveBeenCalledWith(
      'persist_message',
      expect.objectContaining({
        conversationId: 'conv-retry',
        content: 'retry',
      }),
    );
    expect(invoke).toHaveBeenCalledWith(
      'persist_message',
      expect.objectContaining({
        conversationId: 'conv-retry',
        content: 'ok',
      }),
    );
  });

  it('persistTurn() passes null for undefined quotedText on userMsg', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-999' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    invoke.mockClear();

    // userMsg has NO quotedText - should map to null
    const userMsg: Message = {
      id: 'u3',
      role: 'user',
      content: 'No context',
      quotedText: undefined,
    };
    const assistantMsg: Message = {
      id: 'a3',
      role: 'assistant',
      content: 'Sure',
      quotedText: 'assistant ctx',
    };

    await act(async () => {
      await result.current.persistTurn(userMsg, assistantMsg);
    });

    expect(invoke).toHaveBeenCalledWith(
      'persist_message',
      expect.objectContaining({
        quotedText: null, // undefined → null
        thinkingContent: null,
      }),
    );
    expect(invoke).toHaveBeenCalledWith(
      'persist_message',
      expect.objectContaining({
        quotedText: 'assistant ctx',
        thinkingContent: null,
      }),
    );
  });

  it('loadConversation() invokes load_conversation and returns mapped Messages', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'm1',
        role: 'user',
        content: 'Saved question',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        created_at: 1,
      },
      {
        id: 'm2',
        role: 'assistant',
        content: 'Saved answer',
        quoted_text: 'ctx',
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        created_at: 2,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());
    let loaded: Message[] = [];

    await act(async () => {
      loaded = await result.current.loadConversation('conv-456');
    });

    expect(invoke).toHaveBeenCalledWith('load_conversation', {
      conversationId: 'conv-456',
    });

    expect(loaded).toEqual([
      {
        id: 'm1',
        role: 'user',
        content: 'Saved question',
        quotedText: undefined,
        modelName: undefined,
      },
      {
        id: 'm2',
        role: 'assistant',
        content: 'Saved answer',
        quotedText: 'ctx',
        modelName: undefined,
      },
    ]);
  });

  it('loadConversation() restores imagePaths from persisted JSON', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'm1',
        role: 'user',
        content: 'Look at this',
        quoted_text: null,
        image_paths: '["/images/a.jpg","/images/b.jpg"]',
        thinking_content: null,
        search_sources: null,
        created_at: 1,
      },
      {
        id: 'm2',
        role: 'assistant',
        content: 'I see',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        created_at: 2,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());
    let loaded: Message[] = [];

    await act(async () => {
      loaded = await result.current.loadConversation('conv-img');
    });

    expect(loaded[0].imagePaths).toEqual(['/images/a.jpg', '/images/b.jpg']);
    expect(loaded[1].imagePaths).toBeUndefined();
  });

  it('loadConversation() sets conversationId to the loaded id', async () => {
    invoke.mockResolvedValueOnce([]);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.loadConversation('conv-789');
    });

    expect(result.current.conversationId).toBe('conv-789');
    expect(result.current.isSaved).toBe(true);
  });

  it('deleteConversation() invokes delete_conversation with correct id', async () => {
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.deleteConversation('conv-123');
    });

    expect(invoke).toHaveBeenCalledWith('delete_conversation', {
      conversationId: 'conv-123',
    });
  });

  it('listConversations() invokes list_conversations without search', async () => {
    invoke.mockResolvedValue([]);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.listConversations();
    });

    expect(invoke).toHaveBeenCalledWith('list_conversations', { search: null });
  });

  it('listConversations() invokes list_conversations with search term', async () => {
    invoke.mockResolvedValue([]);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.listConversations('react');
    });

    expect(invoke).toHaveBeenCalledWith('list_conversations', {
      search: 'react',
    });
  });

  it('persistTurn() passes thinkingContent for assistant messages', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    invoke.mockClear();

    const userMsg: Message = {
      id: 'u4',
      role: 'user',
      content: 'Think about this',
    };
    const assistantMsg: Message = {
      id: 'a4',
      role: 'assistant',
      content: 'Here is my answer',
      thinkingContent: 'Let me reason step by step',
    };

    await act(async () => {
      await result.current.persistTurn(userMsg, assistantMsg);
    });

    expect(invoke).toHaveBeenCalledWith('persist_message', {
      conversationId: 'conv-123',
      role: 'assistant',
      content: 'Here is my answer',
      quotedText: null,
      imagePaths: null,
      thinkingContent: 'Let me reason step by step',
      searchSources: null,
      modelName: null,
    });
  });

  it('save() includes thinking_content in payload', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-think' });
    invoke.mockResolvedValue(undefined);

    const messagesWithThinking: Message[] = [
      { id: 'u1', role: 'user', content: 'Think hard' },
      {
        id: 'a1',
        role: 'assistant',
        content: 'Answer',
        thinkingContent: 'Deep thoughts',
      },
    ];

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(messagesWithThinking, MODEL);
    });

    expect(invoke).toHaveBeenCalledWith('save_conversation', {
      messages: [
        {
          role: 'user',
          content: 'Think hard',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          model_name: null,
        },
        {
          role: 'assistant',
          content: 'Answer',
          quoted_text: null,
          image_paths: null,
          thinking_content: 'Deep thoughts',
          search_sources: null,
          model_name: null,
        },
      ],
    });
  });

  it('loadConversation() restores thinkingContent from persisted data', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'm1',
        role: 'user',
        content: 'Question',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        created_at: 1,
      },
      {
        id: 'm2',
        role: 'assistant',
        content: 'Answer',
        quoted_text: null,
        image_paths: null,
        thinking_content: 'I thought about it',
        search_sources: null,
        created_at: 2,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());
    let loaded: Message[] = [];

    await act(async () => {
      loaded = await result.current.loadConversation('conv-think');
    });

    expect(loaded[0].thinkingContent).toBeUndefined();
    expect(loaded[1].thinkingContent).toBe('I thought about it');
  });

  it('loadConversation() restores searchSources + fromSearch on assistant messages', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'u1',
        role: 'user',
        content: '/search rust',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        created_at: 1,
      },
      {
        id: 'a1',
        role: 'assistant',
        content: 'Rust is a systems language.',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources:
          '[{"title":"Rust Docs","url":"https://doc.rust-lang.org"},{"title":"Tokio","url":"https://tokio.rs"}]',
        created_at: 2,
      },
      {
        id: 'a2',
        role: 'assistant',
        content: 'No sources here.',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: '[]',
        created_at: 3,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());
    let loaded: Message[] = [];

    await act(async () => {
      loaded = await result.current.loadConversation('conv-search');
    });

    // User message: no sources.
    expect(loaded[0].searchSources).toBeUndefined();
    expect(loaded[0].fromSearch).toBeUndefined();
    // Assistant with real sources: sources parsed, fromSearch flagged.
    expect(loaded[1].searchSources).toHaveLength(2);
    expect(loaded[1].searchSources?.[0].url).toBe('https://doc.rust-lang.org');
    expect(loaded[1].fromSearch).toBe(true);
    // Assistant with empty sources array: treated as no sources.
    expect(loaded[2].searchSources).toBeUndefined();
    expect(loaded[2].fromSearch).toBeUndefined();
  });

  it('persistTurn() sends searchSources on assistant messages', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-src' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });
    invoke.mockClear();

    const userMsg: Message = {
      id: 'u5',
      role: 'user',
      content: 'follow',
    };
    const assistantMsg: Message = {
      id: 'a5',
      role: 'assistant',
      content: 'result [1]',
      searchSources: [{ title: 'Doc', url: 'https://doc.example' }],
    };

    await act(async () => {
      await result.current.persistTurn(userMsg, assistantMsg);
    });

    expect(invoke).toHaveBeenCalledWith(
      'persist_message',
      expect.objectContaining({
        role: 'assistant',
        searchSources: [{ title: 'Doc', url: 'https://doc.example' }],
      }),
    );
  });

  it('reset() clears conversationId and isSaved', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });
    expect(result.current.isSaved).toBe(true);

    act(() => {
      result.current.reset();
    });

    expect(result.current.isSaved).toBe(false);
    expect(result.current.conversationId).toBeNull();
  });

  it('unsave() calls delete_conversation and clears isSaved', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });
    expect(result.current.isSaved).toBe(true);

    invoke.mockClear();

    await act(async () => {
      await result.current.unsave();
    });

    expect(invoke).toHaveBeenCalledWith('delete_conversation', {
      conversationId: 'conv-123',
    });
    expect(result.current.isSaved).toBe(false);
    expect(result.current.conversationId).toBeNull();
  });

  it('unsave() is a no-op when not saved', async () => {
    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.unsave();
    });

    expect(invoke).not.toHaveBeenCalled();
  });

  it('reset() does not call reset_conversation (caller is responsible)', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    invoke.mockClear();

    act(() => {
      result.current.reset();
    });

    expect(invoke).not.toHaveBeenCalledWith(
      'reset_conversation',
      expect.anything(),
    );
  });

  // ─── model_name round trip ───────────────────────────────────────────────────

  it('save() stamps model_name on payloads when Message has modelName', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-model-save' });
    invoke.mockResolvedValue(undefined);

    const messagesWithModel: Message[] = [
      { id: 'u1', role: 'user', content: 'Hi' },
      {
        id: 'a1',
        role: 'assistant',
        content: 'Hello',
        modelName: 'gemma4:e2b',
      },
    ];

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(messagesWithModel, MODEL);
    });

    expect(invoke).toHaveBeenCalledWith(
      'save_conversation',
      expect.objectContaining({
        messages: [
          expect.objectContaining({ role: 'user', model_name: null }),
          expect.objectContaining({
            role: 'assistant',
            model_name: 'gemma4:e2b',
          }),
        ],
      }),
    );
  });

  it('persistTurn() sends modelName for assistant, null for user', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-model-persist' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });
    invoke.mockClear();

    const userMsg: Message = { id: 'u-m', role: 'user', content: 'q' };
    const assistantMsg: Message = {
      id: 'a-m',
      role: 'assistant',
      content: 'answer',
      modelName: 'qwen2.5:7b',
    };

    await act(async () => {
      await result.current.persistTurn(userMsg, assistantMsg);
    });

    expect(invoke).toHaveBeenCalledWith(
      'persist_message',
      expect.objectContaining({
        role: 'user',
        modelName: null,
      }),
    );
    expect(invoke).toHaveBeenCalledWith(
      'persist_message',
      expect.objectContaining({
        role: 'assistant',
        modelName: 'qwen2.5:7b',
      }),
    );
  });

  it('loadConversation() maps model_name back to modelName on restore', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'u1',
        role: 'user',
        content: 'Hi',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        model_name: null,
        created_at: 1,
      },
      {
        id: 'a1',
        role: 'assistant',
        content: 'Hello',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        model_name: 'gemma4:e2b',
        created_at: 2,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());
    let loaded: Message[] = [];

    await act(async () => {
      loaded = await result.current.loadConversation('conv-model-load');
    });

    expect(loaded[0].modelName).toBeUndefined();
    expect(loaded[1].modelName).toBe('gemma4:e2b');
  });
});
