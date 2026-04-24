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
export type OllamaErrorKind = 'NotRunning' | 'ModelNotFound' | 'Other';

/** Represents a single message in the chat thread. */
export interface Message {
  /** Unique identifier for stable React list keys. */
  id: string;
  role: 'user' | 'assistant';
  content: string;
  /** Ollama model slug that produced this message. Present on assistant messages once the stream completes. */
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
 */
export function useOllama(
  onTurnComplete?: (userMsg: Message, assistantMsg: Message) => void,
) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isGenerating, setIsGenerating] = useState(false);
  /** Transient stage indicator for the active `/search` pipeline, if any. */
  const [searchStage, setSearchStage] = useState<SearchStage>(null);
  const activeGenerationRef = useRef<ActiveGeneration | null>(null);
  const nextGenerationIdRef = useRef(0);
  const pendingCancelRef = useRef<Promise<void> | null>(null);

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

      try {
        await invoke('ask_ollama', {
          message: promptOverride ?? displayContent,
          quotedText: quotedText ?? null,
          imagePaths: imagePaths && imagePaths.length > 0 ? imagePaths : null,
          think: think ?? false,
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
    [onTurnComplete],
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
          }
        };

        invoke('search_pipeline', {
          message: trimmed,
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
    [onTurnComplete],
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

  /** Resets all conversation state for a fresh session. */
  const reset = useCallback(() => {
    abortActiveGeneration();
    setMessages([]);
    void invoke('reset_conversation');
  }, [abortActiveGeneration]);

  /** Replaces the current message list with a previously loaded set of messages. */
  const loadMessages = useCallback(
    (msgs: Message[]) => {
      abortActiveGeneration();
      setMessages(msgs);
    },
    [abortActiveGeneration],
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
  };
}
