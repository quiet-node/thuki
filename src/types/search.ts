/**
 * Shared web-search UI types for live progress and source previews.
 */

/** One cited source shown in the answer footer and progress chrome. */
export interface SearchResultPreview {
  title: string;
  url: string;
  /**
   * Optional markdown attribution (licence / provider credit) with
   * `[label](url)` links. Present for Open-Meteo and Wikipedia verticals.
   */
  attribution?: string;
}

/**
 * Transient UI stage for live web search progress.
 * `null` means idle or answer tokens have started.
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
  | { kind: 'composing'; gap?: boolean }
  /** Citation audit (± repair) after answer tokens finished; sources pill status. */
  | { kind: 'verifying_sources' };
