/**
 * Formats a model's context window (in tokens) as a compact, human-readable
 * label for the model picker, e.g. 131072 -> "128K", 262144 -> "256K",
 * 1048576 -> "1M". The scale is 1024-based so the common power-of-two windows
 * read as round numbers, matching how llama.cpp tooling reports them.
 *
 * Defensive by design: the input may be an unvetted GGUF `context_length` from
 * an arbitrary Hugging Face repo, so a non-positive or non-finite value yields
 * an empty string (the caller skips the pill rather than rendering "NaNK").
 */
export function formatContextWindow(tokens: number): string {
  if (!Number.isFinite(tokens) || tokens <= 0) return '';
  const K = 1024;
  if (tokens >= K * K) {
    // Trim a whole-number decimal: 1048576 -> "1M", 1572864 -> "1.5M".
    return `${Number((tokens / (K * K)).toFixed(1))}M`;
  }
  if (tokens >= K) {
    return `${Math.round(tokens / K)}K`;
  }
  return `${Math.round(tokens)}`;
}
