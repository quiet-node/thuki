import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { useConversationHistory } from '../useConversationHistory';
import { invoke } from '../../testUtils/mocks/tauri';
import type { Message } from '../useOllama';

const MESSAGES: Message[] = [
  { id: 'u1', role: 'user', content: 'Hello', quotedText: undefined },
  { id: 'a1', role: 'assistant', content: 'Hi there' },
];

const MODEL = 'gemma4:e2b';

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
          search_warnings: null,
        },
        {
          role: 'assistant',
          content: 'Hi there',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          search_warnings: null,
        },
      ],
      model: MODEL,
    });
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
          search_warnings: null,
        },
        {
          role: 'assistant',
          content: 'Hi there',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          search_warnings: null,
        },
      ],
    });
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

  it('persistTurn() is a no-op when not saved', async () => {
    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.persistTurn(MESSAGES[0], MESSAGES[1]);
    });

    expect(invoke).not.toHaveBeenCalled();
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
      searchWarnings: null,
    });
    expect(invoke).toHaveBeenCalledWith('persist_message', {
      conversationId: 'conv-123',
      role: 'assistant',
      content: 'Reply',
      quotedText: null,
      imagePaths: null,
      thinkingContent: null,
      searchSources: null,
      searchWarnings: null,
    });
  });

  it('persistTurn() passes null for undefined quotedText on userMsg', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-999' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES, MODEL);
    });

    invoke.mockClear();

    // userMsg has NO quotedText — should map to null
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
      },
      {
        id: 'm2',
        role: 'assistant',
        content: 'Saved answer',
        quotedText: 'ctx',
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
      searchWarnings: null,
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
          search_warnings: null,
        },
        {
          role: 'assistant',
          content: 'Answer',
          quoted_text: null,
          image_paths: null,
          thinking_content: 'Deep thoughts',
          search_sources: null,
          search_warnings: null,
        },
      ],
      model: MODEL,
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
});
