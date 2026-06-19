/**
 * "Running model" footer pinned to the bottom of the Settings sidebar.
 *
 * Always visible, it names the model the active provider will answer with,
 * adds a size hint for the built-in engine, and shows a live dot that lights
 * when that model is currently resident in memory.
 *
 * Data sources, kept deliberately small:
 * - The active provider, its label, and (for Ollama/OpenAI) its model come
 *   straight from the config snapshot the parent already owns; the active
 *   model persists onto the provider's `model` field.
 * - The built-in engine's display name + on-disk size come from the manifest
 *   (`list_installed_models`), refreshed whenever the selected built-in model
 *   id changes.
 * - Liveness for the built-in engine follows `get_engine_status` plus the
 *   `engine:status` event stream. Ollama/OpenAI residency is not polled here,
 *   so their dot stays idle.
 */

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';
import type { EngineStatus, InstalledModel } from '../../types/starter';

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "6.6"). */
function gb(bytes: number): string {
  return (bytes / 1e9).toFixed(1);
}

interface RunningModelFooterProps {
  config: RawAppConfig;
}

export function RunningModelFooter({ config }: RunningModelFooterProps) {
  const [installed, setInstalled] = useState<InstalledModel[]>([]);
  const [engineState, setEngineState] =
    useState<EngineStatus['state']>('stopped');

  const providers = config.inference.providers;
  const active = providers.find(
    (p) => p.id === config.inference.active_provider,
  );
  const kind = active?.kind ?? 'ollama';
  const builtinModelId =
    providers.find((p) => p.kind === 'builtin')?.model ?? '';

  // Manifest read seeds the built-in size/name; re-runs when the selected
  // built-in model id changes (a download/delete/switch lifts a new config).
  useEffect(() => {
    void invoke<InstalledModel[]>('list_installed_models')
      .then((rows) => setInstalled(Array.isArray(rows) ? rows : []))
      .catch(() => setInstalled([]));
  }, [builtinModelId]);

  // Engine lifecycle drives the live dot for the built-in engine. Seed from
  // the current snapshot (the backend only emits on transitions) then follow
  // the event stream.
  useEffect(() => {
    invoke<EngineStatus>('get_engine_status')
      .then((status) => setEngineState(status.state))
      .catch(() => {
        // Keep the stopped default; the event stream corrects it.
      });
    const unlisten = listen<EngineStatus>('engine:status', (e) => {
      setEngineState(e.payload.state);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  let name: string | null;
  let meta: string | null;
  if (kind === 'builtin') {
    const row = installed.find((m) => m.id === builtinModelId);
    name = row ? row.display_name : null;
    meta = row ? `Built-in · ${gb(row.size_bytes)} GB` : null;
  } else {
    name = active && active.model !== '' ? active.model : null;
    meta = active ? active.label : null;
  }

  const live = kind === 'builtin' && engineState === 'loaded';

  return (
    <div
      className={styles.runningModel}
      role="status"
      aria-label="Running model"
    >
      <div className={styles.runningModelEyebrow}>Running</div>
      {name ? (
        <>
          <div className={styles.runningModelName}>
            <span
              className={
                live ? styles.runningModelDot : styles.runningModelDotIdle
              }
              aria-hidden
            />
            {name}
          </div>
          {meta ? <div className={styles.runningModelMeta}>{meta}</div> : null}
        </>
      ) : (
        <div className={styles.runningModelMeta}>No model selected</div>
      )}
    </div>
  );
}
