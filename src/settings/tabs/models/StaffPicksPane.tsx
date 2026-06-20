/**
 * Staff-picks pane: the curated front door of Discover.
 *
 * Thuki hand-picks a short catalog of models, grouped by family. Each family is
 * a collapsible accordion section; the one holding the recommended pick is open
 * by default. A model row shows its friendly name, the one quant Thuki chose for
 * it, size, capability pills (Text always, plus Vision / Thinking), a RAM-fit
 * hint, and a single Download that runs the VERIFIED starter path
 * (`download_starter`, pinned revision + sha256), unlike the Browse-all pane's
 * arbitrary repo downloads. A finished install lifts a fresh config snapshot.
 *
 * Data comes from {@link useStarterOptions} (the same rows onboarding's picker
 * uses); the download state machine is the shared {@link useDownloadModel}, so
 * the in-flight / failed UI is the same {@link DownloadProgress} card the rest
 * of the app shows. At most one model downloads at a time (the backend enforces
 * it too); `activeTier` tracks which row owns the progress card.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { DownloadProgress } from '../../../components/DownloadProgress';
import { useDownloadModel } from '../../../hooks/useDownloadModel';
import { useStarterOptions } from '../../../components/StarterPicker';
import { Tooltip } from '../../../components/Tooltip';
import { RAM_FIT_LABEL, RAM_FIT_TOOLTIP } from '../../../utils/ramFit';
import styles from './StaffPicksPane.module.css';
import type { RawAppConfig } from '../../types';
import type { RamFit, StarterOption, StarterTier } from '../../../types/starter';

const HF_BASE_URL = 'https://huggingface.co';

/** The tier marked as the recommended pick (and whose family opens by default). */
const RECOMMENDED_TIER: StarterTier = 'balanced';

/** RAM-fit hint colour class on this pane's stylesheet (labels are shared). */
const FIT_CLASS: Record<RamFit, string> = {
  fits: styles.fitOk,
  tight: styles.fitTight,
  too_big: styles.fitHeavy,
};

/** A plain-language line about what a family is good for, falling back to the
 * model maker when a family has no hand-written blurb. Presentational only. */
const FAMILY_BLURB: Record<string, string> = {
  Qwen: 'Fast, capable all-rounder',
  Gemma: 'Well-rounded, reads images',
  'gpt-oss': 'Strongest reasoning',
};

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "8.2"). */
function gb(bytes: number): string {
  return (bytes / 1e9).toFixed(1);
}

/** Weights + vision companion: the full on-disk cost of one starter. */
function totalBytes(o: StarterOption): number {
  return o.starter.size_bytes + o.starter.mmproj_bytes;
}

/** One family group: its label and the curated models under it, registry order. */
interface FamilyGroup {
  family: string;
  blurb: string;
  options: StarterOption[];
}

/** Groups starter rows by family, preserving first-seen (registry) order. */
function groupByFamily(options: StarterOption[]): FamilyGroup[] {
  const groups: FamilyGroup[] = [];
  for (const o of options) {
    const family = o.starter.family ?? o.starter.display_name;
    const existing = groups.find((g) => g.family === family);
    if (existing) {
      existing.options.push(o);
    } else {
      groups.push({
        family,
        blurb: FAMILY_BLURB[family] ?? o.starter.origin,
        options: [o],
      });
    }
  }
  return groups;
}

const CHEVRON = (
  <svg viewBox="0 0 10 10" aria-hidden="true" className={styles.chev}>
    <path d="M3 2l4 3-4 3" />
  </svg>
);

interface StaffPicksPaneProps {
  /** Lift a fresh config snapshot after a successful install. */
  onSaved: (next: RawAppConfig) => void;
}

export function StaffPicksPane({ onSaved }: StaffPicksPaneProps) {
  const { options, refresh } = useStarterOptions();
  const groups = useMemo(() => groupByFamily(options ?? []), [options]);

  // The family holding the recommended tier opens by default; if the catalog
  // has no recommended tier, the first family opens so the pane is never blank.
  const defaultOpen = useMemo(() => {
    const recommended = groups.find((g) =>
      g.options.some((o) => o.starter.tier === RECOMMENDED_TIER),
    );
    const pick = recommended ?? groups[0];
    return new Set(pick ? [pick.family] : []);
  }, [groups]);

  // Seed the open set ONCE, when the catalog first resolves (the mount fetch
  // arrives after the initial empty render). A later refresh must not collapse
  // families the user opened, so this never re-seeds.
  const [open, setOpen] = useState<Set<string>>(new Set());
  const seededRef = useRef(false);
  useEffect(() => {
    if (seededRef.current || groups.length === 0) return;
    seededRef.current = true;
    setOpen(defaultOpen);
  }, [groups, defaultOpen]);

  // One download at a time; activeTier names the row that owns the progress card.
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
      setActiveTier(null);
      await refresh();
    })();
  }, [state.phase, onSaved, reset, refresh]);

  function toggle(family: string) {
    setOpen((cur) => {
      const next = new Set(cur);
      if (next.has(family)) {
        next.delete(family);
      } else {
        next.add(family);
      }
      return next;
    });
  }

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

  if (options !== null && groups.length === 0) {
    return (
      <div className={styles.pane}>
        <p className={styles.empty}>No curated models are available.</p>
      </div>
    );
  }

  return (
    <div className={styles.pane}>
      <p className={styles.hint}>
        Hand-picked by Thuki, grouped by family. Open a family to choose a size.
      </p>
      <div className={styles.list}>
        {groups.map((group) => {
          const expanded = open.has(group.family);
          return (
            <div className={styles.fam} key={group.family}>
              <button
                type="button"
                className={styles.famHead}
                aria-expanded={expanded}
                onClick={() => toggle(group.family)}
              >
                <span className={styles.famText}>
                  <span className={styles.famName}>{group.family}</span>
                  <span className={styles.famSub}>
                    {group.blurb} · {group.options.length}{' '}
                    {group.options.length === 1 ? 'model' : 'models'}
                  </span>
                </span>
                <span
                  className={`${styles.chevWrap} ${expanded ? styles.chevOpen : ''}`}
                >
                  {CHEVRON}
                </span>
              </button>
              {expanded ? (
                <div className={styles.famBody}>
                  {group.options.map((o) => (
                    <ModelRow
                      key={o.starter.tier}
                      option={o}
                      recommended={o.starter.tier === RECOMMENDED_TIER}
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
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}

interface ModelRowProps {
  option: StarterOption;
  recommended: boolean;
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

function ModelRow({
  option,
  recommended,
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
}: ModelRowProps) {
  const { starter, fit, installed, partial_bytes } = option;
  const showProgress = active && state.phase !== 'idle';

  return (
    <div className={styles.row} data-model-row data-tier={starter.tier}>
      <div className={styles.rowMain}>
        <div className={styles.mid}>
          <div className={styles.name}>
            {starter.display_name}
            {recommended ? (
              <span className={styles.recommended}>Recommended</span>
            ) : null}
          </div>
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
            <RowAction
              option={option}
              recommended={recommended}
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

interface RowActionProps {
  option: StarterOption;
  recommended: boolean;
  installed: boolean;
  partialBytes: number | null;
  onDownload: (tier: StarterTier) => void;
  onResume: (tier: StarterTier) => void;
  onDiscard: (sha256: string) => void;
}

/** The per-row affordance: an installed marker, a resume/discard pair when an
 * interrupted partial exists, or the plain download button. */
function RowAction({
  option,
  recommended,
  installed,
  partialBytes,
  onDownload,
  onResume,
  onDiscard,
}: RowActionProps) {
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
      className={`${styles.getBtn} ${recommended ? styles.getPrimary : ''}`}
      onClick={() => onDownload(starter.tier)}
    >
      Download
    </button>
  );
}
