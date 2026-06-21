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
import { useHfSearch } from './useHfSearch';
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

      <div className={styles.subbar}>
        <span className={styles.count}>
          <b>{results.length}</b> chat models
        </span>
        <span className={styles.sort}>Most downloaded</span>
      </div>

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

  function openHuggingFace() {
    void invoke('open_url', { url: `${HF_BASE_URL}/${model.id}` });
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

  return (
    <div className={styles.rowWrap} data-row>
      <div className={styles.row}>
        <div className={styles.mid}>
          <div className={styles.nm}>
            {/* The title opens the repo on Hugging Face, so the row needs no
                separate link icon. */}
            <button
              type="button"
              className={styles.nmLink}
              onClick={openHuggingFace}
            >
              {model.id}
            </button>
            {model.gated ? (
              <span className={styles.gatedBadge}>Gated</span>
            ) : null}
          </div>
          <div className={styles.org}>
            {org} · {model.downloads.toLocaleString()} downloads
            {contextLabel ? ` · ${contextLabel}` : ''}
          </div>
        </div>
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
  const entry = downloads.get(key);
  const { clear } = downloads;
  const downloading = entry !== undefined;
  const phase = entry?.state.phase;

  // A finished install: the backend recorded the model, so lift the fresh config
  // and re-read the listing (the quant flips to its installed state) and drop
  // the entry. Per quant, so parallel installs settle independently.
  useEffect(() => {
    if (phase !== 'ready') return;
    void (async () => {
      try {
        onSaved(await invoke<RawAppConfig>('get_config'));
      } catch {
        // The focus-driven resync picks the change up on next activation.
      }
      clear(key);
      await refetch();
    })();
  }, [phase, key, clear, onSaved, refetch]);

  // Cancelling keeps the partial on disk; re-read the listing so the file flips
  // to its Paused / Resume / Discard controls once the Cancelled event prunes.
  async function cancelDownload() {
    downloads.cancel(key);
    await refetch();
  }

  async function discardFile() {
    await downloads.discard(file.sha256);
    await refetch();
  }

  // Dismiss this quant's terminal card back to the file rows. Also wired to the
  // confirm-card callbacks, which never fire here (the repo path has no
  // pre-flight confirm step), so all three share one covered handler.
  const dismiss = () => clear(key);

  const paused = !downloading && file.partial_bytes !== null;
  const pausedPct =
    file.partial_bytes !== null
      ? Math.min(100, Math.floor((file.partial_bytes / file.size_bytes) * 100))
      : 0;

  return (
    <div className={styles.quantRow}>
      <span className={styles.quantName}>{file.file}</span>
      {downloading && entry ? (
        <DownloadProgress
          state={entry.state}
          progress={entry.progress}
          etaSeconds={entry.etaSeconds}
          // The repo download flow has no pre-flight confirm step (only the
          // starter picker does), so the confirm card never renders; these
          // share the same covered dismiss handler rather than dead no-op
          // literals.
          onConfirm={dismiss}
          onCancelConfirm={dismiss}
          onCancel={() => void cancelDownload()}
          onRetry={() => downloads.retry(key)}
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
              <span className={styles.quantSize}>{gb(file.size_bytes)} GB</span>
              <button
                type="button"
                className={styles.quantGet}
                aria-label="Download"
                onClick={() => downloads.startRepoDownload(repo, file.file)}
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
