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

          if (cmd === 'search_pipeline') {
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
        channel!.simulateMessage({
          type: 'Trace',
          step: {
            id: 'search',
            kind: 'search',
            status: 'running',
            title: 'Searching the web',
            summary: 'Looking for public pages that can answer the question.',
          },
        });
        channel!.simulateMessage({ type: 'Token', content: 'late answer' });
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
        channel!.simulateMessage({ type: 'Token', content: 'answer' });
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
        channel!.simulateMessage({ type: 'Token', content: 'answer' });
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

  // ─── askSearch() ────────────────────────────────────────────────────────────

  describe('askSearch()', () => {
    it('invokes search_pipeline with the trimmed query', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('  rust async  ');
      });
      expect(invoke).toHaveBeenCalledWith(
        'search_pipeline',
        expect.objectContaining({ message: 'rust async' }),
      );
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', content: 'ok' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
    });

    it('stores quotedText on the /search user message when provided', async () => {
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
        channel!.simulateMessage({ type: 'Token', content: 'ok' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
    });

    it('resolves immediately with final=true on empty query', async () => {
      const { result } = renderHook(() => useModel(''));
      let outcome: { final: boolean } | undefined;
      await act(async () => {
        outcome = await result.current.askSearch('   ');
      });
      expect(outcome).toEqual({ final: true });
      expect(invoke).not.toHaveBeenCalled();
    });

    it('resolves with final=true when a token is received followed by Done', async () => {
      const { result } = renderHook(() => useModel(''));
      const metadata = {
        iterations: [
          {
            stage: { kind: 'initial' as const },
            queries: ['q'],
            urls_fetched: ['https://example.com/a'],
            reader_empty_urls: [],
            judge_verdict: 'sufficient' as const,
            judge_reasoning: 'enough evidence',
            duration_ms: 12,
          },
        ],
        total_duration_ms: 12,
        retries_performed: 0,
      };
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'AnalyzingQuery' });
        channel!.simulateMessage({ type: 'Searching', queries: [] });
        channel!.simulateMessage({ type: 'Token', content: 'hello' });
        channel!.simulateMessage({ type: 'Done', metadata });
      });
      let outcome: { final: boolean } | undefined;
      await act(async () => {
        outcome = await pending;
      });
      expect(outcome).toEqual({ final: true });
      expect(result.current.isGenerating).toBe(false);
      expect(result.current.searchStage).toBeNull();
      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.role).toBe('assistant');
      expect(last.content).toBe('hello');
      expect(last.fromSearch).toBe(true);
      expect(last.searchMetadata).toEqual(metadata);
    });

    it('resolves with final=false when a clarify trace is followed by question tokens and Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('who is him');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Trace',
          step: {
            id: 'clarify',
            kind: 'clarify',
            status: 'completed',
            title: 'Waiting for clarification',
            summary: 'Search is paused until you clarify who or what you mean.',
          },
        });
        channel!.simulateMessage({ type: 'Token', content: 'Which person?' });
        channel!.simulateMessage({ type: 'Done' });
      });

      let outcome: { final: boolean } | undefined;
      await act(async () => {
        outcome = await pending;
      });

      expect(outcome).toEqual({ final: false });
      expect(onTurnComplete).toHaveBeenCalledTimes(1);
      expect(
        result.current.messages[result.current.messages.length - 1],
      ).toMatchObject({
        role: 'assistant',
        content: 'Which person?',
      });
    });

    it('updates searchStage through the pipeline phases', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'AnalyzingQuery' });
      });
      expect(result.current.searchStage).toEqual({ kind: 'analyzing_query' });
      act(() => {
        channel!.simulateMessage({ type: 'Searching', queries: [] });
      });
      expect(result.current.searchStage).toEqual({ kind: 'searching' });
      act(() => {
        channel!.simulateMessage({ type: 'ReadingSources' });
      });
      expect(result.current.searchStage).toEqual({ kind: 'reading_sources' });
      act(() => {
        channel!.simulateMessage({
          type: 'RefiningSearch',
          attempt: 1,
          total: 3,
        });
      });
      expect(result.current.searchStage).toEqual({
        kind: 'refining_search',
        attempt: 1,
        total: 3,
      });
      act(() => {
        channel!.simulateMessage({ type: 'Composing' });
      });
      // RefiningSearch was seen above, so subsequent stages carry gap: true.
      expect(result.current.searchStage).toEqual({
        kind: 'composing',
        gap: true,
      });
      act(() => {
        channel!.simulateMessage({ type: 'Token', content: 'x' });
      });
      expect(result.current.searchStage).toBeNull();
      act(() => {
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
    });

    it('handles FetchingUrl, finalizes traces on IterationComplete, and ignores empty tokens', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;

      await act(async () => {
        pending = result.current.askSearch('q');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Trace',
          step: {
            id: 'round-1-read',
            kind: 'read',
            status: 'running',
            round: 1,
            title: 'Reading the shortlisted pages',
            summary: 'Opened 1 of 2 pages so far.',
            counts: { processed: 1, total: 2 },
          },
        });
        channel!.simulateMessage({
          type: 'FetchingUrl',
          url: 'https://example.com/page',
        });
      });

      expect(result.current.searchStage).toEqual({ kind: 'reading_sources' });

      act(() => {
        channel!.simulateMessage({
          type: 'IterationComplete',
          trace: {
            stage: { kind: 'initial' },
            queries: ['q'],
            urls_fetched: ['https://example.com/page'],
            reader_empty_urls: [],
            judge_verdict: 'partial',
            judge_reasoning: 'needs more evidence',
            duration_ms: 10,
          },
        });
      });

      const assistantAfterIteration = result.current.messages.find(
        (message) => message.role === 'assistant',
      );
      expect(assistantAfterIteration?.searchTraces?.[0]).toEqual(
        expect.objectContaining({ status: 'completed' }),
      );

      act(() => {
        channel!.simulateMessage({ type: 'Token', content: '' });
        channel!.simulateMessage({ type: 'Done' });
      });

      await act(async () => {
        await expect(pending).resolves.toEqual({ final: false });
      });
    });

    it('ignores IterationComplete events when no trace steps have started', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;

      await act(async () => {
        pending = result.current.askSearch('q');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'IterationComplete',
          trace: {
            stage: { kind: 'initial' },
            queries: ['q'],
            urls_fetched: [],
            reader_empty_urls: [],
            judge_verdict: 'partial',
            judge_reasoning: 'needs more evidence',
            duration_ms: 10,
          },
        });
        channel!.simulateMessage({ type: 'Done' });
      });

      await act(async () => {
        await expect(pending).resolves.toEqual({ final: false });
      });

      // Agentic path stamps searchTraces: [] at turn start; no Trace events
      // means it stays an empty list (not undefined).
      expect(
        result.current.messages.find((message) => message.role === 'assistant')
          ?.searchTraces,
      ).toEqual([]);
    });

    it('drops the empty placeholder on Cancelled with no content', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Cancelled' });
      });
      let outcome: { final: boolean } | undefined;
      await act(async () => {
        outcome = await pending;
      });
      expect(outcome).toEqual({ final: true });
      expect(
        result.current.messages.filter((m) => m.role === 'assistant'),
      ).toHaveLength(0);
    });

    it('keeps partial content on Cancelled after tokens arrived', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', content: 'part' });
        channel!.simulateMessage({ type: 'Cancelled' });
      });
      await act(async () => {
        await pending;
      });
      const assistant = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistant?.content).toBe('part');
    });

    it('renders an Error event as an error bubble', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          message: "Ollama isn't running",
        });
      });
      let outcome: { final: boolean } | undefined;
      await act(async () => {
        outcome = await pending;
      });
      expect(outcome).toEqual({ final: true });
      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.content).toBe("Ollama isn't running");
      expect(last.errorKind).toBe('Other');
      expect(onTurnComplete).not.toHaveBeenCalled();
    });

    it('guards against concurrent invocations', async () => {
      const { result } = renderHook(() => useModel(''));
      let firstPending!: Promise<{ final: boolean }>;
      await act(async () => {
        firstPending = result.current.askSearch('first');
      });
      expect(invoke).toHaveBeenCalledTimes(1);
      let secondOutcome: { final: boolean } | undefined;
      await act(async () => {
        secondOutcome = await result.current.askSearch('second');
      });
      expect(secondOutcome).toEqual({ final: true });
      expect(invoke).toHaveBeenCalledTimes(1);
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await firstPending;
      });
    });

    it('waits for a pending cancel before restarting search and only resumes one queued request', async () => {
      let latestChannel: ReturnType<typeof getChannel> = null;
      let resolveFirstSearchInvoke!: () => void;
      let resolveCancel!: () => void;
      const searchMessages: string[] = [];

      invoke.mockImplementation(async (cmd, args) => {
        if (args && 'onEvent' in args) {
          latestChannel = args.onEvent as ReturnType<typeof getChannel>;
        }

        if (cmd === 'search_pipeline') {
          searchMessages.push(String(args?.message ?? ''));
          if (searchMessages.length === 1) {
            return new Promise<void>((resolve) => {
              resolveFirstSearchInvoke = resolve;
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

      let firstPending!: Promise<{ final: boolean }>;
      let secondPending!: Promise<{ final: boolean }>;
      let thirdPending!: Promise<{ final: boolean }>;

      act(() => {
        firstPending = result.current.askSearch('first');
      });

      act(() => {
        void result.current.cancel();
        secondPending = result.current.askSearch('second');
        thirdPending = result.current.askSearch('third');
      });

      expect(searchMessages).toEqual(['first']);

      await act(async () => {
        resolveCancel();
        await Promise.resolve();
        await Promise.resolve();
      });

      expect(searchMessages).toHaveLength(2);
      expect(['second', 'third']).toContain(searchMessages[1]);

      act(() => {
        latestChannel!.simulateMessage({ type: 'Done' });
      });

      await act(async () => {
        await expect(firstPending).resolves.toEqual({ final: true });
        await expect(secondPending).resolves.toEqual({ final: false });
        await expect(thirdPending).resolves.toEqual({ final: true });
      });

      act(() => {
        resolveFirstSearchInvoke();
      });
    });

    it('surfaces a synthetic error when invoke rejects', async () => {
      invoke.mockImplementationOnce(async () => {
        throw new Error('ipc failed');
      });
      const { result } = renderHook(() => useModel(''));
      let outcome: { final: boolean } | undefined;
      await act(async () => {
        outcome = await result.current.askSearch('q');
      });
      expect(outcome).toEqual({ final: true });
      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.errorKind).toBe('Other');
      expect(last.content).toContain('Could not start search');
    });

    it('ignores a late search_pipeline rejection after cancellation', async () => {
      let rejectSearch!: (error: Error) => void;
      let resolveCancel!: () => void;

      invoke.mockImplementation(async (cmd, args) => {
        if (cmd === 'search_pipeline') {
          return new Promise<void>((_, reject) => {
            rejectSearch = reject;
          });
        }

        if (cmd === 'cancel_generation') {
          return new Promise<void>((resolve) => {
            resolveCancel = resolve;
          });
        }

        if (args && 'onEvent' in args) {
          return;
        }
      });

      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;

      act(() => {
        pending = result.current.askSearch('q');
      });

      act(() => {
        void result.current.cancel();
      });

      await act(async () => {
        resolveCancel();
        await expect(pending).resolves.toEqual({ final: true });
      });

      expect(result.current.messages).toHaveLength(1);

      await act(async () => {
        rejectSearch(new Error('late fail'));
        await Promise.resolve();
      });

      expect(result.current.messages).toHaveLength(1);
      expect(
        result.current.messages.find((message) => message.role === 'assistant'),
      ).toBeUndefined();
    });

    it('does not persist an empty turn on Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
      // No tokens: nothing to persist. Done resolves as final=false (sawToken is false).
      expect(onTurnComplete).not.toHaveBeenCalled();
    });

    it('persists searchSources to the assistant message on Sources + Token + Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      const metadata = {
        iterations: [
          {
            stage: { kind: 'initial' as const },
            queries: ['q'],
            urls_fetched: ['https://rust-lang.org'],
            reader_empty_urls: [],
            judge_verdict: 'sufficient' as const,
            judge_reasoning: 'enough evidence',
            duration_ms: 30,
          },
        ],
        total_duration_ms: 30,
        retries_performed: 0,
      };
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Sources',
          results: [
            { title: 'Rust', url: 'https://rust-lang.org' },
            { title: 'Tokio', url: 'https://tokio.rs' },
          ],
        });
        channel!.simulateMessage({ type: 'Token', content: 'answer' });
        channel!.simulateMessage({ type: 'Done', metadata });
      });
      await act(async () => {
        await pending;
      });
      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.searchSources).toHaveLength(2);
      expect(assistantMsg.searchSources[0].url).toBe('https://rust-lang.org');
      expect(assistantMsg.searchMetadata).toEqual(metadata);
      const lastMsg =
        result.current.messages[result.current.messages.length - 1];
      expect(lastMsg.searchSources).toHaveLength(2);
      expect(lastMsg.searchMetadata).toEqual(metadata);
    });

    it('Warning event accumulates into message.searchWarnings while streaming continues', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Searching', queries: [] });
        channel!.simulateMessage({
          type: 'Warning',
          warning: 'reader_unavailable',
        });
        channel!.simulateMessage({ type: 'Token', content: 'ok' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.content).toBe('ok');
      expect(last.searchWarnings).toEqual(['reader_unavailable']);
    });

    it('askSearch accumulates warnings from Warning events into the persisted turn', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'AnalyzingQuery' });
        channel!.simulateMessage({ type: 'Searching', queries: [] });
        channel!.simulateMessage({
          type: 'Sources',
          results: [{ title: 'A', url: 'https://a.com' }],
        });
        channel!.simulateMessage({ type: 'ReadingSources' });
        channel!.simulateMessage({
          type: 'Warning',
          warning: 'reader_unavailable',
        });
        channel!.simulateMessage({ type: 'Composing' });
        channel!.simulateMessage({ type: 'Token', content: 'answer' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
      expect(onTurnComplete).toHaveBeenCalledOnce();
      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.searchWarnings).toEqual(['reader_unavailable']);
    });

    it('askSearch passes multiple warnings through in order', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Warning',
          warning: 'reader_unavailable',
        });
        channel!.simulateMessage({
          type: 'Warning',
          warning: 'iteration_cap_exhausted',
        });
        channel!.simulateMessage({ type: 'Token', content: 'answer' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
      expect(onTurnComplete).toHaveBeenCalledOnce();
      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.searchWarnings).toEqual([
        'reader_unavailable',
        'iteration_cap_exhausted',
      ]);
    });

    it('Trace events accumulate steps on the assistant message', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Trace',
          step: {
            id: 'analyze',
            kind: 'analyze',
            status: 'running' as const,
            title: 'Understanding the question',
            summary: 'Deciding whether to search.',
          },
        });
        channel!.simulateMessage({
          type: 'Trace',
          step: {
            id: 'round-1-search',
            kind: 'search',
            status: 'completed' as const,
            round: 1,
            title: 'Searching the web',
            summary: 'Found 8 results across 4 sites.',
            queries: ['q'],
            counts: { found: 8 },
          },
        });
        channel!.simulateMessage({ type: 'Token', content: 'ok' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.searchTraces).toHaveLength(2);
      expect(last.searchTraces![0]).toEqual(
        expect.objectContaining({ id: 'analyze', status: 'completed' }),
      );
      expect(last.searchTraces![1]).toEqual(
        expect.objectContaining({ id: 'round-1-search' }),
      );
    });

    it('Trace updates replace earlier steps with the same id', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Trace',
          step: {
            id: 'round-1-read',
            kind: 'read',
            status: 'running' as const,
            round: 1,
            title: 'Reading the shortlisted pages',
            summary: 'Opened 1 of 3 pages so far.',
            counts: { processed: 1, total: 3 },
          },
        });
        channel!.simulateMessage({
          type: 'Trace',
          step: {
            id: 'round-1-read',
            kind: 'read',
            status: 'running' as const,
            round: 1,
            title: 'Reading the shortlisted pages',
            summary: 'Opened 2 of 3 pages so far.',
            counts: { processed: 2, total: 3 },
          },
        });
      });

      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.searchTraces).toHaveLength(1);
      expect(last.searchTraces![0]).toEqual(
        expect.objectContaining({ summary: 'Opened 2 of 3 pages so far.' }),
      );

      act(() => {
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
    });

    it('Trace events are passed to onTurnComplete', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Trace',
          step: {
            id: 'compose',
            kind: 'compose',
            status: 'running' as const,
            title: 'Synthesizing the answer',
            summary:
              'Pulling the strongest points together into a clear answer with citations.',
            counts: { sources: 2 },
          },
        });
        channel!.simulateMessage({ type: 'Token', content: 'answer' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
      expect(onTurnComplete).toHaveBeenCalledOnce();
      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.searchTraces).toHaveLength(1);
      expect(assistantMsg.searchTraces![0]).toEqual(
        expect.objectContaining({ id: 'compose', status: 'completed' }),
      );
    });

    it('preserves completed traces on Done when no running steps need finalization', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;

      await act(async () => {
        pending = result.current.askSearch('q');
      });

      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'Trace',
          step: {
            id: 'compose',
            kind: 'compose',
            status: 'completed' as const,
            title: 'Synthesizing the answer',
            summary:
              'Pulling the strongest points together into a clear answer with citations.',
            counts: { sources: 2 },
          },
        });
        channel!.simulateMessage({ type: 'Token', content: 'answer' });
        channel!.simulateMessage({ type: 'Done' });
      });

      await act(async () => {
        await pending;
      });

      expect(onTurnComplete).toHaveBeenCalledOnce();
      const [, assistantMsg] = onTurnComplete.mock.calls[0];
      expect(assistantMsg.searchTraces).toEqual([
        expect.objectContaining({ id: 'compose', status: 'completed' }),
      ]);
      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.searchTraces).toEqual([
        expect.objectContaining({ id: 'compose', status: 'completed' }),
      ]);
    });

    it('persists an empty searchTraces list when no Trace event is received', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', content: 'answer' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
      const last = result.current.messages[result.current.messages.length - 1];
      // Empty array keeps the agentic path marker; auto-search leaves the field unset.
      expect(last.searchTraces).toEqual([]);
    });
  });

  // ─── reset/loadMessages interaction with searchStage ────────────────────────

  describe('search state cleanup', () => {
    it('reset clears the search stage indicator', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Searching', queries: [] });
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
        channel!.simulateMessage({ type: 'AnalyzingQuery' });
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

    it('Searching after RefiningSearch sets gap:true stage', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'RefiningSearch',
          attempt: 1,
          total: 3,
        });
        channel!.simulateMessage({ type: 'Searching', queries: [] });
      });
      expect(result.current.searchStage).toEqual({
        kind: 'searching',
        gap: true,
      });
      act(() => {
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
    });

    it('ReadingSources after RefiningSearch sets gap:true stage', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({
          type: 'RefiningSearch',
          attempt: 1,
          total: 3,
        });
        channel!.simulateMessage({ type: 'ReadingSources' });
      });
      expect(result.current.searchStage).toEqual({
        kind: 'reading_sources',
        gap: true,
      });
      act(() => {
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });
    });

    it('SandboxUnavailable event sets sandboxUnavailable on assistant message', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'SandboxUnavailable' });
      });
      let outcome: { final: boolean } | undefined;
      await act(async () => {
        outcome = await pending;
      });
      expect(outcome).toEqual({ final: true });
      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.sandboxUnavailable).toBe(true);
      // onTurnComplete must not be called: no content was produced.
      expect(onTurnComplete).not.toHaveBeenCalled();
    });

    it('SandboxUnavailable event does not set errorKind', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'SandboxUnavailable' });
      });
      await act(async () => {
        await pending;
      });
      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.errorKind).toBeUndefined();
    });

    it('NoModelSelected event renders no-model error and resolves final', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'NoModelSelected' });
      });
      let outcome: { final: boolean } | undefined;
      await act(async () => {
        outcome = await pending;
      });
      expect(outcome).toEqual({ final: true });
      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.errorKind).toBe('NoModelSelected');
      expect(last.content).toBe(
        'No model selected\nPick a model in the picker.',
      );
      expect(onTurnComplete).not.toHaveBeenCalled();
    });

    it('InsufficientMemory event renders the memory-gate error and resolves final', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useModel('', onTurnComplete));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'InsufficientMemory' });
      });
      let outcome: { final: boolean } | undefined;
      await act(async () => {
        outcome = await pending;
      });
      expect(outcome).toEqual({ final: true });
      const last = result.current.messages[result.current.messages.length - 1];
      expect(last.errorKind).toBe('InsufficientMemory');
      expect(last.content).toBe(
        'This model may not fit in memory\nClose some apps, pick a smaller model, or load it anyway.',
      );
      expect(onTurnComplete).not.toHaveBeenCalled();
    });
  });

  // ─── is_first_turn flag retention across pre-ConversationStart bails ────────
  //
  // The chat backend's `ask_model` and the search backend's `search_pipeline`
  // both bail BEFORE recording `ConversationStart` on no-model and (search
  // only) sandbox-unavailable paths. Frontend must keep `isFirstTurnRef`
  // armed across those bails so the next attempt opens the trace correctly.

  describe('is_first_turn flag retention across bails', () => {
    it('chat NoModelSelected error keeps the flag armed for the next turn', async () => {
      const { result } = renderHook(() => useModel(''));
      await act(async () => {
        await result.current.ask('first');
      });
      const channel1 = getChannel();
      act(() => {
        channel1!.simulateMessage({
          type: 'Error',
          data: { kind: 'NoModelSelected', message: 'no model' },
        });
      });
      const firstCall = invoke.mock.calls.find(([cmd]) => cmd === 'ask_model');
      expect(firstCall?.[1]).toMatchObject({ isFirstTurn: true });

      invoke.mockClear();
      await act(async () => {
        await result.current.ask('second');
      });
      const secondCall = invoke.mock.calls.find(([cmd]) => cmd === 'ask_model');
      expect(secondCall?.[1]).toMatchObject({ isFirstTurn: true });
    });

    it('chat TurnAccepted retires the flag for the next turn', async () => {
      const { result } = renderHook(() => useModel(''));
      await act(async () => {
        await result.current.ask('first');
      });
      const channel1 = getChannel();
      act(() => {
        channel1!.simulateMessage({ type: 'TurnAccepted' });
        channel1!.simulateMessage({ type: 'Token', data: 'hi' });
        channel1!.simulateMessage({ type: 'Done' });
      });

      invoke.mockClear();
      await act(async () => {
        await result.current.ask('second');
      });
      const secondCall = invoke.mock.calls.find(([cmd]) => cmd === 'ask_model');
      expect(secondCall?.[1]).toMatchObject({ isFirstTurn: false });
    });

    it('chat TurnAccepted retires the flag even after cancel clears active generation', async () => {
      // Reproduces the cancel-mid-first-turn race: the backend has
      // already recorded `ConversationStart` (and emitted
      // `TurnAccepted`), the user cancels before any token arrives,
      // and a stale `Cancelled` chunk lands after `activeGenerationRef`
      // is cleared. The flag must still retire so the next turn does
      // NOT trigger a duplicate `ConversationStart`.
      const { result } = renderHook(() => useModel(''));
      await act(async () => {
        await result.current.ask('first');
      });
      const channel1 = getChannel();
      act(() => {
        channel1!.simulateMessage({ type: 'TurnAccepted' });
      });
      // Cancel BEFORE any token arrives: clears activeGenerationRef.
      await act(async () => {
        await result.current.cancel();
      });
      // Stale Cancelled chunk arrives after the cancel cleared state.
      act(() => {
        channel1!.simulateMessage({ type: 'Cancelled' });
      });

      invoke.mockClear();
      await act(async () => {
        await result.current.ask('second');
      });
      const secondCall = invoke.mock.calls.find(([cmd]) => cmd === 'ask_model');
      expect(secondCall?.[1]).toMatchObject({ isFirstTurn: false });
    });

    it('search SandboxUnavailable keeps the flag armed for the next turn', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending1!: Promise<{ final: boolean }>;
      await act(async () => {
        pending1 = result.current.askSearch('q1');
      });
      const channel1 = getChannel();
      act(() => {
        channel1!.simulateMessage({ type: 'SandboxUnavailable' });
      });
      await act(async () => {
        await pending1;
      });
      const firstCall = invoke.mock.calls.find(
        ([cmd]) => cmd === 'search_pipeline',
      );
      expect(firstCall?.[1]).toMatchObject({ isFirstTurn: true });

      invoke.mockClear();
      let pending2!: Promise<{ final: boolean }>;
      await act(async () => {
        pending2 = result.current.askSearch('q2');
      });
      const channel2 = getChannel();
      act(() => {
        channel2!.simulateMessage({ type: 'TurnAccepted' });
        channel2!.simulateMessage({ type: 'Token', content: 'ok' });
        channel2!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending2;
      });
      const secondCall = invoke.mock.calls.find(
        ([cmd]) => cmd === 'search_pipeline',
      );
      expect(secondCall?.[1]).toMatchObject({ isFirstTurn: true });
    });

    it('search NoModelSelected keeps the flag armed for the next turn', async () => {
      const { result } = renderHook(() => useModel(''));
      let pending1!: Promise<{ final: boolean }>;
      await act(async () => {
        pending1 = result.current.askSearch('q1');
      });
      const channel1 = getChannel();
      act(() => {
        channel1!.simulateMessage({ type: 'NoModelSelected' });
      });
      await act(async () => {
        await pending1;
      });

      invoke.mockClear();
      let pending2!: Promise<{ final: boolean }>;
      await act(async () => {
        pending2 = result.current.askSearch('q2');
      });
      const channel2 = getChannel();
      act(() => {
        channel2!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending2;
      });
      const secondCall = invoke.mock.calls.find(
        ([cmd]) => cmd === 'search_pipeline',
      );
      expect(secondCall?.[1]).toMatchObject({ isFirstTurn: true });
    });

    it('search TurnAccepted retires the flag even after cancel clears active generation', async () => {
      // Search-side parity for the chat cancel-mid-first-turn race:
      // backend already opened the trace and emitted TurnAccepted, the
      // user cancels before any token arrives, and a stale Cancelled
      // event lands after activeGenerationRef is cleared. The flag
      // must still retire so the next /search does not duplicate
      // ConversationStart.
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('first');
      });
      const channel1 = getChannel();
      act(() => {
        channel1!.simulateMessage({ type: 'TurnAccepted' });
      });
      await act(async () => {
        await result.current.cancel();
      });
      act(() => {
        channel1!.simulateMessage({ type: 'Cancelled' });
      });
      await act(async () => {
        await pending;
      });

      invoke.mockClear();
      let pending2!: Promise<{ final: boolean }>;
      await act(async () => {
        pending2 = result.current.askSearch('second');
      });
      const channel2 = getChannel();
      act(() => {
        channel2!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending2;
      });
      const secondCall = invoke.mock.calls.find(
        ([cmd]) => cmd === 'search_pipeline',
      );
      expect(secondCall?.[1]).toMatchObject({ isFirstTurn: false });
    });

    it('search TurnAccepted retires the flag for a follow-up chat turn (cross-domain)', async () => {
      // The flag is shared across chat and search; once /search opens
      // the trace, a subsequent chat ask() must see is_first_turn=false.
      const { result } = renderHook(() => useModel(''));
      let pending!: Promise<{ final: boolean }>;
      await act(async () => {
        pending = result.current.askSearch('q');
      });
      const channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'TurnAccepted' });
        channel!.simulateMessage({ type: 'AnalyzingQuery' });
        channel!.simulateMessage({ type: 'Done' });
      });
      await act(async () => {
        await pending;
      });

      invoke.mockClear();
      await act(async () => {
        await result.current.ask('chat after search');
      });
      const chatCall = invoke.mock.calls.find(([cmd]) => cmd === 'ask_model');
      expect(chatCall?.[1]).toMatchObject({ isFirstTurn: false });
    });
  });

  // ─── addOcrTurn ──────────────────────────────────────────────────────────────

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
        userMessageId: user.id,
        assistantMessageId: assistant.id,
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
        'search_pipeline',
        expect.objectContaining({
          message: 'rust async',
          displayedContent: 'display text',
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
        channel!.simulateMessage({ type: 'InsufficientMemory' });
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
        channel!.simulateMessage({ type: 'InsufficientMemory' });
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
        'search_pipeline',
        expect.objectContaining({
          message: 'rust async',
          displayedContent: 'display text',
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
        channel!.simulateMessage({ type: 'InsufficientMemory' });
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
