/**
 * Models tab: a segmented surface over the three model sub-views.
 *
 * The left sidebar selects this section; the segmented control at the top
 * picks Library (installed models), Discover (the Hugging Face browser), or
 * Providers (the active provider plus the shared generation settings). Each
 * sub-view is its own pane component; this file only routes between them.
 */

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { ModelsSegmented, type ModelsSubview } from './models/ModelsSegmented';
import { ProvidersPane } from './models/ProvidersPane';
import { LibraryPane } from './models/LibraryPane';
import { DiscoverPane } from './models/DiscoverPane';
import { BuiltinOnlyGate } from './models/BuiltinOnlyGate';
import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';

interface ModelTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
  /**
   * One-shot request to jump to a specific sub-view (the overlay picker's
   * "download a model" link deep-links to Discover). Applied once, then cleared
   * via `onPendingViewConsumed` so a later manual switch or remount sticks.
   */
  pendingView?: ModelsSubview | null;
  /** Called after `pendingView` has been applied, so the host can clear it. */
  onPendingViewConsumed?: () => void;
}

export function ModelTab({
  config,
  resyncToken,
  onSaved,
  pendingView,
  onPendingViewConsumed,
}: ModelTabProps) {
  // Providers is the default sub-view: the active provider and the shared
  // generation controls are the most-used surface.
  const [view, setView] = useState<ModelsSubview>('providers');
  const goToDiscover = () => setView('discover');

  useEffect(() => {
    if (!pendingView) return;
    // Intentional one-shot: the picker's deep-link can fire while this tab is
    // already mounted, so the view is updated here rather than at init. It runs
    // once per deep-link, so the re-render the rule guards against is a non-issue.
    // eslint-disable-next-line @eslint-react/set-state-in-effect
    setView(pendingView);
    onPendingViewConsumed?.();
  }, [pendingView, onPendingViewConsumed]);

  // Library and Discover manage the built-in engine's models, so they are
  // gated behind a switch prompt while a non-built-in provider is active.
  const { providers, active_provider } = config.inference;
  const activeProvider = providers.find((p) => p.id === active_provider);
  const gated = activeProvider?.kind !== 'builtin';
  const activeLabel = activeProvider?.label ?? 'another provider';
  const builtinId = providers.find((p) => p.kind === 'builtin')?.id;

  // Activate the built-in engine from the gate; a no-op if it is not configured.
  function switchToBuiltin() {
    if (builtinId === undefined) return;
    void invoke<RawAppConfig>('set_active_provider', { providerId: builtinId })
      .then(onSaved)
      .catch(() => {});
  }

  return (
    <>
      <div className={styles.barrow}>
        <ModelsSegmented value={view} onChange={setView} />
      </div>

      {view === 'library' ? (
        <BuiltinOnlyGate
          gated={gated}
          activeLabel={activeLabel}
          onSwitch={switchToBuiltin}
        >
          <LibraryPane
            config={config}
            onSaved={onSaved}
            onAddModel={goToDiscover}
          />
        </BuiltinOnlyGate>
      ) : null}

      {view === 'discover' ? (
        <BuiltinOnlyGate
          gated={gated}
          activeLabel={activeLabel}
          onSwitch={switchToBuiltin}
        >
          <DiscoverPane onSaved={onSaved} />
        </BuiltinOnlyGate>
      ) : null}

      {view === 'providers' ? (
        <ProvidersPane
          config={config}
          resyncToken={resyncToken}
          onSaved={onSaved}
          onAddModel={goToDiscover}
        />
      ) : null}
    </>
  );
}
