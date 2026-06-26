/**
 * Pragmatic email shape check shared by the onboarding subscribe step and the
 * Settings "Help shape Thuki" card: a single run of non-space/non-`@`
 * characters, an `@`, a domain, a dot, and a TLD. This is a client-side guard
 * to keep obviously malformed input out of the subscribe call, not an authority
 * on deliverability; the email service performs the real double-opt-in
 * confirmation and the backend re-validates the same shape as defense-in-depth.
 */
export const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** Whether `email`, once trimmed of surrounding whitespace, looks well-formed. */
export function isValidEmail(email: string): boolean {
  return EMAIL_PATTERN.test(email.trim());
}
