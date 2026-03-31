/**
 * Reads a numeric environment variable exposed by Vite, falling back to the
 * provided default when the variable is unset or not a valid integer.
 */
function envInt(key: string, fallback: number): number {
  const raw = import.meta.env[key];
  if (raw == null || raw === '') return fallback;
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

export const quote = {
  /** Maximum number of lines shown in the quote preview (AskBar and ChatBubble). */
  maxDisplayLines: envInt('VITE_QUOTE_MAX_DISPLAY_LINES', 4),
  /** Maximum total characters shown in the quote preview. */
  maxDisplayChars: envInt('VITE_QUOTE_MAX_DISPLAY_CHARS', 300),
  /** Maximum length of selected context text included in the Ollama prompt. */
  maxContextLength: envInt('VITE_QUOTE_MAX_CONTEXT_LENGTH', 4096),
} as const;
