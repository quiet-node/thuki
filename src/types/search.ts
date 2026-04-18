/**
 * TypeScript mirror of the Rust `SearchEvent` enum emitted by the
 * `search_pipeline` Tauri command. The `type` tag matches the
 * `#[serde(tag = "type")]` attribute on the Rust side.
 */
export interface SearchResultPreview {
  title: string;
  url: string;
}

export type SearchEvent =
  | { type: 'Classifying' }
  | { type: 'Clarifying'; question: string }
  | { type: 'Searching' }
  | { type: 'Sources'; results: SearchResultPreview[] }
  | { type: 'Token'; content: string }
  | { type: 'Done' }
  | { type: 'Cancelled' }
  | { type: 'Error'; message: string };

/**
 * Transient UI stage indicator shown while the search pipeline is running.
 * `null` means the pipeline is idle or has finished streaming tokens.
 */
export type SearchStage = 'classifying' | 'searching' | null;
