/**
 * Models tab: a segmented surface over the three model sub-views.
 *
 * The left sidebar selects this section; the segmented control at the top
 * picks Library (installed models), Discover (the Hugging Face browser), or
 * Providers (the active provider plus the shared generation settings). Each
 * sub-view is its own pane component; this file only routes between them.
 */

import { useState } from 'react';

import { ModelsSegmented, type ModelsSubview } from './models/ModelsSegmented';
import { ProvidersPane } from './models/ProvidersPane';
import { LibraryPane } from './models/LibraryPane';
import { BrowseAllPane } from './models/BrowseAllPane';
import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';

interface ModelTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

export function ModelTab({ config, resyncToken, onSaved }: ModelTabProps) {
  // Providers is the default sub-view: the active provider and the shared
  // generation controls are the most-used surface.
  const [view, setView] = useState<ModelsSubview>('providers');
  const goToDiscover = () => setView('discover');

  return (
    <>
      <div className={styles.barrow}>
        <ModelsSegmented value={view} onChange={setView} />
      </div>

      {view === 'library' ? (
        <LibraryPane
          config={config}
          onSaved={onSaved}
          onAddModel={goToDiscover}
        />
      ) : null}

      {view === 'discover' ? <BrowseAllPane onSaved={onSaved} /> : null}

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
