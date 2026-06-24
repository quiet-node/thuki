/**
 * Comparison-matrix starter picker for the built-in engine.
 *
 * Columns are the three tiers; rows are the dimensions a user actually
 * weighs (speed, quality, vision, memory fit, license). All three tiers are
 * presented as equal peers - no column is singled out as recommended.
 *
 * The matrix also owns the download display: tapping a column's Download
 * starts the download for that tier in place (no confirm step), and that
 * column's button morphs into a filling progress bar while the other two
 * dim. Every download sub-state renders in the same spot, so the picker
 * never gives way to a separate screen.
 */

import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type React from 'react';
import {
  isDownloadInFlight,
  type DownloadUiFailKind,
  type DownloadUiState,
} from '../hooks/useDownloadModel';
import type { RamFit, StarterOption, StarterTier } from '../types/starter';
import { ALWAYS_REASONS_LABEL } from './ModelPickerPanel';

const HF_BASE_URL = 'https://huggingface.co';

/** Column order, left to right. */
const TIER_ORDER: StarterTier[] = ['fast', 'balanced', 'smartest'];

/** Tier labels, keyed by the registry's tier value. */
const TIER_LABELS: Record<StarterTier, string> = {
  fast: 'Fast',
  balanced: 'Balanced',
  smartest: 'Smartest',
};

/**
 * Qualitative speed/quality levels (0..1), relative to each other across the
 * three starters. Display-only: the tier IS the speed/quality position (a 4B
 * model is faster and lower quality than a 14B), so these encode that tradeoff
 * for the comparison bars. Not configuration, purely how the matrix renders.
 */
const TIER_LEVELS: Record<StarterTier, { speed: number; quality: number }> = {
  fast: { speed: 0.95, quality: 0.5 },
  balanced: { speed: 0.62, quality: 0.8 },
  smartest: { speed: 0.4, quality: 0.97 },
};

/** Short "On your Mac" label + color per RAM fit. */
const FIT_SHORT: Record<RamFit, { label: string; color: string }> = {
  fits: { label: 'Comfortable', color: '#5fcf86' },
  tight: { label: 'Tight', color: '#ff8d5c' },
  too_big: { label: 'Heavy', color: '#ef4444' },
};

/** Short failure copy for the in-column failed state. Exhaustive over the
 * failure kinds, so no fallback is needed. */
const FAIL_SHORT: Record<DownloadUiFailKind, string> = {
  offline: "You're offline",
  http: 'Download error',
  checksum: 'Verify failed',
  disk_full: 'Not enough disk',
  engine: 'Engine could not start',
  other: 'Download failed',
};

/** Phases where one column owns the matrix (others dim, no new download). */
const BUSY_PHASES = new Set([
  'downloading',
  'downloading_mmproj',
  'verifying',
  'installing',
  'warming_up',
  'ready',
  'failed',
]);

/** Fixed cell heights so the label column and tier columns stay row-aligned. */
const HEADER_H = 52;
const ROW_H = 44;
/** Fixed action-area height across every column and state, so the Resume +
 * Discard pair, a download fill, or a plain button all occupy the same space
 * and nothing shifts when the secondary Discard link appears or disappears. */
const ACTION_H = 92;

const CELL_BORDER = '1px solid rgba(255,255,255,0.05)';

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "8.2"). */
function gb(bytes: number): string {
  return (bytes / 1e9).toFixed(1);
}

/** Seconds rendered as a compact countdown: "45s", "5m", "2h 1m". */
function formatEta(etaSeconds: number): string {
  if (etaSeconds < 60) return `${etaSeconds}s`;
  if (etaSeconds < 3600) return `${Math.floor(etaSeconds / 60)}m`;
  const hours = Math.floor(etaSeconds / 3600);
  const minutes = Math.floor((etaSeconds % 3600) / 60);
  return `${hours}h ${minutes}m`;
}

/** Weights + vision companion, the full on-disk cost of one starter. */
function totalBytes(option: StarterOption): number {
  return option.starter.size_bytes + option.starter.mmproj_bytes;
}

/** Opens the model's Hugging Face page in the system browser. */
function openHuggingFace(repo: string): void {
  void invoke('open_url', { url: `${HF_BASE_URL}/${repo}` });
}

export interface StarterMatrixProps {
  options: StarterOption[];
  /** Live download state machine, so the active column can render progress. */
  state: DownloadUiState;
  /**
   * Cumulative bytes downloaded across both files (weights + vision
   * companion), or null before the first byte. The two files render as one
   * continuous bar against the card total, never as two separate downloads.
   */
  combinedBytes: number | null;
  /** Rolling download rate in bytes per second, or null until measurable. */
  speedBytesPerSec: number | null;
  /** Which tier the active download belongs to (null when idle). */
  downloadingTier: StarterTier | null;
  onDownload: (tier: StarterTier) => void;
  onResume: (
    tier: StarterTier,
    partialBytes: number,
    sizeBytes: number,
  ) => void;
  onDiscard: (sha256: string) => void;
  onCancel: () => void;
  onRetry: () => void;
  /**
   * When wired, renders a quiet "Continue setup" line while a download is in
   * flight, letting the user leave the picker and let it finish in the
   * background. Omitted in the Settings context, where there is no next step.
   */
  onContinue?: () => void;
  /** When true (and onUseOllama is wired), offers the Ollama escape hatch. */
  ollamaDetected?: boolean;
  onUseOllama?: () => void;
}

export function StarterMatrix({
  options,
  state,
  combinedBytes,
  speedBytesPerSec,
  downloadingTier,
  onDownload,
  onResume,
  onDiscard,
  onCancel,
  onRetry,
  onContinue,
  ollamaDetected,
  onUseOllama,
}: StarterMatrixProps) {
  // Render in a stable left-to-right tier order regardless of the order the
  // backend returns the rows in.
  const ordered = TIER_ORDER.map((tier) =>
    options.find((o) => o.starter.tier === tier),
  ).filter((o): o is StarterOption => o !== undefined);

  const busy = BUSY_PHASES.has(state.phase);
  // A live download locks the other columns (one download at a time); a
  // failure does not, so the user can still start a different tier without
  // an explicit "choose another".
  const lockOthers = busy && state.phase !== 'failed';

  return (
    <div>
      <div
        data-starter-matrix
        style={{
          display: 'flex',
          alignItems: 'flex-start',
          gap: 0,
          borderRadius: 16,
          border: '1px solid rgba(255,255,255,0.07)',
          overflow: 'hidden',
          background: 'rgba(255,255,255,0.015)',
        }}
      >
        <LabelColumn />
        {ordered.map((option) => {
          const active = busy && downloadingTier === option.starter.tier;
          return (
            <TierColumn
              key={option.starter.tier}
              option={option}
              active={active}
              dimmed={lockOthers && !active}
              disabled={lockOthers}
              state={state}
              combinedBytes={combinedBytes}
              speedBytesPerSec={speedBytesPerSec}
              onDownload={onDownload}
              onResume={onResume}
              onDiscard={onDiscard}
              onCancel={onCancel}
              onRetry={onRetry}
            />
          );
        })}
      </div>
      {onContinue && isDownloadInFlight(state.phase) ? (
        <div
          style={{
            textAlign: 'center',
            margin: '14px auto 0',
            fontSize: 11.5,
            color: 'rgba(255,255,255,0.5)',
          }}
        >
          Downloading in the background.{' '}
          <button
            onClick={onContinue}
            style={{
              background: 'transparent',
              border: 'none',
              padding: 0,
              fontFamily: 'inherit',
              fontSize: 11.5,
              fontWeight: 700,
              color: 'rgba(255,141,92,0.7)',
              cursor: 'pointer',
            }}
          >
            Continue setup →
          </button>
        </div>
      ) : null}
      {ollamaDetected && onUseOllama ? (
        <div
          style={{
            textAlign: 'center',
            margin: '14px auto 0',
            fontSize: 11.5,
            color: 'rgba(255,255,255,0.5)',
          }}
        >
          Looks like Ollama&apos;s also running here on this machine.{' '}
          <button
            onClick={onUseOllama}
            style={{
              background: 'transparent',
              border: 'none',
              padding: 0,
              fontFamily: 'inherit',
              fontSize: 11.5,
              fontWeight: 700,
              color: 'rgba(255,141,92,0.7)',
              cursor: 'pointer',
            }}
          >
            Use it instead
          </button>
        </div>
      ) : null}
    </div>
  );
}

/** Left axis: the row labels, height-matched to the tier columns. */
function LabelColumn() {
  const cell = (label: string) => (
    <div
      style={{
        height: ROW_H,
        display: 'flex',
        alignItems: 'center',
        padding: '0 14px',
        fontSize: 11,
        fontWeight: 600,
        color: 'rgba(255,255,255,0.4)',
        borderTop: CELL_BORDER,
      }}
    >
      {label}
    </div>
  );
  return (
    <div style={{ width: 104, flexShrink: 0 }}>
      <div style={{ height: HEADER_H }} />
      {cell('Size')}
      {cell('Speed')}
      {cell('Quality')}
      {cell('Vision')}
      {cell('Reasoning')}
      {cell('On your Mac')}
      {cell('Origin')}
      {cell('License')}
    </div>
  );
}

interface TierColumnProps {
  option: StarterOption;
  active: boolean;
  dimmed: boolean;
  disabled: boolean;
  state: DownloadUiState;
  combinedBytes: number | null;
  speedBytesPerSec: number | null;
  onDownload: (tier: StarterTier) => void;
  onResume: (
    tier: StarterTier,
    partialBytes: number,
    sizeBytes: number,
  ) => void;
  onDiscard: (sha256: string) => void;
  onCancel: () => void;
  onRetry: () => void;
}

function TierColumn({
  option,
  active,
  dimmed,
  disabled,
  state,
  combinedBytes,
  speedBytesPerSec,
  onDownload,
  onResume,
  onDiscard,
  onCancel,
  onRetry,
}: TierColumnProps) {
  const { starter, fit } = option;
  const levels = TIER_LEVELS[starter.tier];
  const fitInfo = FIT_SHORT[fit];

  return (
    <div
      data-tier-column
      data-tier={starter.tier}
      style={{
        flex: 1,
        minWidth: 0,
        opacity: dimmed ? 0.32 : 1,
        transition: 'opacity 0.2s ease',
        boxShadow: 'none',
        background: 'transparent',
      }}
    >
      {/* Header: tier eyebrow, then the model name (size moved to its own row
          so it never truncates next to a long name). */}
      <div style={{ height: HEADER_H, padding: '11px 14px 0' }}>
        <div
          style={{
            fontSize: 10,
            fontWeight: 700,
            letterSpacing: '1px',
            textTransform: 'uppercase',
            color: 'rgba(255,255,255,0.4)',
          }}
        >
          {TIER_LABELS[starter.tier]}
        </div>
        <div
          style={{
            marginTop: 3,
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            fontSize: 15,
            fontWeight: 700,
            color: '#fff',
            letterSpacing: '-0.2px',
          }}
        >
          {starter.display_name}
        </div>
      </div>

      <ValueCell>
        <span style={{ color: '#fff', fontWeight: 600 }}>
          {gb(totalBytes(option))} GB
        </span>
      </ValueCell>

      <BarCell level={levels.speed} />
      <BarCell level={levels.quality} />

      <ValueCell>
        {starter.vision ? (
          <span style={{ color: '#5fcf86', fontWeight: 700 }}>Yes</span>
        ) : (
          <span style={{ color: 'rgba(255,255,255,0.28)' }}>&mdash;</span>
        )}
      </ValueCell>

      <ValueCell>
        {starter.reasoning_always ? (
          <span style={{ color: 'rgba(255,255,255,0.6)', fontWeight: 600 }}>
            {ALWAYS_REASONS_LABEL}
          </span>
        ) : starter.thinking ? (
          <span style={{ color: 'rgba(255,255,255,0.6)', fontWeight: 600 }}>
            On demand
          </span>
        ) : (
          <span style={{ color: 'rgba(255,255,255,0.28)' }}>&mdash;</span>
        )}
      </ValueCell>

      <ValueCell>
        <span style={{ color: fitInfo.color, fontWeight: 700 }}>
          {fitInfo.label}
        </span>
      </ValueCell>

      <ValueCell>
        <ProvenanceLink
          repo={starter.origin_repo}
          ariaLabel={`Verify ${starter.display_name}: open its maker ${starter.origin} on Hugging Face`}
        >
          {starter.origin}
        </ProvenanceLink>
      </ValueCell>

      <ValueCell>
        <ProvenanceLink
          repo={starter.repo}
          ariaLabel={`Open ${starter.display_name} on Hugging Face`}
        >
          {starter.license_note}
        </ProvenanceLink>
      </ValueCell>

      {/* Action: the filling download cell when this column is active,
          otherwise the plain download/resume/installed affordance. Fixed
          height so the optional Discard link never shifts the layout. */}
      <div style={{ height: ACTION_H, padding: '14px 14px 0' }}>
        {active ? (
          <DownloadCell
            state={state}
            combinedBytes={combinedBytes}
            speedBytesPerSec={speedBytesPerSec}
            grandTotalBytes={totalBytes(option)}
            onCancel={onCancel}
            onRetry={onRetry}
          />
        ) : (
          <ColumnAction
            option={option}
            disabled={disabled}
            onDownload={onDownload}
            onResume={onResume}
            onDiscard={onDiscard}
          />
        )}
      </div>
    </div>
  );
}

/** A trait row holding a horizontal level bar. */
function BarCell({ level }: { level: number }) {
  return (
    <div
      style={{
        height: ROW_H,
        display: 'flex',
        alignItems: 'center',
        padding: '0 14px',
        borderTop: CELL_BORDER,
      }}
    >
      <div
        style={{
          position: 'relative',
          width: '100%',
          height: 6,
          borderRadius: 999,
          background: 'rgba(255,255,255,0.07)',
          overflow: 'hidden',
        }}
      >
        <div
          data-bar-fill
          style={{
            position: 'absolute',
            inset: '0 auto 0 0',
            width: `${Math.round(level * 100)}%`,
            borderRadius: 999,
            background: 'linear-gradient(90deg, #ff8d5c, #d45a1e)',
          }}
        />
      </div>
    </div>
  );
}

/** A trait row holding a short text value (Vision, On your Mac, License). */
function ValueCell({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        height: ROW_H,
        display: 'flex',
        alignItems: 'center',
        padding: '0 14px',
        fontSize: 12.5,
        borderTop: CELL_BORDER,
      }}
    >
      {children}
    </div>
  );
}

/** A small "↗" link inside a trait cell that opens a Hugging Face repo page.
 * Shared by the Origin row (the model maker's official page) and the License
 * row (the GGUF download source). */
function ProvenanceLink({
  repo,
  ariaLabel,
  children,
}: {
  repo: string;
  ariaLabel: string;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={() => openHuggingFace(repo)}
      aria-label={ariaLabel}
      style={{
        background: 'transparent',
        border: 'none',
        padding: 0,
        fontFamily: 'inherit',
        fontSize: 11.5,
        fontWeight: 600,
        color: 'rgba(255,141,92,0.78)',
        cursor: 'pointer',
        whiteSpace: 'nowrap',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        maxWidth: '100%',
      }}
    >
      {children} ↗
    </button>
  );
}

interface DownloadCellProps {
  state: DownloadUiState;
  /** Cumulative bytes across both files, or null before the first byte. */
  combinedBytes: number | null;
  /** Rolling download rate in bytes per second, or null until measurable. */
  speedBytesPerSec: number | null;
  /** The card's full on-disk total (weights + vision companion). */
  grandTotalBytes: number;
  onCancel: () => void;
  onRetry: () => void;
}

/**
 * The active column's download display: the pressed button morphs into a
 * filling progress bar, counting up while determinate and showing the
 * post-download steps (verify, install, ready) as a full bar with a label.
 * A failure swaps in a short headline plus Retry.
 */
function DownloadCell({
  state,
  combinedBytes,
  speedBytesPerSec,
  grandTotalBytes,
  onCancel,
  onRetry,
}: DownloadCellProps) {
  const [hover, setHover] = useState(false);

  if (state.phase === 'failed') {
    return (
      <div style={{ textAlign: 'center' }}>
        <div
          style={{
            fontSize: 11.5,
            fontWeight: 700,
            color: '#ff8d5c',
            marginBottom: 9,
            lineHeight: 1.35,
          }}
        >
          {FAIL_SHORT[state.kind]}
        </div>
        <ActionButton label="Retry" recommended onClick={onRetry} />
      </div>
    );
  }

  // While bytes are coming down, the button IS the progress: it fills as one
  // continuous bar against the card's full total (weights + vision companion
  // summed, never two separate downloads), shows the byte counts and ETA
  // inside (no percentage, no speed), and is the cancel control. Hovering eases
  // the warm fill to a neutral "stop" grey and swaps in "Pause download".
  if (state.phase === 'downloading' || state.phase === 'downloading_mmproj') {
    const pct =
      combinedBytes !== null && grandTotalBytes > 0
        ? Math.min(100, Math.floor((combinedBytes / grandTotalBytes) * 100))
        : 0;
    // The rolling rate drives the ETA but is not shown: the ETA already answers
    // "how much longer", and the column is too narrow for a third figure.
    // speedBytesPerSec is null or strictly positive (the hook never reports a
    // zero rate), so a non-null value is always safe to divide by.
    const etaSeconds =
      combinedBytes !== null && speedBytesPerSec !== null
        ? Math.max(
            0,
            Math.round((grandTotalBytes - combinedBytes) / speedBytesPerSec),
          )
        : null;
    const bytesLabel =
      combinedBytes === null
        ? 'Starting…'
        : `${gb(combinedBytes)} / ${gb(grandTotalBytes)} GB${
            etaSeconds !== null ? ` · ${formatEta(etaSeconds)} left` : ''
          }`;
    return (
      <button
        onClick={onCancel}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        aria-label="Pause download"
        style={{
          position: 'relative',
          width: '100%',
          height: 42,
          borderRadius: 12,
          overflow: 'hidden',
          cursor: 'pointer',
          fontFamily: 'inherit',
          padding: 0,
          border: `1px solid ${
            hover ? 'rgba(255,255,255,0.22)' : 'rgba(255,141,92,0.3)'
          }`,
          background: 'rgba(255,255,255,0.06)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          transition: 'border-color 0.4s ease',
        }}
      >
        {/* One warm gradient that desaturates to a neutral "stop" grey via a
            filter (gradients cannot tween, but filters can), so the hover
            shift is smooth. */}
        <span
          data-download-fill
          aria-hidden="true"
          style={{
            position: 'absolute',
            inset: '0 auto 0 0',
            width: `${pct}%`,
            borderRadius: 12,
            background:
              'linear-gradient(135deg, #ffa06f, #ff8d5c 40%, #d45a1e)',
            filter: hover
              ? 'grayscale(0.95) brightness(0.82)'
              : 'grayscale(0) brightness(1)',
            transition: 'width 0.3s ease, filter 0.4s ease',
          }}
        />
        {/* Two labels stacked in the same cell, cross-faded on hover. */}
        <span style={{ position: 'relative', zIndex: 2, display: 'grid' }}>
          <span
            style={{
              gridArea: '1 / 1',
              fontSize: 12,
              fontWeight: 800,
              color: '#fff',
              textShadow: '0 1px 2px rgba(0,0,0,0.35)',
              whiteSpace: 'nowrap',
              // Slightly tightened so even the biggest tier ("10.5 / 10.6 GB ·
              // Em left") fits the ~160px column without clipping.
              letterSpacing: '-0.2px',
              opacity: hover ? 0 : 1,
              transition: 'opacity 0.3s ease',
            }}
          >
            {bytesLabel}
          </span>
          <span
            style={{
              gridArea: '1 / 1',
              fontSize: 12.5,
              fontWeight: 800,
              color: '#fff',
              textShadow: '0 1px 2px rgba(0,0,0,0.35)',
              whiteSpace: 'nowrap',
              opacity: hover ? 1 : 0,
              transition: 'opacity 0.3s ease',
            }}
          >
            Pause download
          </span>
        </span>
      </button>
    );
  }

  // Verifying / installing / warming / ready: a full bar with a label. The
  // bytes are already down, so there is nothing left to cancel.
  const ready = state.phase === 'ready';
  const label =
    state.phase === 'verifying'
      ? 'Verifying'
      : state.phase === 'installing'
        ? 'Installing'
        : state.phase === 'ready'
          ? 'Ready'
          : 'Starting engine';
  return (
    <div
      style={{
        position: 'relative',
        width: '100%',
        height: 42,
        borderRadius: 12,
        overflow: 'hidden',
        border: `1px solid ${
          ready ? 'rgba(95,207,134,0.45)' : 'rgba(255,141,92,0.3)'
        }`,
        background: 'rgba(255,255,255,0.06)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <span
        aria-hidden="true"
        style={{
          position: 'absolute',
          inset: 0,
          borderRadius: 12,
          opacity: 0.92,
          background: ready
            ? 'linear-gradient(135deg, #5fcf86, #3a9d63)'
            : 'linear-gradient(135deg, #ffa06f, #ff8d5c 40%, #d45a1e)',
        }}
      />
      <span
        style={{
          position: 'relative',
          zIndex: 2,
          fontSize: 12.5,
          fontWeight: 800,
          color: '#fff',
          textShadow: '0 1px 2px rgba(0,0,0,0.35)',
        }}
      >
        {label}
      </span>
    </div>
  );
}

interface ColumnActionProps {
  option: StarterOption;
  disabled: boolean;
  onDownload: (tier: StarterTier) => void;
  onResume: (
    tier: StarterTier,
    partialBytes: number,
    sizeBytes: number,
  ) => void;
  onDiscard: (sha256: string) => void;
}

/**
 * Per-column affordance: an installed line, a resume/discard pair when an
 * interrupted partial exists, or the plain download button (quiet outline, the
 * same for every tier). `disabled` dims the buttons while another column's
 * download is in flight.
 */
function ColumnAction({
  option,
  disabled,
  onDownload,
  onResume,
  onDiscard,
}: ColumnActionProps) {
  const { starter, installed, partial_bytes } = option;

  if (installed) {
    return (
      <div
        style={{
          textAlign: 'center',
          fontSize: 12,
          fontWeight: 700,
          color: '#5fcf86',
          padding: '9px 0',
        }}
      >
        Installed
      </div>
    );
  }

  if (partial_bytes !== null) {
    return (
      <div style={{ textAlign: 'center' }}>
        <ResumeButton
          tier={starter.tier}
          sizeBytes={starter.size_bytes}
          partialBytes={partial_bytes}
          disabled={disabled}
          onResume={onResume}
        />
        {!disabled ? (
          <DiscardLink onClick={() => onDiscard(starter.sha256)} />
        ) : null}
      </div>
    );
  }

  return (
    <ActionButton
      label="Download"
      recommended={false}
      disabled={disabled}
      onClick={() => onDownload(starter.tier)}
    />
  );
}

interface ActionButtonProps {
  label: string;
  recommended: boolean;
  disabled?: boolean;
  onClick: () => void;
}

function ActionButton({
  label,
  recommended,
  disabled = false,
  onClick,
}: ActionButtonProps) {
  const [hover, setHover] = useState(false);
  const showHover = hover && !disabled;
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: 'block',
        width: '100%',
        padding: '10px',
        borderRadius: 11,
        fontFamily: 'inherit',
        fontSize: 12.5,
        fontWeight: 700,
        cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.5 : 1,
        color: recommended ? '#fff' : 'rgba(255,255,255,0.9)',
        background: recommended
          ? 'linear-gradient(135deg, #ffa06f 0%, #ff8d5c 35%, #d45a1e 100%)'
          : showHover
            ? 'rgba(255,255,255,0.08)'
            : 'rgba(255,255,255,0.045)',
        border: recommended
          ? 'none'
          : `1px solid ${showHover ? 'rgba(255,141,92,0.35)' : 'rgba(255,255,255,0.1)'}`,
        boxShadow: recommended
          ? '0 10px 24px -10px rgba(255,110,50,0.65), 0 1px 0 rgba(255,255,255,0.22) inset'
          : 'none',
        filter: showHover && recommended ? 'brightness(1.07)' : 'none',
        transition:
          'filter 0.15s ease, background 0.15s ease, border-color 0.15s ease',
      }}
    >
      {label}
    </button>
  );
}

interface ResumeButtonProps {
  tier: StarterTier;
  /** Weights total; the caller has already narrowed partialBytes to non-null. */
  sizeBytes: number;
  partialBytes: number;
  disabled: boolean;
  onResume: (
    tier: StarterTier,
    partialBytes: number,
    sizeBytes: number,
  ) => void;
}

/**
 * Resume affordance for an interrupted partial. The mirror of the downloading
 * button: at rest it shows how far the download got ("2.1 / 2.5 GB") behind a
 * dimmed warm fill; hovering brings the fill to full strength and swaps in
 * "Resume". Both shifts are smooth (opacity tweens, no gradient swap).
 */
function ResumeButton({
  tier,
  sizeBytes,
  partialBytes,
  disabled,
  onResume,
}: ResumeButtonProps) {
  const [hover, setHover] = useState(false);
  const pct = Math.min(100, Math.floor((partialBytes / sizeBytes) * 100));
  const bytesLabel = `${gb(partialBytes)} / ${gb(sizeBytes)} GB`;
  const showHover = hover && !disabled;
  return (
    <button
      onClick={() => onResume(tier, partialBytes, sizeBytes)}
      disabled={disabled}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      aria-label="Resume download"
      style={{
        position: 'relative',
        width: '100%',
        height: 42,
        borderRadius: 12,
        overflow: 'hidden',
        cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.5 : 1,
        fontFamily: 'inherit',
        padding: 0,
        border: '1px solid rgba(255,141,92,0.3)',
        background: 'rgba(255,255,255,0.06)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <span
        aria-hidden="true"
        style={{
          position: 'absolute',
          inset: '0 auto 0 0',
          width: `${pct}%`,
          borderRadius: 12,
          background: 'linear-gradient(135deg, #ffa06f, #ff8d5c 40%, #d45a1e)',
          opacity: showHover ? 1 : 0.5,
          transition: 'opacity 0.4s ease',
        }}
      />
      <span style={{ position: 'relative', zIndex: 2, display: 'grid' }}>
        <span
          style={{
            gridArea: '1 / 1',
            fontSize: 12.5,
            fontWeight: 800,
            color: '#fff',
            textShadow: '0 1px 2px rgba(0,0,0,0.35)',
            whiteSpace: 'nowrap',
            opacity: showHover ? 0 : 1,
            transition: 'opacity 0.3s ease',
          }}
        >
          {bytesLabel}
        </span>
        <span
          style={{
            gridArea: '1 / 1',
            fontSize: 12.5,
            fontWeight: 800,
            color: '#fff',
            textShadow: '0 1px 2px rgba(0,0,0,0.35)',
            whiteSpace: 'nowrap',
            opacity: showHover ? 1 : 0,
            transition: 'opacity 0.3s ease',
          }}
        >
          Resume
        </span>
      </span>
    </button>
  );
}

/** The quiet grey "Discard partial" link beneath a Resume button. */
function DiscardLink({ onClick }: { onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        display: 'block',
        margin: '9px auto 0',
        background: 'transparent',
        border: 'none',
        padding: 0,
        fontFamily: 'inherit',
        fontSize: 11,
        fontWeight: 600,
        color: 'rgba(255,255,255,0.4)',
        cursor: 'pointer',
      }}
    >
      Discard partial
    </button>
  );
}
