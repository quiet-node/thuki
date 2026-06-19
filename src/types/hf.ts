/* v8 ignore file -- type-only declarations, no runtime code */

/**
 * IPC shapes for the in-app Hugging Face GGUF model browser (the Discover
 * pane). Mirrors the serde output of the Rust `search_hf_models` command,
 * which serializes its `HfModelRow` struct (a flattened `HfModelSummary` plus
 * an estimated RAM-fit) as snake_case.
 */

import type { RamFit } from './starter';

/**
 * One repo row from `search_hf_models`. The search payload is deliberately
 * lean: it carries what the Discover list needs to render a row, decide
 * whether anonymous download is allowed, and show an approximate RAM-fit.
 *
 * - `id` is the canonical `owner/repo` slug.
 * - `downloads` is Hugging Face's all-time download count for the repo.
 * - `gated` is true when the repo requires accepting terms or auth; an
 *   anonymous download fails, so the Discover row disables download for it.
 * - `fit` is the estimated RAM-fit for this Mac, derived from the parameter
 *   count in the repo id (no file size is available at search time); it is
 *   `null` when the id carries no `<number>B` token or host RAM is unknown.
 *   Accurate per-quant fit arrives at the expand step.
 */
export interface HfModelSummary {
  /** Canonical `owner/repo` slug. */
  id: string;
  /** All-time Hugging Face download count for the repo. */
  downloads: number;
  /** True when the repo is gated; anonymous downloads fail. */
  gated: boolean;
  /** Estimated RAM-fit for this Mac, or `null` when not derivable. */
  fit?: RamFit | null;
}
