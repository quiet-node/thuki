import { useState, useCallback } from 'react';
import { invoke, Channel } from '@tauri-apps/api/core';

/** Mirrors the Rust OllamaErrorKind enum sent over IPC. */
export type OllamaErrorKind = 'NotRunning' | 'ModelNotFound' | 'Other';

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
  /** Present on assistant messages that represent an Ollama error callout. */
  errorKind?: OllamaErrorKind;
  /** Accumulated thinking/reasoning content from the model, if thinking mode was used. */
  thinkingContent?: string;
}

/**
 * The expected structure of streaming chunks emitted from the Rust backend.
 */
export type StreamChunk =
  | { type: 'Token'; data: string }
  | { type: 'ThinkingToken'; data: string }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; data: { kind: OllamaErrorKind; message: string } };

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
  const [isGenerating, setIsGenerating] = useState(false);

  /**
   * Submits a message to the Ollama backend and initiates the streaming response.
   * The backend manages conversation history — only the new user message is sent.
   *
   * Streams tokens directly into the messages array. An empty assistant placeholder
   * is added immediately, then updated in-place on each token until generation finishes.
   *
   * @param displayContent The user's query as it should appear in the chat bubble.
   * @param quotedText Optional selected text quoted alongside this message.
   * @param imagePaths Optional array of absolute file paths for attached images.
   * @param think When true, enables Ollama's thinking/reasoning mode.
   * @param promptOverride When provided, sent to the backend as the actual message
   *   instead of displayContent. The chat bubble still shows displayContent.
   *   Used by utility slash commands to send a composed prompt template while
   *   displaying the user's original input.
   */
  const ask = useCallback(
    async (
      displayContent: string,
      quotedText?: string,
      imagePaths?: string[],
      think?: boolean,
      promptOverride?: string,
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

      const assistantId = crypto.randomUUID();
      const assistantMsg: Message = {
        id: assistantId,
        role: 'assistant',
        content: '',
      };

      setMessages((prev) => [...prev, userMsg, assistantMsg]);
      setIsGenerating(true);

      const channel = new Channel<StreamChunk>();
      let currentContent = '';
      let currentThinkingContent = '';

      channel.onmessage = (chunk) => {
        if (chunk.type === 'ThinkingToken') {
          currentThinkingContent += chunk.data;
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? { ...m, thinkingContent: currentThinkingContent }
                : m,
            ),
          );
        } else if (chunk.type === 'Token') {
          currentContent += chunk.data;
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId ? { ...m, content: currentContent } : m,
            ),
          );
        } else if (chunk.type === 'Done') {
          setIsGenerating(false);
          // Notify the caller that a complete turn has finished so it can
          // persist both messages to SQLite if the conversation is saved.
          onTurnComplete?.(userMsg, {
            ...assistantMsg,
            content: currentContent,
            thinkingContent: currentThinkingContent || undefined,
          });
        } else if (chunk.type === 'Cancelled') {
          // Remove the empty assistant placeholder if nothing was generated.
          if (!currentContent && !currentThinkingContent) {
            setMessages((prev) => prev.filter((m) => m.id !== assistantId));
          }
          setIsGenerating(false);
        } else {
          // Replace the streaming placeholder with an error message.
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? {
                    ...m,
                    content: chunk.data.message,
                    errorKind: chunk.data.kind,
                  }
                : m,
            ),
          );
          setIsGenerating(false);
        }
      };

      try {
        await invoke('ask_ollama', {
          message: promptOverride ?? displayContent,
          quotedText: quotedText ?? null,
          imagePaths: imagePaths && imagePaths.length > 0 ? imagePaths : null,
          think: think ?? false,
          onEvent: channel,
        });
      } catch {
        setMessages((prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: 'assistant',
            content: 'Something went wrong\nCould not reach Ollama.',
            errorKind: 'Other' as const,
          },
        ]);
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
    setIsGenerating(false);
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
    setIsGenerating(false);
  }, []);

  return {
    messages,
    ask,
    cancel,
    isGenerating,
    reset,
    loadMessages,
  };
}
