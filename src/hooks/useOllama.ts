import { useState, useCallback } from 'react';
import { invoke, Channel } from '@tauri-apps/api/core';
import type {
  SearchEvent,
  SearchResultPreview,
  SearchStage,
} from '../types/search';

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
  /**
   * Marks an assistant message that was produced through the `/search`
   * pipeline rather than the normal chat path.
   */
  fromSearch?: boolean;
  /**
   * Search result source links forwarded by the pipeline. Shown as a sources
   * footer below the answer so users can visit the original pages.
   */
  searchSources?: SearchResultPreview[];
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
/**
 * Result payload delivered to callers when a `/search` pipeline turn finishes.
 * `final: true` means the answer was fully delivered and the caller should
 * clear the sticky "searchActive" flag.
 */
export interface SearchOutcome {
  final: boolean;
}

export function useOllama(
  onTurnComplete?: (userMsg: Message, assistantMsg: Message) => void,
) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isGenerating, setIsGenerating] = useState(false);
  /** Transient stage indicator for the active `/search` pipeline, if any. */
  const [searchStage, setSearchStage] = useState<SearchStage>(null);

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

  /**
   * Submits a `/search` pipeline turn.
   *
   * @param query Text sent to the backend pipeline (stripped of `/search` trigger).
   * @param displayContent Text shown in the user's chat bubble. Defaults to
   *   `query` when omitted; pass the full original input (with `/search`) so
   *   the bubble reflects exactly what the user typed.
   * @returns `{ final: true }` when the answer was delivered or cancelled.
   */
  const askSearch = useCallback(
    async (query: string, displayContent?: string): Promise<SearchOutcome> => {
      const trimmed = query.trim();
      if (!trimmed || isGenerating) return { final: true };

      const userMsg: Message = {
        id: crypto.randomUUID(),
        role: 'user',
        content: displayContent ?? trimmed,
      };
      const assistantId = crypto.randomUUID();
      const assistantMsg: Message = {
        id: assistantId,
        role: 'assistant',
        content: '',
        fromSearch: true,
      };

      setMessages((prev) => [...prev, userMsg, assistantMsg]);
      setIsGenerating(true);
      setSearchStage(null);

      const channel = new Channel<SearchEvent>();
      let currentContent = '';
      let sawToken = false;
      let pendingSources: SearchResultPreview[] | undefined;
      let errored = false;
      let cancelled = false;

      const updateAssistant = (patch: Partial<Message>) => {
        setMessages((prev) =>
          prev.map((m) => (m.id === assistantId ? { ...m, ...patch } : m)),
        );
      };

      return new Promise<SearchOutcome>((resolve) => {
        const finish = (final: boolean) => {
          setIsGenerating(false);
          setSearchStage(null);
          if (!errored && !cancelled && currentContent) {
            updateAssistant({ searchSources: pendingSources });
            onTurnComplete?.(userMsg, {
              ...assistantMsg,
              content: currentContent,
              searchSources: pendingSources,
            });
          }
          resolve({ final });
        };

        channel.onmessage = (event) => {
          switch (event.type) {
            case 'AnalyzingQuery':
              setSearchStage({ kind: 'analyzing_query' });
              updateAssistant({ content: '' });
              break;
            case 'Searching':
              setSearchStage({ kind: 'searching' });
              break;
            case 'ReadingSources':
              setSearchStage({ kind: 'reading_sources' });
              break;
            case 'RefiningSearch':
              setSearchStage({
                kind: 'refining_search',
                attempt: event.attempt,
                total: event.total,
              });
              break;
            case 'Composing':
              setSearchStage({ kind: 'composing' });
              break;
            case 'Sources':
              pendingSources = event.results;
              break;
            case 'Token':
              sawToken = true;
              currentContent += event.content;
              setSearchStage(null);
              updateAssistant({ content: currentContent });
              break;
            case 'Warning':
              // Warnings are informational; no UI state change needed here.
              // Task 21 will add warning icon rendering.
              break;
            case 'Done':
              finish(sawToken);
              break;
            case 'Cancelled':
              cancelled = true;
              if (!currentContent) {
                setMessages((prev) => prev.filter((m) => m.id !== assistantId));
              }
              finish(true);
              break;
            case 'Error':
              errored = true;
              updateAssistant({
                content: event.message,
                errorKind: 'Other',
              });
              finish(true);
              break;
          }
        };

        invoke('search_pipeline', {
          message: trimmed,
          onEvent: channel,
        }).catch(() => {
          /* v8 ignore start -- defensive guard: invoke rejection races
             ahead of channel events in practice so errored/cancelled are
             always false here; the check protects against future changes. */
          if (errored || cancelled) return;
          /* v8 ignore stop */
          errored = true;
          updateAssistant({
            content: 'Something went wrong\nCould not start search.',
            errorKind: 'Other',
          });
          finish(true);
        });
      });
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
    setSearchStage(null);
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
    setSearchStage(null);
  }, []);

  return {
    messages,
    ask,
    askSearch,
    cancel,
    isGenerating,
    searchStage,
    reset,
    loadMessages,
  };
}
