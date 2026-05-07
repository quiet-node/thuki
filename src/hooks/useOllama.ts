import { useCallback, useRef, useState } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  SearchEvent,
  SearchMetadata,
  SearchResultPreview,
  SearchStage,
  SearchTraceStep,
  SearchWarning,
} from '../types/search';

/** Mirrors the Rust OllamaErrorKind enum sent over IPC. */
export type OllamaErrorKind =
  | 'NotRunning'
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
  errorKind?: OllamaErrorKind;
  /** Accumulated thinking content from the model, if thinking mode was used. */
  thinkingContent?: string;
  /** Marks an assistant message produced through the `/search` pipeline. */
  fromSearch?: boolean;
  /** Marks an assistant message produced through a `/think` turn. */
  fromThink?: boolean;
  /** Source links forwarded by the search pipeline. */
  searchSources?: SearchResultPreview[];
  /** Warnings emitted by the `/search` pipeline during this turn. */
  searchWarnings?: SearchWarning[];
  /** When true, renders sandbox setup guidance instead of normal content. */
  sandboxUnavailable?: boolean;
  /** Ordered, user-facing timeline steps for a `/search` turn. */
  searchTraces?: SearchTraceStep[];
  /** Structured retrieval metadata emitted by the backend search pipeline. */
  searchMetadata?: SearchMetadata;
}

/** Raw streaming chunk payload emitted from the Rust chat backend. */
type RawStreamChunk =
  | { type: 'Token'; data: string }
  | { type: 'ThinkingToken'; data: string }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; data: { kind: OllamaErrorKind; message: string } };

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
  | { type: 'Error'; error: { kind: OllamaErrorKind; message: string } };

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
  }
}

/** Result payload delivered to callers when a `/search` pipeline turn finishes. */
export interface SearchOutcome {
  final: boolean;
}

interface ActiveGeneration {
  id: number;
  assistantId: string;
  hasVisibleOutput: boolean;
  resolveSearch?: (outcome: SearchOutcome) => void;
}

function upsertSearchTraceStep(
  steps: SearchTraceStep[],
  nextStep: SearchTraceStep,
): SearchTraceStep[] {
  const index = steps.findIndex((step) => step.id === nextStep.id);
  if (index === -1) {
    return [...steps, nextStep];
  }

  const next = [...steps];
  next[index] = nextStep;
  return next;
}

function finalizeSearchTraceSteps(
  steps: SearchTraceStep[],
): SearchTraceStep[] | undefined {
  if (steps.length === 0) return undefined;

  return steps.map((step) =>
    step.status === 'running' ? { ...step, status: 'completed' } : step,
  );
}

/**
 * Simplifies interactions with the local Ollama backend.
 *
 * Manages message history, streaming state, and the Tauri IPC channels used by
 * both the normal chat path and the `/search` pipeline.
 *
 * @param activeModel Ollama model slug that should be attributed to each
 *   assistant message produced by this hook. Passed as a hook parameter (not
 *   a per-call argument) so the latest App-level selection is captured via
 *   closure on every render. `null` (no model selected) and an empty string
 *   are both coerced to `undefined` on the emitted `Message`, so no
 *   attribution chip is rendered rather than a blank one.
 * @param onTurnComplete Optional callback invoked after each completed turn.
 */
export function useOllama(
  activeModel: string | null,
  onTurnComplete?: (userMsg: Message, assistantMsg: Message) => void,
) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isGenerating, setIsGenerating] = useState(false);
  /** Transient stage indicator for the active `/search` pipeline, if any. */
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
   * Submits a message to the Ollama backend and starts the streaming response.
   *
   * The backend manages conversation history. Only the new user message is sent.
   */
  const ask = useCallback(
    async (
      displayContent: string,
      quotedText?: string,
      imagePaths?: string[],
      think?: boolean,
      promptOverride?: string,
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
        fromThink: think ? true : undefined,
        modelName: activeModel ?? undefined,
      };

      setMessages((prev) => [...prev, userMsg, assistantMsg]);
      setIsGenerating(true);
      const generationId = beginGeneration(assistantId);

      const channel = new Channel<RawStreamChunk>();
      let currentContent = '';
      let currentThinkingContent = '';

      channel.onmessage = (rawChunk) => {
        if (!isActiveGeneration(generationId)) {
          return;
        }

        const chunk = normalizeStreamChunk(rawChunk);

        // Backend bails BEFORE recording `ConversationStart` only when no
        // model is selected. Every other event (Token, Done, Cancelled,
        // and post-`ConversationStart` Errors) means the trace was opened,
        // so the flag must be retired. Keeping the flag armed only on the
        // pre-`ConversationStart` bail preserves trace integrity across a
        // failed-then-retried first turn.
        if (chunk.type !== 'Error' || chunk.error.kind !== 'NoModelSelected') {
          isFirstTurnRef.current = false;
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

        if (chunk.type === 'Done') {
          completeGeneration();
          setIsGenerating(false);
          setSearchStage(null);
          onTurnComplete?.(userMsg, {
            ...assistantMsg,
            content: currentContent,
            thinkingContent: currentThinkingContent || undefined,
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
        await invoke('ask_ollama', {
          message: promptOverride ?? displayContent,
          quotedText: quotedText ?? null,
          imagePaths: imagePaths && imagePaths.length > 0 ? imagePaths : null,
          think: think ?? false,
          conversationId,
          isFirstTurn,
          slashCommand: think ? '/think' : null,
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
   * Submits a `/search` pipeline turn.
   *
   * @param query Text sent to the backend pipeline, without the `/search` trigger.
   * @param displayContent Text shown in the user bubble. Defaults to `query`.
   * @param quotedText Selected host-app text shown above the user bubble, if any.
   */
  const askSearch = useCallback(
    async (
      query: string,
      displayContent?: string,
      quotedText?: string,
    ): Promise<SearchOutcome> => {
      const trimmed = query.trim();
      if (!trimmed) return { final: true };

      if (activeGenerationRef.current) return { final: true };
      const pendingCancel = pendingCancelRef.current;
      if (pendingCancel) {
        await pendingCancel;
      }
      if (activeGenerationRef.current) return { final: true };

      const userMsg: Message = {
        id: crypto.randomUUID(),
        role: 'user',
        content: displayContent ?? trimmed,
        quotedText,
      };
      const assistantId = crypto.randomUUID();
      const assistantMsg: Message = {
        id: assistantId,
        role: 'assistant',
        content: '',
        fromSearch: true,
        modelName: activeModel ?? undefined,
      };

      setMessages((prev) => [...prev, userMsg, assistantMsg]);
      setIsGenerating(true);
      setSearchStage(null);

      const channel = new Channel<SearchEvent>();
      let currentContent = '';
      let sawToken = false;
      let pendingSources: SearchResultPreview[] | undefined;
      let warnings: SearchWarning[] = [];
      let pendingTraces: SearchTraceStep[] = [];
      let pendingMetadata: SearchMetadata | undefined;
      let awaitingClarification = false;
      let errored = false;
      let cancelled = false;

      const updateAssistant = (patch: Partial<Message>) => {
        setMessages((prev) =>
          prev.map((message) =>
            message.id === assistantId ? { ...message, ...patch } : message,
          ),
        );
      };

      return new Promise<SearchOutcome>((resolve) => {
        const generationId = beginGeneration(assistantId, resolve);

        const finish = (final: boolean) => {
          const active = completeGeneration();

          setIsGenerating(false);
          setSearchStage(null);

          const finalizedTraces = finalizeSearchTraceSteps(pendingTraces);
          if (finalizedTraces) {
            pendingTraces = finalizedTraces;
          }
          const persistedTraces = finalizedTraces;

          if (!errored && !cancelled && currentContent) {
            updateAssistant({
              searchSources: pendingSources,
              searchWarnings: warnings.length > 0 ? warnings : undefined,
              searchTraces: persistedTraces,
              searchMetadata: pendingMetadata,
            });
            onTurnComplete?.(userMsg, {
              ...assistantMsg,
              content: currentContent,
              searchSources: pendingSources,
              searchWarnings: warnings.length > 0 ? warnings : undefined,
              searchTraces: persistedTraces,
              searchMetadata: pendingMetadata,
            });
          }

          active.resolveSearch?.({ final });
        };

        // Once the backend emits RefiningSearch, every later searching or
        // reading stage belongs to a follow-up round rather than the initial one.
        let inGapRound = false;

        channel.onmessage = (event) => {
          if (!isActiveGeneration(generationId)) {
            return;
          }

          // Backend bails BEFORE recording `ConversationStart` on the
          // pre-pipeline `NoModelSelected` and `SandboxUnavailable` paths.
          // Every other event proves the trace was opened, so the flag
          // must be retired then. Keeping the flag armed across these
          // bails preserves trace integrity across a failed-then-retried
          // first /search turn.
          if (
            event.type !== 'NoModelSelected' &&
            event.type !== 'SandboxUnavailable'
          ) {
            isFirstTurnRef.current = false;
          }

          switch (event.type) {
            case 'Trace': {
              pendingTraces = upsertSearchTraceStep(pendingTraces, event.step);
              awaitingClarification ||= event.step.kind === 'clarify';
              updateAssistant({ searchTraces: pendingTraces });
              break;
            }
            case 'AnalyzingQuery': {
              setSearchStage({ kind: 'analyzing_query' });
              break;
            }
            case 'Searching': {
              setSearchStage(
                inGapRound
                  ? { kind: 'searching', gap: true }
                  : { kind: 'searching' },
              );
              break;
            }
            case 'FetchingUrl':
            case 'ReadingSources': {
              setSearchStage(
                inGapRound
                  ? { kind: 'reading_sources', gap: true }
                  : { kind: 'reading_sources' },
              );
              break;
            }
            case 'RefiningSearch': {
              inGapRound = true;
              setSearchStage({
                kind: 'refining_search',
                attempt: event.attempt,
                total: event.total,
              });
              break;
            }
            case 'Composing': {
              setSearchStage(
                inGapRound
                  ? { kind: 'composing', gap: true }
                  : { kind: 'composing' },
              );
              break;
            }
            case 'Sources': {
              pendingSources = event.results;
              break;
            }
            case 'Token': {
              sawToken ||= event.content.length > 0;
              currentContent += event.content;
              if (event.content) {
                markVisibleOutput();
              }
              setSearchStage(null);
              updateAssistant({ content: currentContent });
              break;
            }
            case 'IterationComplete': {
              const finalizedTraces = finalizeSearchTraceSteps(pendingTraces);
              if (finalizedTraces) {
                pendingTraces = finalizedTraces;
                updateAssistant({ searchTraces: finalizedTraces });
              }
              break;
            }
            case 'Warning': {
              warnings = [...warnings, event.warning];
              break;
            }
            case 'Done': {
              pendingMetadata = event.metadata ?? pendingMetadata;
              finish(!awaitingClarification && sawToken);
              break;
            }
            case 'Cancelled': {
              const active = completeGeneration();
              cancelled = true;
              if (!currentContent) {
                setMessages((prev) =>
                  prev.filter((message) => message.id !== assistantId),
                );
              }
              setIsGenerating(false);
              setSearchStage(null);
              active.resolveSearch?.({ final: true });
              break;
            }
            case 'Error': {
              errored = true;
              updateAssistant({
                content: event.message,
                errorKind: 'Other',
              });
              finish(true);
              break;
            }
            case 'SandboxUnavailable': {
              errored = true;
              updateAssistant({ sandboxUnavailable: true });
              finish(true);
              break;
            }
            case 'NoModelSelected': {
              errored = true;
              // Mirror the chat path's `OllamaErrorKind::NoModelSelected`
              // bubble copy verbatim so the user sees a single canonical
              // call-to-action regardless of which command tripped the gate.
              updateAssistant({
                content: 'No model selected\nPick a model in the picker.',
                errorKind: 'NoModelSelected',
              });
              finish(true);
              break;
            }
          }
        };

        const searchConversationId = ensureTraceConversationId();
        const searchIsFirstTurn = isFirstTurnRef.current;
        // The ref is flipped inside `channel.onmessage` once the backend
        // emits anything other than the pre-`ConversationStart` bail
        // signals (`NoModelSelected`, `SandboxUnavailable`). Flipping here
        // would burn the flag on those bails and leave the next attempt
        // without an opening trace event.
        invoke('search_pipeline', {
          message: trimmed,
          conversationId: searchConversationId,
          isFirstTurn: searchIsFirstTurn,
          // `displayContent` is the literal text the user typed (with
          // the `/search ` prefix preserved), used by the backend to
          // populate the chat-domain `user_message.content` so the
          // chat trace file shows exactly what the user submitted.
          // `message` (the stripped query) is what the search engine
          // receives.
          displayedContent: displayContent ?? trimmed,
          onEvent: channel,
        }).catch(() => {
          if (!isActiveGeneration(generationId) || errored || cancelled) return;
          errored = true;
          updateAssistant({
            content: 'Something went wrong\nCould not start search.',
            errorKind: 'Other',
          });
          finish(true);
        });
      });
    },
    [onTurnComplete, activeModel, ensureTraceConversationId],
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

    if (!pendingCancelRef.current) {
      const cancelPromise = (async () => {
        try {
          await invoke('cancel_generation');
        } catch {
          // Local hard-abort already reset the UI; backend best-effort only.
        } finally {
          pendingCancelRef.current = null;
        }
      })();
      pendingCancelRef.current = cancelPromise;
    }

    await pendingCancelRef.current;
  }, [abortActiveGeneration, isGenerating]);

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
    abortActiveGeneration();
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
  }, [abortActiveGeneration]);

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
      abortActiveGeneration();
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
    [abortActiveGeneration],
  );

  /**
   * Active trace conversation id for the current session. Exposed so
   * sibling commands invoked from `App.tsx` (notably
   * `capture_full_screen_command` for `/screen`) can route their
   * trace events to the same per-conversation file.
   */
  const getTraceConversationId = ensureTraceConversationId;

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
  };
}
