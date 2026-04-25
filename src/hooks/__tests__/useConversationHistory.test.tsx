import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { useConversationHistory } from '../useConversationHistory';
import { invoke } from '../../testUtils/mocks/tauri';
import type { Message } from '../useOllama';

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
      await result.current.save(MESSAGES);
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
          search_metadata: null,
        },
        {
          role: 'assistant',
          content: 'Hi there',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          search_warnings: null,
          search_metadata: null,
        },
      ],
    });
  });

  it('save() sets isSaved to true and stores conversationId', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES);
    });

    expect(result.current.isSaved).toBe(true);
    expect(result.current.conversationId).toBe('conv-123');
  });

  it('save() fires generate_title as fire-and-forget after saving', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES);
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
          search_metadata: null,
        },
        {
          role: 'assistant',
          content: 'Hi there',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          search_warnings: null,
          search_metadata: null,
        },
      ],
    });
  });

  it('save() is a no-op when already saved', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-123' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES);
    });

    invoke.mockClear();

    await act(async () => {
      await result.current.save(MESSAGES);
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
      await result.current.save(MESSAGES);
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
      searchMetadata: null,
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
      searchMetadata: null,
    });
  });

  it('persistTurn() passes null for undefined quotedText on userMsg', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-999' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES);
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
      await result.current.save(MESSAGES);
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
      searchMetadata: null,
    });
  });

  it('save() serialises searchWarnings to JSON in payload', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-warn-save' });
    invoke.mockResolvedValue(undefined);

    const messagesWithWarnings: Message[] = [
      { id: 'u1', role: 'user', content: 'Search something' },
      {
        id: 'a1',
        role: 'assistant',
        content: 'Here are results',
        searchWarnings: ['reader_unavailable'],
      },
    ];

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(messagesWithWarnings);
    });

    expect(invoke).toHaveBeenCalledWith('save_conversation', {
      messages: [
        {
          role: 'user',
          content: 'Search something',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          search_warnings: null,
          search_metadata: null,
        },
        {
          role: 'assistant',
          content: 'Here are results',
          quoted_text: null,
          image_paths: null,
          thinking_content: null,
          search_sources: null,
          search_warnings: JSON.stringify(['reader_unavailable']),
          search_metadata: null,
        },
      ],
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
      await result.current.save(messagesWithThinking);
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
          search_metadata: null,
        },
        {
          role: 'assistant',
          content: 'Answer',
          quoted_text: null,
          image_paths: null,
          thinking_content: 'Deep thoughts',
          search_sources: null,
          search_warnings: null,
          search_metadata: null,
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
      await result.current.save(MESSAGES);
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
      await result.current.save(MESSAGES);
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
      await result.current.save(MESSAGES);
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

  it('persistTurn() serialises searchWarnings to JSON on assistant messages', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-warn' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES);
    });
    invoke.mockClear();

    const userMsg: Message = {
      id: 'u-w',
      role: 'user',
      content: 'q',
    };
    const assistantMsg: Message = {
      id: 'a-w',
      role: 'assistant',
      content: 'answer',
      searchWarnings: ['reader_unavailable'],
    };

    await act(async () => {
      await result.current.persistTurn(userMsg, assistantMsg);
    });

    expect(invoke).toHaveBeenCalledWith('persist_message', {
      conversationId: 'conv-warn',
      role: 'assistant',
      content: 'answer',
      quotedText: null,
      imagePaths: null,
      thinkingContent: null,
      searchSources: null,
      searchWarnings: JSON.stringify(['reader_unavailable']),
      searchMetadata: null,
    });
  });

  it('loadConversation() parses search_warnings back to SearchWarning array', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'u1',
        role: 'user',
        content: 'query',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        created_at: 1,
      },
      {
        id: 'a1',
        role: 'assistant',
        content: 'answer',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: JSON.stringify(['reader_partial_failure']),
        created_at: 2,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());

    let loaded: Message[] = [];
    await act(async () => {
      loaded = await result.current.loadConversation('conv-load-warn');
    });

    expect(loaded[1].searchWarnings).toEqual(['reader_partial_failure']);
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
      await result.current.save(MESSAGES);
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

  it('save() serialises searchMetadata to JSON in payload', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-meta-save' });
    invoke.mockResolvedValue(undefined);

    const metadata = {
      iterations: [
        {
          stage: { kind: 'initial' as const },
          queries: ['q'],
          urls_fetched: ['https://example.com/rust'],
          reader_empty_urls: [],
          judge_verdict: 'sufficient' as const,
          judge_reasoning: 'enough evidence',
          duration_ms: 42,
        },
      ],
      total_duration_ms: 42,
      retries_performed: 0,
    };
    const messagesWithMeta: Message[] = [
      { id: 'u1', role: 'user', content: '/search q' },
      {
        id: 'a1',
        role: 'assistant',
        content: 'Answer',
        searchMetadata: metadata,
      },
    ];

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(messagesWithMeta);
    });

    expect(invoke).toHaveBeenCalledWith(
      'save_conversation',
      expect.objectContaining({
        messages: expect.arrayContaining([
          expect.objectContaining({
            role: 'assistant',
            search_metadata: JSON.stringify(metadata),
          }),
        ]),
      }),
    );
  });

  it('save() falls back to serialising searchTraces when searchMetadata is absent', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-trace-save' });
    invoke.mockResolvedValue(undefined);

    const traces = [
      {
        id: 'round-1-search',
        kind: 'search' as const,
        status: 'completed' as const,
        round: 1,
        title: 'Searching the web',
        summary: 'Found 4 results across 2 sites.',
        queries: ['q'],
      },
    ];
    const messagesWithTraces: Message[] = [
      { id: 'u1', role: 'user', content: '/search q' },
      {
        id: 'a1',
        role: 'assistant',
        content: 'Answer',
        searchTraces: traces,
      },
    ];

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(messagesWithTraces);
    });

    expect(invoke).toHaveBeenCalledWith(
      'save_conversation',
      expect.objectContaining({
        messages: expect.arrayContaining([
          expect.objectContaining({
            role: 'assistant',
            search_metadata: JSON.stringify(traces),
          }),
        ]),
      }),
    );
  });

  it('persistTurn() serialises searchMetadata to JSON on assistant messages', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-meta' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES);
    });
    invoke.mockClear();

    const metadata = {
      iterations: [
        {
          stage: { kind: 'gap_round' as const, round: 1 },
          queries: ['q', 'follow up'],
          urls_fetched: ['https://example.com/a'],
          reader_empty_urls: ['https://example.com/b'],
          judge_verdict: 'partial' as const,
          judge_reasoning: 'still missing one detail',
          duration_ms: 88,
        },
      ],
      total_duration_ms: 130,
      retries_performed: 1,
    };
    const userMsg: Message = { id: 'u-m', role: 'user', content: 'q' };
    const assistantMsg: Message = {
      id: 'a-m',
      role: 'assistant',
      content: 'answer',
      searchMetadata: metadata,
    };

    await act(async () => {
      await result.current.persistTurn(userMsg, assistantMsg);
    });

    expect(invoke).toHaveBeenCalledWith('persist_message', {
      conversationId: 'conv-meta',
      role: 'assistant',
      content: 'answer',
      quotedText: null,
      imagePaths: null,
      thinkingContent: null,
      searchSources: null,
      searchWarnings: null,
      searchMetadata: JSON.stringify(metadata),
    });
  });

  it('persistTurn() falls back to searchTraces when searchMetadata is absent', async () => {
    invoke.mockResolvedValueOnce({ conversation_id: 'conv-trace' });
    invoke.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConversationHistory());

    await act(async () => {
      await result.current.save(MESSAGES);
    });
    invoke.mockClear();

    const traces = [
      {
        id: 'compose',
        kind: 'compose' as const,
        status: 'completed' as const,
        title: 'Synthesizing the answer',
        summary: 'Pulling the strongest points together into a clear answer.',
        counts: { sources: 2 },
      },
    ];
    const userMsg: Message = { id: 'u-t', role: 'user', content: 'q' };
    const assistantMsg: Message = {
      id: 'a-t',
      role: 'assistant',
      content: 'answer',
      searchTraces: traces,
    };

    await act(async () => {
      await result.current.persistTurn(userMsg, assistantMsg);
    });

    expect(invoke).toHaveBeenCalledWith('persist_message', {
      conversationId: 'conv-trace',
      role: 'assistant',
      content: 'answer',
      quotedText: null,
      imagePaths: null,
      thinkingContent: null,
      searchSources: null,
      searchWarnings: null,
      searchMetadata: JSON.stringify(traces),
    });
  });

  it('loadConversation() parses SearchMetadata from search_metadata', async () => {
    const metadata = {
      iterations: [
        {
          stage: { kind: 'initial' as const },
          queries: ['q'],
          urls_fetched: ['https://example.com/a'],
          reader_empty_urls: [],
          judge_verdict: 'sufficient' as const,
          judge_reasoning: 'enough evidence',
          duration_ms: 10,
        },
      ],
      total_duration_ms: 10,
      retries_performed: 0,
    };

    invoke.mockResolvedValueOnce([
      {
        id: 'u1',
        role: 'user',
        content: 'query',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: null,
        created_at: 1,
      },
      {
        id: 'a1',
        role: 'assistant',
        content: 'answer',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: JSON.stringify(metadata),
        created_at: 2,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());

    let loaded: Message[] = [];
    await act(async () => {
      loaded = await result.current.loadConversation('conv-meta-load');
    });

    expect(loaded[1].searchMetadata).toEqual(metadata);
    expect(loaded[1].searchTraces).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ kind: 'search' }),
        expect.objectContaining({ kind: 'read' }),
        expect.objectContaining({ kind: 'chunk_judge', verdict: 'sufficient' }),
      ]),
    );
    expect(loaded[1].fromSearch).toBe(true);
  });

  it('loadConversation() preserves SearchMetadata with empty iterations', async () => {
    const metadata = {
      iterations: [],
      total_duration_ms: 0,
      retries_performed: 0,
    };

    invoke.mockResolvedValueOnce([
      {
        id: 'a-empty-meta',
        role: 'assistant',
        content: 'answer',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: JSON.stringify(metadata),
        created_at: 1,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());

    let loaded: Message[] = [];
    await act(async () => {
      loaded = await result.current.loadConversation('conv-empty-meta');
    });

    expect(loaded[0].searchMetadata).toEqual(metadata);
    expect(loaded[0].searchTraces).toBeUndefined();
    expect(loaded[0].fromSearch).toBe(true);
  });

  it('loadConversation() parses SearchTraceStep[] from search_metadata', async () => {
    const traces = [
      {
        id: 'round-1-search',
        kind: 'search' as const,
        status: 'completed' as const,
        round: 1,
        title: 'Searching the web',
        summary: 'Found 4 results across 2 sites.',
        queries: ['q'],
      },
    ];

    invoke.mockResolvedValueOnce([
      {
        id: 'u1',
        role: 'user',
        content: 'query',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: null,
        created_at: 1,
      },
      {
        id: 'a1',
        role: 'assistant',
        content: 'answer',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: JSON.stringify(traces),
        created_at: 2,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());

    let loaded: Message[] = [];
    await act(async () => {
      loaded = await result.current.loadConversation('conv-meta-load');
    });

    expect(loaded[0].searchTraces).toBeUndefined();
    expect(loaded[1].searchTraces).toEqual(traces);
  });

  it('loadConversation() converts legacy IterationTrace[] into SearchTraceStep[]', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'a1',
        role: 'assistant',
        content: 'answer',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: JSON.stringify([
          {
            stage: { kind: 'initial' },
            queries: ['legacy query'],
            urls_fetched: ['https://example.com/a'],
            reader_empty_urls: [],
            judge_verdict: 'partial',
            judge_reasoning: 'missing details',
            duration_ms: 120,
          },
        ]),
        created_at: 1,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());

    let loaded: Message[] = [];
    await act(async () => {
      loaded = await result.current.loadConversation('conv-meta-legacy');
    });

    expect(loaded[0].searchTraces).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ kind: 'search' }),
        expect.objectContaining({ kind: 'read' }),
        expect.objectContaining({ kind: 'chunk_judge', verdict: 'partial' }),
      ]),
    );
    expect(loaded[0].fromSearch).toBe(true);
  });

  it('loadConversation() ignores invalid search_metadata payloads', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'a-primitive',
        role: 'assistant',
        content: 'primitive metadata',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: JSON.stringify('not an object'),
        created_at: 0,
      },
      {
        id: 'a-invalid-object',
        role: 'assistant',
        content: 'not array metadata',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: JSON.stringify({ not: 'an array' }),
        created_at: 1,
      },
      {
        id: 'a-invalid-item',
        role: 'assistant',
        content: 'invalid item metadata',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: JSON.stringify(['bad']),
        created_at: 2,
      },
      {
        id: 'a-empty-array',
        role: 'assistant',
        content: 'empty metadata',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: JSON.stringify([]),
        created_at: 3,
      },
      {
        id: 'a-malformed',
        role: 'assistant',
        content: 'malformed metadata',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: '{not json',
        created_at: 4,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());

    let loaded: Message[] = [];
    await act(async () => {
      loaded = await result.current.loadConversation('conv-meta-invalid');
    });

    expect(loaded).toHaveLength(5);
    expect(loaded.every((message) => message.searchTraces === undefined)).toBe(
      true,
    );
    expect(
      loaded.every((message) => message.searchMetadata === undefined),
    ).toBe(true);
  });

  it('loadConversation() covers snippet-only and plural legacy trace variants', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'a-legacy-variants',
        role: 'assistant',
        content: 'answer',
        quoted_text: null,
        image_paths: null,
        thinking_content: null,
        search_sources: null,
        search_warnings: null,
        search_metadata: JSON.stringify([
          {
            stage: { kind: 'gap_round', round: 2 },
            queries: [],
            urls_fetched: [],
            reader_empty_urls: [],
            judge_verdict: 'sufficient',
            judge_reasoning: 'enough evidence',
            duration_ms: 90,
          },
          {
            stage: { kind: 'initial' },
            queries: [],
            urls_fetched: ['not-a-url', 'https://b.com/guide'],
            reader_empty_urls: ['not-a-url'],
            judge_verdict: 'insufficient',
            judge_reasoning: 'still missing details',
            duration_ms: 120,
          },
        ]),
        created_at: 1,
      },
    ]);

    const { result } = renderHook(() => useConversationHistory());

    let loaded: Message[] = [];
    await act(async () => {
      loaded = await result.current.loadConversation('conv-meta-variants');
    });

    expect(loaded[0].fromSearch).toBe(true);
    expect(loaded[0].searchTraces).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: 'legacy-round-3-0-search',
          round: 3,
          title: 'Searched the web again',
          summary: 'Loaded a saved search round.',
        }),
        expect.objectContaining({
          id: 'legacy-round-3-0-judge',
          kind: 'snippet_judge',
          title: 'Checked whether the snippets were enough',
          verdict: 'sufficient',
          summary:
            'This saved round had enough evidence to answer confidently.',
        }),
        expect.objectContaining({
          id: 'legacy-round-1-1-read',
          summary: 'Read 2 saved pages.',
          domains: ['not-a-url', 'b.com'],
          counts: expect.objectContaining({ empty: 1 }),
        }),
        expect.objectContaining({
          id: 'legacy-round-1-1-judge',
          kind: 'chunk_judge',
          verdict: 'insufficient',
          summary: 'This saved round did not gather enough evidence yet.',
        }),
      ]),
    );
  });
});
