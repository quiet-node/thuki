/**
 * Formats a Unix timestamp (seconds) as a human-readable relative time string.
 *
 * Examples: "just now", "2 minutes ago", "2 hours ago", "2 days ago"
 *
 * Accepts Unix timestamps in seconds (as returned by the Rust updater state).
 */
export function formatRelative(unix: number): string {
  const seconds = Math.floor(Date.now() / 1000) - unix;
  if (seconds < 60) return 'just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)} minutes ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} hours ago`;
  return `${Math.floor(seconds / 86400)} days ago`;
}
