/**
 * Stable per-download identity and its backend slot key.
 *
 * The backend keys its concurrent-download slots by an opaque string the
 * frontend supplies (see `DownloadState` in `models/mod.rs`). Deriving that key
 * in one place keeps the onboarding hook ({@link useDownloadModel}) and the
 * Settings download registry ({@link useDownloads}) naming the same download the
 * same way, so the backend's per-key dedupe behaves predictably across both.
 */

/** What a download produces, enough to name it and to replay/display it. */
export type DownloadIdentity =
  | { kind: 'tier'; tier: string }
  | { kind: 'staff'; id: string }
  | { kind: 'repo'; repo: string; file: string };

/**
 * The backend slot key for a download. Kind-prefixed so a Staff Picks id can
 * never collide with a repo path, and newline-joined for repos (a newline
 * cannot appear in a Hugging Face repo id or GGUF filename) so a `repo`/`file`
 * pair maps to exactly one key.
 */
export function downloadKey(identity: DownloadIdentity): string {
  switch (identity.kind) {
    case 'tier':
      return `tier:${identity.tier}`;
    case 'staff':
      return `staff:${identity.id}`;
    case 'repo':
      return `repo:${identity.repo}\n${identity.file}`;
  }
}
