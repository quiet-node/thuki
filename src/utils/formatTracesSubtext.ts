/**
 * Formats the on-disk trace footprint as a compact, muted subtext line,
 * e.g. `12 traces · 4.2 MB on disk`. Pairs with the backend `traces_stats`
 * command (`{ count, bytes }`).
 *
 * Empty state (`count === 0`) returns honest copy ("No traces recorded yet")
 * rather than "0 traces · 0 B on disk", so the row never advertises an empty
 * folder as if it held data.
 */

/** Human-readable byte size, 1024-based, e.g. 4404019 -> "4.2 MB", 1024 -> "1 KB". */
function formatBytes(bytes: number): string {
  const KB = 1024;
  const MB = KB * 1024;
  const GB = MB * 1024;
  // Trim a whole-number decimal so 1 MB reads "1 MB", not "1.0 MB".
  const trim = (n: number): string => `${Number(n.toFixed(1))}`;
  if (bytes < KB) return `${bytes} B`;
  if (bytes < MB) return `${trim(bytes / KB)} KB`;
  if (bytes < GB) return `${trim(bytes / MB)} MB`;
  return `${trim(bytes / GB)} GB`;
}

/**
 * Renders the trace count + total size for the Diagnostics subtext.
 *
 * @param count Number of recorded trace files.
 * @param bytes Combined size of those files, in bytes.
 * @returns A single line: the empty-state string when `count` is 0, otherwise
 *   `"<count> trace(s) · <size> on disk"` with correct singular/plural.
 */
export function formatTracesSubtext(count: number, bytes: number): string {
  if (count === 0) return 'No traces recorded yet';
  const noun = count === 1 ? 'trace' : 'traces';
  return `${count} ${noun} · ${formatBytes(bytes)} on disk`;
}
