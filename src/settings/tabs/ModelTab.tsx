/**
 * AI tab.
 *
 * Holds the local Ollama endpoint, keep-warm controls, and the custom system
 * prompt. The active model picker lives in the main app overlay (see
 * ModelPickerPanel) since model selection is runtime UI state owned by
 * ActiveModelState in the backend, not a TOML-persisted field. The
 * Window/Quote knobs live in the Display tab.
 */

import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { Section, TextField, Textarea } from '../components';
import { SaveField } from '../components/SaveField';
import { useDebouncedSave } from '../hooks/useDebouncedSave';
import { configHelp } from '../configHelpers';
import { Tooltip } from '../../components/Tooltip';
import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';

interface ModelTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

const PROMPT_MAX_CHARS = 8000;
const EJECT_RESET_MS = 2500;
/// Approximate tokens per chat turn used for the "~N turns of context" hint.
/// 400 tokens ≈ a typical user question + assistant reply pair on this app.
const TOKENS_PER_TURN_ESTIMATE = 400;

const KEEP_WARM_TOOLTIP =
  'Keep Warm holds your active model loaded in VRAM after each use. ' +
  'The timer below sets how long before it auto-releases; use -1 to keep it indefinitely. ' +
  'Unload now releases it immediately. ' +
  'If set to 0, Ollama unloads models after its default 5-minute timeout.';

// Log-scale context window slider: slider pos [0..1000] ↔ token count.
// Scale: value = CTX_MIN * (CTX_MAX / CTX_MIN)^(pos/1000)
// With CTX_MAX/CTX_MIN = 512 (= 2^9), each 1/9 of the slider doubles the value.
const CTX_MIN = 2048;
const CTX_MAX = 1_048_576; // 1M
const CTX_LOG_RATIO = Math.log(CTX_MAX / CTX_MIN);

function ctxToPos(v: number): number {
  return Math.round((1000 * Math.log(v / CTX_MIN)) / CTX_LOG_RATIO);
}

function posToCtx(pos: number): number {
  // Snap to nearest 1 KiB boundary (standard Ollama increment).
  return (
    Math.round((CTX_MIN * Math.pow(CTX_MAX / CTX_MIN, pos / 1000)) / 1024) *
    1024
  );
}

const CTX_TICKS = [
  '2K',
  '4K',
  '8K',
  '16K',
  '32K',
  '64K',
  '128K',
  '256K',
  '512K',
  '1M',
];

export function ModelTab({ config, resyncToken, onSaved }: ModelTabProps) {
  const [inactivityMin, setInactivityMin] = useState(
    config.inference.keep_warm_inactivity_minutes,
  );
  const [ejecting, setEjecting] = useState(false);
  const [loadedModel, setLoadedModel] = useState<string | null>(null);

  // Context window: committed value drives the debounced save; local slider
  // pos updates live on drag without committing on every pixel.
  const [numCtx, setNumCtx] = useState(config.inference.num_ctx);
  const [ctxPos, setCtxPos] = useState(() =>
    ctxToPos(config.inference.num_ctx),
  );
  const [ctxChip, setCtxChip] = useState(String(config.inference.num_ctx));
  const ctxDraggingRef = useRef(false);

  useEffect(() => {
    let unlistenLoaded: (() => void) | null = null;
    let unlistenEvicted: (() => void) | null = null;

    async function setup() {
      unlistenLoaded = await listen<string>('warmup:model-loaded', (e) => {
        setLoadedModel(e.payload);
      });
      unlistenEvicted = await listen<null>('warmup:model-evicted', () => {
        setLoadedModel(null);
      });
      invoke<string | null>('get_loaded_model')
        .then(setLoadedModel)
        .catch(() => {});
    }

    setup();

    function handleVisibilityChange() {
      if (!document.hidden) {
        invoke<string | null>('get_loaded_model')
          .then(setLoadedModel)
          .catch(() => {});
      }
    }
    document.addEventListener('visibilitychange', handleVisibilityChange);

    return () => {
      unlistenLoaded?.();
      unlistenEvicted?.();
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, []);

  const { resetTo: resetMin } = useDebouncedSave(
    'inference',
    'keep_warm_inactivity_minutes',
    inactivityMin,
    { onSaved },
  );

  const { resetTo: resetNumCtx } = useDebouncedSave(
    'inference',
    'num_ctx',
    numCtx,
    { onSaved },
  );

  const prevTokenRef = useRef(resyncToken);

  if (prevTokenRef.current !== resyncToken) {
    prevTokenRef.current = resyncToken;
    setInactivityMin(config.inference.keep_warm_inactivity_minutes);
    resetMin(config.inference.keep_warm_inactivity_minutes);
    const nextCtx = config.inference.num_ctx;
    setNumCtx(nextCtx);
    setCtxPos(ctxToPos(nextCtx));
    setCtxChip(String(nextCtx));
    resetNumCtx(nextCtx);
  }

  function commitCtx(v: number) {
    setNumCtx(v);
    setCtxPos(ctxToPos(v));
    setCtxChip(String(v));
  }

  function handleEject() {
    setEjecting(true);
    invoke('evict_model')
      .then(() => {
        setTimeout(() => setEjecting(false), EJECT_RESET_MS);
      })
      .catch(() => setEjecting(false));
  }

  const ctxTurns = Math.round(numCtx / TOKENS_PER_TURN_ESTIMATE);
  const fillPct = `${ctxPos / 10}%`;

  return (
    <>
      <Section heading="Ollama">
        <SaveField
          section="inference"
          fieldKey="ollama_url"
          label="Ollama URL"
          helper={configHelp('inference', 'ollama_url')}
          initialValue={config.inference.ollama_url}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue, errored) => (
            <TextField
              value={value}
              onChange={setValue}
              placeholder="http://127.0.0.1:11434"
              errored={errored}
              ariaLabel="Ollama URL"
            />
          )}
        />
      </Section>

      <Section heading="Keep Warm">
        {/* Row 1: label + [?] on left | Release after [N] min on right */}
        <div className={styles.keepWarmRow1}>
          <div className={styles.keepWarmLabelLine}>
            <span className={styles.keepWarmLabel}>
              Keep active model in VRAM
            </span>
            <Tooltip label={KEEP_WARM_TOOLTIP} multiline>
              <button
                type="button"
                className={styles.infoBtn}
                aria-label="About Keep active model in VRAM"
              >
                ?
              </button>
            </Tooltip>
          </div>
          <div className={styles.keepWarmTimerGroup}>
            <span className={styles.keepWarmBarFieldLabel}>Release after</span>
            <input
              type="number"
              className={styles.keepWarmNumberInput}
              value={inactivityMin}
              min={-1}
              max={1440}
              aria-label="Release after N minutes"
              onChange={(e) => {
                const n = parseInt(e.target.value, 10);
                if (!Number.isNaN(n)) setInactivityMin(n);
              }}
            />
            <span className={styles.keepWarmUnit}>min</span>
          </div>
        </div>

        {/* Row 2: slug status on left | Unload now on right */}
        <div className={styles.keepWarmStatusRow}>
          <div className={styles.keepWarmStatusLeft}>
            {loadedModel !== null ? (
              <div className={styles.keepWarmVramSubtitle}>
                <span
                  className={styles.keepWarmVramDot}
                  data-testid="vram-status-dot"
                  aria-hidden="true"
                />
                <span className={styles.keepWarmVramModelName}>
                  {loadedModel}
                </span>
                <span>&nbsp;· in VRAM</span>
              </div>
            ) : (
              <span className={styles.keepWarmNoModel}>No model loaded</span>
            )}
          </div>

          <button
            type="button"
            className={styles.keepWarmEjectPill}
            aria-label="Unload now"
            disabled={ejecting || loadedModel === null}
            data-ejecting={ejecting}
            onClick={handleEject}
          >
            {ejecting ? (
              <svg
                viewBox="0 0 16 16"
                width="11"
                height="11"
                fill="none"
                aria-hidden="true"
              >
                <circle
                  cx="8"
                  cy="8"
                  r="7"
                  stroke="#5ec98a"
                  strokeWidth="1.6"
                  className={styles.keepWarmCircleAnim}
                  transform="rotate(-90 8 8)"
                />
                <path
                  d="M4.5 8.5L7 11L12 5.5"
                  stroke="#5ec98a"
                  strokeWidth="1.6"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  className={styles.keepWarmCheckAnim}
                />
              </svg>
            ) : (
              <svg
                viewBox="0 0 16 16"
                width="11"
                height="11"
                fill="currentColor"
                aria-hidden="true"
              >
                <polygon points="8,2 14,11 2,11" />
                <rect x="2" y="12.5" width="12" height="2" rx="1" />
              </svg>
            )}
            Unload now
          </button>
        </div>
      </Section>

      <Section heading="Context Window">
        <div className={styles.ctxBlock}>
          {/* Label row: "Context window" left + editable token chip right */}
          <div className={styles.ctxTopRow}>
            <span className={styles.ctxLabel}>Context window</span>
            <div className={styles.ctxChipGroup}>
              <input
                type="number"
                className={styles.ctxChipInput}
                value={ctxChip}
                min={CTX_MIN}
                max={CTX_MAX}
                aria-label="Context window tokens"
                onChange={(e) => setCtxChip(e.target.value)}
                onBlur={() => {
                  const n = parseInt(ctxChip, 10);
                  if (!Number.isNaN(n) && n >= CTX_MIN) {
                    // Clamp upper bound so the UI mirrors the backend
                    // BOUNDS_NUM_CTX cap and the slider stays in sync.
                    commitCtx(Math.min(n, CTX_MAX));
                  } else {
                    setCtxChip(String(numCtx));
                  }
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
                }}
              />
              <span className={styles.ctxChipUnit}>tokens</span>
            </div>
          </div>

          {/* Log-scale slider — fill percentage tracked via CSS custom property */}
          <input
            type="range"
            className={styles.ctxSlider}
            style={{ '--fill': fillPct } as React.CSSProperties}
            min={0}
            max={1000}
            step={1}
            value={ctxPos}
            aria-label="Context window tokens"
            aria-valuemin={CTX_MIN}
            aria-valuemax={CTX_MAX}
            aria-valuenow={numCtx}
            aria-valuetext={`${numCtx} tokens`}
            onChange={(e) => {
              ctxDraggingRef.current = true;
              const pos = Number(e.target.value);
              setCtxPos(pos);
              setCtxChip(String(posToCtx(pos)));
            }}
            onMouseUp={() => {
              ctxDraggingRef.current = false;
              commitCtx(posToCtx(ctxPos));
            }}
            onTouchEnd={() => {
              ctxDraggingRef.current = false;
              commitCtx(posToCtx(ctxPos));
            }}
            onKeyUp={() => {
              if (!ctxDraggingRef.current) commitCtx(posToCtx(ctxPos));
            }}
          />

          <div className={styles.ctxTickRow} aria-hidden="true">
            {CTX_TICKS.map((label, i) => (
              <span
                key={label}
                className={styles.ctxTick}
                style={{ left: `${(i / (CTX_TICKS.length - 1)) * 100}%` }}
              >
                {label}
              </span>
            ))}
          </div>

          <div className={styles.ctxHelper}>
            ~{ctxTurns.toLocaleString()} turns of context
            {' · '}
            Ollama clamps to model max so it's safe to push this up.
          </div>

          <div className={styles.ctxVramNote}>
            <span className={styles.ctxVramIcon} aria-hidden="true">
              ⚠
            </span>
            <span>
              Larger context windows allocate proportionally more VRAM for the
              KV cache. Doubling the context roughly doubles memory use.
              Benchmark with your hardware before pushing it high.
            </span>
          </div>
        </div>
      </Section>

      <Section heading="Prompt">
        <SaveField
          section="prompt"
          fieldKey="system"
          label="System prompt"
          helper={configHelp('prompt', 'system')}
          vertical
          initialValue={config.prompt.system}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <>
              <Textarea
                value={value}
                onChange={setValue}
                placeholder="Use built-in secretary persona…"
                maxLength={PROMPT_MAX_CHARS}
                ariaLabel="System prompt"
              />
              <div className={styles.charCounter}>
                {value.length} / {PROMPT_MAX_CHARS}
              </div>
            </>
          )}
        />
      </Section>
    </>
  );
}
