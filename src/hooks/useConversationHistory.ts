import { useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Message } from './useOllama';
import type {
  ConversationSummary,
  PersistedMessage,
  SaveConversationResponse,
  SaveMessagePayload,
} from '../types/history';

/**
 * Maps a frontend `Message` to the `SaveMessagePayload` shape expected by
 * the `save_conversation` and `generate_title` Tauri commands.
 */
function toPayload(msg: Message): SaveMessagePayload {
  return {
    role: msg.role,
    content: msg.content,
    quoted_text: msg.quotedText ?? null,
    image_paths: msg.imagePaths ?? null,
  };
}

/**
 * Maps a `PersistedMessage` returned by `load_conversation` back to a
 * frontend `Message`, preserving optional `quotedText`.
 */
function fromPersisted(msg: PersistedMessage): Message {
  const imagePaths = msg.image_paths
    ? (JSON.parse(msg.image_paths) as string[])
    : undefined;
  return {
    id: msg.id,
    role: msg.role as 'user' | 'assistant',
    content: msg.content,
    quotedText: msg.quoted_text ?? undefined,
    imagePaths: imagePaths && imagePaths.length > 0 ? imagePaths : undefined,
  };
}

/**
 * Manages conversation persistence state for the current session.
 *
 * Tracks whether the active conversation has been saved to SQLite and provides
 * typed wrappers around all history-related Tauri commands. Intentionally has
 * no knowledge of streaming state or window management — those live in App.tsx
 * and `useOllama`.
 *
 * @returns An object containing the current persistence state and all
 *   history operation callbacks.
 */
export function useConversationHistory() {
  const [conversationId, setConversationId] = useState<string | null>(null);

  /** True once the conversation has been saved to SQLite for the first time. */
  const isSaved = conversationId !== null;

  /**
   * Persists the current conversation to SQLite for the first time.
   * Subsequent calls while `isSaved` is true are no-ops — the bookmark
   * icon on the frontend enforces single-save semantics.
   *
   * Fires `generate_title` as a fire-and-forget background task after saving;
   * the frontend should schedule a `listConversations` refresh to pick up the
   * AI-generated title once it arrives (~2-5 seconds).
   *
   * @param messages The complete message history to persist.
   * @param model The Ollama model name used in this session.
   */
  const save = useCallback(
    async (messages: Message[], model: string): Promise<void> => {
      if (isSaved) return;

      const payloads = messages.map(toPayload);

      const response = await invoke<SaveConversationResponse>(
        'save_conversation',
        {
          messages: payloads,
          model,
        },
      );

      setConversationId(response.conversation_id);

      // Fire-and-forget: ask Rust to generate an AI title for the conversation.
      // The frontend can poll `list_conversations` after a delay to pick up the result.
      void invoke('generate_title', {
        conversationId: response.conversation_id,
        messages: payloads,
      });
    },
    [isSaved],
  );

  /**
   * Appends a completed user/assistant turn to the already-saved conversation.
   * No-op if the conversation has not been saved yet — partial conversations
   * are only persisted after an explicit save.
   *
   * @param userMsg The user message from the completed turn.
   * @param assistantMsg The assistant response from the completed turn.
   */
  const persistTurn = useCallback(
    async (userMsg: Message, assistantMsg: Message): Promise<void> => {
      if (!isSaved || conversationId === null) return;

      await Promise.all([
        invoke('persist_message', {
          conversationId,
          role: userMsg.role,
          content: userMsg.content,
          quotedText: userMsg.quotedText ?? null,
          imagePaths: userMsg.imagePaths ?? null,
        }),
        invoke('persist_message', {
          conversationId,
          role: assistantMsg.role,
          content: assistantMsg.content,
          quotedText: assistantMsg.quotedText ?? null,
          imagePaths: null,
        }),
      ]);
    },
    [isSaved, conversationId],
  );

  /**
   * Loads a saved conversation from SQLite.
   *
   * Calls the `load_conversation` Tauri command, which atomically syncs the
   * backend `ConversationHistory` state and bumps the epoch counter so any
   * in-flight streaming turn cannot corrupt the newly loaded history.
   *
   * @param id The UUID of the conversation to load.
   * @returns The conversation messages mapped to frontend `Message` shape.
   */
  const loadConversation = useCallback(
    async (id: string): Promise<Message[]> => {
      const persisted = await invoke<PersistedMessage[]>('load_conversation', {
        conversationId: id,
      });
      setConversationId(id);
      return persisted.map(fromPersisted);
    },
    [],
  );

  /**
   * Permanently deletes a saved conversation and all its messages from SQLite.
   *
   * @param id The UUID of the conversation to delete.
   */
  const deleteConversation = useCallback(async (id: string): Promise<void> => {
    await invoke('delete_conversation', { conversationId: id });
  }, []);

  /**
   * Removes the current conversation from SQLite without clearing the
   * in-memory message history. After this call `isSaved` is false and the
   * session is treated as unsaved again (the user can re-save if desired).
   */
  const unsave = useCallback(async (): Promise<void> => {
    if (!isSaved || conversationId === null) return;
    await invoke('delete_conversation', { conversationId });
    setConversationId(null);
  }, [isSaved, conversationId]);

  /**
   * Fetches the list of saved conversations, optionally filtered by title.
   *
   * @param search Optional case-insensitive search term applied against
   *   conversation titles.
   * @returns An array of `ConversationSummary` objects ordered by most-recently
   *   updated.
   */
  const listConversations = useCallback(
    async (search?: string): Promise<ConversationSummary[]> => {
      return invoke<ConversationSummary[]>('list_conversations', {
        search: search ?? null,
      });
    },
    [],
  );

  /**
   * Clears the local persistence state, marking the session as unsaved.
   *
   * Does NOT call `reset_conversation` on the backend. When clearing the
   * full session (new conversation), call `useOllama.reset()` alongside this
   * so the backend history is also wiped. When only marking a conversation as
   * unsaved while keeping messages visible (e.g. after deletion from history),
   * calling this alone is correct — `persistTurn` will no-op and the backend
   * context is rebuilt from the frontend messages on the next request.
   */
  const reset = useCallback(() => {
    setConversationId(null);
  }, []);

  return {
    conversationId,
    isSaved,
    save,
    unsave,
    persistTurn,
    loadConversation,
    deleteConversation,
    listConversations,
    reset,
  };
}
