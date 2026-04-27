/**
 * Render-time defense against special turn-boundary tokens that may have
 * leaked into stored assistant content. Backend `sanitize_assistant_content`
 * (Rust) strips these before persisting, but pre-Phase-B conversations on
 * disk may already carry the dirty bytes. The render-time scrub keeps those
 * legacy messages visually clean without a SQLite migration.
 *
 * Keep this list in lock-step with `STRIP_PATTERNS` in
 * `src-tauri/src/commands.rs`. Exact-string match, case-sensitive: these
 * markers are not natural English so a false-positive collision would
 * already be a bug elsewhere.
 */
export const STRIP_PATTERNS: readonly string[] = [
  '<|im_start|>',
  '<|im_end|>',
  '<|begin_of_text|>',
  '<|end_of_text|>',
  '<|start_header_id|>',
  '<|end_header_id|>',
  '<|eot_id|>',
  '[INST]',
  '[/INST]',
  '<start_of_turn>',
  '<end_of_turn>',
  '<|endoftext|>',
  '<|user|>',
  '<|assistant|>',
  '<|system|>',
  '<think>',
  '</think>',
];

/**
 * Removes special turn-boundary tokens (see {@link STRIP_PATTERNS}) from
 * a string before it is handed to the markdown renderer. Idempotent: a
 * clean string is returned unchanged. Pure: no allocation when no
 * pattern is present.
 */
export function cleanForRender(content: string): string {
  if (!content) return content;
  let out = content;
  for (const pattern of STRIP_PATTERNS) {
    if (out.includes(pattern)) {
      out = out.split(pattern).join('');
    }
  }
  return out;
}
