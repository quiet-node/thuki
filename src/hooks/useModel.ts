import { useCallback, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import type { SearchResultPreview, SearchStage } from '../types/search';

/** Mirrors the Rust EngineErrorKind enum sent over IPC. */
export type EngineErrorKind =
  | 'EngineUnreachable'
  | 'EngineStartFailed'
  | 'ModelUnsupported'
  | 'InsufficientMemory'
  | 'ModelNotFound'
  | 'NoModelSelected'
  | 'Other';

/** Represents a single message in the chat thread. */
export interface Message {
  /** Unique identifier for stable React list keys. */
  id: string;
  role: 'user' | 'assistant';
  content: string;
  /** Ollama model slug attributed to this assistant message at creation time.
   *  Remains stable even if the user switches models mid-stream. Undefined for
   *  user messages and for legacy conversations loaded from pre-migration rows. */
  modelName?: string;
  /** Selected text from the host app that was quoted with this message, if any. */
  quotedText?: string;
  /** Absolute file paths of images attached to this message, if any. */
  imagePaths?: string[];
  /** Present on assistant messages that represent an Ollama error callout. */
  errorKind?: EngineErrorKind;
  /** Accumulated thinking content from the model, if thinking mode was used. */
  thinkingContent?: string;
  /** Marks an assistant message produced via web search (auto or `/search`). */
  fromSearch?: boolean;
  /** Marks an assistant message produced through a `/think` turn. */
  fromThink?: boolean;
  /** Trigger of the replaceable utility command (`/rewrite` or `/refine`) that
   *  produced this assistant message. Present only on those results; drives the
   *  in-chat Replace button and the auto-replace path. */
  replaceCommand?: string;
  /** Source links for a web-search answer. */
  searchSources?: SearchResultPreview[];
  /**
   * Immutable snapshot of the request that produced this message, captured
   * once at dispatch time (issue #296). Lets `retryMessageWithOversized`
   * replay the EXACT turn that produced this message with the pre-load
   * memory gate bypassed, even if later turns have since superseded any
   * hook-level "most recent request" state. Present on every assistant
   * message (the outcome isn't known yet at creation time), never mutated
   * afterward, and read only when this message's `errorKind` is
   * `InsufficientMemory`.
   */
  retrySnapshot?: RetrySnapshot;
  /**
   * Memory-fit figures for an `InsufficientMemory` re-attribution (issue #296).
   * Set only by `updateErroredMessageModel` when a "Switch model" pick lands on
   * a model the gate ALSO blocks: it carries the newly-picked model's required/
   * available bytes so the error card swaps its GB figures ATOMICALLY with
   * `modelName`, instead of `ChatBubble` kicking a fresh async
   * `estimate_model_fit` and briefly rendering the previous model's numbers (or
   * rendering them forever if that refetch rejects). Absent on the initial
   * failure, where `ChatBubble` fetches the figures itself.
   */
  memoryFit?: { requiredBytes: number; availableBytes: number };
}

/** Raw streaming chunk payload emitted from the Rust chat backend. */
type RawStreamChunk =
  | { type: 'Token'; data: string }
  | { type: 'ThinkingToken'; data: string }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; data: { kind: EngineErrorKind; message: string } }
  | { type: 'TurnAccepted' }
  | { type: 'SearchStatus'; data: { phase: SearchPhase } }
  | {
      type: 'SearchSources';
      data: Array<{ index: number; url: string; title: string }>;
    }
  | { type: 'SetContent'; data: string };

/** Progress phase of the web-search pipeline (mirrors the Rust `SearchPhase`). */
type SearchPhase = 'deciding' | 'searching' | 'reading' | 'verifying';

/**
 * Normalized chat-stream chunk used inside the hook.
 *
 * The chat IPC payload uses `data` while the search pipeline uses `content`.
 * Normalizing here keeps the internal token contract consistent and prevents
 * accidental cross-assignment between the two event streams.
 */
type StreamChunk =
  | { type: 'Token'; content: string }
  | { type: 'ThinkingToken'; content: string }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; error: { kind: EngineErrorKind; message: string } }
  | { type: 'TurnAccepted' }
  | { type: 'SearchStatus'; phase: SearchPhase }
  | { type: 'SearchSources'; sources: SearchResultPreview[] }
  | { type: 'SetContent'; content: string };

/**
 * Shared swallow-all handler for fire-and-forget trace IPC calls.
 * `record_conversation_end` is a best-effort signal; backend failures
 * (recorder mid-flush, IPC closed during teardown, etc.) must never
 * block a user-visible reset or history-load. Hoisted to module scope
 * so coverage counts the function exactly once.
 *
 * Exported so the unit tests can call it directly when verifying the
 * handler is wired up; production code should never need to reference
 * it by name.
 */
export const ignoreTraceIpcError = (): void => {};

function normalizeStreamChunk(chunk: RawStreamChunk): StreamChunk {
  switch (chunk.type) {
    case 'Token':
      return { type: 'Token', content: chunk.data };
    case 'ThinkingToken':
      return { type: 'ThinkingToken', content: chunk.data };
    case 'Done':
      return chunk;
    case 'Cancelled':
      return chunk;
    case 'Error':
      return { type: 'Error', error: chunk.data };
    case 'TurnAccepted':
      return chunk;
    case 'SearchStatus':
      return { type: 'SearchStatus', phase: chunk.data.phase };
    case 'SearchSources':
      return {
        type: 'SearchSources',
        sources: chunk.data.map((source) => ({
          title: source.title,
          url: source.url,
        })),
      };
    case 'SetContent':
      return { type: 'SetContent', content: chunk.data };
  }
}

/** Maps an auto-search phase to the shared stage indicator. */
const SEARCH_STAGE_BY_PHASE: Record<SearchPhase, SearchStage> = {
  deciding: { kind: 'analyzing_query' },
  searching: { kind: 'searching' },
  reading: { kind: 'reading_sources' },
  verifying: { kind: 'verifying_sources' },
};

/** Result payload delivered to callers when a `/search` pipeline turn finishes. */
export interface SearchOutcome {
  final: boolean;
}

/**
 * Snapshot of an `ask()` call, captured at dispatch time and attached to the
 * specific assistant message it produced, so `retryMessageWithOversized`
 * (issue #296) can replay that exact turn with the pre-load memory gate
 * bypassed without the caller re-supplying every argument.
 */
interface ChatRetrySnapshot {
  kind: 'chat';
  displayContent: string;
  quotedText?: string;
  imagePaths?: string[];
  think?: boolean;
  promptOverride?: string;
  displayImagePaths?: string[];
  replaceCommand?: string;
  /** Ids of the failed turn's own user/assistant pair, so a retry can reuse
   *  them: leaving the user bubble untouched and resetting the assistant
   *  message in place rather than appending a duplicate pair. Reusing the id
   *  keeps React from remounting the bubble (which would replay its entrance
   *  animation), issue #296. */
  userMessageId: string;
  assistantMessageId: string;
}

/** Snapshot of an `askSearch()` call. See {@link ChatRetrySnapshot}. */
interface SearchRetrySnapshot {
  kind: 'search';
  query: string;
  displayContent?: string;
  quotedText?: string;
  /** Ids of the failed turn's own user/assistant pair, so a retry can reuse
   *  them: leaving the user bubble untouched and resetting the assistant
   *  message in place rather than appending a duplicate pair. Reusing the id
   *  keeps React from remounting the bubble (which would replay its entrance
   *  animation), issue #296. */
  userMessageId: string;
  assistantMessageId: string;
}

export type RetrySnapshot = ChatRetrySnapshot | SearchRetrySnapshot;

/**
 * Controls how {@link runChatTurn} and {@link runSearchTurn} establish the
 * user/assistant message pair for a turn.
 *
 * - `append`: mint fresh ids and append a new pair. The normal, hot path for
 *   every first-time turn.
 * - `reuse`: reuse a failed turn's ids and reset its assistant message in
 *   place (issue #296). Reusing the id keeps React from unmounting +
 *   remounting the bubble, which would replay ChatBubble's entrance animation
 *   and flash the whole thread on every retry. The user message is
 *   byte-identical on a retry, so it is left untouched.
 */
type TurnTarget =
  | { mode: 'append' }
  | {
      mode: 'reuse';
      userMessageId: string;
      assistantMessageId: string;
      /** Model name to pin on the reset assistant message. Lets a retry
       *  triggered by a freshly-picked model attribute the correct name even
       *  before the `activeModel` closure has observed the new value (a React
       *  state-update timing race). Falls back to `activeModel` when omitted. */
      modelOverride?: string;
    };

interface ActiveGeneration {
  id: number;
  assistantId: string;
  hasVisibleOutput: boolean;
  resolveSearch?: (outcome: SearchOutcome) => void;
}

/**
 * Simplifies interactions with the local Ollama backend.
 *
 * Manages message history, streaming state, and the Tauri IPC channels used by
 * the normal chat path and force-search (`/search` → `ask_model`).
 *
 * @param activeModel Ollama model slug that should be attributed to each
 *   assistant message produced by this hook. Passed as a hook parameter (not
 *   a per-call argument) so the latest App-level selection is captured via
 *   closure on every render. `null` (no model selected) and an empty string
 *   are both coerced to `undefined` on the emitted `Message`, so no
 *   attribution chip is rendered rather than a blank one.
 * @param onTurnComplete Optional callback invoked after each completed turn.
 */
export function useModel(
  activeModel: string | null,
  onTurnComplete?: (userMsg: Message, assistantMsg: Message) => void,
) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isGenerating, setIsGenerating] = useState(false);
  /** Transient stage indicator while web search is in flight, if any. */
  const [searchStage, setSearchStage] = useState<SearchStage>(null);
  const activeGenerationRef = useRef<ActiveGeneration | null>(null);
  const nextGenerationIdRef = useRef(0);
  const pendingCancelRef = useRef<Promise<void> | null>(null);

  /**
   * Stable trace conversation id for the current in-memory chat session.
   * Lazily initialized on first read by `ensureTraceConversationId`;
   * `useRef(null)` keeps render pure, the lazy init in a callback keeps
   * `crypto.randomUUID()` out of the render path (per
   * `@eslint-react/purity`). Independent of the SQLite "saved
   * conversation" id (which is null until `useConversationHistory.save()`
   * runs); the trace recorder uses this id to route every event for the
   * session into one `traces/chat/<id>.jsonl` and `traces/search/<id>.jsonl`
   * pair. Refreshed on `reset()` and `loadMessages()`, both of which
   * fire `record_conversation_end` for the outgoing id so the chat-domain
   * file gets a clean closing line.
   */
  const traceConversationIdRef = useRef<string | null>(null);
  /**
   * True until the first `ask()` / `askSearch()` for the current trace
   * conversation id has fired. Read by the backend to decide whether to
   * emit `ConversationStart`. Reset to true on `reset()` /
   * `loadMessages()`.
   */
  const isFirstTurnRef = useRef(true);

  /**
   * Returns the active trace conversation id, lazily creating it on
   * first call. Stable for the lifetime of the session; rotated by
   * `reset()` and `loadMessages()`.
   */
  const ensureTraceConversationId = useCallback((): string => {
    if (traceConversationIdRef.current === null) {
      traceConversationIdRef.current = crypto.randomUUID();
    }
    return traceConversationIdRef.current;
  }, []);

  const beginGeneration = (
    assistantId: string,
    resolveSearch?: (outcome: SearchOutcome) => void,
  ) => {
    const generation: ActiveGeneration = {
      id: nextGenerationIdRef.current + 1,
      assistantId,
      hasVisibleOutput: false,
      resolveSearch,
    };
    nextGenerationIdRef.current = generation.id;
    activeGenerationRef.current = generation;
    return generation.id;
  };

  const isActiveGeneration = (generationId: number) =>
    activeGenerationRef.current?.id === generationId;

  const markVisibleOutput = () => {
    activeGenerationRef.current!.hasVisibleOutput = true;
  };

  const completeGeneration = () => {
    const active = activeGenerationRef.current!;
    activeGenerationRef.current = null;
    return active;
  };

  const abortActiveGeneration = useCallback(() => {
    const active = activeGenerationRef.current;
    activeGenerationRef.current = null;
    setIsGenerating(false);
    setSearchStage(null);

    if (!active) {
      return false;
    }

    active.resolveSearch?.({ final: true });

    if (!active.hasVisibleOutput) {
      setMessages((prev) =>
        prev.filter((message) => message.id !== active.assistantId),
      );
    }

    return true;
  }, []);

  /**
   * Signals the backend to stop the active generation and tracks the
   * in-flight cancel in `pendingCancelRef` so the next `ask()` /
   * `askSearch()` awaits the round-trip before starting a new turn. That
   * wait is what stops a fresh request from racing the outgoing one onto
   * the engine's single decode slot. Idempotent while a cancel is already
   * pending, so overlapping callers (double cancel, cancel-then-reset) only
   * fire `cancel_generation` once. Returns the pending-cancel promise.
   */
  const requestBackendCancel = useCallback((): Promise<void> => {
    if (!pendingCancelRef.current) {
      pendingCancelRef.current = (async () => {
        try {
          await invoke('cancel_generation');
        } catch {
          // Local hard-abort already reset the UI; backend best-effort only.
        } finally {
          pendingCancelRef.current = null;
        }
      })();
    }
    return pendingCancelRef.current;
  }, []);

  /**
   * Core chat-turn driver shared by the public {@link ask} wrapper and the
   * retry path. Establishes the user/assistant pair per `target`, then runs
   * exactly one copy of the guard-and-streaming logic. The single copy is
   * deliberate: duplicated safety-critical streaming logic is the class of bug
   * that produced issue #296.
   *
   * The backend manages conversation history. Only the new user message is sent.
   *
   * @param displayContent Text shown in the user bubble.
   * @param quotedText Selected host-app text shown above the bubble, if any.
   * @param imagePaths Absolute image paths sent to the model, if any.
   * @param think When true, runs the turn as a `/think` (reasoning) turn.
   * @param promptOverride Text sent to the model in place of `displayContent`.
   * @param displayImagePaths Image paths shown in the bubble when they differ
   *   from the ones sent to the model.
   * @param replaceCommand Trigger of the replaceable command (`/rewrite` or
   *   `/refine`) that produced this turn, if any.
   * @param allowOversized Bypasses the pre-load memory gate (issue #296).
   * @param target How to establish the message pair: append a fresh pair or
   *   reuse a failed turn's ids and reset its assistant message in place.
   */
  const runChatTurn = useCallback(
    async (
      displayContent: string,
      quotedText: string | undefined,
      imagePaths: string[] | undefined,
      think: boolean | undefined,
      promptOverride: string | undefined,
      displayImagePaths: string[] | undefined,
      replaceCommand: string | undefined,
      allowOversized: boolean | undefined,
      target: TurnTarget,
      /**
       * When true, force built-in engines-only web search (`/search` alias).
       * Stamps the turn as a search turn for Variant B progress chrome.
       */
      forceSearch?: boolean,
    ) => {
      if (!displayContent.trim() && (!imagePaths || imagePaths.length === 0)) {
        return;
      }

      if (activeGenerationRef.current) return;
      const pendingCancel = pendingCancelRef.current;
      if (pendingCancel) {
        await pendingCancel;
      }
      if (activeGenerationRef.current) return;

      // Reuse the failed turn's ids on a retry so React keeps the bubble
      // mounted; mint fresh ids for a normal turn. Model attribution follows
      // the override only on the retry path (issue #296).
      const userMessageId =
        target.mode === 'reuse' ? target.userMessageId : crypto.randomUUID();
      const assistantId =
        target.mode === 'reuse'
          ? target.assistantMessageId
          : crypto.randomUUID();
      const resolvedModel =
        target.mode === 'reuse'
          ? (target.modelOverride ?? activeModel)
          : activeModel;

      const bubbleImages = displayImagePaths ?? imagePaths;
      const userMsg: Message = {
        id: userMessageId,
        role: 'user',
        content: displayContent,
        quotedText,
        imagePaths:
          bubbleImages && bubbleImages.length > 0 ? bubbleImages : undefined,
      };

      const assistantMsg: Message = {
        id: assistantId,
        role: 'assistant',
        content: '',
        fromThink: think ? true : undefined,
        // Force-search turns own the Variant B progress chrome immediately
        fromSearch: forceSearch ? true : undefined,
        replaceCommand,
        modelName: resolvedModel ?? undefined,
        // Captured once, here, so a later turn can never overwrite the
        // retry data this specific message needs (issue #296). Force-search
        // turns use the search snapshot shape so Load-anyway / Switch-model
        // replay through `runSearchTurn` (and re-send forceSearch).
        retrySnapshot: forceSearch
          ? {
              kind: 'search',
              query: promptOverride ?? displayContent,
              displayContent,
              quotedText,
              userMessageId,
              assistantMessageId: assistantId,
            }
          : {
              kind: 'chat',
              displayContent,
              quotedText,
              imagePaths,
              think,
              promptOverride,
              displayImagePaths,
              replaceCommand,
              userMessageId,
              assistantMessageId: assistantId,
            },
      };

      if (target.mode === 'reuse') {
        // Reset the assistant message in place: same id, and the freshly
        // built object carries none of the prior turn's error or streaming
        // state (content, errorKind, thinkingContent, etc.), so all of it
        // clears at once. Same id -> React reuses the element instead of
        // remounting it, so ChatBubble's entrance animation does not replay
        // (issue #296 remount flash). The user message is byte-identical on a
        // retry, so it is left untouched.
        setMessages((prev) =>
          prev.map((message) =>
            message.id === assistantId ? assistantMsg : message,
          ),
        );
      } else {
        setMessages((prev) => [...prev, userMsg, assistantMsg]);
      }
      setIsGenerating(true);
      const generationId = beginGeneration(assistantId);

      const channel = new Channel<RawStreamChunk>();
      let currentContent = '';
      let currentThinkingContent = '';
      let pendingSources: SearchResultPreview[] | undefined;

      channel.onmessage = (rawChunk) => {
        const chunk = normalizeStreamChunk(rawChunk);

        // `TurnAccepted` is the backend's authoritative signal that the
        // trace was opened for this conversation_id. Retire the flag
        // BEFORE the active-generation guard so a cancel-mid-first-turn
        // (which clears `activeGenerationRef`) cannot leave the flag
        // armed and cause the next turn to record a duplicate
        // `ConversationStart`. The chunk is hook-internal: it never
        // reaches the UI.
        if (chunk.type === 'TurnAccepted') {
          isFirstTurnRef.current = false;
          return;
        }

        if (!isActiveGeneration(generationId)) {
          return;
        }

        if (chunk.type === 'ThinkingToken') {
          currentThinkingContent += chunk.content;
          if (chunk.content) {
            markVisibleOutput();
          }
          setMessages((prev) =>
            prev.map((message) =>
              message.id === assistantId
                ? { ...message, thinkingContent: currentThinkingContent }
                : message,
            ),
          );
          return;
        }

        if (chunk.type === 'Token') {
          currentContent += chunk.content;
          if (chunk.content) {
            markVisibleOutput();
          }
          setMessages((prev) =>
            prev.map((message) =>
              message.id === assistantId
                ? { ...message, content: currentContent }
                : message,
            ),
          );
          return;
        }

        // Citation repair/strip changed the live draft: replace the full body.
        if (chunk.type === 'SetContent') {
          currentContent = chunk.content;
          if (chunk.content) {
            markVisibleOutput();
          }
          setMessages((prev) =>
            prev.map((message) =>
              message.id === assistantId
                ? { ...message, content: currentContent }
                : message,
            ),
          );
          return;
        }

        if (chunk.type === 'SearchStatus') {
          // Stamp fromSearch early so the bubble owns Variant B progress chrome
          // for the whole retrieval, not only after sources arrive.
          setSearchStage(SEARCH_STAGE_BY_PHASE[chunk.phase]);
          setMessages((prev) =>
            prev.map((message) =>
              message.id === assistantId
                ? { ...message, fromSearch: true }
                : message,
            ),
          );
          return;
        }

        if (chunk.type === 'SearchSources') {
          pendingSources = chunk.sources;
          setMessages((prev) =>
            prev.map((message) =>
              message.id === assistantId
                ? { ...message, fromSearch: true, searchSources: chunk.sources }
                : message,
            ),
          );
          return;
        }

        if (chunk.type === 'Done') {
          completeGeneration();
          setIsGenerating(false);
          setSearchStage(null);
          onTurnComplete?.(userMsg, {
            ...assistantMsg,
            content: currentContent,
            thinkingContent: currentThinkingContent || undefined,
            fromSearch: pendingSources ? true : assistantMsg.fromSearch,
            searchSources: pendingSources,
          });
          return;
        }

        if (chunk.type === 'Cancelled') {
          completeGeneration();
          if (!currentContent && !currentThinkingContent) {
            setMessages((prev) =>
              prev.filter((message) => message.id !== assistantId),
            );
          }
          setIsGenerating(false);
          setSearchStage(null);
          return;
        }

        completeGeneration();

        setMessages((prev) =>
          prev.map((message) =>
            message.id === assistantId
              ? {
                  ...message,
                  content: chunk.error.message,
                  errorKind: chunk.error.kind,
                }
              : message,
          ),
        );
        setIsGenerating(false);
        setSearchStage(null);
      };

      const conversationId = ensureTraceConversationId();
      const isFirstTurn = isFirstTurnRef.current;
      // The ref is flipped inside `channel.onmessage` once the backend
      // confirms it accepted the turn. Flipping here would burn the flag
      // on no-model bails that return before `ConversationStart` fires,
      // leaving the next attempt without an opening trace event.
      try {
        await invoke('ask_model', {
          message: promptOverride ?? displayContent,
          quotedText: quotedText ?? null,
          imagePaths: imagePaths && imagePaths.length > 0 ? imagePaths : null,
          think: think ?? false,
          conversationId,
          isFirstTurn,
          slashCommand: forceSearch ? '/search' : think ? '/think' : null,
          allowOversized: allowOversized ?? false,
          forceSearch: forceSearch ?? false,
          onEvent: channel,
        });
      } catch {
        if (!isActiveGeneration(generationId)) {
          return;
        }
        completeGeneration();
        setMessages((prev) =>
          prev.map((message) =>
            message.id === assistantId
              ? {
                  ...message,
                  content: 'Something went wrong\nCould not reach Ollama.',
                  errorKind: 'Other',
                }
              : message,
          ),
        );
        setIsGenerating(false);
        setSearchStage(null);
      }
    },
    [onTurnComplete, activeModel, ensureTraceConversationId],
  );

  /**
   * Submits a message to the backend and streams the response, appending a
   * fresh user/assistant pair. The normal, hot path for a new chat turn.
   * Public signature unchanged; delegates to {@link runChatTurn}.
   *
   * The backend manages conversation history. Only the new user message is sent.
   */
  const ask = useCallback(
    (
      displayContent: string,
      quotedText?: string,
      imagePaths?: string[],
      think?: boolean,
      promptOverride?: string,
      displayImagePaths?: string[],
      replaceCommand?: string,
      /** Bypasses the pre-load memory gate (issue #296). Set by
       *  `retryMessageWithOversized` when the user clicks "Load anyway";
       *  omitted by every normal caller. */
      allowOversized?: boolean,
    ) =>
      runChatTurn(
        displayContent,
        quotedText,
        imagePaths,
        think,
        promptOverride,
        displayImagePaths,
        replaceCommand,
        allowOversized,
        { mode: 'append' },
      ),
    [runChatTurn],
  );

  /**
   * Core `/search` force-search driver shared by the public {@link askSearch}
   * wrapper and the retry path. Forces web search via `ask_model`
   * (`forceSearch`). Always one-shot (`final: true`); follow-ups use normal
   * auto-search rules.
   *
   * @param query Text sent to the backend without the `/search` trigger.
   * @param displayContent Text shown in the user bubble. Defaults to `query`.
   * @param quotedText Selected host-app text shown above the user bubble, if any.
   * @param allowOversized Bypasses the pre-load memory gate (issue #296).
   * @param target How to establish the message pair: append a fresh pair or
   *   reuse a failed turn's ids and reset its assistant message in place.
   */
  const runSearchTurn = useCallback(
    async (
      query: string,
      displayContent: string | undefined,
      quotedText: string | undefined,
      allowOversized: boolean | undefined,
      target: TurnTarget,
    ): Promise<SearchOutcome> => {
      const trimmed = query.trim();
      if (!trimmed) return { final: true };

      // Bubble shows what the user typed; backend gets the stripped query and
      // forceSearch so the engines-only path runs. Reuses runChatTurn so
      // allowOversized + TurnTarget (issue #296) stay single-sourced.
      await runChatTurn(
        displayContent ?? trimmed,
        quotedText,
        undefined,
        false,
        trimmed,
        undefined,
        undefined,
        allowOversized,
        target,
        true,
      );
      return { final: true };
    },
    [runChatTurn],
  );

  /**
   * Submits a `/search` force-search turn, appending a fresh user/assistant pair.
   * The normal, hot path for a new search turn. Public signature unchanged;
   * delegates to {@link runSearchTurn}.
   *
   * @param query Text sent to the backend pipeline, without the `/search` trigger.
   * @param displayContent Text shown in the user bubble. Defaults to `query`.
   * @param quotedText Selected host-app text shown above the user bubble, if any.
   * @param allowOversized Bypasses the pre-load memory gate (issue #296). Set
   *   by `retryMessageWithOversized`; omitted by every normal caller.
   */
  const askSearch = useCallback(
    (
      query: string,
      displayContent?: string,
      quotedText?: string,
      allowOversized?: boolean,
    ): Promise<SearchOutcome> =>
      runSearchTurn(query, displayContent, quotedText, allowOversized, {
        mode: 'append',
      }),
    [runSearchTurn],
  );

  /**
   * Shared replay core for both retry entry points below: redispatches a
   * failed turn's exact `RetrySnapshot` (read off the failed message itself,
   * never off shared hook state) through the turn cores in `reuse` mode, which
   * keeps the failed turn's ids and resets its assistant message in place
   * rather than appending a duplicate pair (issue #296). Scoped per-message so
   * a later turn superseding the hook's in-flight state can never cause the
   * wrong turn to be replayed.
   *
   * @param snapshot The failed turn's captured request.
   * @param allowOversized Whether to bypass the pre-load memory gate on replay.
   * @param modelOverride Model name to pin on the reset assistant message,
   *   forwarded to the turn core. See {@link TurnTarget}.
   */
  const replaySnapshot = useCallback(
    (
      snapshot: RetrySnapshot,
      allowOversized: boolean,
      modelOverride?: string,
    ) => {
      if (snapshot.kind === 'chat') {
        void runChatTurn(
          snapshot.displayContent,
          snapshot.quotedText,
          snapshot.imagePaths,
          snapshot.think,
          snapshot.promptOverride,
          snapshot.displayImagePaths,
          snapshot.replaceCommand,
          allowOversized,
          {
            mode: 'reuse',
            userMessageId: snapshot.userMessageId,
            assistantMessageId: snapshot.assistantMessageId,
            modelOverride,
          },
        );
      } else {
        void runSearchTurn(
          snapshot.query,
          snapshot.displayContent,
          snapshot.quotedText,
          allowOversized,
          {
            mode: 'reuse',
            userMessageId: snapshot.userMessageId,
            assistantMessageId: snapshot.assistantMessageId,
            modelOverride,
          },
        );
      }
    },
    [runChatTurn, runSearchTurn],
  );

  /**
   * Replays a specific turn's `ask()` / `askSearch()` call with the pre-load
   * memory gate bypassed (issue #296). Wired to the `InsufficientMemory`
   * error card's "Load anyway" button.
   *
   * @param snapshot The failed turn's captured request.
   * @param modelOverride Optional model name to pin on the reset assistant
   *   message, for when the caller has just picked a model whose value the
   *   `activeModel` closure has not yet observed.
   */
  const retryMessageWithOversized = useCallback(
    (snapshot: RetrySnapshot, modelOverride?: string) =>
      replaySnapshot(snapshot, true, modelOverride),
    [replaySnapshot],
  );

  /**
   * Replays a specific turn's `ask()` / `askSearch()` call with the pre-load
   * memory gate left in place (issue #296). Wired to the `InsufficientMemory`
   * error card's "Switch model" flow: once the user picks a new, presumably-
   * fitting model, the abandoned turn is replayed against it normally rather
   * than left stuck describing the old failed attempt forever.
   *
   * @param snapshot The failed turn's captured request.
   * @param modelOverride Optional model name to pin on the reset assistant
   *   message. The "Switch model" flow passes the just-picked model here so
   *   the retried bubble attributes the new model even before the `activeModel`
   *   closure observes it (a React state-update timing race).
   */
  const retryMessage = useCallback(
    (snapshot: RetrySnapshot, modelOverride?: string) =>
      replaySnapshot(snapshot, false, modelOverride),
    [replaySnapshot],
  );

  /**
   * Re-attributes an errored assistant message to a different model IN PLACE,
   * without resetting it or re-sending the turn (issue #296). Used by the
   * `InsufficientMemory` "switch model" flow when the freshly-picked model would
   * ALSO be blocked: retrying would just re-fail and unmount + remount the error
   * card (the flash the user reported), so instead only `modelName` changes.
   *
   * Everything else is preserved deliberately: `errorKind` stays
   * `InsufficientMemory` (so `ErrorCard` never unmounts, which is what kills the
   * flash) and `retrySnapshot` stays intact (so a later pick of a fitting model
   * can still replay this exact turn). The caller (`handleModelSelect`) already
   * fetched the new model's fit figures to make the `would_block` decision, so
   * they are threaded straight onto the message as `memoryFit`; `ChatBubble`
   * renders the card from those carried numbers, swapping the model name and GB
   * figures together in the same commit with no async refetch window. No-op
   * when no message with `assistantMessageId` exists.
   *
   * @param assistantMessageId Id of the errored assistant message to re-attribute.
   * @param modelName New model slug to attribute the message to.
   * @param memoryFit Fit figures for `modelName`, carried onto the message so the
   *   card renders them atomically with the name swap (issue #296).
   */
  const updateErroredMessageModel = useCallback(
    (
      assistantMessageId: string,
      modelName: string,
      memoryFit: { requiredBytes: number; availableBytes: number },
    ) => {
      setMessages((prev) =>
        prev.map((message) =>
          message.id === assistantMessageId
            ? { ...message, modelName, memoryFit }
            : message,
        ),
      );
    },
    [],
  );

  /** Cancels the currently active generation. */
  const cancel = useCallback(async () => {
    if (
      !activeGenerationRef.current &&
      !isGenerating &&
      !pendingCancelRef.current
    ) {
      return;
    }

    abortActiveGeneration();
    await requestBackendCancel();
  }, [abortActiveGeneration, isGenerating, requestBackendCancel]);

  /** Resets all conversation state for a fresh session.
   *
   * Closes the outgoing trace (`ConversationEnd { reason: "user_reset" }`)
   * IFF at least one chat turn has already fired against the current
   * trace conversation id (otherwise there is no `ConversationStart`
   * to pair with and emitting an end would produce an empty file).
   * Then drops the current id back to `null` so the next `ask()` /
   * `askSearch()` lazily mints a fresh one. `record_conversation_end`
   * is fire-and-forget; trace failures must never block the
   * user-visible reset.
   */
  const reset = useCallback(() => {
    const hadActiveGeneration = abortActiveGeneration();
    // Starting a fresh session must also stop any in-flight backend stream,
    // not just the frontend view abort above. Otherwise the outgoing
    // generation keeps the engine's single decode slot and the next turn
    // queues behind it. Routed through the same `pendingCancelRef` plumbing
    // `cancel()` uses so the next `ask()` awaits the cancel round-trip.
    if (hadActiveGeneration) {
      void requestBackendCancel();
    }
    setMessages([]);
    const outgoingId = traceConversationIdRef.current;
    if (outgoingId !== null && !isFirstTurnRef.current) {
      void invoke('record_conversation_end', {
        conversationId: outgoingId,
        reason: 'user_reset',
      }).catch(ignoreTraceIpcError);
    }
    traceConversationIdRef.current = null;
    isFirstTurnRef.current = true;
    void invoke('reset_conversation');
  }, [abortActiveGeneration, requestBackendCancel]);

  /** Replaces the current message list with a previously loaded set of messages.
   *
   * Loading a different conversation from the history panel is also a
   * trace-conversation boundary: the outgoing trace is closed with
   * reason `"history_load"` and a fresh id is minted for the loaded
   * messages. Without this the loaded conversation's first ask() would
   * append to the outgoing trace's file, mixing two unrelated chats.
   */
  const loadMessages = useCallback(
    (msgs: Message[]) => {
      const hadActiveGeneration = abortActiveGeneration();
      // Loading another conversation is a session boundary too: stop the
      // in-flight backend stream so it does not hold the engine slot and
      // stall the loaded conversation's next turn. Same plumbing as reset().
      if (hadActiveGeneration) {
        void requestBackendCancel();
      }
      const outgoingId = traceConversationIdRef.current;
      if (outgoingId !== null && !isFirstTurnRef.current) {
        void invoke('record_conversation_end', {
          conversationId: outgoingId,
          reason: 'history_load',
        }).catch(ignoreTraceIpcError);
      }
      traceConversationIdRef.current = null;
      isFirstTurnRef.current = true;
      setMessages(msgs);
    },
    [abortActiveGeneration, requestBackendCancel],
  );

  /**
   * Active trace conversation id for the current session. Exposed so
   * sibling commands invoked from `App.tsx` (notably
   * `capture_full_screen_command` for `/screen`) can route their
   * trace events to the same per-conversation file.
   */
  const getTraceConversationId = ensureTraceConversationId;

  /**
   * Inserts a completed user/assistant turn directly, bypassing the Ollama
   * streaming pipeline. Used by the `/extract` command path, where OCR
   * produces the assistant content locally via the Vision framework.
   * Calls `onTurnComplete` so the turn is persisted to history.
   */
  const addOcrTurn = useCallback(
    (
      userContent: string,
      userQuotedText: string | undefined,
      userImagePaths: string[] | undefined,
      assistantContent: string,
    ) => {
      const userMsg: Message = {
        id: crypto.randomUUID(),
        role: 'user',
        content: userContent,
        quotedText: userQuotedText,
        imagePaths: userImagePaths,
      };
      const assistantMsg: Message = {
        id: crypto.randomUUID(),
        role: 'assistant',
        content: assistantContent,
      };
      setMessages((prev) => [...prev, userMsg, assistantMsg]);
      onTurnComplete?.(userMsg, assistantMsg);
    },
    [onTurnComplete],
  );

  return {
    messages,
    ask,
    askSearch,
    cancel,
    isGenerating,
    searchStage,
    reset,
    loadMessages,
    getTraceConversationId,
    addOcrTurn,
    retryMessageWithOversized,
    retryMessage,
    updateErroredMessageModel,
  };
}
