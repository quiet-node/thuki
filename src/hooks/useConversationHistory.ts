import { useState, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Message } from './useModel';
import type { SearchResultPreview } from '../types/search';
import type {
  ConversationSummary,
  PersistedMessage,
  SaveConversationResponse,
  SaveMessagePayload,
} from '../types/history';

/**
 * Optional flags for {@link useConversationHistory}'s `save`.
 *
 * Create-on-submit passes `{ generateTitle: false }` so the title LLM does
 * not race the live chat stream; the Done path calls `requestTitle` once.
 * Bookmark / full-save keeps the default (title generation on).
 */
export type SaveOptions = {
  /**
   * When true (default), fire `generate_title` after a successful first save.
   * Pass false for create-on-submit mid-stream.
   */
  generateTitle?: boolean;
};

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
    thinking_content: msg.thinkingContent ?? null,
    search_sources: msg.searchSources ?? null,
    model_name: msg.modelName ?? null,
  };
}

/**
 * Maps a `PersistedMessage` returned by `load_conversation` back to a
 * frontend `Message`, preserving optional fields.
 */
function fromPersisted(msg: PersistedMessage): Message {
  const imagePaths = msg.image_paths
    ? (JSON.parse(msg.image_paths) as string[])
    : undefined;
  const searchSources = msg.search_sources
    ? (JSON.parse(msg.search_sources) as SearchResultPreview[])
    : undefined;
  return {
    id: msg.id,
    role: msg.role as 'user' | 'assistant',
    content: msg.content,
    quotedText: msg.quoted_text ?? undefined,
    imagePaths: imagePaths && imagePaths.length > 0 ? imagePaths : undefined,
    thinkingContent: msg.thinking_content ?? undefined,
    searchSources:
      searchSources && searchSources.length > 0 ? searchSources : undefined,
    fromSearch:
      searchSources !== undefined && searchSources.length > 0
        ? true
        : undefined,
    modelName: msg.model_name ?? undefined,
  };
}

/**
 * Manages conversation persistence state for the current session.
 *
 * Tracks whether the active conversation has been saved to SQLite and provides
 * typed wrappers around all history-related Tauri commands. Intentionally has
 * no knowledge of streaming state or window management - those live in App.tsx
 * and `useModel`.
 *
 * Identity for the write chain (`save` / `persistTurn`) is kept on
 * `conversationIdRef`, updated synchronously on create / load / unsave /
 * reset so sequential turns after the first auto-save do not close over stale
 * React state and skip `persist_message`.
 *
 * @returns An object containing the current persistence state and all
 *   history operation callbacks.
 */
export function useConversationHistory() {
  const [conversationId, setConversationId] = useState<string | null>(null);
  /**
   * Synchronous mirror of `conversationId` for the auto-save write chain.
   * React state alone is too late after `await save()` for the next chained
   * `persistTurn` in the same tick.
   */
  const conversationIdRef = useRef<string | null>(null);

  /**
   * True after `generate_title` has been requested for the current identity
   * (or a loaded conversation that already has a title). Prevents a second
   * title LLM call on later Done / re-save paths.
   */
  const titleRequestedRef = useRef(false);

  /** True once the conversation has been saved to SQLite for the first time. */
  const isSaved = conversationId !== null;

  /**
   * Writes identity to the ref and React state together.
   *
   * @param id New conversation UUID, or null when unsaved / cleared.
   */
  const setConversationIdentity = useCallback((id: string | null) => {
    conversationIdRef.current = id;
    setConversationId(id);
  }, []);

  /**
   * Fire-and-forget `generate_title` once per conversation id.
   * No-op without identity, model, or if a title was already requested.
   *
   * @param messages Transcript snapshot for the title prompt.
   * @param model Active model slug that should produce the title.
   */
  const requestTitle = useCallback(
    (messages: Message[], model: string | null): void => {
      const id = conversationIdRef.current;
      if (id === null || model == null || titleRequestedRef.current) return;
      titleRequestedRef.current = true;
      void invoke('generate_title', {
        conversationId: id,
        messages: messages.map(toPayload),
        model,
      });
    },
    [],
  );

  /**
   * Persists the current conversation to SQLite for the first time.
   * Subsequent calls while a conversation id is already held are no-ops -
   * the bookmark icon on the frontend enforces single-save semantics.
   *
   * Gates on `conversationIdRef` (not React `isSaved`) so a second call in the
   * same write chain after a successful create does not double-insert.
   *
   * By default fires `generate_title` as a fire-and-forget background task
   * after saving (bookmark / full-save). Create-on-submit should pass
   * `{ generateTitle: false }` and call `requestTitle` after the first Done
   * so the title LLM does not race the live chat stream.
   *
   * @param messages The complete message history to persist.
   * @param model The active Ollama model slug used for title generation,
   *   or `null` when no model is selected. A null model short-circuits the
   *   save (no conversation can be attributed to a missing model). The
   *   backend `save_conversation` command also enforces this contract;
   *   gating here keeps the IPC surface clean.
   * @param options Optional save flags; see {@link SaveOptions}.
   */
  const save = useCallback(
    async (
      messages: Message[],
      model: string | null,
      options?: SaveOptions,
    ): Promise<void> => {
      if (conversationIdRef.current !== null) return;
      if (model == null) return;

      const payloads = messages.map(toPayload);

      const response = await invoke<SaveConversationResponse>(
        'save_conversation',
        {
          messages: payloads,
        },
      );

      if (response == null || typeof response.conversation_id !== 'string') {
        throw new Error('save_conversation returned no conversation id');
      }

      setConversationIdentity(response.conversation_id);

      // Default true: bookmark / Done full-save. Create-on-submit opts out so
      // title generation waits until the first completed assistant turn.
      if (options?.generateTitle !== false) {
        titleRequestedRef.current = true;
        void invoke('generate_title', {
          conversationId: response.conversation_id,
          messages: payloads,
          model,
        });
      }
    },
    [setConversationIdentity],
  );

  /**
   * Appends a completed user/assistant turn to the already-saved conversation.
   * No-op if no conversation id is held yet - partial conversations are only
   * persisted after an explicit or auto first save.
   *
   * Reads `conversationIdRef` so chained turns after `await save()` still
   * invoke `persist_message` even when React state has not re-rendered.
   *
   * @param userMsg The user message from the completed turn.
   * @param assistantMsg The assistant response from the completed turn.
   */
  const persistTurn = useCallback(
    async (userMsg: Message, assistantMsg: Message): Promise<void> => {
      const id = conversationIdRef.current;
      if (id === null) return;

      await Promise.all([
        invoke('persist_message', {
          conversationId: id,
          role: userMsg.role,
          content: userMsg.content,
          quotedText: userMsg.quotedText ?? null,
          imagePaths: userMsg.imagePaths ?? null,
          thinkingContent: null,
          searchSources: null,
          modelName: null,
        }),
        invoke('persist_message', {
          conversationId: id,
          role: assistantMsg.role,
          content: assistantMsg.content,
          quotedText: assistantMsg.quotedText ?? null,
          imagePaths: null,
          thinkingContent: assistantMsg.thinkingContent ?? null,
          searchSources: assistantMsg.searchSources ?? null,
          modelName: assistantMsg.modelName ?? null,
        }),
      ]);
    },
    [],
  );

  /**
   * Appends only the assistant half of a turn when the user message was already
   * written by create-on-submit (`save` with a user-only or partial transcript).
   * No-op without a conversation id.
   *
   * @param assistantMsg Completed assistant message to append.
   */
  const persistAssistant = useCallback(
    async (assistantMsg: Message): Promise<void> => {
      const id = conversationIdRef.current;
      if (id === null) return;

      await invoke('persist_message', {
        conversationId: id,
        role: assistantMsg.role,
        content: assistantMsg.content,
        quotedText: assistantMsg.quotedText ?? null,
        imagePaths: null,
        thinkingContent: assistantMsg.thinkingContent ?? null,
        searchSources: assistantMsg.searchSources ?? null,
        modelName: assistantMsg.modelName ?? null,
      });
    },
    [],
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
      // Loaded rows already have a title (or a prior request); do not re-fire.
      titleRequestedRef.current = true;
      setConversationIdentity(id);
      return persisted.map(fromPersisted);
    },
    [setConversationIdentity],
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
    const id = conversationIdRef.current;
    if (id === null) return;
    await invoke('delete_conversation', { conversationId: id });
    titleRequestedRef.current = false;
    setConversationIdentity(null);
  }, [setConversationIdentity]);

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
   * full session (new conversation), call `useModel.reset()` alongside this
   * so the backend history is also wiped. When only marking a conversation as
   * unsaved while keeping messages visible (e.g. after deletion from history
   * or Settings Clear all), calling this alone is correct - `persistTurn` will
   * no-op and the backend context is rebuilt from the frontend messages on the
   * next request.
   */
  const reset = useCallback(() => {
    titleRequestedRef.current = false;
    setConversationIdentity(null);
  }, [setConversationIdentity]);

  return {
    conversationId,
    /**
     * Live conversation UUID for write-chain gates (sync; prefer over state
     * after `await save()`).
     */
    conversationIdRef,
    isSaved,
    save,
    unsave,
    persistTurn,
    persistAssistant,
    requestTitle,
    loadConversation,
    deleteConversation,
    listConversations,
    reset,
  };
}
