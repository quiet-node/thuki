/**
 * Returns true when `url` points at a non-local host: not localhost, not a
 * loopback address, and not an RFC1918 / link-local private range. Drives the
 * Providers settings warning that a remote Ollama server is the user's
 * responsibility to secure (Ollama has no built-in auth).
 *
 * Malformed or empty URLs are treated as local (no warning): the field may
 * still be mid-edit, and the backend normalizes anything unusable.
 *
 * Private/loopback IPv4 ranges are matched only when the host is a complete
 * dotted-quad literal, so a DNS name that merely begins with a private prefix
 * (`192.168.1.1.evil.com`) is correctly treated as remote and still warns.
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

  // localhost is reserved to loopback (RFC6761), as is any `*.localhost` name.
  if (host === 'localhost' || host.endsWith('.localhost')) {
    return false;
  }

  // IPv6 literals contain a colon (the port is not part of URL.hostname).
  // Only loopback `::1` is local; every other IPv6 address is treated as
  // remote so the warning still fires.
  if (host.includes(':')) {
    return host !== '::1';
  }

  // IPv4 literal: apply the loopback/private ranges. A non-IPv4 DNS name
  // falls through to the remote verdict below.
  if (isIpv4Literal(host)) {
    return !isPrivateIpv4(host);
  }

  return true;
}

/**
 * True when `host` is a four-group dotted-decimal IPv4 literal. The octet
 * ranges are not re-validated here: a host this shape only reaches us via
 * `URL.hostname`, which already rejects out-of-range IPv4 literals.
 */
function isIpv4Literal(host: string): boolean {
  return /^\d{1,3}(?:\.\d{1,3}){3}$/.test(host);
}

/**
 * True when an IPv4 literal falls in a loopback or private range: the whole
 * 127.0.0.0/8 loopback block, RFC1918 (10/8, 172.16-31/16, 192.168/16), or
 * 169.254/16 link-local. Callers must pass a value that already satisfies
 * {@link isIpv4Literal}, so the unanchored prefixes are safe.
 */
function isPrivateIpv4(host: string): boolean {
  return (
    /^127\./.test(host) ||
    /^10\./.test(host) ||
    /^192\.168\./.test(host) ||
    /^169\.254\./.test(host) ||
    /^172\.(1[6-9]|2\d|3[0-1])\./.test(host)
  );
}
