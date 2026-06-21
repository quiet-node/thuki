/**
 * Settings-window download registry: many model downloads at once.
 *
 * Unlike onboarding (one starter at a time, {@link useDownloadModel}), the
 * Settings → Discover panes let a user fire off several downloads in parallel.
 * This provider holds one live download per key (the backend allows concurrent
 * downloads keyed the same way; see `DownloadState` in `models/mod.rs`) and,
 * sitting at the Settings window root, keeps every one of them alive across the
 * Library / Discover / Providers and Staff picks / Browse all tab switches that
 * unmount the panes.
 *
 * Each entry advances through the shared {@link reduceDownloadEvent} reducer
 * (engine handoff off: a Settings download finishes at `ready`). A row looks up
 * its own download by {@link downloadKey}; absence means "not downloading".
 */

import {
  createContext,
  use,
  useCallback,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import {
  type DownloadAccumulator,
  type DownloadProgressInfo,
  type DownloadUiState,
  reduceDownloadEvent,
  startingAccumulator,
} from '../hooks/downloadReducer';
import { downloadKey, type DownloadIdentity } from '../hooks/downloadKey';
import type { DownloadEvent } from '../types/starter';

/** What the Settings panes start: a Staff Picks id or a Browse-all repo file. */
type RegistryIdentity = Extract<
  DownloadIdentity,
  { kind: 'staff' } | { kind: 'repo' }
>;

/** The render-facing view of one live download. */
export interface DownloadView {
  state: DownloadUiState;
  progress: DownloadProgressInfo | null;
  etaSeconds: number | null;
  combinedBytes: number | null;
  speedBytesPerSec: number | null;
}

/**
 * Per-repo roll-up of a family's live downloads, by state, for the collapsed
 * Browse-all row pills. Counts only the in-memory registry's active states:
 * `downloading` (weights or its mmproj companion), `verifying`, and `failed`.
 * Terminal-success (`ready`) is omitted (it clears immediately), and paused
 * partials are not registry state at all (they live in the per-file listing
 * read on expand), so neither is summarisable here.
 */
export interface RepoDownloadSummary {
  downloading: number;
  verifying: number;
  failed: number;
}

/** Internal record: the identity (for retry replay) plus its accumulator. */
interface RegistryEntry {
  identity: RegistryIdentity;
  acc: DownloadAccumulator;
}

export interface DownloadsContextValue {
  /** The live download for `key` ({@link downloadKey}), or undefined when none. */
  get: (key: string) => DownloadView | undefined;
  /**
   * Whether any live download belongs to `repo`. Lets a Browse-all repo row
   * re-expand itself after a tab switch remounts it collapsed, before its quant
   * list (which would reveal the per-file downloads) has been fetched.
   */
  hasRepoDownload: (repo: string) => boolean;
  /**
   * Live download counts for `repo`, by state, for the collapsed-row pills.
   * Counts only repo-kind downloads belonging to `repo`; see
   * {@link RepoDownloadSummary}.
   */
  repoDownloadSummary: (repo: string) => RepoDownloadSummary;
  /** Start (or resume) a Staff Picks catalog download by its stable id. */
  startStaffPick: (id: string) => void;
  /** Start (or resume) a Browse-all repo download by repo + GGUF file. */
  startRepoDownload: (repo: string, file: string) => void;
  /** Cancel the download for `key`; the partial is kept for a later resume. */
  cancel: (key: string) => void;
  /** Retry the failed download for `key` (replays its original command). */
  retry: (key: string) => void;
  /** Discard a kept partial by blob sha256. */
  discard: (sha256: string) => Promise<void>;
  /** Drop a terminal (ready / failed) entry so its row returns to normal. */
  clear: (key: string) => void;
}

const DownloadsContext = createContext<DownloadsContextValue | null>(null);

/** The download command + args for a registry identity. */
function commandFor(
  identity: RegistryIdentity,
): [string, Record<string, unknown>] {
  switch (identity.kind) {
    case 'staff':
      return ['download_staff_pick', { id: identity.id }];
    case 'repo':
      return [
        'download_repo_model',
        { repo: identity.repo, file: identity.file },
      ];
  }
}

export function DownloadsProvider({ children }: { children: ReactNode }) {
  const [entries, setEntries] = useState<Map<string, RegistryEntry>>(
    () => new Map(),
  );
  // Latest entries for the imperative retry path (reads identity outside React
  // state). Mirrored every render so it never lags the rendered map.
  const entriesRef = useRef(entries);
  entriesRef.current = entries;

  const begin = useCallback((identity: RegistryIdentity) => {
    const key = downloadKey(identity);
    const [command, args] = commandFor(identity);
    setEntries((prev) => {
      const next = new Map(prev);
      next.set(key, { identity, acc: startingAccumulator() });
      return next;
    });
    const channel = new Channel<DownloadEvent>();
    channel.onmessage = (event) =>
      setEntries((prev) => {
        const cur = prev.get(key);
        // Entry cleared (Choose another) while a late event was in flight: drop.
        if (!cur) return prev;
        const acc = reduceDownloadEvent(cur.acc, event, false);
        const next = new Map(prev);
        // A Cancelled event resets to idle: prune so the row returns to its
        // Paused/partial controls instead of lingering as a dead download.
        if (acc.state.phase === 'idle') {
          next.delete(key);
        } else {
          next.set(key, { ...cur, acc });
        }
        return next;
      });
    void invoke(command, { ...args, key, onEvent: channel }).catch((err) =>
      // A rejected invoke means the command failed before streaming (e.g. the
      // repo spec could not be resolved), so no channel event will arrive: mark
      // the entry failed from the identity in scope.
      setEntries((prev) => {
        const next = new Map(prev);
        next.set(key, {
          identity,
          acc: {
            ...startingAccumulator(),
            state: { phase: 'failed', kind: 'other', message: String(err) },
          },
        });
        return next;
      }),
    );
  }, []);

  const startStaffPick = useCallback(
    (id: string) => begin({ kind: 'staff', id }),
    [begin],
  );

  const startRepoDownload = useCallback(
    (repo: string, file: string) => begin({ kind: 'repo', repo, file }),
    [begin],
  );

  const cancel = useCallback((key: string) => {
    void invoke('cancel_model_download', { key });
  }, []);

  const retry = useCallback(
    (key: string) => {
      const entry = entriesRef.current.get(key);
      if (entry) begin(entry.identity);
    },
    [begin],
  );

  const discard = useCallback(async (sha256: string) => {
    await invoke('discard_partial_download', { sha256 });
  }, []);

  const clear = useCallback((key: string) => {
    setEntries((prev) => {
      if (!prev.has(key)) return prev;
      const next = new Map(prev);
      next.delete(key);
      return next;
    });
  }, []);

  const get = useCallback(
    (key: string): DownloadView | undefined => {
      const entry = entries.get(key);
      if (!entry) return undefined;
      const { state, progress, etaSeconds, combinedBytes, speedBytesPerSec } =
        entry.acc;
      return { state, progress, etaSeconds, combinedBytes, speedBytesPerSec };
    },
    [entries],
  );

  const hasRepoDownload = useCallback(
    (repo: string): boolean => {
      for (const entry of entries.values()) {
        if (entry.identity.kind === 'repo' && entry.identity.repo === repo) {
          return true;
        }
      }
      return false;
    },
    [entries],
  );

  const repoDownloadSummary = useCallback(
    (repo: string): RepoDownloadSummary => {
      const summary: RepoDownloadSummary = {
        downloading: 0,
        verifying: 0,
        failed: 0,
      };
      for (const entry of entries.values()) {
        if (entry.identity.kind !== 'repo' || entry.identity.repo !== repo) {
          continue;
        }
        switch (entry.acc.state.phase) {
          case 'downloading':
          case 'downloading_mmproj':
            summary.downloading += 1;
            break;
          case 'verifying':
            summary.verifying += 1;
            break;
          case 'failed':
            summary.failed += 1;
            break;
        }
      }
      return summary;
    },
    [entries],
  );

  const value = useMemo<DownloadsContextValue>(
    () => ({
      get,
      hasRepoDownload,
      repoDownloadSummary,
      startStaffPick,
      startRepoDownload,
      cancel,
      retry,
      discard,
      clear,
    }),
    [
      get,
      hasRepoDownload,
      repoDownloadSummary,
      startStaffPick,
      startRepoDownload,
      cancel,
      retry,
      discard,
      clear,
    ],
  );

  return <DownloadsContext value={value}>{children}</DownloadsContext>;
}

/**
 * Returns the Settings download registry. Throws when no `DownloadsProvider`
 * wraps the caller: a live multi-download has no sensible static fallback, so a
 * missing provider is a wiring bug.
 */
export function useDownloads(): DownloadsContextValue {
  const value = use(DownloadsContext);
  if (value === null) {
    throw new Error('useDownloads must be used within a DownloadsProvider');
  }
  return value;
}
