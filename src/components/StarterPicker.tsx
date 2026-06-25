/**
 * Three-tier starter model picker for the built-in engine.
 *
 * Presentational: the rows come in through `options` and every action is a
 * callback, so onboarding and Settings can wire the same picker into their
 * own flows. Data fetching lives in the colocated `useStarterOptions` hook
 * (mirrors how ModelCheckStep keeps its probe beside its render tree).
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type {
  RamFit,
  StaffPickOption,
  StarterOption,
  StarterTier,
} from '../types/starter';
import { InlineLink } from './InlineLink';

const HF_BASE_URL = 'https://huggingface.co';

/** Tier pill labels, keyed by the registry's tier value. */
const TIER_LABELS: Record<StarterTier, string> = {
  fast: 'Fast',
  balanced: 'Balanced',
  smartest: 'Smartest',
};

/** RAM-fit badge copy. Exact strings; consumed verbatim by tests. Exported so
 * onboarding can pass the same caution into the confirm card's RAM warning. */
export const FIT_COPY: Record<RamFit, string> = {
  fits: 'Runs comfortably on this Mac',
  tight: "Will run, but close to this Mac's memory limit",
  too_big:
    "Larger than this Mac's memory can comfortably hold. Expect heavy slowdown.",
};

const FIT_COLORS: Record<RamFit, { color: string; background: string }> = {
  fits: { color: '#22c55e', background: 'rgba(34,197,94,0.1)' },
  tight: { color: '#ff8d5c', background: 'rgba(255,141,92,0.1)' },
  too_big: { color: '#ef4444', background: 'rgba(239,68,68,0.1)' },
};

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "8.2"). */
function gb(bytes: number): string {
  return (bytes / 1e9).toFixed(1);
}

/** Weights + vision companion, the full on-disk cost of one starter. */
function totalBytes(option: StarterOption): number {
  return option.starter.size_bytes + option.starter.mmproj_bytes;
}

export interface UseStarterOptionsResult {
  /** The picker rows; `null` while the first fetch is in flight. */
  options: StarterOption[] | null;
  /** Re-fetch (e.g. after a cancel kept a resumable partial). */
  refresh: () => Promise<void>;
}

/**
 * Loads the starter picker rows from the backend. A fetch failure degrades
 * to an empty list so the picker renders nothing rather than crashing.
 */
export function useStarterOptions(): UseStarterOptionsResult {
  const [options, setOptions] = useState<StarterOption[] | null>(null);

  const refresh = useCallback(async () => {
    try {
      const rows = await invoke<StarterOption[]>('get_starter_options');
      // Guard the IPC boundary: a malformed (non-array) payload becomes an
      // empty list so consumers that iterate the rows never crash.
      setOptions(Array.isArray(rows) ? rows : []);
    } catch {
      setOptions([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { options, refresh };
}

export interface UseStaffPicksResult {
  /** The Staff Picks catalog rows; `null` while the first fetch is in flight. */
  options: StaffPickOption[] | null;
  /** Re-fetch (e.g. after a cancel kept a resumable partial). */
  refresh: () => Promise<void>;
}

/**
 * Loads the full Staff Picks catalog from the backend. A fetch failure (or a
 * malformed non-array payload) degrades to an empty list so the pane renders
 * nothing rather than crashing. Sibling of {@link useStarterOptions}: the
 * catalog is id-keyed and category-grouped, not capped at one model per tier.
 */
export function useStaffPicks(): UseStaffPicksResult {
  const [options, setOptions] = useState<StaffPickOption[] | null>(null);

  const refresh = useCallback(async () => {
    try {
      const rows = await invoke<StaffPickOption[]>('get_staff_picks');
      setOptions(Array.isArray(rows) ? rows : []);
    } catch {
      setOptions([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { options, refresh };
}

export interface StarterPickerProps {
  options: StarterOption[];
  /** The highlighted tier. Consumers default this to 'balanced'. */
  selected: StarterTier;
  onSelect: (tier: StarterTier) => void;
  onDownload: (tier: StarterTier) => void;
  onResume: (tier: StarterTier) => void;
  onDiscard: (sha256: string) => void;
  /** When true (and onUseOllama is wired), offers the Ollama escape hatch. */
  ollamaDetected?: boolean;
  onUseOllama?: () => void;
}

export function StarterPicker({
  options,
  selected,
  onSelect,
  onDownload,
  onResume,
  onDiscard,
  ollamaDetected,
  onUseOllama,
}: StarterPickerProps) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      {options.map((option) => (
        <StarterCard
          key={option.starter.tier}
          option={option}
          selected={option.starter.tier === selected}
          onSelect={onSelect}
          onDownload={onDownload}
          onResume={onResume}
          onDiscard={onDiscard}
        />
      ))}
      {ollamaDetected && onUseOllama ? (
        <button
          onClick={onUseOllama}
          style={{
            background: 'transparent',
            border: 'none',
            padding: '6px 0 0',
            fontFamily: 'inherit',
            fontSize: 11,
            fontWeight: 500,
            color: 'rgba(255,141,92,0.7)',
            cursor: 'pointer',
            textAlign: 'center',
          }}
        >
          Use my existing Ollama instead
        </button>
      ) : null}
    </div>
  );
}

interface StarterCardProps {
  option: StarterOption;
  selected: boolean;
  onSelect: (tier: StarterTier) => void;
  onDownload: (tier: StarterTier) => void;
  onResume: (tier: StarterTier) => void;
  onDiscard: (sha256: string) => void;
}

function StarterCard({
  option,
  selected,
  onSelect,
  onDownload,
  onResume,
  onDiscard,
}: StarterCardProps) {
  const { starter, fit, installed, partial_bytes } = option;
  const fitColors = FIT_COLORS[fit];

  return (
    <div
      data-starter-card
      data-tier={starter.tier}
      data-selected={selected}
      onClick={() => onSelect(starter.tier)}
      style={{
        padding: '12px 14px',
        borderRadius: 14,
        border: `1px solid ${
          selected ? 'rgba(255,141,92,0.4)' : 'rgba(255,255,255,0.06)'
        }`,
        background: selected
          ? 'rgba(255,141,92,0.07)'
          : 'rgba(255,255,255,0.03)',
        boxShadow: selected
          ? '0 0 20px rgba(255,141,92,0.08), inset 0 1px 0 rgba(255,141,92,0.1)'
          : 'none',
        cursor: 'pointer',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 10,
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            minWidth: 0,
          }}
        >
          <span
            style={{
              fontSize: 14,
              fontWeight: 600,
              color: '#f0f0f2',
              letterSpacing: '-0.1px',
            }}
          >
            {starter.display_name}
          </span>
          <span
            style={{
              fontSize: 10.5,
              fontWeight: 600,
              padding: '2px 8px',
              borderRadius: 20,
              color: selected ? '#ff8d5c' : 'rgba(255,255,255,0.55)',
              background: selected
                ? 'rgba(255,141,92,0.1)'
                : 'rgba(255,255,255,0.05)',
            }}
          >
            {TIER_LABELS[starter.tier]}
          </span>
        </div>
        <span
          style={{
            fontSize: 11.5,
            color: 'rgba(255,255,255,0.45)',
            flexShrink: 0,
          }}
        >
          {gb(totalBytes(option))} GB
        </span>
      </div>

      <div
        style={{
          display: 'inline-block',
          marginTop: 7,
          fontSize: 10.5,
          fontWeight: 500,
          padding: '3px 9px',
          borderRadius: 20,
          lineHeight: 1.4,
          ...fitColors,
        }}
      >
        {FIT_COPY[fit]}
      </div>

      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 5,
          marginTop: 7,
          fontSize: 10.5,
          color: 'rgba(255,255,255,0.4)',
        }}
      >
        <span>{starter.license_note}</span>
        <span aria-hidden="true">·</span>
        {/* The card itself is clickable (it selects the tier), so this wrapper
            stops the link click from bubbling up and also selecting the card. */}
        <span
          onClick={(e) => e.stopPropagation()}
          style={{ display: 'inline-flex' }}
        >
          <InlineLink
            url={`${HF_BASE_URL}/${starter.repo}`}
            ariaLabel={`Open ${starter.display_name} on Hugging Face`}
            style={{ fontSize: 10.5 }}
          >
            View on Hugging Face ↗
          </InlineLink>
        </span>
      </div>

      <div style={{ marginTop: 9 }}>
        <CardAction
          option={option}
          installed={installed}
          partialBytes={partial_bytes}
          onDownload={onDownload}
          onResume={onResume}
          onDiscard={onDiscard}
        />
      </div>
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

/**
 * The per-card affordance: an installed checkmark, a resume/discard pair
 * when an interrupted partial exists, or the plain download button.
 */
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
    return (
      <span
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 5,
          fontSize: 11,
          fontWeight: 600,
          color: '#22c55e',
        }}
      >
        <svg width="11" height="11" viewBox="0 0 16 16" fill="none">
          <path
            d="M3 8.5l3.2 3.2L13 5"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
        Installed
      </span>
    );
  }

  if (partialBytes !== null) {
    return (
      <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
        <ActionButton
          label={`Resume download (${gb(partialBytes)} of ${gb(
            totalBytes(option),
          )} GB)`}
          onClick={() => onResume(starter.tier)}
        />
        <ActionButton
          label="Discard"
          muted
          onClick={() => onDiscard(starter.sha256)}
        />
      </span>
    );
  }

  return (
    <ActionButton label="Download" onClick={() => onDownload(starter.tier)} />
  );
}

interface ActionButtonProps {
  label: string;
  onClick: () => void;
  muted?: boolean;
}

function ActionButton({ label, onClick, muted = false }: ActionButtonProps) {
  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      style={{
        padding: '5px 10px',
        borderRadius: 8,
        background: muted ? 'rgba(255,255,255,0.04)' : 'rgba(255,141,92,0.1)',
        border: `1px solid ${
          muted ? 'rgba(255,255,255,0.1)' : 'rgba(255,141,92,0.28)'
        }`,
        color: muted ? 'rgba(255,255,255,0.55)' : '#ff8d5c',
        fontSize: 11,
        fontWeight: 600,
        fontFamily: 'inherit',
        cursor: 'pointer',
      }}
    >
      {label}
    </button>
  );
}
