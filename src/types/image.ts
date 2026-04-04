/**
 * Represents an image attached to the current (unsent) message.
 *
 * The `blobUrl` is available immediately on paste/drop for instant thumbnail
 * rendering. The `filePath` is set asynchronously once the Rust backend
 * finishes compressing and saving the image to disk.
 */
export interface AttachedImage {
  /** Unique identifier for stable React list keys. */
  id: string;
  /** Browser object URL for instant thumbnail rendering (no disk round-trip). */
  blobUrl: string;
  /** Absolute file path on disk, set once Rust processing completes. */
  filePath: string | null;
}

/** Maximum file size in bytes (30 MB). Files exceeding this are rejected. */
export const MAX_IMAGE_SIZE_BYTES = 30 * 1024 * 1024;
