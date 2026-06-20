/**
 * Browse-all pane: the in-app Hugging Face GGUF model browser, the advanced
 * pathway of Discover (behind the "Browse all" tab; the curated "Staff picks"
 * accordion is the default front door).
 *
 * A search field (driven by {@link useHfSearch}) plus a row of family filter
 * chips feed one debounced backend query that returns chat/text-generation
 * GGUF repos. Each lean row shows the repo id, an org + downloads sub-line, a
 * link out to the repo on Hugging Face, and an icon-only download button. That
 * button expands a quant accordion listing the repo's `.gguf` files
 * (`list_hf_repo_ggufs`, each with an accurate per-quant RAM-fit, the only
 * place fit is shown) and downloads the chosen one through the shared
 * {@link useDownloadModel} kit. A "Load more" control pages past the first
 * batch. A finished install lifts a fresh config snapshot and collapses the row.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { DownloadProgress } from '../../../components/DownloadProgress';
import { useDownloadCtx } from '../../../contexts/DownloadContext';
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
 * One repo row plus its lazy quant accordion. The GGUF file list is fetched
 * the first time the row expands; the download state machine is local to the
 * row so two rows cannot share an in-flight download.
 */
function BrowseAllRow({ model, onSaved }: BrowseAllRowProps) {
  // The download machine lives at the app root (DownloadProvider), shared with
  // every other row and pane, so a tab switch that unmounts Browse all never
  // drops an in-flight download: the single-slot backend download outlives the
  // pane. `activeDownload` names the repo + file that owns it.
  const {
    state,
    progress,
    etaSeconds,
    startRepoDownload,
    cancel,
    retry,
    reset,
    activeDownload,
    clearActiveDownload,
  } = useDownloadCtx();

  // The file this repo's row is currently downloading, or null when another row
  // (or no download) owns the single in-flight slot. Drives which quant swaps to
  // the live progress card and, on a remount after a tab switch, the re-expand.
  const activeRepoFile =
    activeDownload?.kind === 'repo' && activeDownload.repo === model.id
      ? activeDownload.file
      : null;
  const ownsActiveDownload = activeRepoFile !== null;

  const [expanded, setExpanded] = useState(ownsActiveDownload);
  const [files, setFiles] = useState<HfGgufFile[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);

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

  // A remount that lands on the row owning the in-flight download (re-expanded
  // above) loads its quant list once so the live progress shows again instead
  // of staying behind a collapsed row. Fires only on mount.
  const restoreActiveRow = useRef(ownsActiveDownload);
  useEffect(() => {
    if (restoreActiveRow.current) void loadFiles();
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

  // A finished install: the backend already wrote the builtin provider's model
  // field, so lift the fresh config snapshot and collapse the row. The machine
  // is shared across rows, so only the row that owns the download reacts.
  useEffect(() => {
    if (state.phase !== 'ready' || !ownsActiveDownload) return;
    void (async () => {
      try {
        onSaved(await invoke<RawAppConfig>('get_config'));
      } catch {
        // The focus-driven resync picks the change up on next activation.
      }
      reset();
      clearActiveDownload();
      setExpanded(false);
    })();
  }, [state.phase, ownsActiveDownload, onSaved, reset, clearActiveDownload]);

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

  // Cancelling leaves the partial on disk; forget the active row and re-read the
  // listing so the file flips straight to its Paused / Resume / Discard controls.
  async function cancelDownload() {
    await cancel();
    clearActiveDownload();
    await refetchFiles();
  }

  // A terminal card's exit (Choose a different model, and the unused confirm
  // fallbacks): return to the quant list and forget the active row.
  function returnToList() {
    reset();
    clearActiveDownload();
  }

  async function discardFile(sha256: string) {
    await invoke('discard_partial_download', { sha256 });
    await refetchFiles();
  }

  // True while ANY download runs (the engine handles one at a time): every other
  // row's controls disable; the owning file swaps to the live progress card.
  const anyInFlight = state.phase !== 'idle';
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
            ? files.map((f) => {
                // Only the file that owns the in-flight download swaps its
                // controls for the inline progress. A file with an interrupted
                // partial reads as Paused with Resume / Discard; everything else
                // is a normal, browsable quant. Resume and Discard are disabled
                // while any download runs, since the engine handles one at a time.
                const downloading = anyInFlight && activeRepoFile === f.file;
                const paused = !downloading && f.partial_bytes !== null;
                const pausedPct =
                  f.partial_bytes !== null
                    ? Math.min(
                        100,
                        Math.floor((f.partial_bytes / f.size_bytes) * 100),
                      )
                    : 0;
                return (
                  <div className={styles.quantRow} key={f.file}>
                    <span className={styles.quantName}>{f.file}</span>
                    {downloading ? (
                      <DownloadProgress
                        state={state}
                        progress={progress}
                        etaSeconds={etaSeconds}
                        // The repo download flow has no pre-flight confirm step
                        // (only the starter picker does), so the confirm card
                        // never renders; these required props point at the same
                        // covered handlers rather than dead no-op literals.
                        onConfirm={returnToList}
                        onCancelConfirm={returnToList}
                        onCancel={() => void cancelDownload()}
                        onRetry={() => void retry()}
                        // A terminal failure must leave a path back to the quant
                        // list, not just Retry; this returns to the file rows.
                        onChooseAnother={returnToList}
                      />
                    ) : (
                      <>
                        {f.fit ? (
                          <Tooltip
                            label={RAM_FIT_TOOLTIP[f.fit]}
                            placement="top"
                          >
                            <span
                              className={`${styles.fit} ${FIT_CLASS[f.fit]}`}
                            >
                              {RAM_FIT_LABEL[f.fit]}
                            </span>
                          </Tooltip>
                        ) : null}
                        {paused ? (
                          <>
                            <span className={styles.quantPaused}>
                              Paused · {pausedPct}%
                            </span>
                            <button
                              type="button"
                              className={styles.quantResume}
                              disabled={anyInFlight}
                              onClick={() =>
                                startRepoDownload(model.id, f.file)
                              }
                            >
                              Resume
                            </button>
                            <button
                              type="button"
                              className={styles.quantDiscard}
                              aria-label="Discard"
                              disabled={anyInFlight}
                              onClick={() => void discardFile(f.sha256)}
                            >
                              Discard
                            </button>
                          </>
                        ) : (
                          <>
                            <span className={styles.quantSize}>
                              {gb(f.size_bytes)} GB
                            </span>
                            <button
                              type="button"
                              className={styles.quantGet}
                              aria-label="Download"
                              disabled={anyInFlight}
                              onClick={() =>
                                startRepoDownload(model.id, f.file)
                              }
                            >
                              {DOWNLOAD_ICON}
                            </button>
                          </>
                        )}
                      </>
                    )}
                  </div>
                );
              })
            : null}
        </div>
      ) : null}
    </div>
  );
}
