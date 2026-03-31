/**
 * Formats a selected text quote for display in the UI.
 * Preserves line breaks, caps at maxLines, truncates at maxChars.
 *
 * @param text The raw selected text
 * @param maxLines Maximum number of lines to display (default: 4)
 * @param maxChars Maximum total characters (default: 300)
 * @returns Formatted quote suitable for display
 */
export function formatQuotedText(
  text: string,
  maxLines: number = 4,
  maxChars: number = 300,
): string {
  if (!text) return '';
  const lines = text.split('\n');
  const result: string[] = [];
  let totalChars = 0;

  for (const line of lines) {
    // Stop if we've hit the line limit
    if (result.length >= maxLines) {
      result.push('...');
      break;
    }

    const trimmed = line.trim();
    // Skip empty lines but don't count them against the line limit
    if (!trimmed) continue;

    // If adding this line would exceed char limit, truncate and stop
    if (totalChars + trimmed.length > maxChars) {
      const remaining = maxChars - totalChars;
      result.push(trimmed.substring(0, remaining) + '...');
      break;
    }

    result.push(trimmed);
    totalChars += trimmed.length;
  }

  return result.join('\n');
}
