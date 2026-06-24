/**
 * IPC shapes for the built-in engine's starter model downloads.
 *
 * Mirrors the serde output of the Rust side:
 * - `src-tauri/src/models/registry.rs` (Starter, Tier, RamFit; snake_case)
 * - `src-tauri/src/models/mod.rs` (StarterOption)
 * - `src-tauri/src/models/download.rs` (DownloadEvent; adjacently tagged
 *   with `type`/`data`, variant names verbatim, kind values snake_case)
 * - `src-tauri/src/engine/runner.rs` (EngineStatus, emitted on the
 *   `engine:status` Tauri event)
 */

/** Coarse speed/quality dial; the picker's three rows. */
export type StarterTier = 'fast' | 'balanced' | 'smartest';

/** RAM-fit hint computed by the backend from `hw.memsize`. */
export type RamFit = 'fits' | 'tight' | 'too_big';

/** One curated starter model from the compile-time registry. */
export interface Starter {
  /** Stable slug, unique across the registry; the Staff Picks row key and the
   * id-keyed download key. Backend always sends it; optional here for
   * test-fixture ergonomics (onboarding keys on `tier` and never reads it). */
  id?: string;
  tier: StarterTier;
  /** Model family this entry belongs to (e.g. "Gemma", "Qwen", "gpt-oss").
   * Backend always sends it; optional here for test-fixture ergonomics. */
  family?: string;
  /** Use-case section the Discover staff-picks list groups this entry under
   * (e.g. "Everyday chat", "Compact & fast", "Deep reasoning"). Backend always
   * sends it; optional here for test-fixture ergonomics. */
  category?: string;
  display_name: string;
  repo: string;
  revision: string;
  file_name: string;
  sha256: string;
  size_bytes: number;
  quant: string;
  vision: boolean;
  thinking: boolean;
  /** Whether reasoning cannot be turned off (always reasons); true for gpt-oss.
   * Backend always sends it; optional here for test-fixture ergonomics. */
  reasoning_always?: boolean;
  mmproj_file: string | null;
  mmproj_sha256: string | null;
  mmproj_bytes: number;
  est_runtime_gb: number;
  /** Maximum context window in tokens the model was trained for (its GGUF
   * `context_length`). Backend always sends it for catalog rows; optional here
   * for test-fixture ergonomics and for sources that cannot determine it. */
  context_length?: number;
  license_note: string;
  /** Model maker shown in the Origin row (e.g. "OpenAI"). */
  origin: string;
  /** The maker's own official HF repo, opened from the Origin row to verify provenance. */
  origin_repo: string;
}

/** One starter picker row: registry entry plus machine-specific facts. */
export interface StarterOption {
  starter: Starter;
  fit: RamFit;
  installed: boolean;
  partial_bytes: number | null;
}

/** One Staff Picks catalog row. Same shape as {@link StarterOption}, but the
 * catalog is id-keyed: `starter.id` is always present, so the pane keys rows
 * and starts downloads by it. */
export interface StaffPickOption extends StarterOption {
  starter: Starter & { id: string };
}

/** Failure category carried by a `Failed` download event. */
export type DownloadFailKind =
  | 'offline'
  | 'http'
  | 'checksum'
  | 'disk_full'
  | 'other';

/** Progress events streamed over the `download_starter` channel. */
export type DownloadEvent =
  | {
      type: 'Started';
      data: { file: string; total_bytes: number; resumed_from: number };
    }
  | {
      type: 'Progress';
      data: { file: string; bytes: number; total_bytes: number };
    }
  | { type: 'Verifying'; data: { file: string } }
  | { type: 'FileDone'; data: { file: string } }
  | { type: 'AllDone' }
  | { type: 'Cancelled' }
  | { type: 'Failed'; data: { kind: DownloadFailKind; message: string } };

/** One installed-model manifest row (`list_installed_models`). Mirrors the
 * serde output of `models::manifest::InstalledModel`; only the fields the
 * Settings UI consumes are declared. */
export interface InstalledModel {
  /** Stable key: `"<repo>:<file_name>"`. Written to the builtin provider's `model` field. */
  id: string;
  /** Human-readable label (e.g. the GGUF file stem). */
  display_name: string;
  /** Weights file size in bytes, for the installed-list size column. */
  size_bytes: number;
  /** Quantisation label (e.g. "Q4_K_M"); empty when unknown. */
  quant: string;
  /** RAM-fit on this Mac, computed by the backend from the recorded size.
   * `null`/absent when host RAM or the size is unknown. */
  fit?: RamFit | null;
  /** Trained context window in tokens, healed from the curated registry by the
   * backend; `null`/absent for a pasted model with no registry entry. */
  context_length?: number | null;
  /** Vision projector size in bytes, healed from the registry; added to
   * `size_bytes` for the displayed total so it matches Discover. `0`/absent for
   * a text model or a pasted repo with no registry entry. */
  mmproj_bytes?: number;
  /** Model maker (e.g. "Google"), healed from the registry; `null`/absent for a
   * pasted repo, where the row falls back to the repo id. */
  origin?: string | null;
}

/** One `.gguf` row from `list_hf_repo_ggufs`, for the paste-a-repo browser.
 * `fit` is the accurate per-quant RAM-fit computed from the real file size. */
export interface HfGgufFile {
  file: string;
  size_bytes: number;
  fit?: RamFit | null;
  /** LFS content digest; the key used to discard this file's partial. */
  sha256: string;
  /** Bytes of an interrupted partial for this file on disk, or null when none. */
  partial_bytes: number | null;
  /** Whether this exact repo file is already recorded in the installed
   * manifest, so Browse-all shows an "Installed" marker, not a download button. */
  installed: boolean;
}

/**
 * A snapshot of one in-flight download for a window that did not start it.
 * Mirrors `models::ActiveDownload`: the backend slot `key`, the blob `shas` it
 * writes (the cross-window match discriminator), and its latest progress
 * `event` (`null` until the first event arrives). Returned in bulk by
 * `get_active_downloads` and broadcast singly on the `thuki://download-progress`
 * event.
 */
export interface ActiveDownload {
  key: string;
  shas: string[];
  event: DownloadEvent | null;
}

/** Engine lifecycle snapshot published on the `engine:status` event. */
export interface EngineStatus {
  state: 'stopped' | 'starting' | 'loaded' | 'stopping' | 'failed';
  model_path: string;
  port: number | null;
  error: string | null;
}
