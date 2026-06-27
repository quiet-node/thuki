/**
 * Staff-picks pane: the curated front door of Discover.
 *
 * Thuki hand-picks a short catalog and groups it into use-case sections
 * ("Everyday chat", "Compact & fast", "Deep reasoning", ...) so a non-expert
 * can pick by intent. Known sections show first in a fixed order, then any
 * extra category alphabetically; within a section models are alphabetical. Each
 * compact row shows the model name (a link that opens the repo on Hugging
 * Face), capability pills (Text always, plus Vision / Thinking), a `size ·
 * context · maker · quant` sub-line, a RAM-fit hint, and a single icon
 * download that runs the VERIFIED catalog path (`download_staff_pick`, keyed by
 * the entry's stable id, pinned revision + sha256), unlike the Browse-all
 * pane's arbitrary repo downloads. A finished install lifts a fresh config
 * snapshot.
 *
 * Data comes from {@link useStaffPicks}, the id-keyed catalog (decoupled from
 * onboarding's three tier heroes so a category can hold any number of models).
 * Downloads run through the Settings {@link useDownloads} registry, so several
 * models can download in parallel: each row binds to its own download by
 * {@link downloadKey} and shows the shared {@link DownloadProgress} card.
 */

import { useEffect, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { DownloadProgress } from '../../../components/DownloadProgress';
import {
  useDownloads,
  type DownloadsContextValue,
} from '../../../contexts/DownloadsContext';
import { downloadKey } from '../../../hooks/downloadKey';
import { CapabilityPills } from './CapabilityPills';
import { useStaffPicks } from '../../../components/StarterPicker';
import { InlineLink } from '../../../components/InlineLink';
import { Tooltip } from '../../../components/Tooltip';
import { formatContextWindow } from '../../../utils/contextWindow';
import { RAM_FIT_LABEL, RAM_FIT_TOOLTIP } from '../../../utils/ramFit';
import styles from './StaffPicksPane.module.css';
import type { RawAppConfig } from '../../types';
import type { RamFit, StaffPickOption } from '../../../types/starter';

const HF_BASE_URL = 'https://huggingface.co';

/** RAM-fit hint colour class on this pane's stylesheet (labels are shared). */
const FIT_CLASS: Record<RamFit, string> = {
  fits: styles.fitOk,
  tight: styles.fitTight,
  too_big: styles.fitHeavy,
};

/** The order use-case sections appear in. Categories outside this list follow
 * it, alphabetically. */
const CATEGORY_ORDER = ['Everyday chat', 'Compact & fast', 'Deep reasoning'];

/** Bucket for a model that carries no category. */
const UNCATEGORIZED = 'Other';

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "8.2"). */
function gb(bytes: number): string {
  return (bytes / 1e9).toFixed(1);
}

/** Weights + vision companion: the full on-disk cost of one starter. */
function totalBytes(o: StaffPickOption): number {
  return o.starter.size_bytes + o.starter.mmproj_bytes;
}

/** One use-case section: its label and the models under it. */
interface Section {
  category: string;
  options: StaffPickOption[];
}

/** Groups models into use-case sections: known categories first in their fixed
 * order, then any extra category alphabetically; models within a section are
 * alphabetical by name. */
function groupByCategory(options: StaffPickOption[]): Section[] {
  const buckets = new Map<string, StaffPickOption[]>();
  for (const o of options) {
    const category = o.starter.category ?? UNCATEGORIZED;
    const list = buckets.get(category);
    if (list) {
      list.push(o);
    } else {
      buckets.set(category, [o]);
    }
  }
  const known = CATEGORY_ORDER.filter((c) => buckets.has(c));
  const extra = [...buckets.keys()]
    .filter((c) => !CATEGORY_ORDER.includes(c))
    .sort();
  return [...known, ...extra].map((category) => ({
    category,
    options: (buckets.get(category) as StaffPickOption[]).sort((a, b) =>
      a.starter.display_name.localeCompare(b.starter.display_name, undefined, {
        sensitivity: 'base',
      }),
    ),
  }));
}

interface StaffPicksPaneProps {
  /** Lift a fresh config snapshot after a successful install. */
  onSaved: (next: RawAppConfig) => void;
}

export function StaffPicksPane({ onSaved }: StaffPicksPaneProps) {
  const { options, refresh } = useStaffPicks();
  const sections = useMemo(() => groupByCategory(options ?? []), [options]);
  const downloads = useDownloads();

  if (options !== null && sections.length === 0) {
    return (
      <div className={styles.pane}>
        <p className={styles.empty}>No curated models are available.</p>
      </div>
    );
  }

  return (
    <div className={styles.pane}>
      {sections.map((section) => (
        <div className={styles.section} key={section.category}>
          <div className={styles.secLabel} data-testid="staff-section-label">
            {section.category}
          </div>
          {section.options.map((o) => (
            <ModelRow
              key={o.starter.id}
              option={o}
              downloads={downloads}
              onSaved={onSaved}
              refresh={refresh}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

interface ModelRowProps {
  option: StaffPickOption;
  /** The Settings download registry; each row owns its own download by key. */
  downloads: DownloadsContextValue;
  /** Lift a fresh config snapshot after this row's install completes. */
  onSaved: (next: RawAppConfig) => void;
  /** Re-read the curated rows so installed / paused state reflects on disk. */
  refresh: () => Promise<void>;
}

function ModelRow({ option, downloads, onSaved, refresh }: ModelRowProps) {
  const { starter, fit, installed, partial_bytes } = option;
  const key = downloadKey({ kind: 'staff', id: starter.id });
  const { clear } = downloads;
  // This row's live download: its own (started here, found by key) or one
  // started in another window (e.g. onboarding), matched by the weights blob
  // sha. The cross-window match carries the real backend key, so cancel and the
  // post-install clear target the right slot.
  const local = downloads.get(key);
  const active = local
    ? { key, view: local }
    : downloads.getActiveDownload(starter.sha256);
  const entry = active?.view;
  // The live download's real backend key: this row's own when it started here,
  // or the cross-window download's when matched by sha. Falls back to the row
  // key when nothing is live, so cancel/clear always have a concrete target.
  const activeKey = active?.key ?? key;
  // An entry exists only while this row's download is live (downloading,
  // verifying, ready-pending, or failed); a Cancelled download is pruned.
  const showProgress = entry !== undefined;
  const phase = entry?.state.phase;

  // A finished install (phase 'ready') lifts the fresh config, drops the entry
  // (its own or a cross-window one), and refreshes the rows so the new model
  // flips to Installed. Per row, so parallel installs each settle independently.
  useEffect(() => {
    if (phase !== 'ready') return;
    void (async () => {
      try {
        onSaved(await invoke<RawAppConfig>('get_config'));
      } catch {
        // The focus-driven resync picks the change up on next activation.
      }
      clear(activeKey);
      await refresh();
    })();
  }, [phase, activeKey, clear, onSaved, refresh]);

  async function discardPartial() {
    await downloads.discard(starter.sha256);
    await refresh();
  }

  // Dismiss this row's terminal card back to its normal controls. Also wired to
  // the confirm-card callbacks, which never fire here (the curated path has no
  // pre-flight confirm step), so all three share one covered handler.
  const dismiss = () => clear(activeKey);

  // Cancelling keeps the partial on disk; re-read the options so the row flips
  // to its Paused/Resume state once the Cancelled event prunes the entry. Uses
  // the live download's real key, so cancelling a cross-window download targets
  // its actual backend slot.
  async function cancelDownload() {
    downloads.cancel(activeKey);
    await refresh();
  }

  // Empty when the model carries no context window, so the pill is skipped.
  const contextLabel = formatContextWindow(starter.context_length ?? 0);
  // An interrupted partial (not installed, not actively downloading) reads as a
  // calm "Paused · N%" rather than a size line, with quiet resume/discard.
  const paused = !showProgress && !installed && partial_bytes !== null;
  const pausedPct =
    partial_bytes !== null
      ? Math.min(100, Math.floor((partial_bytes / totalBytes(option)) * 100))
      : 0;

  return (
    <div className={styles.row} data-model-row data-id={starter.id}>
      <div className={styles.rowMain}>
        <div className={styles.mid}>
          <div className={styles.top}>
            <InlineLink
              url={`${HF_BASE_URL}/${starter.repo}`}
              subtle
              style={{ fontSize: 12.5, fontWeight: 560, textAlign: 'left' }}
            >
              {starter.display_name}
            </InlineLink>
            <CapabilityPills
              vision={starter.vision}
              thinking={starter.thinking}
            />
          </div>
          <div className={styles.sub}>
            {paused
              ? `Paused · ${pausedPct}%`
              : `${gb(totalBytes(option))} GB${
                  contextLabel ? ` · ${contextLabel}` : ''
                } · ${starter.origin} · ${starter.quant}`}
          </div>
        </div>
        {!showProgress ? (
          <div className={styles.right}>
            <Tooltip label={RAM_FIT_TOOLTIP[fit]} placement="top">
              <span className={`${styles.fit} ${FIT_CLASS[fit]}`}>
                {RAM_FIT_LABEL[fit]}
              </span>
            </Tooltip>
            {paused ? (
              <>
                <button
                  type="button"
                  className={styles.resumeBtn}
                  onClick={() => downloads.startStaffPick(starter.id)}
                >
                  Resume
                </button>
                <button
                  type="button"
                  className={styles.discardBtn}
                  aria-label="Discard"
                  onClick={() => void discardPartial()}
                >
                  Discard
                </button>
              </>
            ) : (
              <RowAction
                installed={installed}
                onDownload={() => downloads.startStaffPick(starter.id)}
              />
            )}
          </div>
        ) : null}
      </div>
      {showProgress && entry ? (
        <div className={styles.progress}>
          <DownloadProgress
            state={entry.state}
            progress={entry.progress}
            etaSeconds={entry.etaSeconds}
            combinedBytes={entry.combinedBytes}
            grandTotalBytes={totalBytes(option)}
            speedBytesPerSec={entry.speedBytesPerSec}
            // The curated path has no pre-flight confirm card, so onConfirm /
            // onCancelConfirm never fire; they share the same covered dismiss
            // handler rather than dead no-op literals.
            onConfirm={dismiss}
            onCancelConfirm={dismiss}
            onCancel={() => void cancelDownload()}
            onRetry={() => downloads.retry(activeKey)}
            onChooseAnother={dismiss}
          />
        </div>
      ) : null}
    </div>
  );
}

interface RowActionProps {
  installed: boolean;
  onDownload: () => void;
}

const DOWNLOAD_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M12 4v11M7 11l5 5 5-5M5 20h14" />
  </svg>
);

/** The per-row download affordance. An already-installed model shows nothing
 * (no download button, no badge): it lives in Library, so on this Discover
 * surface the absence of a download is the signal. The interrupted-partial
 * resume/discard pair is owned by the row itself; this renders the plain icon
 * download button otherwise. */
function RowAction({ installed, onDownload }: RowActionProps) {
  if (installed) {
    return null;
  }

  return (
    <button
      type="button"
      className={styles.getBtn}
      aria-label="Download"
      onClick={onDownload}
    >
      {DOWNLOAD_ICON}
    </button>
  );
}
