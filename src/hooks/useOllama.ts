import { useState, useCallback } from 'react';
import { invoke, Channel } from '@tauri-apps/api/core';

/**
 * Represents a single message in the chat thread.
 */
export interface Message {
  /** Unique identifier for stable React list keys. */
  id: string;
  role: 'user' | 'assistant';
  content: string;
  /** Selected text from the host app that was quoted with this message, if any. */
  quotedText?: string;
  /** Absolute file paths of images attached to this message, if any. */
  imagePaths?: string[];
}

/**
 * The expected structure of streaming chunks emitted from the Rust backend.
 */
export type StreamChunk =
  | { type: 'Token'; data: string }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; data: string };

/**
 * A custom hook that simplifies interactions with the local Ollama LLM.
 * It manages message history, streaming state, and sets up Rust IPC channels.
 *
 * @param onTurnComplete Optional callback invoked after a complete user/assistant
 *   turn (i.e., when the `Done` chunk is received). Receives the user message
 *   and the finalized assistant message. Not called on `Cancelled` or `Error`.
 *   Used by the caller to persist completed turns to SQLite.
 * @returns An object containing the message history, a submit callback function, and operational states.
 */
export function useOllama(
  onTurnComplete?: (userMsg: Message, assistantMsg: Message) => void,
) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [streamingContent, setStreamingContent] = useState('');
  const [isGenerating, setIsGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  /**
   * Submits a message to the Ollama backend and initiates the streaming response.
   * The backend manages conversation history — only the new user message is sent.
   *
   * Avoids continuous array copy operations during streaming by maintaining the streaming
   * chunk state separately from the main messages state until generation finishes.
   *
   * @param displayContent The user's query as it should appear in the chat bubble.
   * @param quotedText Optional selected text quoted alongside this message.
   * @param imagePaths Optional array of absolute file paths for attached images.
   */
  const ask = useCallback(
    async (
      displayContent: string,
      quotedText?: string,
      imagePaths?: string[],
    ) => {
      if (
        (!displayContent.trim() && (!imagePaths || imagePaths.length === 0)) ||
        isGenerating
      )
        return;

      const userMsg: Message = {
        id: crypto.randomUUID(),
        role: 'user',
        content: displayContent,
        quotedText,
        imagePaths:
          imagePaths && imagePaths.length > 0 ? imagePaths : undefined,
      };

      setMessages((prev) => [...prev, userMsg]);
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
          const assistantMsg: Message = {
            id: crypto.randomUUID(),
            role: 'assistant',
            content: currentContent,
          };
          setMessages((prev) => [...prev, assistantMsg]);
          setStreamingContent('');
          setIsGenerating(false);
          // Notify the caller that a complete turn has finished so it can
          // persist both messages to SQLite if the conversation is saved.
          onTurnComplete?.(userMsg, assistantMsg);
        } else if (chunk.type === 'Cancelled') {
          // Finalize partial content as a complete message so the user
          // retains everything generated before they hit stop.
          if (currentContent) {
            setMessages((prev) => [
              ...prev,
              {
                id: crypto.randomUUID(),
                role: 'assistant',
                content: currentContent,
              },
            ]);
          }
          setStreamingContent('');
          setIsGenerating(false);
        } else {
          setError(chunk.data);
          setMessages((prev) => [
            ...prev,
            {
              id: crypto.randomUUID(),
              role: 'assistant',
              content: currentContent + '\n\n**Error:** ' + chunk.data,
            },
          ]);
          setStreamingContent('');
          setIsGenerating(false);
        }
      };

      try {
        await invoke('ask_ollama', {
          message: displayContent,
          quotedText: quotedText ?? null,
          imagePaths: imagePaths && imagePaths.length > 0 ? imagePaths : null,
          onEvent: channel,
        });
      } catch (err) {
        setError(String(err));
        setMessages((prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: 'assistant',
            content: currentContent + '\n\n**Error:** ' + String(err),
          },
        ]);
        setStreamingContent('');
        setIsGenerating(false);
      }
    },
    [isGenerating, onTurnComplete],
  );

  /** Cancels the currently active generation by signalling the Rust backend. */
  const cancel = useCallback(async () => {
    if (!isGenerating) return;
    await invoke('cancel_generation');
  }, [isGenerating]);

  /** Resets all conversation state to prepare for a fresh session. */
  const reset = useCallback(() => {
    setMessages([]);
    setStreamingContent('');
    setIsGenerating(false);
    setError(null);
    void invoke('reset_conversation');
  }, []);

  /**
   * Replaces the current message list with a previously loaded set of messages.
   *
   * Called after `load_conversation` returns from the backend (which already
   * synced the Rust `ConversationHistory`). Does NOT call `reset_conversation`
   * to avoid conflicting with the epoch bump performed by `load_conversation`.
   *
   * @param msgs The complete message array to load into React state.
   */
  const loadMessages = useCallback((msgs: Message[]) => {
    setMessages(msgs);
    setStreamingContent('');
    setIsGenerating(false);
    setError(null);
  }, []);

  return {
    messages,
    streamingContent,
    ask,
    cancel,
    isGenerating,
    error,
    reset,
    loadMessages,
  };
}
