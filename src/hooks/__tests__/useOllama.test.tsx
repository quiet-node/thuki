import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useOllama } from '../useOllama';
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

describe('useOllama', () => {
  beforeEach(() => {
    invoke.mockClear();
    enableChannelCapture();
    resetChannelCapture();
  });

  // ─── ask() ──────────────────────────────────────────────────────────────────

  describe('ask()', () => {
    it('sends message via invoke with correct command name and args', async () => {
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('hello world');
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
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

      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('summarize', 'selected text');
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: 'summarize',
          quotedText: 'selected text',
        }),
      );
    });

    it('accumulates streaming tokens into the assistant message', async () => {
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('');
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('does nothing for whitespace-only prompt', async () => {
      const { result } = renderHook(() => useOllama(''));

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

      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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
        'ask_ollama',
        expect.objectContaining({
          message: 'composed prompt for model',
        }),
      );

      // User message in state shows displayContent, not the override.
      const userMsg = result.current.messages.find((m) => m.role === 'user');
      expect(userMsg?.content).toBe('user visible text');
    });

    it('sends displayContent as message when no promptOverride provided', async () => {
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('hello world');
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          message: 'hello world',
        }),
      );
    });

    it('sends displayContent when promptOverride is undefined', async () => {
      const { result } = renderHook(() => useOllama(''));

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
        'ask_ollama',
        expect.objectContaining({
          message: 'hello world',
        }),
      );
    });
  });

  // ─── imagePaths handling ─────────────────────────────────────────────────────

  describe('imagePaths handling', () => {
    it('allows ask() with empty text but valid imagePaths', async () => {
      const { result } = renderHook(() => useOllama(''));

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
        'ask_ollama',
        expect.objectContaining({
          message: '',
          imagePaths: ['/tmp/img1.jpg'],
        }),
      );
    });

    it('returns early for empty text AND no imagePaths', async () => {
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('', undefined, undefined);
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('returns early for empty text AND empty imagePaths array', async () => {
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('', undefined, []);
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('includes imagePaths in message and invoke when text AND imagePaths are provided', async () => {
      const { result } = renderHook(() => useOllama(''));

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
        'ask_ollama',
        expect.objectContaining({
          message: 'describe this',
          imagePaths: ['/tmp/img1.jpg', '/tmp/img2.jpg'],
        }),
      );
    });

    it('sets message.imagePaths to undefined and invoke imagePaths to null when no imagePaths', async () => {
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('hello');
      });

      expect(result.current.messages[0].imagePaths).toBeUndefined();
      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          imagePaths: null,
        }),
      );
    });
  });

  // ─── Error handling ──────────────────────────────────────────────────────────

  describe('error handling', () => {
    it('Error chunk sets isGenerating to false', async () => {
      const { result } = renderHook(() => useOllama(''));

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

      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('test');
      });

      expect(result.current.isGenerating).toBe(false);
    });

    it('Error chunk updates assistant placeholder with errorKind', async () => {
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('test');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({
          type: 'Error',
          data: {
            kind: 'NotRunning',
            message: "Ollama isn't running\nStart Ollama and try again.",
          },
        });
      });

      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg?.errorKind).toBe('NotRunning');
      expect(assistantMsg?.content).toBe(
        "Ollama isn't running\nStart Ollama and try again.",
      );
    });

    it('Error chunk with partial tokens replaces content with error', async () => {
      const { result } = renderHook(() => useOllama(''));

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

      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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

        if (cmd === 'ask_ollama') {
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

      const { result } = renderHook(() => useOllama(''));

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
        if (cmd === 'ask_ollama') {
          channel = args?.onEvent as ReturnType<typeof getChannel>;
          return new Promise<void>((_, reject) => {
            rejectInvoke = reject;
          });
        }
      });

      const { result } = renderHook(() => useOllama(''));

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

      const { result } = renderHook(() => useOllama(''));

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

      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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
  });

  // ─── onTurnComplete callback ─────────────────────────────────────────────────

  describe('onTurnComplete callback', () => {
    it('is called with user and assistant messages on Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useOllama('', onTurnComplete));

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
      const { result } = renderHook(() => useOllama('', onTurnComplete));

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
      const { result } = renderHook(() => useOllama('', onTurnComplete));

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
        useOllama('gemma4:e2b', onTurnComplete),
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
      const { result } = renderHook(() => useOllama(null, onTurnComplete));

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
        useOllama('qwen2.5:7b', onTurnComplete),
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
      const { result } = renderHook(() => useOllama(null, onTurnComplete));

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
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('fail');
      });
      expect(result.current.isGenerating).toBe(false);

      act(() => {
        result.current.loadMessages([]);
      });

      expect(result.current.isGenerating).toBe(false);
    });
  });

  // ─── ThinkingToken handling ──────────────────────────────────────────────────

  describe('ThinkingToken handling', () => {
    it('marks the assistant placeholder as a /think turn when think is true', async () => {
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('hello', undefined, undefined, true);
      });

      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg?.fromThink).toBe(true);
    });

    it('accumulates ThinkingTokens into thinkingContent', async () => {
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('hello', undefined, undefined, true);
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          think: true,
        }),
      );
    });

    it('passes think as false by default', async () => {
      const { result } = renderHook(() => useOllama(''));

      await act(async () => {
        await result.current.ask('hello');
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          think: false,
        }),
      );
    });

    it('includes thinkingContent in onTurnComplete on Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useOllama('', onTurnComplete));

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
      const { result } = renderHook(() => useOllama('', onTurnComplete));

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
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama(''));
      let outcome: { final: boolean } | undefined;
      await act(async () => {
        outcome = await result.current.askSearch('   ');
      });
      expect(outcome).toEqual({ final: true });
      expect(invoke).not.toHaveBeenCalled();
    });

    it('resolves with final=true when a token is received followed by Done', async () => {
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama('', onTurnComplete));
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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama(''));
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

      expect(
        result.current.messages.find((message) => message.role === 'assistant')
          ?.searchTraces,
      ).toBeUndefined();
    });

    it('drops the empty placeholder on Cancelled with no content', async () => {
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama('', onTurnComplete));
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
      const { result } = renderHook(() => useOllama(''));
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

      const { result } = renderHook(() => useOllama(''));

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
      const { result } = renderHook(() => useOllama(''));
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

      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama('', onTurnComplete));
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
      const { result } = renderHook(() => useOllama('', onTurnComplete));
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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama('', onTurnComplete));
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
      const { result } = renderHook(() => useOllama('', onTurnComplete));
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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama('', onTurnComplete));
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
      const { result } = renderHook(() => useOllama('', onTurnComplete));
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

    it('searchTraces is undefined when no Trace event is received', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useOllama('', onTurnComplete));
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
      expect(last.searchTraces).toBeUndefined();
    });
  });

  // ─── reset/loadMessages interaction with searchStage ────────────────────────

  describe('search state cleanup', () => {
    it('reset clears the search stage indicator', async () => {
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama(''));
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
      const { result } = renderHook(() => useOllama('', onTurnComplete));
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
      const { result } = renderHook(() => useOllama(''));
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
  });
});
