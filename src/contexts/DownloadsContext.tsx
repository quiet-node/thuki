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
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  type DownloadAccumulator,
  type DownloadProgressInfo,
  type DownloadUiState,
  isDownloadInFlight,
  reduceDownloadEvent,
  startingAccumulator,
} from '../hooks/downloadReducer';
import { downloadKey, type DownloadIdentity } from '../hooks/downloadKey';
import type { ActiveDownload, DownloadEvent } from '../types/starter';

/**
 * Tauri event the backend broadcasts on every download progress update, to
 * every webview. Lets a window that did not start a download (this Settings
 * registry while onboarding downloads in the main window) render its live
 * progress, matched by blob sha. Mirrors `models::DOWNLOAD_PROGRESS_EVENT`.
 */
const DOWNLOAD_PROGRESS_EVENT = 'thuki://download-progress';

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

/**
 * Internal record for a download started in ANOTHER window: the blob shas it
 * writes (the cross-window match) plus its accumulator, folded from the global
 * progress broadcast. No identity: this window cannot retry someone else's
 * download, only watch and cancel it (by its real backend key).
 */
interface RemoteEntry {
  shas: string[];
  acc: DownloadAccumulator;
}

/** A live download resolved by blob sha: its real backend key and render view. */
export interface ActiveDownloadView {
  key: string;
  view: DownloadView;
}

/**
 * Folds a cross-window progress snapshot into the remote registry.
 *
 * Skips keys this window already owns locally: the local channel drives those,
 * and {@link reduceDownloadEvent} is NOT idempotent (a second fold of one event
 * phantom-counts bytes and mislabels the vision phase), so the channel and the
 * global-broadcast streams must never cross. A non-install terminal
 * (`Cancelled` -> idle, `Failed`) drops the entry so the row reverts to its
 * normal controls; a successful `AllDone` is kept as `ready` so the row's
 * install effect can flip it to Installed.
 */
export function applyRemoteEvent(
  prev: Map<string, RemoteEntry>,
  localKeys: ReadonlySet<string>,
  active: ActiveDownload,
): Map<string, RemoteEntry> {
  if (localKeys.has(active.key)) return prev;
  const base = prev.get(active.key)?.acc ?? startingAccumulator();
  const acc = active.event
    ? reduceDownloadEvent(base, active.event, false)
    : base;
  const next = new Map(prev);
  if (acc.state.phase === 'idle' || acc.state.phase === 'failed') {
    next.delete(active.key);
  } else {
    next.set(active.key, { shas: active.shas, acc });
  }
  return next;
}

/**
 * Seeds the remote registry from the mount snapshot, non-clobbering: a live
 * event that already arrived (during the `get_active_downloads` await) is
 * fresher than the snapshot, so an existing entry is left untouched.
 */
export function seedRemoteSnapshot(
  prev: Map<string, RemoteEntry>,
  localKeys: ReadonlySet<string>,
  active: ActiveDownload,
): Map<string, RemoteEntry> {
  if (prev.has(active.key)) return prev;
  return applyRemoteEvent(prev, localKeys, active);
}

export interface DownloadsContextValue {
  /** The live download for `key` ({@link downloadKey}), or undefined when none. */
  get: (key: string) => DownloadView | undefined;
  /**
   * A live download started in ANOTHER window that is writing the blob `sha`,
   * with its real backend key (for cancel) and render view, or undefined when
   * none. Lets a Settings row reflect a download started from onboarding (whose
   * slot key differs) by matching on the underlying weights blob.
   */
  getActiveDownload: (sha: string) => ActiveDownloadView | undefined;
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
  /**
   * This download's 1-indexed position among all currently-queued downloads
   * (phase `queued`), across both `remote` and `entries` in their Map
   * insertion order (a stand-in for start order, since neither map ever
   * reorders an existing key). Returns `undefined` if `key` is not currently
   * queued. Feeds a row's own "#N in queue" badge while it is queued.
   */
  queuePosition: (key: string) => number | undefined;
  /**
   * Count of downloads currently in the `queued` phase, across both `remote`
   * and `entries`. Derived from the same ordering as {@link queuePosition} so
   * the two can never disagree; a row uses this alongside its own position to
   * decide whether the "#N in queue" badge is worth showing (a lone queued
   * item has no other position to be numbered against).
   */
  queuedTotal: number;
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

  // Downloads started in OTHER windows, folded from the global progress
  // broadcast and the mount snapshot, keyed by their real backend key. Disjoint
  // from `entries` by construction (the broadcast handler skips locally-owned
  // keys), so the non-idempotent reducer is never fed an event twice.
  const [remote, setRemote] = useState<Map<string, RemoteEntry>>(
    () => new Map(),
  );

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;
    const localKeys = () => new Set(entriesRef.current.keys());

    // Subscribe BEFORE fetching the snapshot so a live event arriving during the
    // await is not lost (and, being fresher, wins over the older snapshot). On
    // teardown the subscription is removed (here when it resolves after unmount,
    // or by the cleanup's unlisten), so the handler never runs post-unmount.
    void listen<ActiveDownload>(DOWNLOAD_PROGRESS_EVENT, ({ payload }) => {
      setRemote((prev) => applyRemoteEvent(prev, localKeys(), payload));
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {
        // Event bridge unavailable (test env / Tauri not ready): local channel
        // downloads still work; cross-window live progress is simply absent.
      });

    void invoke<ActiveDownload[]>('get_active_downloads')
      .then((list) => {
        // A missing/non-array result (no Tauri, or a malformed response) leaves
        // the registry empty; the live event stream still hydrates rows.
        if (cancelled || !Array.isArray(list)) return;
        setRemote((prev) => {
          const keys = localKeys();
          return list.reduce((m, a) => seedRemoteSnapshot(m, keys, a), prev);
        });
      })
      .catch(() => {
        // Not under Tauri (tests) or nothing downloading: no rows to hydrate.
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const begin = useCallback((identity: RegistryIdentity) => {
    const key = downloadKey(identity);
    // A fast double-click, or a click landing before the row re-renders to hide
    // its button, would fire a second backend download that claim_download
    // rejects, flashing a spurious failure over the live one. Ignore re-entry
    // while this key is already downloading; a retry of a terminal
    // (failed/ready) entry is not in flight, so it still proceeds.
    const existing = entriesRef.current.get(key);
    if (existing && isDownloadInFlight(existing.acc.state.phase)) {
      return;
    }
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
    // Every call here originates from a real click (a row's Download/Resume
    // button, or `retry`'s replay of one) — no auto-invoke path in this
    // registry — so `userInitiated: true` is unconditional (issue #296).
    void invoke(command, {
      ...args,
      key,
      userInitiated: true,
      onEvent: channel,
    }).catch((err) =>
      // A rejected invoke means the command failed before streaming (e.g. the
      // repo spec could not be resolved), so no channel event will arrive.
      setEntries((prev) => {
        const next = new Map(prev);
        // The backend's by-sha guard rejects a start whose blob is already
        // downloading under another key (e.g. the same model running in the
        // onboarding window). That is not a failure: drop the optimistic entry
        // so the row falls back to the live cross-window view instead of
        // painting a spurious, non-self-healing failure card over it.
        if (String(err).includes('already in progress')) {
          next.delete(key);
        } else {
          next.set(key, {
            identity,
            acc: {
              ...startingAccumulator(),
              state: { phase: 'failed', kind: 'other', message: String(err) },
            },
          });
        }
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

  // Keys currently in the `queued` phase, in FIFO order: `remote` (other
  // windows) before local `entries`, matching the two maps' natural iteration
  // order since neither ever reorders an existing key. The single source both
  // `queuePosition` and `queuedTotal` derive from, so the two can never
  // disagree.
  const queuedKeys = useMemo<string[]>(() => {
    const keys: string[] = [];
    for (const [k, entry] of remote) {
      if (entry.acc.state.phase === 'queued') keys.push(k);
    }
    for (const [k, entry] of entries) {
      if (entry.acc.state.phase === 'queued') keys.push(k);
    }
    return keys;
  }, [entries, remote]);

  const queuePosition = useCallback(
    (key: string): number | undefined => {
      const index = queuedKeys.indexOf(key);
      return index === -1 ? undefined : index + 1;
    },
    [queuedKeys],
  );

  const queuedTotal = queuedKeys.length;

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
    // `key` may name a local download (this window's) or a remote one (another
    // window's, after its install effect settles); drop it from whichever map
    // holds it.
    setEntries((prev) => {
      if (!prev.has(key)) return prev;
      const next = new Map(prev);
      next.delete(key);
      return next;
    });
    setRemote((prev) => {
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

  const getActiveDownload = useCallback(
    (sha: string): ActiveDownloadView | undefined => {
      for (const [key, entry] of remote) {
        if (entry.shas.includes(sha)) {
          const {
            state,
            progress,
            etaSeconds,
            combinedBytes,
            speedBytesPerSec,
          } = entry.acc;
          return {
            key,
            view: {
              state,
              progress,
              etaSeconds,
              combinedBytes,
              speedBytesPerSec,
            },
          };
        }
      }
      return undefined;
    },
    [remote],
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
      getActiveDownload,
      hasRepoDownload,
      repoDownloadSummary,
      startStaffPick,
      startRepoDownload,
      cancel,
      queuePosition,
      queuedTotal,
      retry,
      discard,
      clear,
    }),
    [
      get,
      getActiveDownload,
      hasRepoDownload,
      repoDownloadSummary,
      startStaffPick,
      startRepoDownload,
      cancel,
      queuePosition,
      queuedTotal,
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
