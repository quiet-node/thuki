/**
 * Maps a rejected `subscribe_email` invocation to the inline line shown to the
 * user.
 *
 * The backend returns stable discriminants, never the proxy's raw error body
 * (trust boundary). Only the `rate_limited` code (HTTP 429) earns its own copy;
 * every other failure (network, other non-2xx) collapses to one generic,
 * retryable line, so an unexpected rejection can never leak an internal string.
 */
export function subscribeErrorMessage(error: unknown): string {
  if (error === 'rate_limited') {
    return 'Too many requests right now. Please wait a minute and try again.';
  }
  return "Couldn't send right now. Please try again.";
}
