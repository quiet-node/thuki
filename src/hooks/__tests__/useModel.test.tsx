import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { ignoreTraceIpcError, useModel } from '../useModel';
import {
  invoke,
  enableChannelCapture,
  getLastChannel,
  resetChannelCapture,
} from '../../testUtils/mocks/tauri';

// Wrapper around getLastChannel() for clarity: reads the captured channel
// that was set by enableChannelCapture when invoke() is called with onEvent.
function getChannel() {
  return getLastChannel();
}

describe('ignoreTraceIpcError', () => {
  it('returns void without throwing when invoked as a Promise.catch handler', () => {
    // Shared handler used for fire-and-forget record_conversation_end
    // IPC calls. Production calls
    // invoke('record_conversation_end').catch(ignoreTraceIpcError); the
    // unit-test path here exercises the swallow contract directly so
    // coverage hits the handler exactly once.
    expect(() => ignoreTraceIpcError()).not.toThrow();
    expect(ignoreTraceIpcError()).toBeUndefined();
  });
});

describe('useModel', () => {
  beforeEach(() => {
    invoke.mockClear();
    enableChannelCapture();
    resetChannelCapture();
  });

  // ─── ask() ──────────────────────────────────────────────────────────────────

  describe('ask()', () => {
    it('sends message via invoke with correct command name and args', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello world');
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'hello world',
          quotedText: null,
        }),
      );
    });

    it('sets isGenerating to true during generation', async () => {
      // Prevent invoke from resolving immediately so we can observe mid-flight state.
      // We capture the channel then stall invoke indefinitely.
      let resolveInvoke!: () => void;
      invoke.mockImplementationOnce(
        async (_cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            // Stall - never resolves until we manually resolve
            return new Promise<void>((res) => {
              resolveInvoke = res;
            });
          }
        },
      );

      const { result } = renderHook(() => useModel(''));

      // Start ask but don't await so we can read state while in-flight
      act(() => {
        void result.current.ask('test prompt');
      });

      // isGenerating should be true right after ask sets it
      expect(result.current.isGenerating).toBe(true);

      // Cleanup
      act(() => {
        resolveInvoke?.();
      });
    });

    it('adds user message and empty assistant placeholder immediately on ask', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('my question');
      });

      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0]).toEqual(
        expect.objectContaining({
          role: 'user',
          content: 'my question',
        }),
      );
      expect(result.current.messages[0].id).toEqual(expect.any(String));
      expect(result.current.messages[1]).toEqual(
        expect.objectContaining({
          role: 'assistant',
          content: '',
        }),
      );
    });

    it('stores quotedText on user message when provided', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('what is this?', 'code snippet');
      });

      expect(result.current.messages[0]).toEqual(
        expect.objectContaining({
          role: 'user',
          content: 'what is this?',
          quotedText: 'code snippet',
        }),
      );
    });

    it('sends quotedText to invoke when provided', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('summarize', 'selected text');
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'summarize',
          quotedText: 'selected text',
        }),
      );
    });

    it('accumulates streaming tokens into the assistant message', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Hello' });
        channel!.simulateMessage({ type: 'Token', data: ', world' });
      });

      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg?.content).toBe('Hello, world');
    });

    it('keeps assistant message in place on Done chunk', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Hi there' });
        channel!.simulateMessage({ type: 'Done' });
      });

      expect(result.current.isGenerating).toBe(false);
      expect(result.current.messages).toContainEqual(
        expect.objectContaining({
          role: 'assistant',
          content: 'Hi there',
        }),
      );
    });

    it('does nothing for empty prompt', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('');
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('does nothing for whitespace-only prompt', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('   ');
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('does nothing when already generating', async () => {
      let resolveInvoke!: () => void;
      invoke.mockImplementationOnce(
        async (_cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            return new Promise<void>((res) => {
              resolveInvoke = res;
            });
          }
        },
      );

      const { result } = renderHook(() => useModel(''));

      // Start the first ask (stalls)
      act(() => {
        void result.current.ask('first');
      });

      expect(result.current.isGenerating).toBe(true);
      const callCountAfterFirst = invoke.mock.calls.length;

      // Try a second ask while generating
      await act(async () => {
        await result.current.ask('second');
      });

      // invoke should NOT have been called again
      expect(invoke.mock.calls.length).toBe(callCountAfterFirst);

      // Cleanup
      act(() => {
        resolveInvoke?.();
      });
    });

    it('sends promptOverride as message to backend when provided', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask(
          'user visible text',
          undefined,
          undefined,
          false,
          'composed prompt for model',
        );
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'composed prompt for model',
        }),
      );

      // User message in state shows displayContent, not the override.
      const userMsg = result.current.messages.find((m) => m.role === 'user');
      expect(userMsg?.content).toBe('user visible text');
    });

    it('sends utility slashCommand so backend can skip auto-search', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask(
          '/rewrite fix this text',
          undefined,
          undefined,
          false,
          'composed rewrite prompt',
          undefined,
          '/rewrite',
          undefined,
          '/rewrite',
        );
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          slashCommand: '/rewrite',
          forceSearch: false,
          think: false,
        }),
      );
    });

    it('utility slashCommand wins over think for IPC slashCommand', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask(
          '/think /tldr long text',
          undefined,
          undefined,
          true,
          'composed tldr prompt',
          undefined,
          undefined,
          undefined,
          '/tldr',
        );
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          slashCommand: '/tldr',
          think: true,
          forceSearch: false,
        }),
      );
    });

    it('sends /think as slashCommand when only think is set', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello', undefined, undefined, true);
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          slashCommand: '/think',
          think: true,
          forceSearch: false,
        }),
      );
    });

    it('does not send a skip slashCommand for /explain (auto-search still applies)', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask(
          '/explain JWT',
          undefined,
          undefined,
          false,
          'composed explain prompt',
          undefined,
          undefined,
          undefined,
          '/explain',
        );
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          slashCommand: '/explain',
          forceSearch: false,
        }),
      );
    });

    it('sends displayContent as message when no promptOverride provided', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello world');
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'hello world',
        }),
      );
    });

    it('sends displayContent when promptOverride is undefined', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask(
          'hello world',
          undefined,
          undefined,
          false,
          undefined,
        );
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'hello world',
        }),
      );
    });
  });

  // ─── auto-search chunks ──────────────────────────────────────────────────────

  describe('auto-search chunks', () => {
    it('maps SearchStatus phases to the shared stage indicator', async () => {
      const { result } = renderHook(() => useModel(''));
      await act(async () => {
        await result.current.ask('what is the news today');
      });
      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'deciding' },
        });
      });
      expect(result.current.searchStage).toEqual({ kind: 'analyzing_query' });
      // Variant B: bubble owns progress chrome from the first status event.
      const assistantAfterStatus = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantAfterStatus?.fromSearch).toBe(true);

      act(() => {
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'searching' },
        });
      });
      expect(result.current.searchStage).toEqual({ kind: 'searching' });

      act(() => {
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'reading' },
        });
      });
      expect(result.current.searchStage).toEqual({ kind: 'reading_sources' });

      act(() => {
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'verifying' },
        });
      });
      expect(result.current.searchStage).toEqual({
        kind: 'verifying_sources',
      });
    });

    it('clears decide-only search chrome when the model streams a plain answer', async () => {
      // Auto-search emits deciding, then NoSearch → plain stream. Phantom
      // fromSearch + analyzing_query would paint bare "Sources" after reasoning.
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('m', onTurnComplete));
      await act(async () => {
        await result.current.ask('hello, hi');
      });
      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'deciding' },
        });
      });
      expect(result.current.searchStage).toEqual({ kind: 'analyzing_query' });
      expect(
        result.current.messages.find((m) => m.role === 'assistant')?.fromSearch,
      ).toBe(true);

      act(() => {
        channel!.simulateMessage({
          type: 'ThinkingToken',
          data: 'User is greeting me.',
        });
      });
      expect(result.current.searchStage).toBeNull();
      expect(
        result.current.messages.find((m) => m.role === 'assistant')?.fromSearch,
      ).toBeUndefined();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Hello!' });
        channel!.simulateMessage({ type: 'Done' });
      });
      expect(onTurnComplete).toHaveBeenCalledWith(
        expect.objectContaining({ role: 'user' }),
        expect.objectContaining({
          content: 'Hello!',
          thinkingContent: 'User is greeting me.',
          fromSearch: undefined,
        }),
      );
    });

    it('keeps fromSearch when retrieval advances past deciding before tokens', async () => {
      const { result } = renderHook(() => useModel('m'));
      await act(async () => {
        await result.current.ask('latest news');
      });
      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'deciding' },
        });
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'searching' },
        });
        channel!.simulateMessage({
          type: 'SearchSources',
          data: [{ index: 1, url: 'https://a/', title: 'A' }],
        });
        channel!.simulateMessage({ type: 'Token', data: 'Headlines...' });
      });
      expect(result.current.searchStage).toEqual({ kind: 'searching' });
      expect(
        result.current.messages.find((m) => m.role === 'assistant')?.fromSearch,
      ).toBe(true);
      expect(
        result.current.messages.find((m) => m.role === 'assistant')
          ?.searchSources,
      ).toHaveLength(1);
    });

    it('attaches SearchSources to the assistant message and persists them on Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('m', onTurnComplete));
      await act(async () => {
        await result.current.ask('who signed the treaty');
      });
      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({
          type: 'SearchSources',
          data: [
            { index: 1, url: 'https://a/', title: 'A' },
            { index: 2, url: 'https://b/', title: 'B' },
          ],
        });
      });

      const assistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistant?.fromSearch).toBe(true);
      expect(assistant?.searchSources).toEqual([
        { title: 'A', url: 'https://a/' },
        { title: 'B', url: 'https://b/' },
      ]);

      act(() => {
        channel!.simulateMessage({
          type: 'Token',
          data: 'It was signed in 1919.',
        });
        channel!.simulateMessage({ type: 'Done' });
      });

      expect(onTurnComplete).toHaveBeenCalledWith(
        expect.objectContaining({ role: 'user' }),
        expect.objectContaining({
          content: 'It was signed in 1919.',
          fromSearch: true,
          searchSources: [
            { title: 'A', url: 'https://a/' },
            { title: 'B', url: 'https://b/' },
          ],
        }),
      );
    });

    it('buffers the SearchFailed reason without stamping it while streaming, then applies it on Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('m', onTurnComplete));
      await act(async () => {
        await result.current.ask('what is the latest rust version');
      });
      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({
          type: 'SearchFailed',
          data: { reason: 'unreachable' },
        });
      });

      // Mid-stream: fromSearch flips on immediately (drives progress chrome),
      // but the failure note itself must stay hidden until the answer is done.
      const midStreamAssistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(midStreamAssistant?.fromSearch).toBe(true);
      expect(midStreamAssistant?.searchFailReason).toBeUndefined();

      act(() => {
        channel!.simulateMessage({
          type: 'Token',
          data: 'From what I recall, ...',
        });
        channel!.simulateMessage({ type: 'Done' });
      });

      const doneAssistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(doneAssistant?.searchFailReason).toBe('unreachable');
      expect(onTurnComplete).toHaveBeenCalledWith(
        expect.objectContaining({ role: 'user' }),
        expect.objectContaining({
          content: 'From what I recall, ...',
          fromSearch: true,
          searchFailReason: 'unreachable',
        }),
      );
    });

    it('carries a no_results SearchFailed reason through to Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('m', onTurnComplete));
      await act(async () => {
        await result.current.ask('who won the local match last night');
      });
      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({
          type: 'SearchFailed',
          data: { reason: 'no_results' },
        });
        channel!.simulateMessage({ type: 'Done' });
      });

      const assistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistant?.searchFailReason).toBe('no_results');
      expect(onTurnComplete).toHaveBeenCalledWith(
        expect.objectContaining({ role: 'user' }),
        expect.objectContaining({ searchFailReason: 'no_results' }),
      );
    });

    it('does not apply the SearchFailed reason when the turn is cancelled before Done', async () => {
      const { result } = renderHook(() => useModel('m'));
      await act(async () => {
        await result.current.ask('what is the latest rust version');
      });
      const channel = getChannel();

      act(() => {
        // Visible output first so the assistant message survives cancel
        // (an empty in-flight message is dropped on Cancelled).
        channel!.simulateMessage({ type: 'Token', data: 'partial answer' });
        channel!.simulateMessage({
          type: 'SearchFailed',
          data: { reason: 'unreachable' },
        });
        channel!.simulateMessage({ type: 'Cancelled' });
      });

      const assistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistant?.searchFailReason).toBeUndefined();
    });

    it('does not apply the SearchFailed reason when the turn errors before Done', async () => {
      const { result } = renderHook(() => useModel('m'));
      await act(async () => {
        await result.current.ask('what is the latest rust version');
      });
      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({
          type: 'SearchFailed',
          data: { reason: 'no_results' },
        });
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'Other', message: 'boom' },
        });
      });

      const assistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistant?.searchFailReason).toBeUndefined();
    });

    it('replaces the full assistant content on SetContent after a live draft', async () => {
      const { result } = renderHook(() => useModel('m'));
      await act(async () => {
        await result.current.ask('who owns Figma');
      });
      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({
          type: 'Token',
          data: 'Draft with bad [9].',
        });
        channel!.simulateMessage({
          type: 'SetContent',
          data: 'Clean answer with [1].',
        });
        channel!.simulateMessage({ type: 'Done' });
      });

      const assistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistant?.content).toBe('Clean answer with [1].');
    });

    it('accepts an empty SetContent replacement without hanging', async () => {
      const { result } = renderHook(() => useModel('m'));
      await act(async () => {
        await result.current.ask('who owns Figma');
      });
      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'draft' });
        channel!.simulateMessage({ type: 'SetContent', data: '' });
        channel!.simulateMessage({ type: 'Done' });
      });

      const assistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistant?.content).toBe('');
    });
  });

  // ─── imagePaths handling ─────────────────────────────────────────────────────

  describe('imagePaths handling', () => {
    it('allows ask() with empty text but valid imagePaths', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('', undefined, ['/tmp/img1.jpg']);
      });

      // Should have created a user message + assistant placeholder
      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0]).toEqual(
        expect.objectContaining({
          role: 'user',
          content: '',
          imagePaths: ['/tmp/img1.jpg'],
        }),
      );
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: '',
          imagePaths: ['/tmp/img1.jpg'],
        }),
      );
    });

    it('returns early for empty text AND no imagePaths', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('', undefined, undefined);
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('returns early for empty text AND empty imagePaths array', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('', undefined, []);
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('includes imagePaths in message and invoke when text AND imagePaths are provided', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('describe this', undefined, [
          '/tmp/img1.jpg',
          '/tmp/img2.jpg',
        ]);
      });

      expect(result.current.messages[0]).toEqual(
        expect.objectContaining({
          role: 'user',
          content: 'describe this',
          imagePaths: ['/tmp/img1.jpg', '/tmp/img2.jpg'],
        }),
      );
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'describe this',
          imagePaths: ['/tmp/img1.jpg', '/tmp/img2.jpg'],
        }),
      );
    });

    it('sets message.imagePaths to undefined and invoke imagePaths to null when no imagePaths', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello');
      });

      expect(result.current.messages[0].imagePaths).toBeUndefined();
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          imagePaths: null,
        }),
      );
    });

    it('displayImagePaths shows in bubble but imagePaths=undefined keeps null in backend call', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask(
          'summarize this',
          undefined,
          undefined,
          undefined,
          undefined,
          ['/tmp/staged/img1.jpg'],
        );
      });

      // Bubble should show the display image.
      expect(result.current.messages[0].imagePaths).toEqual([
        '/tmp/staged/img1.jpg',
      ]);
      // Backend must NOT receive image bytes (OCR path: model only sees text).
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          imagePaths: null,
        }),
      );
    });
  });

  // ─── Error handling ──────────────────────────────────────────────────────────

  describe('error handling', () => {
    it('Error chunk sets isGenerating to false', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('test');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: {
            kind: 'ModelNotFound',
            message: 'Model not found\nRun: ollama pull gemma3:4b',
          },
        });
      });

      expect(result.current.isGenerating).toBe(false);
    });

    it('invoke rejection sets isGenerating to false', async () => {
      invoke.mockRejectedValueOnce(new Error('connection refused'));

      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('test');
      });

      expect(result.current.isGenerating).toBe(false);
    });

    it('Error chunk updates assistant placeholder with errorKind', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('test');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: {
            kind: 'EngineUnreachable',
            message: "Ollama isn't running\nStart Ollama and try again.",
          },
        });
      });

      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg?.errorKind).toBe('EngineUnreachable');
      expect(assistantMsg?.content).toBe(
        "Ollama isn't running\nStart Ollama and try again.",
      );
    });

    it('Error chunk with partial tokens replaces content with error', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('test');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Partial answer' });
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'Other', message: 'Something went wrong\nHTTP 500' },
        });
      });

      // The error replaces the assistant placeholder content
      const errorMsg = result.current.messages.find((m) => m.errorKind);
      expect(errorMsg).toBeDefined();
      expect(errorMsg?.errorKind).toBe('Other');
      expect(errorMsg?.content).toBe('Something went wrong\nHTTP 500');
    });

    it('invoke rejection creates assistant message with Other errorKind', async () => {
      invoke.mockRejectedValueOnce(new Error('network error'));

      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('test');
      });

      const errorMsg = result.current.messages.find(
        (m) => m.errorKind === 'Other',
      );
      expect(errorMsg?.errorKind).toBe('Other');
      expect(errorMsg?.content).toBeTruthy();
    });
  });

  // ─── Streaming edge cases ────────────────────────────────────────────────────

  describe('streaming edge cases', () => {
    it('handles Token with empty string', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: '' });
      });

      // Assistant content should still be empty (no crash)
      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg?.content).toBe('');
    });

    it('drops the placeholder when only an empty ThinkingToken arrives before cancellation', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello', undefined, undefined, true);
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'ThinkingToken', data: '' });
        channel!.simulateMessage({ type: 'Cancelled' });
      });

      expect(
        result.current.messages.find((message) => message.role === 'assistant'),
      ).toBeUndefined();
    });
  });

  describe('ask() race handling', () => {
    it('waits for a pending cancel before restarting and only resumes one queued ask', async () => {
      let latestChannel: ReturnType<typeof getChannel> = null;
      let resolveFirstAskInvoke!: () => void;
      let resolveCancel!: () => void;
      const askMessages: string[] = [];

      invoke.mockImplementation(async (cmd, args) => {
        if (args && 'onEvent' in args) {
          latestChannel = args.onEvent as ReturnType<typeof getChannel>;
        }

        if (cmd === 'ask_model') {
          askMessages.push(String(args?.message ?? ''));
          if (askMessages.length === 1) {
            return new Promise<void>((resolve) => {
              resolveFirstAskInvoke = resolve;
            });
          }
          return;
        }

        if (cmd === 'cancel_generation') {
          return new Promise<void>((resolve) => {
            resolveCancel = resolve;
          });
        }
      });

      const { result } = renderHook(() => useModel(''));

      let secondAsk!: Promise<void>;
      let thirdAsk!: Promise<void>;

      act(() => {
        void result.current.ask('first');
      });

      act(() => {
        void result.current.cancel();
        void result.current.cancel();
        secondAsk = result.current.ask('second');
        thirdAsk = result.current.ask('third');
      });

      expect(askMessages).toEqual(['first']);
      expect(invoke).toHaveBeenCalledWith('cancel_generation');
      expect(
        invoke.mock.calls.filter(([cmd]) => cmd === 'cancel_generation'),
      ).toHaveLength(1);

      await act(async () => {
        resolveCancel();
        await Promise.resolve();
        await Promise.resolve();
      });

      await act(async () => {
        await Promise.all([secondAsk, thirdAsk]);
      });

      expect(askMessages).toHaveLength(2);
      expect(['second', 'third']).toContain(askMessages[1]);

      act(() => {
        latestChannel!.simulateMessage({ type: 'Done' });
        resolveFirstAskInvoke();
      });
    });

    it('ignores late ask events and invoke rejection after reset', async () => {
      let channel: ReturnType<typeof getChannel> = null;
      let rejectInvoke!: (error: Error) => void;

      invoke.mockImplementation(async (cmd, args) => {
        if (cmd === 'ask_model') {
          channel = args?.onEvent as ReturnType<typeof getChannel>;
          return new Promise<void>((_, reject) => {
            rejectInvoke = reject;
          });
        }
      });

      const { result } = renderHook(() => useModel(''));

      act(() => {
        void result.current.ask('late failure');
      });

      act(() => {
        result.current.reset();
      });

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'late' });
        channel!.simulateMessage({ type: 'Done' });
      });

      expect(result.current.messages).toEqual([]);

      await act(async () => {
        rejectInvoke(new Error('late fail'));
        await Promise.resolve();
      });

      expect(result.current.messages).toEqual([]);
      expect(result.current.isGenerating).toBe(false);
    });
  });

  // ─── cancel() ───────────────────────────────────────────────────────────────

  describe('cancel()', () => {
    it('invokes cancel_generation on the backend', async () => {
      let resolveInvoke!: () => void;
      invoke.mockImplementationOnce(
        async (_cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            return new Promise<void>((res) => {
              resolveInvoke = res;
            });
          }
        },
      );

      const { result } = renderHook(() => useModel(''));

      act(() => {
        void result.current.ask('hello');
      });

      expect(result.current.isGenerating).toBe(true);

      await act(async () => {
        await result.current.cancel();
      });

      expect(result.current.isGenerating).toBe(false);
      expect(invoke).toHaveBeenCalledWith('cancel_generation');

      act(() => {
        resolveInvoke?.();
      });
    });

    it('hard-aborts an active /search turn locally and ignores late events', async () => {
      let resolveSearchInvoke!: () => void;
      let resolveCancel!: () => void;
      let channel: ReturnType<typeof getChannel> = null;

      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            channel = args.onEvent as ReturnType<typeof getChannel>;
          }

          if (cmd === 'ask_model') {
            return new Promise<void>((res) => {
              resolveSearchInvoke = res;
            });
          }

          if (cmd === 'cancel_generation') {
            return new Promise<void>((res) => {
              resolveCancel = res;
            });
          }
        },
      );

      const { result } = renderHook(() => useModel(''));

      act(() => {
        void result.current.askSearch('rust');
      });

      expect(channel).not.toBeNull();
      expect(result.current.isGenerating).toBe(true);
      expect(result.current.messages).toHaveLength(2);

      act(() => {
        void result.current.cancel();
      });

      expect(result.current.isGenerating).toBe(false);
      expect(result.current.searchStage).toBeNull();
      expect(result.current.messages).toHaveLength(1);
      expect(result.current.messages[0].role).toBe('user');

      act(() => {
        // Late StreamChunk events after local cancel must be ignored.
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'searching' },
        });
        channel!.simulateMessage({ type: 'Token', data: 'late answer' });
        channel!.simulateMessage({ type: 'Done' });
      });

      expect(result.current.isGenerating).toBe(false);
      expect(result.current.messages).toHaveLength(1);

      act(() => {
        resolveCancel?.();
        resolveSearchInvoke?.();
      });
    });

    it('does nothing when not generating', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.cancel();
      });

      // cancel_generation should NOT have been called
      expect(invoke).not.toHaveBeenCalledWith('cancel_generation');
    });
  });

  // ─── Cancelled chunk handling ───────────────────────────────────────────────

  describe('Cancelled chunk', () => {
    it('keeps partial content as assistant message on Cancelled', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Partial ' });
        channel!.simulateMessage({ type: 'Token', data: 'response' });
        channel!.simulateMessage({ type: 'Cancelled' });
      });

      expect(result.current.isGenerating).toBe(false);
      expect(result.current.messages).toContainEqual(
        expect.objectContaining({
          role: 'assistant',
          content: 'Partial response',
        }),
      );
    });

    it('removes assistant placeholder when cancelled with no tokens', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Cancelled' });
      });

      expect(result.current.isGenerating).toBe(false);
      // Only the user message should exist - empty assistant placeholder was removed
      expect(result.current.messages).toHaveLength(1);
      expect(result.current.messages[0].role).toBe('user');
    });
  });

  // ─── reset() ────────────────────────────────────────────────────────────────

  describe('reset()', () => {
    it('clears all state', async () => {
      const { result } = renderHook(() => useModel(''));

      // Build up some state
      await act(async () => {
        await result.current.ask('hello');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Hi' });
      });

      // Confirm state is non-empty before reset
      expect(result.current.messages.length).toBeGreaterThan(0);

      act(() => {
        result.current.reset();
      });

      expect(result.current.messages).toEqual([]);
      expect(result.current.isGenerating).toBe(false);
      // Should also reset backend conversation history
      expect(invoke).toHaveBeenCalledWith('reset_conversation');
    });

    it('fires record_conversation_end with user_reset when a turn was accepted', async () => {
      const { result } = renderHook(() => useModel(''));
      await act(async () => {
        await result.current.ask('hello');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'TurnAccepted' });
        channel!.simulateMessage({ type: 'Token', data: 'Hi' });
        channel!.simulateMessage({ type: 'Done' });
      });

      invoke.mockClear();
      act(() => {
        result.current.reset();
      });
      expect(invoke).toHaveBeenCalledWith(
        'record_conversation_end',
        expect.objectContaining({ reason: 'user_reset' }),
      );
    });

    it('cancels the in-flight backend generation when starting a new session', async () => {
      // Stall ask_model so the generation stays active while we reset.
      let resolveInvoke!: () => void;
      invoke.mockImplementationOnce(
        async (_cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            return new Promise<void>((res) => {
              resolveInvoke = res;
            });
          }
        },
      );

      const { result } = renderHook(() => useModel(''));

      act(() => {
        void result.current.ask('hello');
      });
      expect(result.current.isGenerating).toBe(true);

      await act(async () => {
        result.current.reset();
        await Promise.resolve();
      });

      // A new session must stop the backend stream, not just the frontend
      // view - otherwise the old generation holds the engine's single slot
      // and the next turn queues behind it.
      expect(invoke).toHaveBeenCalledWith('cancel_generation');

      act(() => {
        resolveInvoke?.();
      });
    });

    it('does not call cancel_generation when reset runs with no active generation', () => {
      const { result } = renderHook(() => useModel(''));

      act(() => {
        result.current.reset();
      });

      expect(invoke).not.toHaveBeenCalledWith('cancel_generation');
    });
  });

  // ─── onTurnComplete callback ─────────────────────────────────────────────────

  describe('onTurnComplete callback', () => {
    it('is called with user and assistant messages on Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));

      await act(async () => {
        await result.current.ask('ping');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'pong' });
        channel!.simulateMessage({ type: 'Done' });
      });

      expect(onTurnComplete).toHaveBeenCalledOnce();
      const [userMsg, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(userMsg).toMatchObject({ role: 'user', content: 'ping' });
      expect(assistantMsg).toMatchObject({
        role: 'assistant',
        content: 'pong',
      });
    });

    it('is not called when Cancelled', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));

      await act(async () => {
        await result.current.ask('ping');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'partial' });
        channel!.simulateMessage({ type: 'Cancelled' });
      });

      expect(onTurnComplete).not.toHaveBeenCalled();
    });

    it('is not called when an Error chunk is received', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));

      await act(async () => {
        await result.current.ask('ping');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'Other', message: 'Something went wrong\nHTTP 500' },
        });
      });

      expect(onTurnComplete).not.toHaveBeenCalled();
    });
  });

  // ─── modelName attribution ───────────────────────────────────────────────────

  describe('modelName attribution', () => {
    it('stamps the assistant message with activeModel on ask() completion', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() =>
        useModel('gemma4:e2b', onTurnComplete),
      );

      await act(async () => {
        await result.current.ask('hi');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'hello' });
        channel!.simulateMessage({ type: 'Done' });
      });

      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.modelName).toBe('gemma4:e2b');
      expect(result.current.messages[1]).toMatchObject({
        role: 'assistant',
        modelName: 'gemma4:e2b',
      });
    });

    it('leaves modelName undefined when activeModel is null', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel(null, onTurnComplete));

      await act(async () => {
        await result.current.ask('hi');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'hello' });
        channel!.simulateMessage({ type: 'Done' });
      });

      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.modelName).toBeUndefined();
    });

    it('stamps the assistant message with activeModel on askSearch() turns', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() =>
        useModel('qwen2.5:7b', onTurnComplete),
      );

      let pending: Promise<unknown> | undefined;
      await act(async () => {
        pending = result.current.askSearch('rust async');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'answer' });
        channel!.simulateMessage({ type: 'Done' });
      });

      await act(async () => {
        await pending;
      });

      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.modelName).toBe('qwen2.5:7b');
    });

    it('leaves modelName undefined when activeModel is null on askSearch()', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel(null, onTurnComplete));

      let pending: Promise<unknown> | undefined;
      await act(async () => {
        pending = result.current.askSearch('rust async');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'answer' });
        channel!.simulateMessage({ type: 'Done' });
      });

      await act(async () => {
        await pending;
      });

      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.modelName).toBeUndefined();
    });
  });

  // ─── loadMessages() ──────────────────────────────────────────────────────────

  describe('loadMessages()', () => {
    it('replaces messages state with provided array', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('original question');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Done' });
      });
      expect(result.current.messages).toHaveLength(2);

      const loaded = [
        { id: 'l1', role: 'user' as const, content: 'loaded question' },
        { id: 'l2', role: 'assistant' as const, content: 'loaded answer' },
      ];

      act(() => {
        result.current.loadMessages(loaded);
      });

      expect(result.current.messages).toEqual(loaded);
    });

    it('clears generating state when loading messages', async () => {
      invoke.mockRejectedValueOnce(new Error('boom'));
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('fail');
      });
      expect(result.current.isGenerating).toBe(false);

      act(() => {
        result.current.loadMessages([]);
      });

      expect(result.current.isGenerating).toBe(false);
    });

    it('fires record_conversation_end with history_load when a turn was accepted', async () => {
      const { result } = renderHook(() => useModel(''));
      await act(async () => {
        await result.current.ask('original');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'TurnAccepted' });
        channel!.simulateMessage({ type: 'Done' });
      });

      invoke.mockClear();
      act(() => {
        result.current.loadMessages([
          { id: 'l1', role: 'user', content: 'loaded' },
        ]);
      });
      expect(invoke).toHaveBeenCalledWith(
        'record_conversation_end',
        expect.objectContaining({ reason: 'history_load' }),
      );
    });

    it('cancels the in-flight backend generation when loading another conversation', async () => {
      // Stall ask_model so the generation stays active while we load.
      let resolveInvoke!: () => void;
      invoke.mockImplementationOnce(
        async (_cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            return new Promise<void>((res) => {
              resolveInvoke = res;
            });
          }
        },
      );

      const { result } = renderHook(() => useModel(''));

      act(() => {
        void result.current.ask('original');
      });
      expect(result.current.isGenerating).toBe(true);

      await act(async () => {
        result.current.loadMessages([
          { id: 'l1', role: 'user', content: 'loaded' },
        ]);
        await Promise.resolve();
      });

      expect(invoke).toHaveBeenCalledWith('cancel_generation');

      act(() => {
        resolveInvoke?.();
      });
    });
  });

  // ─── ThinkingToken handling ──────────────────────────────────────────────────

  describe('ThinkingToken handling', () => {
    it('marks the assistant placeholder as a /think turn when think is true', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello', undefined, undefined, true);
      });

      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg?.fromThink).toBe(true);
    });

    it('accumulates ThinkingTokens into thinkingContent', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello', undefined, undefined, true);
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'ThinkingToken', data: 'Let me ' });
        channel!.simulateMessage({ type: 'ThinkingToken', data: 'think...' });
      });

      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg?.thinkingContent).toBe('Let me think...');
    });

    it('passes think parameter to invoke', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello', undefined, undefined, true);
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          think: true,
        }),
      );
    });

    it('passes think as false by default', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello');
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          think: false,
        }),
      );
    });

    it('includes thinkingContent in onTurnComplete on Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));

      await act(async () => {
        await result.current.ask('hello', undefined, undefined, true);
      });

      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({
          type: 'ThinkingToken',
          data: 'thinking deeply',
        });
        channel!.simulateMessage({ type: 'Token', data: 'the answer' });
        channel!.simulateMessage({ type: 'Done' });
      });

      expect(onTurnComplete).toHaveBeenCalledOnce();
      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.content).toBe('the answer');
      expect(assistantMsg.thinkingContent).toBe('thinking deeply');
    });

    it('does not set thinkingContent when no thinking happened', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));

      await act(async () => {
        await result.current.ask('hello');
      });

      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'direct answer' });
        channel!.simulateMessage({ type: 'Done' });
      });

      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.thinkingContent).toBeUndefined();
    });

    it('preserves thinking content when cancelled with thinking but no regular tokens', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello', undefined, undefined, true);
      });

      const channel = getChannel();

      act(() => {
        channel!.simulateMessage({
          type: 'ThinkingToken',
          data: 'partial thinking',
        });
        channel!.simulateMessage({ type: 'Cancelled' });
      });

      expect(result.current.isGenerating).toBe(false);
      // Should keep the assistant message since thinkingContent is non-empty
      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg).toBeDefined();
      expect(assistantMsg?.thinkingContent).toBe('partial thinking');
    });
  });

  // ─── History ─────────────────────────────────────────────────────────────────

  describe('history', () => {
    it('maintains message history across multiple sequential asks', async () => {
      const { result } = renderHook(() => useModel(''));

      // First ask + response
      await act(async () => {
        await result.current.ask('first question');
      });
      let channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'First answer' });
        channel!.simulateMessage({ type: 'Done' });
      });

      // Reset capture so we get fresh channel for second ask
      resetChannelCapture();
      enableChannelCapture();

      // Second ask + response
      await act(async () => {
        await result.current.ask('second question');
      });
      channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Second answer' });
        channel!.simulateMessage({ type: 'Done' });
      });

      expect(result.current.messages).toEqual([
        expect.objectContaining({ role: 'user', content: 'first question' }),
        expect.objectContaining({ role: 'assistant', content: 'First answer' }),
        expect.objectContaining({ role: 'user', content: 'second question' }),
        expect.objectContaining({
          role: 'assistant',
          content: 'Second answer',
        }),
      ]);
    });
  });

  // ─── askSearch() (force-search alias onto ask_model) ───────────────────────

  describe('askSearch()', () => {
    it('invokes ask_model with forceSearch and the trimmed query', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('  rust async  ');
      });
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'rust async',
          forceSearch: true,
          slashCommand: '/search',
        }),
      );
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'searching' },
        });
        channel!.simulateMessage({ type: 'Token', data: 'ok' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await expect(pending).resolves.toEqual({ final: true });
      });
      const assistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistant?.fromSearch).toBe(true);
    });

    it('stores display content and quotedText on the user bubble', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch(
          'rust async',
          '/search rust async',
          'selected snippet',
        );
      });
      expect(result.current.messages[0]).toMatchObject({
        role: 'user',
        content: '/search rust async',
        quotedText: 'selected snippet',
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'ok' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
    });

    it('resolves immediately with final=true on empty query', async () => {
      const { result } = renderHook(() => useModel(''));
      let outcome!: { final: boolean };
      await act(async () => {
        outcome = await result.current.askSearch('   ');
      });
      expect(outcome).toEqual({ final: true });
      expect(invoke).not.toHaveBeenCalled();
    });

    it('stamps the assistant message with activeModel on askSearch() turns', async () => {
      const { result } = renderHook(() => useModel('gemma-x'));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('rust async');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'ok' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
      const assistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistant?.modelName).toBe('gemma-x');
    });

    it('leaves modelName undefined when activeModel is null on askSearch()', async () => {
      const { result } = renderHook(() => useModel(null));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('rust async');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'ok' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
      const assistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistant?.modelName).toBeUndefined();
    });
  });

  // ─── reset/loadMessages interaction with searchStage ────────────────────────

  describe('search state cleanup', () => {
    it('reset clears the search stage indicator after force-search status', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'searching' },
        });
      });
      expect(result.current.searchStage).toEqual({ kind: 'searching' });
      act(() => {
        result.current.reset();
      });
      expect(result.current.searchStage).toBeNull();
      act(() => {
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
    });

    it('loadMessages clears the search stage indicator', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'SearchStatus',
          data: { phase: 'deciding' },
        });
      });
      expect(result.current.searchStage).toEqual({ kind: 'analyzing_query' });
      act(() => {
        result.current.loadMessages([]);
      });
      expect(result.current.searchStage).toBeNull();
      act(() => {
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
    });
  });

  describe('addOcrTurn', () => {
    it('appends user and assistant messages to the conversation', async () => {
      const { result } = renderHook(() => useModel(''));

      act(() => {
        result.current.addOcrTurn(
          '/extract',
          undefined,
          ['/tmp/img.jpg'],
          '```\nhello world\n```',
        );
      });

      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0]).toMatchObject({
        role: 'user',
        content: '/extract',
        quotedText: undefined,
        imagePaths: ['/tmp/img.jpg'],
      });
      expect(result.current.messages[1]).toMatchObject({
        role: 'assistant',
        content: '```\nhello world\n```',
      });
    });

    it('calls onTurnComplete with the user and assistant messages', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));

      act(() => {
        result.current.addOcrTurn(
          '/extract',
          'selected text',
          undefined,
          'extracted',
        );
      });

      expect(onTurnComplete).toHaveBeenCalledOnce();
      const [userMsg, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(userMsg).toMatchObject({
        role: 'user',
        content: '/extract',
        quotedText: 'selected text',
      });
      expect(assistantMsg).toMatchObject({
        role: 'assistant',
        content: 'extracted',
      });
    });
  });

  // ─── retryMessageWithOversized (issue #296) ─────────────────────────────────

  describe('retryMessageWithOversized()', () => {
    it('attaches a chat retrySnapshot to the assistant message at creation time', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('hello world', 'quoted', ['/tmp/img.jpg']);
      });

      const user = result.current.messages[0];
      const assistant = result.current.messages[1];
      expect(assistant.retrySnapshot).toEqual({
        kind: 'chat',
        displayContent: 'hello world',
        quotedText: 'quoted',
        imagePaths: ['/tmp/img.jpg'],
        think: undefined,
        promptOverride: undefined,
        displayImagePaths: undefined,
        replaceCommand: undefined,
        slashCommand: undefined,
        userMessageId: user.id,
        assistantMessageId: assistant.id,
      });
    });

    it('stores slashCommand on chat retrySnapshot for utility turns', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask(
          '/rewrite fix',
          undefined,
          undefined,
          false,
          'composed',
          undefined,
          '/rewrite',
          undefined,
          '/rewrite',
        );
      });

      const assistant = result.current.messages[1];
      expect(assistant.retrySnapshot).toMatchObject({
        kind: 'chat',
        slashCommand: '/rewrite',
        replaceCommand: '/rewrite',
      });
    });

    it('attaches a search retrySnapshot to the assistant message at creation time', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        void result.current.askSearch('rust async', 'display text', 'quoted');
      });

      const user = result.current.messages[0];
      const assistant = result.current.messages[1];
      expect(assistant.retrySnapshot).toEqual({
        kind: 'search',
        query: 'rust async',
        displayContent: 'display text',
        quotedText: 'quoted',
        userMessageId: user.id,
        assistantMessageId: assistant.id,
      });
    });

    it('replays a chat snapshot with allowOversized: true', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        result.current.retryMessageWithOversized({
          kind: 'chat',
          displayContent: 'hello world',
          quotedText: 'quoted',
          imagePaths: ['/tmp/img.jpg'],
          userMessageId: 'stale-user-id',
          assistantMessageId: 'stale-assistant-id',
        });
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'hello world',
          quotedText: 'quoted',
          imagePaths: ['/tmp/img.jpg'],
          allowOversized: true,
        }),
      );
    });

    it('replays a search snapshot with allowOversized: true', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        result.current.retryMessageWithOversized({
          kind: 'search',
          query: 'rust async',
          displayContent: 'display text',
          quotedText: 'quoted',
          userMessageId: 'stale-user-id',
          assistantMessageId: 'stale-assistant-id',
        });
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'rust async',
          forceSearch: true,
          slashCommand: '/search',
          allowOversized: true,
        }),
      );
    });

    // Regression test (issue #296 follow-up review): retry data used to live
    // in a single shared `lastRequestRef`, overwritten unconditionally by
    // every dispatched turn. A superseding turn B sent after turn A's
    // `InsufficientMemory` card was left on screen (nothing blocks sending
    // after an error card) silently hijacked A's stale "Load anyway" click
    // and replayed B's content instead of retrying A. The fix moves the
    // snapshot onto each message itself; this asserts clicking A's card
    // replays A's original content even after B has since succeeded.
    it('replays the failed turn A original content, not a later turn B that superseded it', async () => {
      const { result } = renderHook(() => useModel(''));

      // Turn A: fails with InsufficientMemory. Its error card stays mounted
      // (errorKind is set once and never cleared).
      await act(async () => {
        await result.current.ask('turn A content');
      });
      const channelA = getChannel();
      act(() => {
        channelA!.simulateMessage({
          type: 'Error',
          data: { kind: 'InsufficientMemory', message: 'may not fit' },
        });
      });

      const turnASnapshot = result.current.messages[1].retrySnapshot;
      expect(turnASnapshot).toMatchObject({ displayContent: 'turn A content' });
      const turnAUserId = result.current.messages[0].id;
      const turnAAssistantId = result.current.messages[1].id;

      // Turn B: sent without touching turn A's still-visible card, and
      // succeeds normally.
      resetChannelCapture();
      enableChannelCapture();
      await act(async () => {
        await result.current.ask('turn B content');
      });
      const channelB = getChannel();
      act(() => {
        channelB!.simulateMessage({ type: 'Token', data: 'B answer' });
        channelB!.simulateMessage({ type: 'Done' });
      });
      const turnBUserId = result.current.messages[2].id;
      const turnBAssistantId = result.current.messages[3].id;

      invoke.mockClear();

      // User scrolls back and clicks "Load anyway" on turn A's card, using
      // ONLY turn A's own retained snapshot (as the real UI wiring does via
      // ConversationView's per-message closure) - never a shared hook ref.
      await act(async () => {
        result.current.retryMessageWithOversized(turnASnapshot!);
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'turn A content',
          allowOversized: true,
        }),
      );
      expect(invoke).not.toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({ message: 'turn B content' }),
      );

      // Turn A is retried IN PLACE (issue #296): its own ids are reused rather
      // than deleted and recreated, so React keeps the bubble mounted and the
      // entrance animation never replays. Turn B's completed pair is
      // untouched. No pair is added or removed: the thread stays four
      // messages with all four original ids.
      const ids = result.current.messages.map((m) => m.id);
      expect(ids).toContain(turnAUserId);
      expect(ids).toContain(turnAAssistantId);
      expect(ids).toContain(turnBUserId);
      expect(ids).toContain(turnBAssistantId);
      expect(result.current.messages).toHaveLength(4);
      // Turn A's assistant message is reset in place: same id, error state
      // cleared, ready to stream the replayed response.
      expect(result.current.messages[0]).toMatchObject({
        role: 'user',
        content: 'turn A content',
      });
      expect(result.current.messages[1].id).toBe(turnAAssistantId);
      expect(result.current.messages[1].role).toBe('assistant');
      expect(result.current.messages[1].content).toBe('');
      expect(result.current.messages[1].errorKind).toBeUndefined();
    });

    // Regression test for the duplicate-turn bug the user's screenshot
    // showed (issue #296 follow-up): clicking "Load anyway" must NOT append a
    // brand-new pair, which would leave the old failed user bubble and amber
    // warning card stacked above the retried turn. The fix reuses the failed
    // turn's ids and resets its assistant message in place, so the thread
    // keeps exactly one pair and React does not remount the bubble.
    it('chat retry reuses the failed pair ids and resets the assistant in place', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('oversized content');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'InsufficientMemory', message: 'may not fit' },
        });
      });

      expect(result.current.messages).toHaveLength(2);
      const oldUserId = result.current.messages[0].id;
      const oldAssistantId = result.current.messages[1].id;
      const snapshot = result.current.messages[1].retrySnapshot!;
      expect(result.current.messages[1].errorKind).toBe('InsufficientMemory');

      resetChannelCapture();
      enableChannelCapture();
      await act(async () => {
        result.current.retryMessageWithOversized(snapshot);
      });

      // Same two ids, no add/remove: the turn is retried in place.
      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0].id).toBe(oldUserId);
      expect(result.current.messages[1].id).toBe(oldAssistantId);
      expect(result.current.messages[0]).toMatchObject({
        role: 'user',
        content: 'oversized content',
      });
      // Assistant reset in place: error state cleared, content blanked.
      expect(result.current.messages[1].role).toBe('assistant');
      expect(result.current.messages[1].content).toBe('');
      expect(result.current.messages[1].errorKind).toBeUndefined();
    });

    it('search retry reuses the failed pair ids and resets the assistant in place', async () => {
      const { result } = renderHook(() => useModel(''));

      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('rust async');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'InsufficientMemory', message: 'may not fit' },
        });
      });
      await act(async () => {
        await pending;
      });

      expect(result.current.messages).toHaveLength(2);
      const oldUserId = result.current.messages[0].id;
      const oldAssistantId = result.current.messages[1].id;
      const snapshot = result.current.messages[1].retrySnapshot!;
      expect(result.current.messages[1].errorKind).toBe('InsufficientMemory');

      resetChannelCapture();
      enableChannelCapture();
      await act(async () => {
        result.current.retryMessageWithOversized(snapshot);
      });

      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0].id).toBe(oldUserId);
      expect(result.current.messages[1].id).toBe(oldAssistantId);
      expect(result.current.messages[0]).toMatchObject({
        role: 'user',
        content: 'rust async',
      });
      // Reset in place clears the error and re-marks it as a search bubble.
      expect(result.current.messages[1].role).toBe('assistant');
      expect(result.current.messages[1].content).toBe('');
      expect(result.current.messages[1].errorKind).toBeUndefined();
      expect(result.current.messages[1].fromSearch).toBe(true);
    });

    it('pins modelOverride on the reset assistant message when provided (chat)', async () => {
      const { result } = renderHook(() => useModel('old-model'));

      await act(async () => {
        await result.current.ask('needs a bigger model');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'InsufficientMemory', message: 'may not fit' },
        });
      });
      expect(result.current.messages[1].modelName).toBe('old-model');
      const snapshot = result.current.messages[1].retrySnapshot!;

      resetChannelCapture();
      enableChannelCapture();
      await act(async () => {
        result.current.retryMessageWithOversized(snapshot, 'freshly-picked');
      });

      // The override wins over the activeModel closure, which still reads the
      // stale value at retry time (issue #296 attribution race).
      expect(result.current.messages[1].modelName).toBe('freshly-picked');
    });

    it('pins modelOverride on the reset assistant message when provided (search)', async () => {
      const { result } = renderHook(() => useModel('old-model'));

      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('rust async');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'InsufficientMemory', message: 'may not fit' },
        });
      });
      await act(async () => {
        await pending;
      });
      expect(result.current.messages[1].modelName).toBe('old-model');
      const snapshot = result.current.messages[1].retrySnapshot!;

      resetChannelCapture();
      enableChannelCapture();
      await act(async () => {
        result.current.retryMessageWithOversized(snapshot, 'freshly-picked');
      });

      expect(result.current.messages[1].modelName).toBe('freshly-picked');
    });
  });

  // ─── retryMessage (issue #296 follow-up, bug 2) ─────────────────────────────
  //
  // "Switch model" replays the abandoned turn against the newly-picked model
  // WITHOUT bypassing the pre-load memory gate (unlike "Load anyway"), since
  // the whole point is that the new model is presumed to actually fit. Shares
  // the same reuse-in-place core as retryMessageWithOversized.
  describe('retryMessage()', () => {
    it('replays a chat snapshot with allowOversized: false', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        result.current.retryMessage({
          kind: 'chat',
          displayContent: 'hello world',
          quotedText: 'quoted',
          imagePaths: ['/tmp/img.jpg'],
          userMessageId: 'stale-user-id',
          assistantMessageId: 'stale-assistant-id',
        });
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'hello world',
          quotedText: 'quoted',
          imagePaths: ['/tmp/img.jpg'],
          allowOversized: false,
        }),
      );
    });

    it('replays a search snapshot with allowOversized: false', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        result.current.retryMessage({
          kind: 'search',
          query: 'rust async',
          displayContent: 'display text',
          quotedText: 'quoted',
          userMessageId: 'stale-user-id',
          assistantMessageId: 'stale-assistant-id',
        });
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'rust async',
          forceSearch: true,
          slashCommand: '/search',
          allowOversized: false,
        }),
      );
    });

    it('chat retry reuses the failed pair ids and resets the assistant in place', async () => {
      const { result } = renderHook(() => useModel(''));

      await act(async () => {
        await result.current.ask('gpt-oss content');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'InsufficientMemory', message: 'may not fit' },
        });
      });

      expect(result.current.messages).toHaveLength(2);
      const oldUserId = result.current.messages[0].id;
      const oldAssistantId = result.current.messages[1].id;
      const snapshot = result.current.messages[1].retrySnapshot!;

      resetChannelCapture();
      enableChannelCapture();
      await act(async () => {
        result.current.retryMessage(snapshot);
      });

      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0].id).toBe(oldUserId);
      expect(result.current.messages[1].id).toBe(oldAssistantId);
      expect(result.current.messages[0]).toMatchObject({
        role: 'user',
        content: 'gpt-oss content',
      });
      expect(result.current.messages[1].role).toBe('assistant');
      expect(result.current.messages[1].content).toBe('');
      expect(result.current.messages[1].errorKind).toBeUndefined();
    });

    it('search retry reuses the failed pair ids and resets the assistant in place', async () => {
      const { result } = renderHook(() => useModel(''));

      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('rust async');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'InsufficientMemory', message: 'may not fit' },
        });
      });
      await act(async () => {
        await pending;
      });

      expect(result.current.messages).toHaveLength(2);
      const oldUserId = result.current.messages[0].id;
      const oldAssistantId = result.current.messages[1].id;
      const snapshot = result.current.messages[1].retrySnapshot!;

      resetChannelCapture();
      enableChannelCapture();
      await act(async () => {
        result.current.retryMessage(snapshot);
      });

      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0].id).toBe(oldUserId);
      expect(result.current.messages[1].id).toBe(oldAssistantId);
      expect(result.current.messages[0]).toMatchObject({
        role: 'user',
        content: 'rust async',
      });
      expect(result.current.messages[1].role).toBe('assistant');
      expect(result.current.messages[1].content).toBe('');
      expect(result.current.messages[1].errorKind).toBeUndefined();
    });

    it('falls back to activeModel on the reset assistant when no override is given', async () => {
      const { result } = renderHook(() => useModel('active-model'));

      await act(async () => {
        await result.current.ask('needs a bigger model');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'InsufficientMemory', message: 'may not fit' },
        });
      });
      const snapshot = result.current.messages[1].retrySnapshot!;

      resetChannelCapture();
      enableChannelCapture();
      await act(async () => {
        result.current.retryMessage(snapshot);
      });

      expect(result.current.messages[1].modelName).toBe('active-model');
    });
  });

  describe('updateErroredMessageModel()', () => {
    it('changes modelName and carries the memory-fit figures on the targeted message, preserving errorKind, retrySnapshot, content, and id', async () => {
      const { result } = renderHook(() => useModel('gemma4:e2b'));

      await act(async () => {
        await result.current.ask('needs a bigger model');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'InsufficientMemory', message: 'may not fit' },
        });
      });

      const assistantId = result.current.messages[1].id;
      const originalContent = result.current.messages[1].content;
      const originalSnapshot = result.current.messages[1].retrySnapshot;

      act(() => {
        result.current.updateErroredMessageModel(assistantId, 'qwen2.5:7b', {
          requiredBytes: 8 * 1024 ** 3,
          availableBytes: 4 * 1024 ** 3,
        });
      });

      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[1].id).toBe(assistantId);
      expect(result.current.messages[1].modelName).toBe('qwen2.5:7b');
      expect(result.current.messages[1].memoryFit).toEqual({
        requiredBytes: 8 * 1024 ** 3,
        availableBytes: 4 * 1024 ** 3,
      });
      expect(result.current.messages[1].errorKind).toBe('InsufficientMemory');
      expect(result.current.messages[1].retrySnapshot).toBe(originalSnapshot);
      expect(result.current.messages[1].content).toBe(originalContent);
    });

    it('is a no-op when no message matches the given id', async () => {
      const { result } = renderHook(() => useModel('gemma4:e2b'));

      await act(async () => {
        await result.current.ask('needs a bigger model');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: { kind: 'InsufficientMemory', message: 'may not fit' },
        });
      });

      const before = result.current.messages;

      expect(() => {
        act(() => {
          result.current.updateErroredMessageModel('no-such-id', 'qwen2.5:7b', {
            requiredBytes: 8 * 1024 ** 3,
            availableBytes: 4 * 1024 ** 3,
          });
        });
      }).not.toThrow();

      expect(result.current.messages).toEqual(before);
    });
  });
});
