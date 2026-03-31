import { useState, useCallback } from 'react';
import { invoke, Channel } from '@tauri-apps/api/core';

/**
 * Represents a single message in the chat thread.
 */
export interface Message {
  role: 'user' | 'assistant';
  content: string;
}

/**
 * The expected structure of streaming chunks emitted from the Rust backend.
 */
export type StreamChunk =
  | { type: 'Token'; data: string }
  | { type: 'Done' }
  | { type: 'Error'; data: string };

/**
 * Custom hook to abstract the interaction with the local Ollama LLM.
 * Manages the message history, streaming state, and Rust IPC channels.
 *
 * @returns Object containing the message history, submit callback, and operational states.
 */
export function useOllama() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [streamingContent, setStreamingContent] = useState('');
  const [isGenerating, setIsGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  /**
   * Submits a prompt to the Ollama backend and initiates the streaming response.
   * Modifies local state optimistically.
   *
   * Avoids continuous array copy operations during streaming by maintaining the streaming
   * chunk state separately from the main messages state until generation finishes.
   *
   * @param prompt The user's input string.
   */
  const ask = useCallback(
    async (prompt: string) => {
      if (!prompt.trim() || isGenerating) return;

      setMessages((prev) => [...prev, { role: 'user', content: prompt }]);
      setStreamingContent('');
      setIsGenerating(true);
      setError(null);

      const channel = new Channel<StreamChunk>();
      // Use block-scoped variable to accumulate the stream and occasionally flush to React state,
      // mitigating rendering lag from hundreds of fast chunk events.
      let currentContent = '';

      channel.onmessage = (chunk) => {
        if (chunk.type === 'Token') {
          currentContent += chunk.data;
          setStreamingContent(currentContent);
        } else if (chunk.type === 'Done') {
          setMessages((prev) => [
            ...prev,
            { role: 'assistant', content: currentContent },
          ]);
          setStreamingContent('');
          setIsGenerating(false);
        } else {
          setError(chunk.data);
          setMessages((prev) => [
            ...prev,
            {
              role: 'assistant',
              content: currentContent + '\n\n**Error:** ' + chunk.data,
            },
          ]);
          setStreamingContent('');
          setIsGenerating(false);
        }
      };

      try {
        await invoke('ask_ollama', { prompt, onEvent: channel });
      } catch (err) {
        setError(String(err));
        setMessages((prev) => [
          ...prev,
          {
            role: 'assistant',
            content: currentContent + '\n\n**Error:** ' + String(err),
          },
        ]);
        setStreamingContent('');
        setIsGenerating(false);
      }
    },
    [isGenerating],
  );

  /** Resets all conversation state to prepare for a fresh session. */
  const reset = useCallback(() => {
    setMessages([]);
    setStreamingContent('');
    setIsGenerating(false);
    setError(null);
  }, []);

  return { messages, streamingContent, ask, isGenerating, error, reset };
}
