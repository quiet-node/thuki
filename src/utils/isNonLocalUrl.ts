/**
 * Returns true when `url` points at a non-local host: not localhost, not a
 * loopback address, and not an RFC1918 / link-local private range. Drives the
 * Providers settings warning that a remote Ollama server is the user's
 * responsibility to secure (Ollama has no built-in auth).
 *
 * Malformed or empty URLs are treated as local (no warning): the field may
 * still be mid-edit, and the backend normalizes anything unusable.
 */
export function isNonLocalUrl(url: string): boolean {
  let hostname: string;
  try {
    hostname = new URL(url).hostname.toLowerCase();
  } catch {
    return false;
  }
  // URL.hostname keeps the brackets around IPv6 literals; strip them.
  const host = hostname.replace(/^\[/, '').replace(/\]$/, '');

  if (
    host === 'localhost' ||
    host.endsWith('.localhost') ||
    host === '127.0.0.1' ||
    host === '::1'
  ) {
    return false;
  }

  // RFC1918 private ranges + 169.254/16 link-local.
  if (
    /^10\./.test(host) ||
    /^192\.168\./.test(host) ||
    /^169\.254\./.test(host) ||
    /^172\.(1[6-9]|2\d|3[0-1])\./.test(host)
  ) {
    return false;
  }

  return true;
}
