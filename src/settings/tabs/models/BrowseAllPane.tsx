/**
 * Browse-all pane: the in-app Hugging Face GGUF model browser, the advanced
 * pathway of Discover (behind the "Browse all" tab; the curated "Staff picks"
 * accordion is the default front door).
 *
 * A search field (driven by {@link useHfSearch}) plus a row of family filter
 * chips feed one debounced backend query that returns chat/text-generation
 * GGUF repos. Each lean row shows the repo id, an org + downloads sub-line, a
 * link out to the repo on Hugging Face, and a disclosure chevron. Expanding a
 * row lists the repo's `.gguf` files (`list_hf_repo_ggufs`, each with an
 * accurate per-quant RAM-fit, the only place fit is shown); each quant downloads
 * through the Settings {@link useDownloads} registry, so multiple quants (and
 * multiple repos) can download in parallel. A "Load more" control pages past
 * the first batch; a finished install lifts a fresh config snapshot.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { DownloadProgress } from '../../../components/DownloadProgress';
import {
  useDownloads,
  type DownloadsContextValue,
} from '../../../contexts/DownloadsContext';
import { downloadKey } from '../../../hooks/downloadKey';
import { CapabilityPills } from './CapabilityPills';
import { DownloadRiskConfirm } from './DownloadRiskConfirm';
import { HF_SEARCH_QUERY_MAX_LEN, useHfSearch } from './useHfSearch';
import { InlineLink } from '../../../components/InlineLink';
import { Tooltip } from '../../../components/Tooltip';
import { formatContextWindow } from '../../../utils/contextWindow';
import { RAM_FIT_LABEL, RAM_FIT_TOOLTIP } from '../../../utils/ramFit';
import styles from './BrowseAllPane.module.css';
import type { HfModelSummary } from '../../../types/hf';
import type { HfGgufFile, RamFit } from '../../../types/starter';
import type { RawAppConfig } from '../../types';

const HF_BASE_URL = 'https://huggingface.co';

/** RAM-fit hint colour class on this pane's stylesheet (labels are shared). */
const FIT_CLASS: Record<RamFit, string> = {
  fits: styles.fitOk,
  tight: styles.fitTight,
  too_big: styles.fitHeavy,
};

/**
 * Family filter chips. Clicking a chip sets the search query to its name;
 * `All` (empty query) is the browse-popular default. No backend beyond the
 * shared search: the chips just preset the query.
 */
const FAMILIES = [
  'All',
  'Qwen',
  'Llama',
  'Gemma',
  'gpt-oss',
  'DeepSeek',
  'Phi',
] as const;

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "8.2"). */
function gb(bytes: number): string {
  return (bytes / 1e9).toFixed(1);
}

/** The org segment of an `owner/repo` id, or the whole id when there is no slash. */
function orgOf(id: string): string {
  const slash = id.indexOf('/');
  return slash === -1 ? id : id.slice(0, slash);
}

const DOWNLOAD_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M12 4v11M7 11l5 5 5-5M5 20h14" />
  </svg>
);
// A disclosure chevron for the repo row: it expands the quant list, so it must
// NOT wear the download icon (which now lives on the rows that actually
// download). The chevron rotates to point up when the row is open.
const CHEVRON_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M6 9l6 6 6-6" />
  </svg>
);
// Amber caution triangle bookending the live-fetch notice.
const CAUTION_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h16.9a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" />
    <path d="M12 9v4" />
    <path d="M12 17h.01" />
  </svg>
);
interface BrowseAllPaneProps {
  /** Lift a fresh config snapshot after a successful install. */
  onSaved: (next: RawAppConfig) => void;
}

export function BrowseAllPane({ onSaved }: BrowseAllPaneProps) {
  const { query, setQuery, results, loading, loadMore, canLoadMore } =
    useHfSearch();

  return (
    <div className={styles.pane}>
      <div className={styles.search}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="11" cy="11" r="7" />
          <path d="m20 20-3.5-3.5" />
        </svg>
        <input
          type="search"
          className={styles.searchInput}
          aria-label="Search Hugging Face models"
          placeholder="Search Hugging Face models"
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          maxLength={HF_SEARCH_QUERY_MAX_LEN}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      <div className={styles.chips}>
        {FAMILIES.map((family) => {
          const target = family === 'All' ? '' : family;
          const active = query === target;
          return (
            <button
              key={family}
              type="button"
              aria-pressed={active}
              className={`${styles.chip} ${active ? styles.chipOn : ''}`}
              onClick={() => setQuery(target)}
            >
              {family}
            </button>
          );
        })}
      </div>

      <p className={styles.notice}>
        {CAUTION_ICON}
        Live from Hugging Face. Quality and safety vary. Research any model
        before you download it.
        {CAUTION_ICON}
      </p>

      <div className={styles.list}>
        {loading ? <p className={styles.state}>Searching…</p> : null}
        {!loading && results.length === 0 ? (
          <p className={styles.state}>No models found.</p>
        ) : null}
        {results.map((model) => (
          <BrowseAllRow key={model.id} model={model} onSaved={onSaved} />
        ))}
        {canLoadMore ? (
          <button type="button" className={styles.loadMore} onClick={loadMore}>
            Load more
          </button>
        ) : null}
      </div>
    </div>
  );
}

interface BrowseAllRowProps {
  model: HfModelSummary;
  onSaved: (next: RawAppConfig) => void;
}

/**
 * One repo row plus its lazy quant accordion. The GGUF file list is fetched the
 * first time the row expands; each quant downloads independently through the
 * registry, so several quants of one repo can run at once.
 */
function BrowseAllRow({ model, onSaved }: BrowseAllRowProps) {
  const downloads = useDownloads();
  const [files, setFiles] = useState<HfGgufFile[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);

  // Re-expand a repo that still has a live download after a tab switch remounts
  // this row collapsed. The registry survives the unmount; this row's
  // expand/files state does not, so it rebuilds from the registry on mount.
  const hasLiveDownload = downloads.hasRepoDownload(model.id);
  const [expanded, setExpanded] = useState(hasLiveDownload);

  const org = orgOf(model.id);

  const loadFiles = useCallback(async () => {
    setListError(null);
    setFiles(null);
    try {
      const rows = await invoke<HfGgufFile[]>('list_hf_repo_ggufs', {
        repo: model.id,
      });
      setFiles(Array.isArray(rows) ? rows : []);
    } catch (err) {
      setListError(String(err));
    }
  }, [model.id]);

  // On a remount that auto-expanded the row (a download is still live), fetch
  // the quant list once so the live progress shows again. Fires only on mount.
  const restoreOnMountRef = useRef(hasLiveDownload);
  useEffect(() => {
    if (restoreOnMountRef.current) void loadFiles();
  }, [loadFiles]);

  function toggle() {
    if (expanded) {
      setExpanded(false);
      return;
    }
    setExpanded(true);
    void loadFiles();
  }

  // Silent re-read of the listing (no loading flash): the rows carry fresh
  // `partial_bytes`, so a file flips to/from its Paused state in place.
  const refetchFiles = useCallback(async () => {
    try {
      // The listing was already validated on first load; trust the typed array.
      setFiles(
        await invoke<HfGgufFile[]>('list_hf_repo_ggufs', { repo: model.id }),
      );
    } catch {
      // Keep the current list; the partial indicator self-heals on next expand.
    }
  }, [model.id]);

  // The context window is a per-repo property (the search carries it via
  // expand[]=gguf), so it shows on the collapsed row without expanding. Empty
  // when unknown, which skips it.
  const contextLabel = formatContextWindow(model.context_length ?? 0);

  // Live download counts for this family, surfaced as pills on the row so a
  // collapsed repo still tells you what it has in flight. Reads the registry,
  // so it survives the accordion collapse that hides the per-quant rows. One
  // pill per active state with a non-zero count, in active-first order.
  const dl = downloads.repoDownloadSummary(model.id);
  const statusPills = (
    [
      [dl.downloading, 'downloading', styles.pillDownloading],
      [dl.verifying, 'verifying', styles.pillVerifying],
      [dl.failed, 'failed', styles.pillFailed],
    ] as const
  ).filter(([count]) => count > 0);

  return (
    <div className={styles.rowWrap} data-row>
      <div className={styles.row}>
        <div className={styles.mid}>
          <div className={styles.nm}>
            {/* The title opens the repo on Hugging Face, so the row needs no
                separate link icon. */}
            <InlineLink
              url={`${HF_BASE_URL}/${model.id}`}
              subtle
              style={{
                display: 'inline-block',
                fontSize: 12.5,
                fontWeight: 540,
                textAlign: 'left',
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                minWidth: 0,
                maxWidth: '100%',
              }}
            >
              {model.id}
            </InlineLink>
            <CapabilityPills vision={model.vision} thinking={model.thinking} />
            {model.gated ? (
              <span className={styles.gatedBadge}>Gated</span>
            ) : null}
          </div>
          <div className={styles.org}>
            {org} · {model.downloads.toLocaleString()} downloads
            {contextLabel ? ` · ${contextLabel}` : ''}
          </div>
        </div>
        {statusPills.length > 0 ? (
          <div className={styles.statusPills}>
            {statusPills.map(([count, label, cls]) => (
              <span key={label} className={`${styles.statusPill} ${cls}`}>
                {count} {label}
              </span>
            ))}
          </div>
        ) : null}
        <button
          type="button"
          className={`${styles.disclose} ${expanded ? styles.discloseOpen : ''}`}
          aria-label="Show files"
          aria-expanded={expanded}
          disabled={model.gated}
          onClick={toggle}
        >
          {CHEVRON_ICON}
        </button>
      </div>

      {expanded ? (
        <div className={styles.expand}>
          {listError !== null ? (
            <p className={styles.error}>{listError}</p>
          ) : null}
          {files !== null && files.length === 0 && listError === null ? (
            <p className={styles.note}>No GGUF files in this repo.</p>
          ) : null}
          {files !== null && files.length > 0
            ? files.map((f) => (
                <QuantRow
                  key={f.file}
                  file={f}
                  repo={model.id}
                  downloads={downloads}
                  onSaved={onSaved}
                  refetch={refetchFiles}
                />
              ))
            : null}
        </div>
      ) : null}
    </div>
  );
}

interface QuantRowProps {
  file: HfGgufFile;
  repo: string;
  downloads: DownloadsContextValue;
  onSaved: (next: RawAppConfig) => void;
  refetch: () => Promise<void>;
}

/**
 * One quant file row. Owns its own download by key, so each quant in a repo can
 * download in parallel with the others: a downloading quant shows the inline
 * progress card while its siblings stay browsable and downloadable.
 */
function QuantRow({ file, repo, downloads, onSaved, refetch }: QuantRowProps) {
  const key = downloadKey({ kind: 'repo', repo, file: file.file });
  const { clear } = downloads;
  // This quant's live download: its own (by key) or one started in another
  // window, matched by the file's blob sha. The cross-window match carries the
  // real backend key, so cancel and the post-install clear target the right slot.
  const local = downloads.get(key);
  const active = local
    ? { key, view: local }
    : downloads.getActiveDownload(file.sha256);
  const entry = active?.view;
  // The live download's real backend key: this quant's own when it started here,
  // or the cross-window download's when matched by sha. Falls back to the quant
  // key when nothing is live, so cancel/clear always have a concrete target.
  const activeKey = active?.key ?? key;
  const downloading = entry !== undefined;
  const phase = entry?.state.phase;
  // Browse-all is a live Hugging Face fetch, so a fresh download click first
  // asks the user to accept an unreviewed third-party model. Resume of an
  // already-accepted partial skips this.
  const [confirming, setConfirming] = useState(false);

  // A finished install: the backend recorded the model, so lift the fresh config
  // and re-read the listing (the quant flips to its installed state) and drop
  // the entry (its own or a cross-window one). Per quant, so parallel installs
  // settle independently.
  useEffect(() => {
    if (phase !== 'ready') return;
    void (async () => {
      try {
        onSaved(await invoke<RawAppConfig>('get_config'));
      } catch {
        // The focus-driven resync picks the change up on next activation.
      }
      clear(activeKey);
      await refetch();
    })();
  }, [phase, activeKey, clear, onSaved, refetch]);

  // Cancelling keeps the partial on disk; re-read the listing so the file flips
  // to its Paused / Resume / Discard controls once the Cancelled event prunes.
  // Uses the live download's real key, so cancelling a cross-window download
  // targets its actual backend slot.
  async function cancelDownload() {
    downloads.cancel(activeKey);
    await refetch();
  }

  async function discardFile() {
    await downloads.discard(file.sha256);
    await refetch();
  }

  // Dismiss this quant's terminal card back to the file rows. Also wired to the
  // confirm-card callbacks, which never fire here (the repo path has no
  // pre-flight confirm step), so all three share one covered handler.
  const dismiss = () => clear(activeKey);

  const paused = !downloading && file.partial_bytes !== null;
  const pausedPct =
    file.partial_bytes !== null
      ? Math.min(100, Math.floor((file.partial_bytes / file.size_bytes) * 100))
      : 0;

  // A split (multi-part) GGUF downloads its shards sequentially; the backend
  // emits per-shard Started/Progress/FileDone events that the reducer already
  // folds into one continuous `combinedBytes`. The unified bar then needs the
  // combined total (file.size_bytes, already summed by the backend) as its
  // denominator, and a quiet "Part N of M" subline. N is the 1-based index of
  // the currently streaming shard, matched by filename (Started.file equals the
  // shard's parts[].file), so it stays correct across a resume. Single-file
  // downloads pass neither and keep the per-file figures unchanged.
  const isMultipart = (file.parts?.length ?? 0) > 1;
  const currentPartIndex =
    isMultipart && entry?.progress
      ? file.parts.findIndex((p) => p.file === entry.progress!.file)
      : -1;
  const partLabel =
    currentPartIndex >= 0
      ? `Part ${currentPartIndex + 1} of ${file.parts.length}`
      : null;

  if (confirming) {
    return (
      <div className={styles.quantRow}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <DownloadRiskConfirm
            onConfirm={() => {
              setConfirming(false);
              downloads.startRepoDownload(repo, file.file);
            }}
            onCancel={() => setConfirming(false)}
          />
        </div>
      </div>
    );
  }

  return (
    <div className={styles.quantRow}>
      {/* The filename links to that exact file on Hugging Face. `subtle` keeps
          it reading as plain text until hover, where it underlines to reveal it
          is clickable; the native title surfaces the URL. */}
      <InlineLink
        url={`${HF_BASE_URL}/${repo}/blob/main/${file.file}`}
        subtle
        subtleColor="var(--t2)"
        style={{
          display: 'inline-block',
          flex: 1,
          minWidth: 0,
          maxWidth: '100%',
          fontSize: 11.5,
          textAlign: 'left',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {file.file}
      </InlineLink>
      {downloading && entry ? (
        <DownloadProgress
          state={entry.state}
          progress={entry.progress}
          etaSeconds={entry.etaSeconds}
          // A multi-part model renders one unified bar over the combined size
          // (the reducer's accumulated bytes against file.size_bytes) with a
          // "Part N of M" subline. A single-file download passes neither, so it
          // keeps the per-file figures from its own Progress events.
          combinedBytes={isMultipart ? entry.combinedBytes : null}
          grandTotalBytes={isMultipart ? file.size_bytes : null}
          speedBytesPerSec={entry.speedBytesPerSec}
          partLabel={partLabel}
          queuePosition={
            phase === 'queued' ? downloads.queuePosition(activeKey) : undefined
          }
          queuedTotal={phase === 'queued' ? downloads.queuedTotal : undefined}
          // The repo download flow has no pre-flight confirm step (only the
          // starter picker does), so the confirm card never renders; these
          // share the same covered dismiss handler rather than dead no-op
          // literals.
          onConfirm={dismiss}
          onCancelConfirm={dismiss}
          onCancel={() => void cancelDownload()}
          onRetry={() => downloads.retry(activeKey)}
          // A terminal failure must leave a path back to the quant list, not
          // just Retry; this returns to the file rows.
          onChooseAnother={dismiss}
        />
      ) : (
        <>
          {file.fit ? (
            <Tooltip label={RAM_FIT_TOOLTIP[file.fit]} placement="top">
              <span className={`${styles.fit} ${FIT_CLASS[file.fit]}`}>
                {RAM_FIT_LABEL[file.fit]}
              </span>
            </Tooltip>
          ) : null}
          {/* An already-installed quant shows nothing here: no download button,
              no badge. It lives in Library, so on this Discover surface the
              absence of a download is the signal, matching Staff picks. */}
          {file.installed ? null : paused ? (
            <>
              <span className={styles.quantPaused}>Paused · {pausedPct}%</span>
              <button
                type="button"
                className={styles.quantResume}
                onClick={() => downloads.startRepoDownload(repo, file.file)}
              >
                Resume
              </button>
              <button
                type="button"
                className={styles.quantDiscard}
                aria-label="Discard"
                onClick={() => void discardFile()}
              >
                Discard
              </button>
            </>
          ) : (
            <>
              {/* A split (multi-part) GGUF collapses into this one row: the size
                  is already the combined total, and a quiet "· N parts" whisper
                  notes it is multi-file. Single-file rows render unchanged. One
                  download fetches every shard; shards are never separate rows. */}
              <span className={styles.quantSize}>
                {gb(file.size_bytes)} GB
                {(file.parts?.length ?? 0) > 1 ? (
                  <span className={styles.partsWhisper}>
                    {' '}
                    · {file.parts.length} parts
                  </span>
                ) : null}
              </span>
              <button
                type="button"
                className={styles.quantGet}
                aria-label="Download"
                onClick={() => setConfirming(true)}
              >
                {DOWNLOAD_ICON}
              </button>
            </>
          )}
        </>
      )}
    </div>
  );
}
