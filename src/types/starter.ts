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
  tier: StarterTier;
  display_name: string;
  repo: string;
  revision: string;
  file_name: string;
  sha256: string;
  size_bytes: number;
  quant: string;
  vision: boolean;
  thinking: boolean;
  mmproj_file: string | null;
  mmproj_sha256: string | null;
  mmproj_bytes: number;
  est_runtime_gb: number;
  license_note: string;
}

/** One starter picker row: registry entry plus machine-specific facts. */
export interface StarterOption {
  starter: Starter;
  fit: RamFit;
  installed: boolean;
  partial_bytes: number | null;
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

/** Engine lifecycle snapshot published on the `engine:status` event. */
export interface EngineStatus {
  state: 'stopped' | 'starting' | 'loaded' | 'stopping' | 'failed';
  model_path: string;
  port: number | null;
  error: string | null;
}
