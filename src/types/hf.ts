/* v8 ignore file -- type-only declarations, no runtime code */

/**
 * IPC shapes for the in-app Hugging Face GGUF model browser (the Discover
 * pane). Mirrors the serde output of the Rust `search_hf_models` command,
 * which serializes its `HfModelSummary` struct as snake_case.
 */

/**
 * One repo row from `search_hf_models`. The search payload is deliberately
 * lean: it carries what the Discover list needs to render a row and decide
 * whether anonymous download is allowed. RAM-fit is not carried here; it shows
 * only on the per-quant rows (where a real file size makes it accurate), which
 * arrive at the expand step.
 *
 * - `id` is the canonical `owner/repo` slug.
 * - `downloads` is Hugging Face's all-time download count for the repo.
 * - `gated` is true when the repo requires accepting terms or auth; an
 *   anonymous download fails, so the Discover row disables download for it.
 * - `vision` / `thinking` are per-model capabilities (every quant shares them),
 *   rendered as pills on the repo row.
 */
export interface HfModelSummary {
  /** Canonical `owner/repo` slug. */
  id: string;
  /** All-time Hugging Face download count for the repo. */
  downloads: number;
  /** True when the repo is gated; anonymous downloads fail. */
  gated: boolean;
  /** Model's trained context window in tokens, from the repo's parsed GGUF
   * metadata (a per-repo property shared by every quant); `null`/absent when
   * unknown or untrusted. */
  context_length?: number | null;
  /** True when the repo ships an mmproj vision companion (accepts image input). */
  vision: boolean;
  /** True when the model emits reasoning tokens. */
  thinking: boolean;
}

/**
 * One page of search results from `search_hf_models`. `has_more` is derived from
 * the raw Hub entry count, not `rows.length`, so the backend's chat-model
 * allowlist (which drops non-chat repos) never ends pagination early.
 */
export interface HfSearchPage {
  /** The chat-capable repo rows for this page. */
  rows: HfModelSummary[];
  /** True when the Hub returned a full page, so a next page may exist. */
  has_more: boolean;
}
