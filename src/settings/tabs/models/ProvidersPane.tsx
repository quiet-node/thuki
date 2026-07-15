/**
 * Providers pane (the "Active Hero" layout).
 *
 * Whichever provider is active occupies a prominent hero block at the top
 * (its name, a one-line description, an Active marker, and a Model row that
 * lets you pick the model that provider answers with). The remaining
 * providers are compact rows under "Other providers", each with a Switch.
 *
 * Below the provider list sits the shared "Generation" section: the context
 * window, keep-warm timer, and system prompt are GLOBAL settings that apply
 * to whichever provider is active, so they live in their own section rather
 * than inside any one provider card.
 *
 * Model downloads live in the Discover pane and per-model deletion lives in
 * Library; this pane only selects the active provider and its model.
 */

import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { ConfirmDialog, Textarea } from '../../components';
import { OpenAiProviderCard, AddOpenAiProvider } from '../ProviderCards';
import { useDebouncedSave } from '../../hooks/useDebouncedSave';
import { useModelSelection } from '../../../hooks/useModelSelection';
import { useModelCapabilities } from '../../../hooks/useModelCapabilities';
import { isNonLocalUrl } from '../../../utils/isNonLocalUrl';
import { formatContextWindow } from '../../../utils/contextWindow';
import { OLLAMA_DOWNLOAD_URL } from '../../../utils/capabilityConflicts';
import { configHelp } from '../../configHelpers';
import { Tooltip } from '../../../components/Tooltip';
import { InlineLink } from '../../../components/InlineLink';
import { ModelSelect, type ModelSelectItem } from './ModelSelect';
import styles from '../../../styles/settings.module.css';
import type { RawAppConfig, RawProvider } from '../../types';
import type { EngineStatus, InstalledModel } from '../../../types/starter';

interface ProvidersPaneProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
  /** Navigate to the Discover view (used by the no-model-installed hint). */
  onAddModel: () => void;
}

const PROMPT_MAX_CHARS = 32000;
const PROMPT_TEXTAREA_ROWS = 12;

const KEEP_WARM_TOOLTIP =
  'Keep Warm holds your active model resident in memory after each use, ' +
  'for both the built-in engine and Ollama. ' +
  'The timer sets how long before it auto-releases; use -1 to keep it indefinitely. ' +
  'Unload now releases it immediately. ' +
  'If set to 0, each provider uses its natural short default (about 5 minutes).';

// Context window slider: slider pos [0..1000] maps logarithmically to a token
// count between CTX_MIN and CTX_MAX. The milestones double each step (2K, 4K,
// ... 1M), so on the log track they land at equal intervals and the thumb
// always sits on the milestone it reads.
const CTX_MIN = 2048;
const CTX_MAX = 1_048_576;
const CTX_LOG_RATIO = Math.log(CTX_MAX / CTX_MIN);
const CTX_TICKS: { label: string; value: number }[] = [
  { label: '2K', value: 2048 },
  { label: '4K', value: 4096 },
  { label: '8K', value: 8192 },
  { label: '16K', value: 16384 },
  { label: '32K', value: 32768 },
  { label: '64K', value: 65536 },
  { label: '128K', value: 131072 },
  { label: '256K', value: 262144 },
  { label: '512K', value: 524288 },
  { label: '1M', value: 1048576 },
];

function ctxToPos(v: number): number {
  return Math.round((1000 * Math.log(v / CTX_MIN)) / CTX_LOG_RATIO);
}
function posToCtx(pos: number): number {
  return (
    Math.round((CTX_MIN * Math.pow(CTX_MAX / CTX_MIN, pos / 1000)) / 1024) *
    1024
  );
}

// Deep link to the 5-minute benchmark recipe, opened via the open_url command.
const CTX_TUNING_URL =
  'https://github.com/quiet-node/thuki/blob/main/docs/tuning-context-window.md#the-5-minute-benchmark-recipe';

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "8.2"). */
function gb(bytes: number): string {
  return (bytes / 1e9).toFixed(1);
}

/** One-line description shown under a provider's name. */
function providerSubtitle(p: RawProvider): string {
  if (p.kind === 'builtin') return "Thuki's bundled llama.cpp engine";
  if (p.kind === 'ollama') return p.base_url || 'Local or remote Ollama';
  return p.base_url || 'OpenAI-compatible server';
}

export function ProvidersPane({
  config,
  resyncToken,
  onSaved,
  onAddModel,
}: ProvidersPaneProps) {
  const providers = config.inference.providers;
  const activeId = config.inference.active_provider;
  const activeProvider = providers.find((p) => p.id === activeId);
  const activeKind = activeProvider?.kind ?? 'ollama';
  const builtinProvider = providers.find((p) => p.kind === 'builtin');
  const openaiProvider = providers.find((p) => p.kind === 'openai');

  // The OpenAI-compatible provider kind is gated behind a compile-time,
  // dev-only env flag, off by default and tree-shaken from shipped builds.
  const openaiProviderEnabled =
    import.meta.env.VITE_ENABLE_OPENAI_PROVIDER === 'true';

  // Installed models drive the built-in hero's model picker; refreshed when
  // the selected built-in model id changes (a switch lifts a new config).
  const [installed, setInstalled] = useState<InstalledModel[]>([]);
  const builtinModelId = builtinProvider?.model ?? '';
  useEffect(() => {
    void invoke<InstalledModel[]>('list_installed_models')
      .then((rows) => setInstalled(Array.isArray(rows) ? rows : []))
      .catch(() => setInstalled([]));
  }, [builtinModelId]);

  // Engine lifecycle + the active provider's resident model, for the keep-warm
  // status line.
  const [engineState, setEngineState] =
    useState<EngineStatus['state']>('stopped');
  const [loadedModel, setLoadedModel] = useState<string | null>(null);
  // True while the built-in engine is priming: the model is resident
  // (`/health` OK) but the system-prompt prefill has not finished, so it still
  // answers as slowly as a cold start. The status reads "warming…" until the
  // prime completes, then flips to "in memory".
  const [warming, setWarming] = useState(false);
  useEffect(() => {
    // Re-reads which model the active provider actually has resident. The
    // built-in engine names it from its loaded blob, so this must be re-run on
    // every engine transition rather than derived from the frontend selection.
    const refreshLoaded = () =>
      void invoke<string | null>('get_loaded_model')
        .then(setLoadedModel)
        .catch(() => {});
    invoke<EngineStatus>('get_engine_status')
      .then((s) => setEngineState(s.state))
      .catch(() => {});
    refreshLoaded();
    // Seed the warming flag in case the panel mounts mid-prime, before the
    // warming event below has a chance to fire.
    void invoke<boolean>('get_builtin_warm_state')
      .then(setWarming)
      .catch(() => {});
    const unlistenStatus = listen<EngineStatus>('engine:status', (e) => {
      setEngineState(e.payload.state);
      refreshLoaded();
    });
    const unlistenLoaded = listen<string>('warmup:model-loaded', (e) =>
      setLoadedModel(e.payload),
    );
    const unlistenEvicted = listen<null>('warmup:model-evicted', () => {
      setLoadedModel(null);
      setWarming(false);
    });
    const unlistenWarming = listen('warmup:builtin-warming', () =>
      setWarming(true),
    );
    const unlistenWarmed = listen('warmup:builtin-warmed', () =>
      setWarming(false),
    );
    return () => {
      void unlistenStatus.then((fn) => fn());
      void unlistenLoaded.then((fn) => fn());
      void unlistenEvicted.then((fn) => fn());
      void unlistenWarming.then((fn) => fn());
      void unlistenWarmed.then((fn) => fn());
    };
  }, []);

  // Keep-warm minutes (debounced save).
  const [inactivityMin, setInactivityMin] = useState(
    config.inference.keep_warm_inactivity_minutes,
  );
  const [rawMin, setRawMin] = useState(
    String(config.inference.keep_warm_inactivity_minutes),
  );
  const minFocusedRef = useRef(false);
  const { resetTo: resetMin } = useDebouncedSave(
    'inference',
    'keep_warm_inactivity_minutes',
    inactivityMin,
    { onSaved },
  );

  // Context window (debounced save); local slider pos updates live on drag.
  const [numCtx, setNumCtx] = useState(config.inference.num_ctx);
  const [ctxPos, setCtxPos] = useState(() =>
    ctxToPos(config.inference.num_ctx),
  );
  const [ctxChip, setCtxChip] = useState(String(config.inference.num_ctx));
  const ctxDraggingRef = useRef(false);
  const ctxInputFocusedRef = useRef(false);
  const { resetTo: resetNumCtx } = useDebouncedSave(
    'inference',
    'num_ctx',
    numCtx,
    { onSaved },
  );

  // Ollama URL (committed on blur / Enter via the dedicated command).
  const ollamaBaseUrl =
    providers.find((p) => p.kind === 'ollama')?.base_url ?? '';
  const [ollamaUrl, setOllamaUrl] = useState(ollamaBaseUrl);
  const ollamaUrlFocusedRef = useRef(false);

  // System prompt (debounced save); the editor mounts inline under the
  // Generation list with a single header, so it does not use SaveField's row.
  const [promptValue, setPromptValue] = useState(config.prompt.system);
  const { resetTo: resetPrompt } = useDebouncedSave(
    'prompt',
    'system',
    promptValue,
    { onSaved },
  );

  const [promptOpen, setPromptOpen] = useState(false);
  // A provider switch is confirmed before it takes effect.
  const [pendingSwitch, setPendingSwitch] = useState<RawProvider | null>(null);

  const {
    activeModel,
    availableModels,
    setActiveModel,
    refreshModels,
    ollamaReachable,
  } = useModelSelection();

  // Per-model capabilities (vision/thinking) drive the built-in picker's pills.
  const { capabilities } = useModelCapabilities();

  // The picker hook fetches once on mount; re-fetch whenever the active
  // provider changes so the hero's Model dropdown reflects the newly-active
  // provider's inventory instead of the previous provider's cached list.
  // Without this, switching Built-in -> Ollama would keep showing the built-in
  // model id (the stale `availableModels`/`activeModel` from before the switch).
  const lastProviderRef = useRef(activeId);
  useEffect(() => {
    if (lastProviderRef.current === activeId) return;
    lastProviderRef.current = activeId;
    void refreshModels();
  }, [activeId, refreshModels]);

  // Re-seed local editable state from a resync without scheduling saves.
  const prevTokenRef = useRef(resyncToken);
  if (prevTokenRef.current !== resyncToken) {
    prevTokenRef.current = resyncToken;
    if (!minFocusedRef.current) {
      setInactivityMin(config.inference.keep_warm_inactivity_minutes);
      setRawMin(String(config.inference.keep_warm_inactivity_minutes));
      resetMin(config.inference.keep_warm_inactivity_minutes);
    }
    if (!ctxInputFocusedRef.current) {
      const nextCtx = config.inference.num_ctx;
      setNumCtx(nextCtx);
      setCtxPos(ctxToPos(nextCtx));
      setCtxChip(String(nextCtx));
      resetNumCtx(nextCtx);
    }
    setPromptValue(config.prompt.system);
    resetPrompt(config.prompt.system);
    if (!ollamaUrlFocusedRef.current) setOllamaUrl(ollamaBaseUrl);
  }

  function commitCtx(v: number) {
    setNumCtx(v);
    setCtxPos(ctxToPos(v));
    setCtxChip(String(v));
  }

  // The token field accepts a typed value: commit it clamped to the valid
  // range on blur/Enter, or revert to the current value when it is not a number.
  function commitCtxInput() {
    ctxInputFocusedRef.current = false;
    const n = parseInt(ctxChip, 10);
    if (Number.isNaN(n)) {
      setCtxChip(String(numCtx));
    } else {
      commitCtx(Math.max(CTX_MIN, Math.min(CTX_MAX, n)));
    }
  }

  function commitOllamaUrl() {
    const next = ollamaUrl.trim();
    if (next === ollamaBaseUrl) return;
    void invoke<RawAppConfig>('set_ollama_url', { baseUrl: next })
      .then((cfg) => onSaved(cfg))
      .catch(() => setOllamaUrl(ollamaBaseUrl));
  }

  function selectProvider(id: string) {
    void invoke<RawAppConfig>('set_active_provider', { providerId: id })
      .then((cfg) => onSaved(cfg))
      .catch(() => {});
  }

  function commitBuiltinModel(id: string) {
    void invoke<RawAppConfig>('update_provider_field', {
      providerId: 'builtin',
      field: 'model',
      value: id,
    })
      .then(onSaved)
      .catch(() => {});
  }

  // Ollama selection persists onto the active provider's model field via
  // set_active_model; lift the fresh config so the Running footer (and the
  // hero) re-render with the newly-selected model instead of the old name.
  function commitOllamaModel(model: string) {
    void setActiveModel(model)
      .then(async () => onSaved(await invoke<RawAppConfig>('get_config')))
      .catch(() => {
        // The focus-driven resync picks the change up on next activation.
      });
  }

  function handleEngineEject() {
    void invoke('evict_model').catch(() => {});
  }

  const fillPct = `${ctxPos / 10}%`;

  // Keep-warm live status. `loadedModel` is the display name of the model the
  // active provider actually has resident (the built-in engine's loaded blob,
  // or Ollama's /api/ps), never the frontend selection; when set it renders as
  // a truncating name + "in memory" suffix in the JSX below so a long name can
  // never break the row. This fallback text covers the non-resident states
  // (priming or mid-load for the built-in engine, otherwise nothing loaded).
  //
  // The built-in engine reports `loaded` (`/health` OK) before the system
  // prompt is prefilled, so `builtinWarming` distinguishes "resident but still
  // priming" (slow first message) from "ready". Scoped to the built-in engine
  // because only it emits the warming events.
  const builtinWarming = activeKind === 'builtin' && warming;
  let warmStatusText: string;
  if (builtinWarming) {
    warmStatusText = 'Warming up…';
  } else if (activeKind === 'builtin' && engineState === 'starting') {
    warmStatusText = 'Loading…';
  } else {
    warmStatusText = 'No model loaded';
  }

  // The active Ollama model value, constrained to the installed list.
  const ollamaModelValue =
    activeModel && availableModels.includes(activeModel)
      ? activeModel
      : (availableModels[0] ?? '');
  const builtinModelValue = installed.some((m) => m.id === builtinModelId)
    ? builtinModelId
    : '';
  // Display names match the picker / Library / running footer (friendly name,
  // no quant). The quant is appended only to disambiguate two installs that
  // share a display name, so the common case reads consistently everywhere.
  const duplicateDisplayNames = new Set(
    installed
      .map((m) => m.display_name)
      .filter((name, i, all) => all.indexOf(name) !== i),
  );

  // Built-in picker rows: name (quant-disambiguated only when a display name
  // repeats), capability pills, a `size · context · maker · quant` sub-line, and
  // a RAM-fit badge. Mirrors the Library pane's grammar so the two surfaces read
  // the same.
  const builtinItems: ModelSelectItem[] = installed.map((m) => {
    const caps = capabilities[m.id];
    const maker = m.origin || m.id.split(':')[0];
    const totalBytes = m.size_bytes + (m.mmproj_bytes ?? 0);
    const sub = [
      `${gb(totalBytes)} GB`,
      formatContextWindow(m.context_length ?? 0),
      maker,
      m.quant,
    ]
      .filter((part) => part !== '')
      .join(' · ');
    const quantSuffix =
      duplicateDisplayNames.has(m.display_name) && m.quant !== ''
        ? ` · ${m.quant}`
        : '';
    return {
      id: m.id,
      label: `${m.display_name}${quantSuffix}`,
      sub,
      vision: !!caps?.vision,
      thinking: !!caps?.thinking,
      fit: m.fit ?? null,
    };
  });

  // Ollama exposes no capability metadata, so its rows fall back to the slug.
  const ollamaItems: ModelSelectItem[] = availableModels.map((m) => ({
    id: m,
    label: m,
  }));

  // Providers other than the active one, in a stable order.
  const otherProviders = providers.filter((p) => p.id !== activeId);

  return (
    <>
      <div className={styles.shead}>Active provider</div>
      <div className={styles.hero}>
        <div className={styles.heroHead}>
          <div>
            <div className={styles.heroName}>
              {activeProvider?.label ?? 'Ollama'}
            </div>
            <div className={styles.heroSub}>
              {activeProvider
                ? providerSubtitle(activeProvider)
                : 'Local or remote Ollama'}
            </div>
          </div>
        </div>

        {activeKind === 'builtin' ? (
          <div className={styles.heroModel}>
            <span className={styles.heroModelLabel}>Model</span>
            {installed.length > 0 ? (
              <ModelSelect
                value={builtinModelValue}
                items={builtinItems}
                onChange={commitBuiltinModel}
                ariaLabel="Built-in model"
                placeholder="Choose a model"
              />
            ) : (
              <button
                type="button"
                className={styles.heroModelLink}
                onClick={onAddModel}
              >
                Download a model in Discover ›
              </button>
            )}
          </div>
        ) : null}

        {activeKind === 'ollama' ? (
          <>
            <div className={styles.heroModel}>
              <span className={styles.heroModelLabel}>Endpoint</span>
              <input
                type="text"
                className={styles.input}
                value={ollamaUrl}
                aria-label="Ollama URL"
                spellCheck={false}
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="off"
                placeholder="http://127.0.0.1:11434"
                onFocus={() => {
                  ollamaUrlFocusedRef.current = true;
                }}
                onChange={(e) => setOllamaUrl(e.target.value)}
                onBlur={() => {
                  ollamaUrlFocusedRef.current = false;
                  commitOllamaUrl();
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
                }}
              />
            </div>
            {isNonLocalUrl(ollamaUrl) ? (
              <p className={styles.providerWarning} role="alert">
                This points Thuki at a non-local Ollama server. You are
                responsible for securing it: prefer a VPN/Tailscale or SSH
                tunnel over exposing the port directly.
              </p>
            ) : null}
            <div className={styles.heroModel}>
              <span className={styles.heroModelLabel}>Model</span>
              {availableModels.length > 0 ? (
                <ModelSelect
                  value={ollamaModelValue}
                  items={ollamaItems}
                  onChange={commitOllamaModel}
                  ariaLabel="Active Ollama model"
                />
              ) : ollamaReachable ? (
                <span className={styles.providerHint}>No models installed</span>
              ) : (
                <span className={styles.providerHint}>
                  Ollama isn&apos;t reachable.{' '}
                  <InlineLink url={OLLAMA_DOWNLOAD_URL} ariaLabel="Get Ollama">
                    Get Ollama ↗
                  </InlineLink>
                  {/* Plain text, not a link: the built-in provider's switch is
                      already on this screen (Other providers below), so the
                      user can flip back without leaving Settings. */}
                  {builtinProvider ? ' or switch to Built-in.' : null}
                </span>
              )}
            </div>
          </>
        ) : null}

        {activeProvider?.kind === 'openai' && openaiProviderEnabled ? (
          <OpenAiProviderCard
            provider={activeProvider}
            resyncToken={resyncToken}
            onSaved={onSaved}
          />
        ) : null}
      </div>

      <div className={styles.shead}>Other providers</div>
      <div className={styles.listcard}>
        {otherProviders.map((p) =>
          p.kind === 'openai' && !openaiProviderEnabled ? null : (
            <div className={styles.providerRow} key={p.id}>
              <span className={styles.providerRowName}>{p.label}</span>
              <span className={styles.providerRowSub}>
                {providerSubtitle(p)}
              </span>
              <span className={styles.grow} />
              <button
                type="button"
                className={styles.switchBtn}
                onClick={() => setPendingSwitch(p)}
              >
                Switch
              </button>
            </div>
          ),
        )}
        {openaiProviderEnabled && !openaiProvider ? (
          <div className={styles.providerRow}>
            <AddOpenAiProvider onSaved={onSaved} />
          </div>
        ) : null}
      </div>

      <div className={styles.shead}>
        Generation
        <span className={styles.sheadNote}>
          {' '}
          · applies to whichever provider is active
        </span>
      </div>
      <div className={styles.listcard}>
        {/* Context window: the header carries the label, an info tooltip, a
            deep link to the tuning guide, and an editable token field; the
            slider spans the full card width below. */}
        <div className={`${styles.genRow} ${styles.genRowCtx}`}>
          <div className={styles.genCtxHead}>
            <div className={styles.genName}>
              Context window
              <Tooltip label={configHelp('inference', 'num_ctx')} multiline>
                <button
                  type="button"
                  className={styles.infoBtn}
                  aria-label="About Context window"
                >
                  ?
                </button>
              </Tooltip>
              <Tooltip label="Learn how to tune Context Window ↗">
                <button
                  type="button"
                  className={`${styles.infoBtn} ${styles.genCtxLearnBtn}`}
                  aria-label="Learn how to tune Context Window"
                  onClick={() =>
                    void invoke('open_url', { url: CTX_TUNING_URL })
                  }
                >
                  <svg
                    viewBox="0 0 16 16"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.5"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    aria-hidden="true"
                  >
                    <path d="M9 3.5h3.5V7" />
                    <path d="M12.5 3.5 7.5 8.5" />
                    <path d="M11 9.5V12a.5.5 0 0 1-.5.5H4a.5.5 0 0 1-.5-.5V5.5A.5.5 0 0 1 4 5h2.5" />
                  </svg>
                </button>
              </Tooltip>
            </div>
            <span className={styles.genCtxValue}>
              <input
                type="number"
                className={styles.genCtxInput}
                value={ctxChip}
                min={CTX_MIN}
                max={CTX_MAX}
                aria-label="Context window size in tokens"
                onFocus={() => {
                  ctxInputFocusedRef.current = true;
                }}
                onChange={(e) => setCtxChip(e.target.value)}
                onBlur={commitCtxInput}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
                }}
              />
              <span className={styles.genCtxValueUnit}>tokens</span>
            </span>
          </div>
          <div>
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
              {CTX_TICKS.map(({ label, value }) => (
                <span
                  key={label}
                  className={styles.ctxTick}
                  style={{ left: `${ctxToPos(value) / 10}%` }}
                >
                  {label}
                </span>
              ))}
            </div>
          </div>
        </div>

        {/* Keep Warm: status rides the header line next to the name; the
            release timer and Unload sit on their own row beneath it. */}
        <div className={`${styles.genRow} ${styles.genRowWarm}`}>
          <div className={styles.genWarmHead}>
            <div className={styles.genName}>
              Keep Warm
              <Tooltip label={KEEP_WARM_TOOLTIP} multiline>
                <button
                  type="button"
                  className={styles.infoBtn}
                  aria-label="About Keep Warm"
                >
                  ?
                </button>
              </Tooltip>
            </div>
            <span
              className={styles.genWarmStatus}
              data-testid="keep-warm-status"
            >
              {loadedModel ? (
                <>
                  <span className={styles.genWarmModel} title={loadedModel}>
                    {loadedModel}
                  </span>
                  <span className={styles.genWarmSuffix}>
                    {builtinWarming ? 'warming…' : 'in memory'}
                  </span>
                </>
              ) : (
                warmStatusText
              )}
            </span>
          </div>
          <div className={styles.genWarmControls}>
            <span className={styles.genWarmPrefix}>Release after</span>
            <input
              type="number"
              className={styles.keepWarmNumberInput}
              value={rawMin}
              min={-1}
              max={1440}
              aria-label="Release after N minutes"
              onFocus={() => {
                minFocusedRef.current = true;
              }}
              onChange={(e) => {
                const n = parseInt(e.target.value, 10);
                if (Number.isNaN(n)) {
                  setRawMin(e.target.value);
                } else {
                  const clamped = Math.max(-1, Math.min(1440, n));
                  setRawMin(String(clamped));
                  setInactivityMin(clamped);
                }
              }}
              onBlur={() => {
                minFocusedRef.current = false;
                if (Number.isNaN(parseInt(rawMin, 10))) {
                  setRawMin('0');
                  setInactivityMin(0);
                }
              }}
            />
            <span className={styles.keepWarmUnit}>min</span>
            <button
              type="button"
              className={`${styles.switchBtn} ${styles.genWarmUnload}`}
              aria-label="Unload now"
              disabled={activeKind === 'builtin' && engineState !== 'loaded'}
              onClick={handleEngineEject}
            >
              Unload
            </button>
          </div>
        </div>

        {/* System prompt: one header (with the ? help), Edit/Done toggles the
            inline editor below it. */}
        <div className={styles.genRow}>
          <div className={styles.genLabel}>
            <div className={styles.genName}>
              System prompt
              <Tooltip
                label={configHelp('prompt', 'system')}
                multiline
                placement="top"
              >
                <button
                  type="button"
                  className={styles.infoBtn}
                  aria-label="About System prompt"
                >
                  ?
                </button>
              </Tooltip>
            </div>
          </div>
          <button
            type="button"
            className={styles.heroModelLink}
            aria-expanded={promptOpen}
            onClick={() => setPromptOpen((o) => !o)}
          >
            {promptOpen ? 'Done' : 'Edit ›'}
          </button>
        </div>
        {promptOpen ? (
          <div className={styles.genPromptEditor}>
            <Textarea
              value={promptValue}
              onChange={setPromptValue}
              placeholder="Persona prompt…"
              maxLength={PROMPT_MAX_CHARS}
              ariaLabel="System prompt"
              rows={PROMPT_TEXTAREA_ROWS}
            />
            <div className={styles.charCounter}>
              {promptValue.length} / {PROMPT_MAX_CHARS}
            </div>
          </div>
        ) : null}
      </div>

      {pendingSwitch ? (
        <ConfirmDialog
          open
          primary
          title={`Switch to ${pendingSwitch.label}?`}
          message={`New chats will be answered by ${pendingSwitch.label}. The model currently held in memory is released to free up RAM.`}
          confirmLabel={`Switch to ${pendingSwitch.label}`}
          onConfirm={() => {
            selectProvider(pendingSwitch.id);
            setPendingSwitch(null);
          }}
          onCancel={() => setPendingSwitch(null)}
        />
      ) : null}
    </>
  );
}
