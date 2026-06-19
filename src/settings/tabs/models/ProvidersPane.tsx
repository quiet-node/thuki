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

import { ConfirmDialog, Textarea, Toggle } from '../../components';
import { SaveField } from '../../components/SaveField';
import { OpenAiProviderCard, AddOpenAiProvider } from '../ProviderCards';
import { useDebouncedSave } from '../../hooks/useDebouncedSave';
import { useModelSelection } from '../../../hooks/useModelSelection';
import { isNonLocalUrl } from '../../../utils/isNonLocalUrl';
import { configHelp } from '../../configHelpers';
import { Tooltip } from '../../../components/Tooltip';
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
const TOKENS_PER_TURN_ESTIMATE = 400;

const KEEP_WARM_TOOLTIP =
  'Keep Warm holds your active model resident in memory after each use, ' +
  'for both the built-in engine and Ollama. ' +
  'The timer sets how long before it auto-releases; use -1 to keep it indefinitely. ' +
  'Unload now releases it immediately. ' +
  'If set to 0, each provider uses its natural short default (about 5 minutes).';

// Log-scale context window slider: slider pos [0..1000] maps to a token count.
const CTX_MIN = 2048;
const CTX_MAX = 1_048_576;
const CTX_LOG_RATIO = Math.log(CTX_MAX / CTX_MIN);

function ctxToPos(v: number): number {
  return Math.round((1000 * Math.log(v / CTX_MIN)) / CTX_LOG_RATIO);
}
function posToCtx(pos: number): number {
  return (
    Math.round((CTX_MIN * Math.pow(CTX_MAX / CTX_MIN, pos / 1000)) / 1024) *
    1024
  );
}
const CTX_TICKS = ['2K', '8K', '32K', '128K', '512K', '1M'];

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

  // Engine lifecycle + Ollama VRAM residency for the keep-warm status line.
  const [engineState, setEngineState] =
    useState<EngineStatus['state']>('stopped');
  const [loadedModel, setLoadedModel] = useState<string | null>(null);
  useEffect(() => {
    invoke<EngineStatus>('get_engine_status')
      .then((s) => setEngineState(s.state))
      .catch(() => {});
    invoke<string | null>('get_loaded_model')
      .then(setLoadedModel)
      .catch(() => {});
    const unlistenStatus = listen<EngineStatus>('engine:status', (e) =>
      setEngineState(e.payload.state),
    );
    const unlistenLoaded = listen<string>('warmup:model-loaded', (e) =>
      setLoadedModel(e.payload),
    );
    const unlistenEvicted = listen<null>('warmup:model-evicted', () =>
      setLoadedModel(null),
    );
    return () => {
      void unlistenStatus.then((fn) => fn());
      void unlistenLoaded.then((fn) => fn());
      void unlistenEvicted.then((fn) => fn());
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
  const [devOpen, setDevOpen] = useState(false);
  // A provider switch is confirmed before it takes effect.
  const [pendingSwitch, setPendingSwitch] = useState<RawProvider | null>(null);

  const { activeModel, availableModels, setActiveModel, refreshModels } =
    useModelSelection();

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
    const nextCtx = config.inference.num_ctx;
    setNumCtx(nextCtx);
    setCtxPos(ctxToPos(nextCtx));
    setCtxChip(String(nextCtx));
    resetNumCtx(nextCtx);
    setPromptValue(config.prompt.system);
    resetPrompt(config.prompt.system);
    if (!ollamaUrlFocusedRef.current) setOllamaUrl(ollamaBaseUrl);
  }

  function commitCtx(v: number) {
    setNumCtx(v);
    setCtxPos(ctxToPos(v));
    setCtxChip(String(v));
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

  function handleEngineEject() {
    void invoke('evict_model').catch(() => {});
  }

  const ctxTurns = Math.round(numCtx / TOKENS_PER_TURN_ESTIMATE);
  const fillPct = `${ctxPos / 10}%`;

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
          <span className={styles.heroActive}>
            <span className={styles.heroLiveDot} aria-hidden />
            Active
          </span>
        </div>

        {activeKind === 'builtin' ? (
          <div className={styles.heroModel}>
            <span className={styles.heroModelLabel}>Model</span>
            {installed.length > 0 ? (
              <select
                className={styles.dropdown}
                aria-label="Built-in model"
                value={builtinModelValue}
                onChange={(e) => commitBuiltinModel(e.target.value)}
              >
                {builtinModelValue === '' ? (
                  <option value="" disabled>
                    Choose a model
                  </option>
                ) : null}
                {installed.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.display_name}
                    {duplicateDisplayNames.has(m.display_name) && m.quant !== ''
                      ? ` · ${m.quant}`
                      : ''}
                  </option>
                ))}
              </select>
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
                <select
                  className={styles.dropdown}
                  aria-label="Active Ollama model"
                  value={ollamaModelValue}
                  onChange={(e) => void setActiveModel(e.target.value)}
                >
                  {availableModels.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
              ) : (
                <span className={styles.providerHint}>No models installed</span>
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
        {/* Context window */}
        <div className={styles.genRow}>
          <div className={styles.genLabel}>
            <div className={styles.genName}>Context window</div>
            <div className={styles.genHelp}>
              How much conversation the model remembers
            </div>
          </div>
          <div className={styles.genCtxControl}>
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
            <div className={styles.genCtxValue}>
              {Number(ctxChip).toLocaleString()} tokens ·{' '}
              {ctxTurns.toLocaleString()} turns
            </div>
          </div>
        </div>

        {/* Keep model warm */}
        <div className={styles.genRow}>
          <div className={styles.genLabel}>
            <div className={styles.genName}>
              Keep model warm
              <Tooltip label={KEEP_WARM_TOOLTIP} multiline>
                <button
                  type="button"
                  className={styles.infoBtn}
                  aria-label="About Keep model warm"
                >
                  ?
                </button>
              </Tooltip>
            </div>
            <div className={styles.genHelp}>
              {activeKind === 'builtin'
                ? `Engine: ${engineState}`
                : loadedModel !== null
                  ? `${loadedModel} in VRAM`
                  : 'No model loaded'}
            </div>
          </div>
          <div className={styles.genWarmControl}>
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
              className={styles.switchBtn}
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
            <div className={styles.genHelp}>
              Persona sent at the start of every chat
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

      {/* A small installed-count footer mirrors the other panes. The active
          model's identity already lives in the hero and the Running footer, so
          this stays a neutral count rather than restating it. */}
      <div className={styles.genFootnote}>
        {installed.length} installed{' '}
        {installed.length === 1 ? 'model' : 'models'}
      </div>

      <div className={styles.devSection}>
        <button
          type="button"
          className={styles.devTrigger}
          aria-expanded={devOpen}
          aria-controls="dev-diagnostics"
          onClick={() => setDevOpen((o) => !o)}
        >
          <span className={styles.devTriggerLabel}>Diagnostics</span>
          <span className={styles.devTag}>DEV</span>
          <svg
            className={`${styles.devChevron} ${devOpen ? styles.devChevronOpen : ''}`}
            viewBox="0 0 10 10"
            fill="currentColor"
            aria-hidden
          >
            <path
              d="M3 2l4 3-4 3"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
              fill="none"
            />
          </svg>
        </button>
        {devOpen && (
          <div id="dev-diagnostics">
            <SaveField
              section="debug"
              fieldKey="trace_enabled"
              label="Trace recording"
              helper={configHelp('debug', 'trace_enabled')}
              initialValue={config.debug.trace_enabled}
              resyncToken={resyncToken}
              onSaved={onSaved}
              tooltipPlacement="top"
              rightAlign
              render={(value, setValue) => (
                <Toggle
                  checked={value}
                  onChange={setValue}
                  ariaLabel="Enable trace recording"
                />
              )}
            />
          </div>
        )}
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
