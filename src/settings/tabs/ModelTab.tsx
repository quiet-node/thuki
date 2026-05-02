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

const KEEP_WARM_TOOLTIP =
  'Keep Warm holds your active model loaded in VRAM after each use. ' +
  'The timer below sets how long before it auto-releases; use -1 to keep it indefinitely. ' +
  'Unload now releases it immediately. ' +
  'If set to 0, Ollama unloads models after its default 5-minute timeout.';

export function ModelTab({ config, resyncToken, onSaved }: ModelTabProps) {
  const [inactivityMin, setInactivityMin] = useState(
    config.inference.keep_warm_inactivity_minutes,
  );
  const [ejecting, setEjecting] = useState(false);
  const [loadedModel, setLoadedModel] = useState<string | null>(null);

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

  const prevTokenRef = useRef(resyncToken);

  if (prevTokenRef.current !== resyncToken) {
    prevTokenRef.current = resyncToken;
    setInactivityMin(config.inference.keep_warm_inactivity_minutes);
    resetMin(config.inference.keep_warm_inactivity_minutes);
  }

  function handleEject() {
    setEjecting(true);
    invoke('evict_model')
      .then(() => {
        setTimeout(() => setEjecting(false), EJECT_RESET_MS);
      })
      .catch(() => setEjecting(false));
  }

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
