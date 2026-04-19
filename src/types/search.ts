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
 * Warnings emitted by the agentic search pipeline. String values match the
 * Rust `SearchWarning` enum under `#[serde(rename_all = "snake_case")]`.
 */
export type SearchWarning =
  | 'reader_unavailable'
  | 'reader_partial_failure'
  | 'no_results_initial'
  | 'iteration_cap_exhausted'
  | 'router_failure'
  | 'synthesis_interrupted';

export type SearchEvent =
  | { type: 'AnalyzingQuery' }
  | { type: 'Searching' }
  | { type: 'ReadingSources' }
  | { type: 'RefiningSearch'; attempt: number; total: number }
  | { type: 'Composing' }
  | { type: 'Sources'; results: SearchResultPreview[] }
  | { type: 'Token'; content: string }
  | { type: 'Warning'; warning: SearchWarning }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; message: string }
  /** Pre-flight sandbox probe failed: the SearXNG or reader container is not
   * running. The frontend renders a static setup-guidance card. */
  | { type: 'SandboxUnavailable' };

/**
 * Transient UI stage indicator shown while the search pipeline is running.
 * `null` means the pipeline is idle or has finished streaming tokens.
 *
 * `refining_search` carries attempt/total so the UI can render
 * "Refining search (2/3)". Initial round renders `searching` only.
 */
export type SearchStage =
  | null
  | { kind: 'analyzing_query' }
  | { kind: 'searching'; gap?: boolean }
  | { kind: 'reading_sources'; gap?: boolean }
  | { kind: 'refining_search'; attempt: number; total: number }
  | { kind: 'composing'; gap?: boolean };
