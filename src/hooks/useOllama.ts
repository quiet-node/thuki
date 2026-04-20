import { useCallback, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  SearchEvent,
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
  /** Source links forwarded by the search pipeline. */
  searchSources?: SearchResultPreview[];
  /** Warnings emitted by the `/search` pipeline during this turn. */
  searchWarnings?: SearchWarning[];
  /** When true, renders sandbox setup guidance instead of normal content. */
  sandboxUnavailable?: boolean;
  /** Ordered, user-facing timeline steps for a `/search` turn. */
  searchTraces?: SearchTraceStep[];
}

/** The expected structure of streaming chunks emitted from the Rust backend. */
export type StreamChunk =
  | { type: 'Token'; data: string }
  | { type: 'ThinkingToken'; data: string }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; data: { kind: OllamaErrorKind; message: string } };

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

  const markVisibleOutput = (generationId: number) => {
    if (activeGenerationRef.current?.id === generationId) {
      activeGenerationRef.current.hasVisibleOutput = true;
    }
  };

  const completeGeneration = (generationId: number) => {
    if (activeGenerationRef.current?.id !== generationId) {
      return null;
    }
    const active = activeGenerationRef.current;
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
      };

      setMessages((prev) => [...prev, userMsg, assistantMsg]);
      setIsGenerating(true);
      const generationId = beginGeneration(assistantId);

      const channel = new Channel<StreamChunk>();
      let currentContent = '';
      let currentThinkingContent = '';

      channel.onmessage = (chunk) => {
        if (!isActiveGeneration(generationId)) {
          return;
        }

        if (chunk.type === 'ThinkingToken') {
          currentThinkingContent += chunk.data;
          if (chunk.data) {
            markVisibleOutput(generationId);
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
          currentContent += chunk.data;
          if (chunk.data) {
            markVisibleOutput(generationId);
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
          const active = completeGeneration(generationId);
          if (!active) {
            return;
          }
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
          const active = completeGeneration(generationId);
          if (!active) {
            return;
          }
          if (!currentContent && !currentThinkingContent) {
            setMessages((prev) =>
              prev.filter((message) => message.id !== assistantId),
            );
          }
          setIsGenerating(false);
          setSearchStage(null);
          return;
        }

        const active = completeGeneration(generationId);
        if (!active) {
          return;
        }

        setMessages((prev) =>
          prev.map((message) =>
            message.id === assistantId
              ? {
                  ...message,
                  content: chunk.data.message,
                  errorKind: chunk.data.kind,
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
        completeGeneration(generationId);
        setMessages((prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: 'assistant',
            content: 'Something went wrong\nCould not reach Ollama.',
            errorKind: 'Other',
          },
        ]);
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
          const active = completeGeneration(generationId);
          if (!active) {
            return;
          }

          setIsGenerating(false);
          setSearchStage(null);

          const finalizedTraces = finalizeSearchTraceSteps(pendingTraces);
          if (finalizedTraces) {
            pendingTraces = finalizedTraces;
          }

          if (!errored && !cancelled && currentContent) {
            updateAssistant({
              searchSources: pendingSources,
              searchWarnings: warnings.length > 0 ? warnings : undefined,
              searchTraces: finalizedTraces,
            });
            onTurnComplete?.(userMsg, {
              ...assistantMsg,
              content: currentContent,
              searchSources: pendingSources,
              searchWarnings: warnings.length > 0 ? warnings : undefined,
              searchTraces: finalizedTraces,
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
              // flushSync makes the trace feel live instead of waiting for the
              // next paint batch.
              // eslint-disable-next-line @eslint-react/dom/no-flush-sync
              flushSync(() => {
                updateAssistant({ searchTraces: pendingTraces });
              });
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
                markVisibleOutput(generationId);
              }
              setSearchStage(null);
              updateAssistant({ content: currentContent });
              break;
            }
            case 'IterationComplete': {
              const finalizedTraces = finalizeSearchTraceSteps(pendingTraces);
              if (finalizedTraces) {
                pendingTraces = finalizedTraces;
                // eslint-disable-next-line @eslint-react/dom/no-flush-sync
                flushSync(() => {
                  updateAssistant({ searchTraces: finalizedTraces });
                });
              }
              break;
            }
            case 'Warning': {
              warnings = [...warnings, event.warning];
              break;
            }
            case 'Done': {
              finish(!awaitingClarification && sawToken);
              break;
            }
            case 'Cancelled': {
              const active = completeGeneration(generationId);
              if (!active) {
                return;
              }
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
      let cancelPromise: Promise<void> | undefined = undefined;
      cancelPromise = (async () => {
        try {
          await invoke('cancel_generation');
        } catch {
          // Local hard-abort already reset the UI; backend best-effort only.
        } finally {
          if (pendingCancelRef.current === cancelPromise) {
            pendingCancelRef.current = null;
          }
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
