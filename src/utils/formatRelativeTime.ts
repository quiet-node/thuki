/**
 * Formats a Unix timestamp (milliseconds) as a human-readable relative time string.
 *
 * Examples: "just now", "2m ago", "5h ago", "3d ago", "2w ago"
 */
export function formatRelativeTime(
  unixMillis: number,
  nowMillis?: number,
): string {
  const now = nowMillis ?? Date.now();
  const diffSec = Math.max(0, Math.floor((now - unixMillis) / 1000));

  if (diffSec < 60) return 'just now';

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;

  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;

  const diffDay = Math.floor(diffHr / 24);
  if (diffDay < 14) return `${diffDay}d ago`;

  const diffWeek = Math.floor(diffDay / 7);
  return `${diffWeek}w ago`;
}
