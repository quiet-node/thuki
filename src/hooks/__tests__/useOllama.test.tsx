import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
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
    it('sends prompt via invoke with correct command name and args', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('hello world', 'hello world');
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({ prompt: 'hello world' }),
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
        void result.current.ask('test prompt', 'test prompt');
      });

      // isGenerating should be true right after ask sets it
      expect(result.current.isGenerating).toBe(true);

      // Cleanup
      act(() => {
        resolveInvoke?.();
      });
    });

    it('adds user message immediately on ask', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('my question', 'my question');
      });

      expect(result.current.messages[0]).toEqual({
        role: 'user',
        content: 'my question',
      });
    });

    it('stores quotedText on user message when provided', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask(
          'what is this?',
          'Context: "code snippet"\n\nwhat is this?',
          'code snippet',
        );
      });

      expect(result.current.messages[0]).toEqual({
        role: 'user',
        content: 'what is this?',
        quotedText: 'code snippet',
      });
    });

    it('sends ollamaPrompt (not displayContent) to invoke', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask(
          'summarize',
          'Context: "selected text"\n\nsummarize',
        );
      });

      expect(invoke).toHaveBeenCalledWith(
        'ask_ollama',
        expect.objectContaining({
          prompt: 'Context: "selected text"\n\nsummarize',
        }),
      );
    });

    it('accumulates streaming tokens into streamingContent', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('hello', 'hello');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Hello' });
        channel!.simulateMessage({ type: 'Token', data: ', world' });
      });

      expect(result.current.streamingContent).toBe('Hello, world');
    });

    it('moves content to messages on Done chunk', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('hello', 'hello');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Hi there' });
        channel!.simulateMessage({ type: 'Done' });
      });

      expect(result.current.streamingContent).toBe('');
      expect(result.current.isGenerating).toBe(false);
      expect(result.current.messages).toContainEqual({
        role: 'assistant',
        content: 'Hi there',
      });
    });

    it('does nothing for empty prompt', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('', '');
      });

      expect(invoke).not.toHaveBeenCalled();
      expect(result.current.messages).toHaveLength(0);
    });

    it('does nothing for whitespace-only prompt', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('   ', '   ');
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
        void result.current.ask('first', 'first');
      });

      expect(result.current.isGenerating).toBe(true);
      const callCountAfterFirst = invoke.mock.calls.length;

      // Try a second ask while generating
      await act(async () => {
        await result.current.ask('second', 'second');
      });

      // invoke should NOT have been called again
      expect(invoke.mock.calls.length).toBe(callCountAfterFirst);

      // Cleanup
      act(() => {
        resolveInvoke?.();
      });
    });
  });

  // ─── Error handling ──────────────────────────────────────────────────────────

  describe('error handling', () => {
    it('sets error on Error chunk, isGenerating becomes false', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('test', 'test');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Error', data: 'model not found' });
      });

      expect(result.current.error).toBe('model not found');
      expect(result.current.isGenerating).toBe(false);
    });

    it('sets error on invoke rejection', async () => {
      invoke.mockRejectedValueOnce(new Error('connection refused'));

      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('test', 'test');
      });

      expect(result.current.error).toBe('Error: connection refused');
      expect(result.current.isGenerating).toBe(false);
    });

    it('appends error to assistant message content on Error chunk', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('test', 'test');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Partial answer' });
        channel!.simulateMessage({ type: 'Error', data: 'timed out' });
      });

      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg?.content).toBe(
        'Partial answer\n\n**Error:** timed out',
      );
    });

    it('appends error to assistant message content on invoke rejection', async () => {
      invoke.mockRejectedValueOnce(new Error('network error'));

      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('test', 'test');
      });

      const assistantMsg = result.current.messages.find(
        (m) => m.role === 'assistant',
      );
      expect(assistantMsg?.content).toBe('\n\n**Error:** Error: network error');
    });

    it('clears previous error on new ask', async () => {
      invoke.mockRejectedValueOnce(new Error('first error'));

      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('first ask', 'first ask');
      });

      expect(result.current.error).toBe('Error: first error');

      // Second ask — succeeds, channel sends Done
      await act(async () => {
        await result.current.ask('second ask', 'second ask');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Done' });
      });

      expect(result.current.error).toBeNull();
    });
  });

  // ─── Streaming edge cases ────────────────────────────────────────────────────

  describe('streaming edge cases', () => {
    it('handles Token with empty string', async () => {
      const { result } = renderHook(() => useOllama());

      await act(async () => {
        await result.current.ask('hello', 'hello');
      });

      const channel = getChannel();
      expect(channel).not.toBeNull();

      act(() => {
        channel!.simulateMessage({ type: 'Token', data: '' });
      });

      // streamingContent should still be empty (no crash)
      expect(result.current.streamingContent).toBe('');
    });
  });

  // ─── reset() ────────────────────────────────────────────────────────────────

  describe('reset()', () => {
    it('clears all state', async () => {
      const { result } = renderHook(() => useOllama());

      // Build up some state
      await act(async () => {
        await result.current.ask('hello', 'hello');
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
      expect(result.current.streamingContent).toBe('');
      expect(result.current.isGenerating).toBe(false);
      expect(result.current.error).toBeNull();
    });
  });

  // ─── History ─────────────────────────────────────────────────────────────────

  describe('history', () => {
    it('maintains message history across multiple sequential asks', async () => {
      const { result } = renderHook(() => useOllama());

      // First ask + response
      await act(async () => {
        await result.current.ask('first question', 'first question');
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
        await result.current.ask('second question', 'second question');
      });
      channel = getChannel();
      act(() => {
        channel!.simulateMessage({ type: 'Token', data: 'Second answer' });
        channel!.simulateMessage({ type: 'Done' });
      });

      expect(result.current.messages).toEqual([
        { role: 'user', content: 'first question' },
        { role: 'assistant', content: 'First answer' },
        { role: 'user', content: 'second question' },
        { role: 'assistant', content: 'Second answer' },
      ]);
    });
  });
});
