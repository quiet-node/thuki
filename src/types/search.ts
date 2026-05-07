/**
 * TypeScript mirror of the Rust `SearchEvent` enum emitted by the
 * `search_pipeline` Tauri command. The `type` tag matches the
 * `#[serde(tag = "type")]` attribute on the Rust side (CamelCase variants).
 */
export interface SearchResultPreview {
  title: string;
  url: string;
}

/**
 * User-facing trace step streamed from the backend while a `/search` turn is
 * running. The backend updates a step by re-emitting the same `id` with fresh
 * status, counts, or summary text.
 */
export interface SearchTraceStep {
  /** Stable identifier used to update the same timeline step over time. */
  id: string;
  /** Semantic stage within the search pipeline. */
  kind:
    | 'analyze'
    | 'clarify'
    | 'history_answer'
    | 'search'
    | 'url_rerank'
    | 'snippet_judge'
    | 'read'
    | 'chunk'
    | 'chunk_rerank'
    | 'chunk_judge'
    | 'refine'
    | 'compose';
  /** `running` while the stage is in flight; `completed` once finished. */
  status: 'running' | 'completed';
  /** 1-indexed retrieval round for search-loop stages. Omitted for non-round steps. */
  round?: number;
  /** Short stage title shown in the trace timeline. */
  title: string;
  /** Primary user-facing explanation for this stage. */
  summary: string;
  /** Optional secondary explanation such as missing details or fallback context. */
  detail?: string;
  /** Queries used or planned during this stage. */
  queries?: string[];
  /** Concrete page URLs considered during this stage, when showing exact pages matters. */
  urls?: string[];
  /** Deduplicated source domains relevant to this stage. */
  domains?: string[];
  /** Sufficiency verdict when a judge step finishes. */
  verdict?: 'sufficient' | 'partial' | 'insufficient';
  /** Lightweight counts surfaced as compact chips in the UI. */
  counts?: {
    found?: number;
    kept?: number;
    processed?: number;
    total?: number;
    pages?: number;
    chunks?: number;
    empty?: number;
    failed?: number;
    sources?: number;
  };
}

/**
 * Diagnostic record for a single retrieval iteration. Mirrors the Rust
 * `IterationTrace` struct.
 *
 * Legacy-only: retained so older persisted traces can still be parsed and so
 * coarse backend events continue to type-check while the UI uses
 * `SearchTraceStep`.
 */
export interface IterationTrace {
  /** Stage: `{ kind: 'initial' }` or `{ kind: 'gap_round', round: number }`. */
  stage: { kind: 'initial' } | { kind: 'gap_round'; round: number };
  queries: string[];
  urls_fetched: string[];
  reader_empty_urls: string[];
  judge_verdict: 'sufficient' | 'partial' | 'insufficient';
  judge_reasoning: string;
  duration_ms: number;
}

/**
 * End-of-pipeline metadata emitted with the final `Done` event for `/search`.
 * Mirrors the Rust `SearchMetadata` struct.
 */
export interface SearchMetadata {
  iterations: IterationTrace[];
  total_duration_ms: number;
  retries_performed: number;
}

/**
 * Warnings emitted by the agentic search pipeline. String values match the
 * Rust `SearchWarning` enum under `#[serde(rename_all = "snake_case")]`.
 */
export type SearchWarning =
  | 'reader_unavailable'
  | 'reader_partial_failure'
  | 'no_results_initial'
  | 'iteration_cap_exhausted'
  | 'router_failure'
  | 'judge_failure'
  | 'budget_exhausted'
  | 'no_progress'
  | 'synthesis_interrupted';

export type SearchEvent =
  | { type: 'Trace'; step: SearchTraceStep }
  | { type: 'AnalyzingQuery' }
  | { type: 'Searching'; queries: string[] }
  | { type: 'FetchingUrl'; url: string }
  | { type: 'ReadingSources' }
  | { type: 'RefiningSearch'; attempt: number; total: number }
  | { type: 'Composing' }
  | { type: 'Sources'; results: SearchResultPreview[] }
  | { type: 'Token'; content: string }
  | { type: 'Warning'; warning: SearchWarning }
  | { type: 'Done'; metadata?: SearchMetadata }
  | { type: 'Cancelled' }
  | { type: 'Error'; message: string }
  /** Pre-flight sandbox probe failed: the SearXNG or reader container is not
   * running. The frontend renders a static setup-guidance card. */
  | { type: 'SandboxUnavailable' }
  /** No active model is selected. Mirror of the chat path's
   * `Error { kind: 'NoModelSelected' }`; emitted instead of a generic Error so
   * the hook can keep `isFirstTurnRef` armed across a bail-and-retry. */
  | { type: 'NoModelSelected' }
  /** Backend confirmed it cleared every pre-`ConversationStart` gate and
   * opened the trace. Hook-only signal: retires `isFirstTurnRef` before
   * any token can arrive, so cancel-before-first-token cannot leave the
   * flag set and produce a duplicate `ConversationStart` next turn. */
  | { type: 'TurnAccepted' }
  /** Emitted after each retrieval iteration completes. Allows the frontend to
   * accumulate and render trace rows live as the pipeline progresses. */
  | { type: 'IterationComplete'; trace: IterationTrace };

/**
 * Transient state of the current in-progress search iteration. Populated by
 * `AnalyzingQuery`, `Searching`, and `FetchingUrl` events and cleared when
 * `IterationComplete` fires. Never persisted; exists only in React state during
 * an active search.
 */
export interface LiveIterationState {
  /**
   * Current phase of the in-progress iteration.
   * - `analyzing`: router LLM is running, no queries yet
   * - `searching`: queries dispatched to SearXNG, waiting for results
   * - `reading`: reader is fetching pages; fetchingUrls accumulates as each completes
   */
  kind: 'analyzing' | 'searching' | 'reading';
  /** Queries submitted to SearXNG for this iteration. */
  queries: string[];
  /** URLs that have completed reader fetching so far this iteration. */
  fetchingUrls: string[];
}

/**
 * Transient UI stage indicator shown while the search pipeline is running.
 * `null` means the pipeline is idle or has finished streaming tokens.
 *
 * `refining_search` carries the 1-indexed gap-round number and total gap
 * rounds so the UI can render text like "Refining search (1/2)". Initial
 * round renders `searching` only.
 */
export type SearchStage =
  | null
  | { kind: 'analyzing_query' }
  | { kind: 'searching'; gap?: boolean }
  | { kind: 'reading_sources'; gap?: boolean }
  | { kind: 'refining_search'; attempt: number; total: number }
  | { kind: 'composing'; gap?: boolean };
