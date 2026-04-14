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
      const { result } = renderHook(() => useOllama());

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
            // Stall — never resolves until we manually resolve
            return new Promise<void>((res) => {
              resolveInvoke = res;
            });
          }
        },
      );

      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('');
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('does nothing for whitespace-only prompt', async () => {
      const { result } = renderHook(() => useOllama());

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

      const { result } = renderHook(() => useOllama());

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
  });

  // ─── imagePaths handling ─────────────────────────────────────────────────────

  describe('imagePaths handling', () => {
    it('allows ask() with empty text but valid imagePaths', async () => {
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('', undefined, undefined);
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('returns early for empty text AND empty imagePaths array', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('', undefined, []);
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('includes imagePaths in message and invoke when text AND imagePaths are provided', async () => {
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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

      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('test');
      });

      expect(result.current.isGenerating).toBe(false);
    });

    it('Error chunk updates assistant placeholder with errorKind', async () => {
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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

      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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

      const { result } = renderHook(() => useOllama());

      act(() => {
        void result.current.ask('hello');
      });

      expect(result.current.isGenerating).toBe(true);

      await act(async () => {
        await result.current.cancel();
      });

      expect(invoke).toHaveBeenCalledWith('cancel_generation');

      act(() => {
        resolveInvoke?.();
      });
    });

    it('does nothing when not generating', async () => {
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('hello');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Cancelled' });
      });

      expect(result.current.isGenerating).toBe(false);
      // Only the user message should exist — empty assistant placeholder was removed
      expect(result.current.messages).toHaveLength(1);
      expect(result.current.messages[0].role).toBe('user');
    });
  });

  // ─── reset() ────────────────────────────────────────────────────────────────

  describe('reset()', () => {
    it('clears all state', async () => {
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama(onTurnComplete));

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
      const { result } = renderHook(() => useOllama(onTurnComplete));

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
      const { result } = renderHook(() => useOllama(onTurnComplete));

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

  // ─── loadMessages() ──────────────────────────────────────────────────────────

  describe('loadMessages()', () => {
    it('replaces messages state with provided array', async () => {
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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
    it('accumulates ThinkingTokens into thinkingContent', async () => {
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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

    it('tracks thinking duration from first ThinkingToken to first Token', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('hello', undefined, undefined, true);
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({
          type: 'ThinkingToken',
          data: 'reasoning',
        });
        channel!.simulateMessage({ type: 'Token', data: 'answer' });
      });

      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg?.thinkingDurationMs).toBeGreaterThanOrEqual(0);
      expect(assistantMsg?.thinkingDurationMs).toBeDefined();
    });

    it('includes thinkingContent and thinkingDurationMs in onTurnComplete on Done', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useOllama(onTurnComplete));

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
      expect(assistantMsg.thinkingDurationMs).toBeGreaterThanOrEqual(0);
    });

    it('does not set thinkingContent/thinkingDurationMs when no thinking happened', async () => {
      const onTurnComplete = vi.fn();
      const { result } = renderHook(() => useOllama(onTurnComplete));

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
      expect(assistantMsg.thinkingDurationMs).toBeUndefined();
    });

    it('preserves thinking content when cancelled with thinking but no regular tokens', async () => {
      const { result } = renderHook(() => useOllama());

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
      const { result } = renderHook(() => useOllama());

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
});
