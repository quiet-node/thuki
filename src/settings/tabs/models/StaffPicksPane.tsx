/**
 * Staff-picks pane: the curated front door of Discover.
 *
 * A flat, alphabetically-ordered list of rich model cards. Thuki hand-picks a
 * short catalog and shows each model directly (no family grouping, no
 * recommended highlight): its friendly name, maker and a one-line blurb,
 * capability pills (Text always, plus Vision / Thinking), the one quant Thuki
 * chose with its size and license, a RAM-fit hint, and a single icon download
 * that runs the VERIFIED starter path (`download_starter`, pinned revision +
 * sha256), unlike the Browse-all pane's arbitrary repo downloads. A finished
 * install lifts a fresh config snapshot.
 *
 * Data comes from {@link useStarterOptions} (the same rows onboarding's picker
 * uses); the download state machine is the shared {@link useDownloadModel}, so
 * the in-flight / failed UI is the same {@link DownloadProgress} card the rest
 * of the app shows. At most one model downloads at a time (the backend enforces
 * it too); `activeTier` tracks which card owns the progress card.
 */

import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { DownloadProgress } from '../../../components/DownloadProgress';
import { useDownloadModel } from '../../../hooks/useDownloadModel';
import { useStarterOptions } from '../../../components/StarterPicker';
import { Tooltip } from '../../../components/Tooltip';
import { RAM_FIT_LABEL, RAM_FIT_TOOLTIP } from '../../../utils/ramFit';
import styles from './StaffPicksPane.module.css';
import type { RawAppConfig } from '../../types';
import type {
  RamFit,
  StarterOption,
  StarterTier,
} from '../../../types/starter';

const HF_BASE_URL = 'https://huggingface.co';

/** RAM-fit hint colour class on this pane's stylesheet (labels are shared). */
const FIT_CLASS: Record<RamFit, string> = {
  fits: styles.fitOk,
  tight: styles.fitTight,
  too_big: styles.fitHeavy,
};

/** A plain-language line about what a model is good for, shown after the maker.
 * Keyed by family so several sizes of one model share it; a model with no entry
 * shows just its maker. Presentational only. */
const MODEL_BLURB: Record<string, string> = {
  Qwen: 'Fast, capable all-rounder',
  Gemma: 'Well-rounded, reads images',
  'gpt-oss': 'Strongest reasoning',
};

const DOWNLOAD_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M12 4v11M7 11l5 5 5-5M5 20h14" />
  </svg>
);

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "8.2"). */
function gb(bytes: number): string {
  return (bytes / 1e9).toFixed(1);
}

/** Weights + vision companion: the full on-disk cost of one starter. */
function totalBytes(o: StarterOption): number {
  return o.starter.size_bytes + o.starter.mmproj_bytes;
}

/** The maker line: the maker, plus a blurb when the family has one. */
function makerLine(o: StarterOption): string {
  const blurb = o.starter.family ? MODEL_BLURB[o.starter.family] : undefined;
  return blurb ? `${o.starter.origin} · ${blurb}` : o.starter.origin;
}

interface StaffPicksPaneProps {
  /** Lift a fresh config snapshot after a successful install. */
  onSaved: (next: RawAppConfig) => void;
}

export function StaffPicksPane({ onSaved }: StaffPicksPaneProps) {
  const { options, refresh } = useStarterOptions();

  // Flat, case-insensitive alphabetical order by model name.
  const ordered = useMemo(
    () =>
      [...(options ?? [])].sort((a, b) =>
        a.starter.display_name.localeCompare(
          b.starter.display_name,
          undefined,
          {
            sensitivity: 'base',
          },
        ),
      ),
    [options],
  );

  // One download at a time; activeTier names the card that owns the progress card.
  const [activeTier, setActiveTier] = useState<StarterTier | null>(null);
  const {
    state,
    progress,
    etaSeconds,
    start,
    resume,
    cancel,
    retry,
    reset,
    discard,
  } = useDownloadModel();

  // A finished install (phase 'ready') lifts the fresh config, clears the
  // active card, and refreshes the rows so the new model flips to Installed.
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
      setActiveTier(null);
      await refresh();
    })();
  }, [state.phase, onSaved, reset, refresh]);

  function startDownload(tier: StarterTier) {
    setActiveTier(tier);
    void start(tier);
  }

  function resumeDownload(tier: StarterTier) {
    setActiveTier(tier);
    void resume(tier);
  }

  async function discardPartial(sha256: string) {
    await discard(sha256);
    await refresh();
  }

  function returnToPicker() {
    reset();
    setActiveTier(null);
  }

  if (options !== null && ordered.length === 0) {
    return (
      <div className={styles.pane}>
        <p className={styles.empty}>No curated models are available.</p>
      </div>
    );
  }

  return (
    <div className={styles.pane}>
      <p className={styles.hint}>
        Hand-picked by Thuki and tuned for Apple Silicon.
      </p>
      <div className={styles.list}>
        {ordered.map((o) => (
          <ModelCard
            key={o.starter.tier}
            option={o}
            active={activeTier === o.starter.tier}
            state={state}
            progress={progress}
            etaSeconds={etaSeconds}
            onDownload={startDownload}
            onResume={resumeDownload}
            onDiscard={discardPartial}
            onCancel={() => void cancel()}
            onRetry={() => void retry()}
            onChooseAnother={returnToPicker}
          />
        ))}
      </div>
    </div>
  );
}

interface ModelCardProps {
  option: StarterOption;
  active: boolean;
  state: ReturnType<typeof useDownloadModel>['state'];
  progress: ReturnType<typeof useDownloadModel>['progress'];
  etaSeconds: number | null;
  onDownload: (tier: StarterTier) => void;
  onResume: (tier: StarterTier) => void;
  onDiscard: (sha256: string) => void;
  onCancel: () => void;
  onRetry: () => void;
  onChooseAnother: () => void;
}

function ModelCard({
  option,
  active,
  state,
  progress,
  etaSeconds,
  onDownload,
  onResume,
  onDiscard,
  onCancel,
  onRetry,
  onChooseAnother,
}: ModelCardProps) {
  const { starter, fit, installed, partial_bytes } = option;
  const showProgress = active && state.phase !== 'idle';

  return (
    <div className={styles.card} data-model-card data-tier={starter.tier}>
      <div className={styles.cardMain}>
        <div className={styles.mid}>
          <div className={styles.name} data-testid="staff-model-name">
            {starter.display_name}
          </div>
          <div className={styles.maker}>{makerLine(option)}</div>
          <div className={styles.pills}>
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
          </div>
          <div className={styles.meta}>
            {starter.quant} · {gb(totalBytes(option))} GB ·{' '}
            <button
              type="button"
              className={styles.hfLink}
              onClick={() =>
                void invoke('open_url', {
                  url: `${HF_BASE_URL}/${starter.repo}`,
                })
              }
              aria-label={`View ${starter.display_name} on Hugging Face`}
            >
              {starter.license_note} ↗
            </button>
          </div>
        </div>
        {!showProgress ? (
          <div className={styles.right}>
            <Tooltip label={RAM_FIT_TOOLTIP[fit]} multiline placement="top">
              <span className={`${styles.fit} ${FIT_CLASS[fit]}`}>
                {RAM_FIT_LABEL[fit]}
              </span>
            </Tooltip>
            <CardAction
              option={option}
              installed={installed}
              partialBytes={partial_bytes}
              onDownload={onDownload}
              onResume={onResume}
              onDiscard={onDiscard}
            />
          </div>
        ) : null}
      </div>
      {showProgress ? (
        <div className={styles.progress}>
          <DownloadProgress
            state={state}
            progress={progress}
            etaSeconds={etaSeconds}
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

interface CardActionProps {
  option: StarterOption;
  installed: boolean;
  partialBytes: number | null;
  onDownload: (tier: StarterTier) => void;
  onResume: (tier: StarterTier) => void;
  onDiscard: (sha256: string) => void;
}

/** The per-card affordance: an installed marker, a resume/discard pair when an
 * interrupted partial exists, or the icon download button. */
function CardAction({
  option,
  installed,
  partialBytes,
  onDownload,
  onResume,
  onDiscard,
}: CardActionProps) {
  const { starter } = option;

  if (installed) {
    return <span className={styles.installed}>Installed</span>;
  }

  if (partialBytes !== null) {
    return (
      <span className={styles.resumeWrap}>
        <button
          type="button"
          className={styles.resumeBtn}
          onClick={() => onResume(starter.tier)}
        >
          Resume ({gb(partialBytes)} GB)
        </button>
        <button
          type="button"
          className={styles.discardBtn}
          onClick={() => onDiscard(starter.sha256)}
        >
          Discard
        </button>
      </span>
    );
  }

  return (
    <button
      type="button"
      className={styles.getBtn}
      aria-label="Download"
      onClick={() => onDownload(starter.tier)}
    >
      {DOWNLOAD_ICON}
    </button>
  );
}
