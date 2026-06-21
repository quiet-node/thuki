/**
 * Provider card bodies for the Providers pane's OpenAI-compatible provider.
 *
 * - `OpenAiProviderCard`: editable label/base URL/model for the single
 *   OpenAI-compatible provider, write-only API key (Keychain), manual vision
 *   toggle, and removal with confirm.
 * - `AddOpenAiProvider`: the inline "add a server" affordance shown while no
 *   OpenAI-compatible provider exists.
 *
 * The cards lift every config write back through `onSaved` so the parent's
 * `RawAppConfig` snapshot stays in lock-step with disk.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { SettingRow, Toggle } from '../components';
import { configHelp } from '../configHelpers';
import { describeConfigError } from '../types';
import { isNonLocalUrl } from '../../utils/isNonLocalUrl';
import styles from '../../styles/settings.module.css';
import type { RawAppConfig, RawProvider } from '../types';

/** Shared remote-URL caution, same mechanism as the Ollama URL warning. */
function NonLocalWarning() {
  return (
    <p className={styles.providerWarning} role="alert">
      This points Thuki at a non-local server. You are responsible for securing
      it: prefer a VPN/Tailscale or SSH tunnel over exposing the port directly.
    </p>
  );
}

// ─── OpenAI-compatible card body ─────────────────────────────────────────────

interface OpenAiProviderCardProps {
  provider: RawProvider;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

export function OpenAiProviderCard({
  provider,
  resyncToken,
  onSaved,
}: OpenAiProviderCardProps) {
  const [label, setLabel] = useState(provider.label);
  const labelFocusedRef = useRef(false);
  const [baseUrl, setBaseUrl] = useState(provider.base_url);
  const baseUrlFocusedRef = useRef(false);
  const [fieldError, setFieldError] = useState<string | null>(null);

  const [models, setModels] = useState<string[] | null>(null);
  const [modelsError, setModelsError] = useState<string | null>(null);

  const [apiKey, setApiKey] = useState('');
  const [hasKey, setHasKey] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [confirmingRemove, setConfirmingRemove] = useState(false);

  const prevTokenRef = useRef(resyncToken);
  if (prevTokenRef.current !== resyncToken) {
    prevTokenRef.current = resyncToken;
    if (!labelFocusedRef.current) setLabel(provider.label);
    if (!baseUrlFocusedRef.current) setBaseUrl(provider.base_url);
  }

  // Monotonic token guarding against out-of-order refreshes: a base URL or
  // key change can leave an earlier `list_openai_models` call in flight, so a
  // slow earlier response must not overwrite a newer one's result.
  const refreshSeqRef = useRef(0);
  const refreshModels = useCallback(async () => {
    const seq = ++refreshSeqRef.current;
    setModelsError(null);
    try {
      const rows = await invoke<string[]>('list_openai_models');
      if (seq !== refreshSeqRef.current) return;
      setModels(Array.isArray(rows) ? rows : []);
    } catch (err) {
      if (seq !== refreshSeqRef.current) return;
      setModels(null);
      setModelsError(String(err));
    }
  }, []);

  // `provider.base_url` in the deps re-lists after a successful base URL
  // commit (the parent lifts the new config, which changes the prop), so the
  // dropdown never keeps offering the old server's models. A failed commit
  // reverts locally without touching the prop, so it never refetches.
  useEffect(() => {
    void refreshModels();
  }, [refreshModels, provider.base_url]);

  useEffect(() => {
    void invoke<boolean>('has_provider_api_key', { providerId: provider.id })
      .then((v) => setHasKey(v === true))
      .catch(() => {
        // Unknown key state just hides the chip.
      });
  }, [provider.id]);

  function commitField(
    field: 'label' | 'base_url' | 'model' | 'vision',
    value: string,
    revert: () => void,
    onSuccess?: (cfg: RawAppConfig) => void,
  ) {
    void invoke<RawAppConfig>('update_provider_field', {
      providerId: provider.id,
      field,
      value,
    })
      .then((cfg) => {
        setFieldError(null);
        onSaved(cfg);
        onSuccess?.(cfg);
      })
      .catch((err) => {
        setFieldError(describeConfigError(err));
        revert();
      });
  }

  function commitLabel() {
    const next = label.trim();
    if (next === provider.label) return;
    // The backend heals an empty label to its compiled default; resync the
    // unfocused input to whatever actually persisted.
    commitField(
      'label',
      next,
      () => setLabel(provider.label),
      (cfg) => {
        if (labelFocusedRef.current) return;
        const saved = cfg.inference.providers.find((p) => p.id === provider.id);
        setLabel(saved ? saved.label : next);
      },
    );
  }

  function commitBaseUrl() {
    const next = baseUrl.trim();
    if (next === provider.base_url) return;
    commitField('base_url', next, () => setBaseUrl(provider.base_url));
  }

  function saveKey() {
    void invoke('set_provider_api_key', {
      providerId: provider.id,
      key: apiKey,
    })
      .then(() => {
        setApiKey('');
        setHasKey(true);
        setKeyError(null);
        // The key affects what the server lists; refresh with auth applied.
        void refreshModels();
      })
      .catch((err) => setKeyError(String(err)));
  }

  function clearKey() {
    void invoke('clear_provider_api_key', { providerId: provider.id })
      .then(() => {
        setHasKey(false);
        setKeyError(null);
        void refreshModels();
      })
      .catch((err) => setKeyError(String(err)));
  }

  function removeProvider() {
    void invoke<RawAppConfig>('remove_openai_provider')
      .then(onSaved)
      .catch(() => setConfirmingRemove(false));
  }

  // The persisted model may no longer be listed by the server; keep it
  // selectable so the dropdown reflects what chat actually uses.
  const modelOptions =
    models !== null && provider.model !== '' && !models.includes(provider.model)
      ? [provider.model, ...models]
      : (models ?? []);

  return (
    <>
      <SettingRow label="Label">
        <input
          type="text"
          className={styles.input}
          aria-label="Provider label"
          value={label}
          onFocus={() => {
            labelFocusedRef.current = true;
          }}
          onChange={(e) => setLabel(e.target.value)}
          onBlur={() => {
            labelFocusedRef.current = false;
            commitLabel();
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
          }}
        />
      </SettingRow>

      <SettingRow
        label="Base URL"
        helper={configHelp('inference', 'openai_base_url')}
      >
        <input
          type="text"
          className={styles.input}
          aria-label="OpenAI-compatible base URL"
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          placeholder="http://127.0.0.1:1234"
          value={baseUrl}
          onFocus={() => {
            baseUrlFocusedRef.current = true;
          }}
          onChange={(e) => setBaseUrl(e.target.value)}
          onBlur={() => {
            baseUrlFocusedRef.current = false;
            commitBaseUrl();
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
          }}
        />
      </SettingRow>
      {isNonLocalUrl(baseUrl) ? <NonLocalWarning /> : null}
      {fieldError !== null ? (
        <p className={styles.providerError} role="alert">
          {fieldError}
        </p>
      ) : null}

      <SettingRow label="Model">
        {models === null && modelsError === null ? (
          <span className={styles.providerHint}>Loading models…</span>
        ) : modelsError !== null ? (
          <span className={styles.providerHint}>Couldn’t list models</span>
        ) : modelOptions.length === 0 ? (
          <span className={styles.providerHint}>
            No models reported by the server
          </span>
        ) : (
          <select
            className={styles.dropdown}
            aria-label="OpenAI-compatible model"
            value={provider.model}
            onChange={(e) => commitField('model', e.target.value, () => {})}
          >
            {provider.model === '' ? (
              <option value="" disabled>
                Choose a model
              </option>
            ) : null}
            {modelOptions.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        )}
      </SettingRow>
      {modelsError !== null ? (
        <p className={styles.providerError} role="alert">
          {modelsError}{' '}
          <button
            type="button"
            className={`${styles.button} ${styles.buttonGhost}`}
            onClick={() => void refreshModels()}
          >
            Retry
          </button>
        </p>
      ) : null}

      <SettingRow
        label="API key"
        helper={configHelp('inference', 'openai_api_key')}
      >
        <div className={styles.providerInlineRow} style={{ marginTop: 0 }}>
          <input
            type="password"
            className={styles.input}
            aria-label="API key"
            autoComplete="off"
            placeholder={hasKey ? '••••••••' : 'sk-…'}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
          />
          <button
            type="button"
            className={styles.button}
            disabled={apiKey === ''}
            onClick={saveKey}
          >
            Save key
          </button>
          {hasKey ? (
            <>
              <span className={styles.keySavedChip}>Key saved</span>
              <button
                type="button"
                className={`${styles.button} ${styles.buttonGhost}`}
                onClick={clearKey}
              >
                Clear key
              </button>
            </>
          ) : null}
        </div>
      </SettingRow>
      {keyError !== null ? (
        <p className={styles.providerError} role="alert">
          {keyError}
        </p>
      ) : null}

      <SettingRow
        label="Vision"
        helper={configHelp('inference', 'openai_vision')}
      >
        <Toggle
          checked={provider.vision}
          onChange={(next) =>
            commitField('vision', next ? 'true' : 'false', () => {})
          }
          ariaLabel="Model accepts image inputs"
        />
      </SettingRow>

      <div className={styles.providerInlineRow}>
        {confirmingRemove ? (
          <>
            <span className={styles.providerHint}>
              Remove this provider? Its saved API key is deleted too.
            </span>
            <button
              type="button"
              className={`${styles.button} ${styles.buttonDestructive}`}
              onClick={removeProvider}
            >
              Remove
            </button>
            <button
              type="button"
              className={`${styles.button} ${styles.buttonGhost}`}
              onClick={() => setConfirmingRemove(false)}
            >
              Cancel
            </button>
          </>
        ) : (
          <button
            type="button"
            className={`${styles.button} ${styles.buttonGhost}`}
            onClick={() => setConfirmingRemove(true)}
          >
            Remove provider
          </button>
        )}
      </div>
    </>
  );
}

// ─── Add affordance (no OpenAI-compatible provider configured) ───────────────

interface AddOpenAiProviderProps {
  onSaved: (next: RawAppConfig) => void;
}

export function AddOpenAiProvider({ onSaved }: AddOpenAiProviderProps) {
  const [open, setOpen] = useState(false);
  const [label, setLabel] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [error, setError] = useState<string | null>(null);

  function handleAdd() {
    void invoke<RawAppConfig>('add_openai_provider', {
      label,
      baseUrl: baseUrl.trim(),
    })
      .then((cfg) => {
        setOpen(false);
        setLabel('');
        setBaseUrl('');
        setError(null);
        onSaved(cfg);
      })
      .catch((err) => setError(describeConfigError(err)));
  }

  if (!open) {
    return (
      <div className={styles.providerCard}>
        <button
          type="button"
          className={`${styles.button} ${styles.buttonGhost}`}
          onClick={() => setOpen(true)}
        >
          Add OpenAI-compatible server
        </button>
      </div>
    );
  }

  return (
    <div className={styles.providerCard}>
      <span className={styles.providerName}>OpenAI-compatible server</span>
      <SettingRow label="Label">
        <input
          type="text"
          className={styles.input}
          aria-label="Provider label"
          placeholder="LM Studio"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
        />
      </SettingRow>
      <SettingRow
        label="Base URL"
        helper={configHelp('inference', 'openai_base_url')}
      >
        <input
          type="text"
          className={styles.input}
          aria-label="OpenAI-compatible base URL"
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          placeholder="http://127.0.0.1:1234"
          value={baseUrl}
          onChange={(e) => setBaseUrl(e.target.value)}
        />
      </SettingRow>
      {isNonLocalUrl(baseUrl) ? <NonLocalWarning /> : null}
      {error !== null ? (
        <p className={styles.providerError} role="alert">
          {error}
        </p>
      ) : null}
      <div className={styles.providerInlineRow}>
        <button
          type="button"
          className={styles.button}
          disabled={baseUrl.trim() === ''}
          onClick={handleAdd}
        >
          Add
        </button>
        <button
          type="button"
          className={`${styles.button} ${styles.buttonGhost}`}
          onClick={() => {
            setOpen(false);
            setError(null);
          }}
        >
          Cancel
        </button>
      </div>
    </div>
  );
}
