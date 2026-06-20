/**
 * Staff-picks pane: the curated front door of Discover.
 *
 * Thuki hand-picks a short catalog and groups it into use-case sections
 * ("Everyday chat", "Compact & fast", "Deep reasoning", ...) so a non-expert
 * can pick by intent. Known sections show first in a fixed order, then any
 * extra category alphabetically; within a section models are alphabetical. Each
 * compact row shows the model name, capability pills (Text always, plus Vision
 * / Thinking), a `size · maker` sub-line, a RAM-fit hint, and a single icon
 * download that runs the VERIFIED catalog path (`download_staff_pick`, keyed by
 * the entry's stable id, pinned revision + sha256), unlike the Browse-all
 * pane's arbitrary repo downloads. A finished install lifts a fresh config
 * snapshot.
 *
 * Data comes from {@link useStaffPicks}, the id-keyed catalog (decoupled from
 * onboarding's three tier heroes so a category can hold any number of models);
 * the download state machine is the shared {@link useDownloadModel}, so the
 * in-flight / failed UI is the same {@link DownloadProgress} card the rest of
 * the app shows. At most one model downloads at a time (the backend enforces it
 * too); `activeId` tracks which row owns the progress card.
 */

import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { DownloadProgress } from '../../../components/DownloadProgress';
import { useDownloadModel } from '../../../hooks/useDownloadModel';
import { useStaffPicks } from '../../../components/StarterPicker';
import { Tooltip } from '../../../components/Tooltip';
import { formatContextWindow } from '../../../utils/contextWindow';
import { RAM_FIT_LABEL, RAM_FIT_TOOLTIP } from '../../../utils/ramFit';
import styles from './StaffPicksPane.module.css';
import type { RawAppConfig } from '../../types';
import type { RamFit, StaffPickOption } from '../../../types/starter';

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

  // One download at a time; activeId names the row that owns the progress card.
  const [activeId, setActiveId] = useState<string | null>(null);
  const {
    state,
    progress,
    etaSeconds,
    combinedBytes,
    speedBytesPerSec,
    startById,
    cancel,
    retry,
    reset,
    discard,
  } = useDownloadModel();

  // A finished install (phase 'ready') lifts the fresh config, clears the
  // active row, and refreshes the rows so the new model flips to Installed.
  // An effect (not a render-time call) so it fires exactly once per transition.
  useEffect(() => {
    if (state.phase !== 'ready') return;
    void (async () => {
      try {
        onSaved(await invoke<RawAppConfig>('get_config'));
      } catch {
        // The focus-driven resync picks the change up on next activation.
      }
      reset();
      setActiveId(null);
      await refresh();
    })();
  }, [state.phase, onSaved, reset, refresh]);

  // Download and resume both run the same id-keyed verified path; the backend
  // resumes from a kept partial via Range, so resume is just starting again.
  function startDownload(id: string) {
    setActiveId(id);
    void startById(id);
  }

  async function discardPartial(sha256: string) {
    await discard(sha256);
    await refresh();
  }

  // Cancelling leaves the partial on disk; re-read the options so the row flips
  // straight to its Paused/Resume state instead of snapping back to a fresh
  // download until the next remount.
  async function cancelDownload() {
    await cancel();
    await refresh();
  }

  function returnToPicker() {
    reset();
    setActiveId(null);
  }

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
              active={activeId === o.starter.id}
              state={state}
              progress={progress}
              etaSeconds={etaSeconds}
              combinedBytes={combinedBytes}
              speedBytesPerSec={speedBytesPerSec}
              onDownload={startDownload}
              onResume={startDownload}
              onDiscard={discardPartial}
              onCancel={() => void cancelDownload()}
              onRetry={() => void retry()}
              onChooseAnother={returnToPicker}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

interface ModelRowProps {
  option: StaffPickOption;
  active: boolean;
  state: ReturnType<typeof useDownloadModel>['state'];
  progress: ReturnType<typeof useDownloadModel>['progress'];
  etaSeconds: number | null;
  combinedBytes: number | null;
  speedBytesPerSec: number | null;
  onDownload: (id: string) => void;
  onResume: (id: string) => void;
  onDiscard: (sha256: string) => void;
  onCancel: () => void;
  onRetry: () => void;
  onChooseAnother: () => void;
}

function ModelRow({
  option,
  active,
  state,
  progress,
  etaSeconds,
  combinedBytes,
  speedBytesPerSec,
  onDownload,
  onResume,
  onDiscard,
  onCancel,
  onRetry,
  onChooseAnother,
}: ModelRowProps) {
  const { starter, fit, installed, partial_bytes } = option;
  const showProgress = active && state.phase !== 'idle';
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
            <span className={styles.name} data-testid="staff-model-name">
              {starter.display_name}
            </span>
            <span className={styles.pills}>
              <span className={`${styles.pill} ${styles.pillText}`}>Text</span>
              {starter.vision ? (
                <span className={`${styles.pill} ${styles.pillVision}`}>
                  Vision
                </span>
              ) : null}
              {starter.thinking ? (
                <span className={`${styles.pill} ${styles.pillThinking}`}>
                  Thinking
                </span>
              ) : null}
            </span>
          </div>
          <div className={styles.sub}>
            {paused
              ? `Paused · ${pausedPct}%`
              : `${gb(totalBytes(option))} GB${
                  contextLabel ? ` · ${contextLabel}` : ''
                } · ${starter.origin}`}
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
                  onClick={() => onResume(starter.id)}
                >
                  Resume
                </button>
                <button
                  type="button"
                  className={styles.discardBtn}
                  aria-label="Discard"
                  onClick={() => onDiscard(starter.sha256)}
                >
                  Discard
                </button>
              </>
            ) : (
              <RowAction
                option={option}
                installed={installed}
                onDownload={onDownload}
              />
            )}
          </div>
        ) : null}
      </div>
      {showProgress ? (
        <div className={styles.progress}>
          <DownloadProgress
            state={state}
            progress={progress}
            etaSeconds={etaSeconds}
            combinedBytes={combinedBytes}
            grandTotalBytes={totalBytes(option)}
            speedBytesPerSec={speedBytesPerSec}
            // The curated path has no pre-flight confirm card, so onConfirm /
            // onCancelConfirm never fire; they point at the same covered
            // handlers rather than dead no-op literals.
            onConfirm={onChooseAnother}
            onCancelConfirm={onChooseAnother}
            onCancel={onCancel}
            onRetry={onRetry}
            onChooseAnother={onChooseAnother}
          />
        </div>
      ) : null}
    </div>
  );
}

interface RowActionProps {
  option: StaffPickOption;
  installed: boolean;
  onDownload: (id: string) => void;
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
function RowAction({ option, installed, onDownload }: RowActionProps) {
  const { starter } = option;

  if (installed) {
    return null;
  }

  return (
    <button
      type="button"
      className={styles.getBtn}
      aria-label="Download"
      onClick={() => onDownload(starter.id)}
    >
      {DOWNLOAD_ICON}
    </button>
  );
}
