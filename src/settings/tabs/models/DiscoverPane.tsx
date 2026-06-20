/**
 * Discover host: the two-pathway shell for finding a model.
 *
 * A tab control switches between the curated front door and the advanced
 * browser:
 * - "Staff picks" ({@link StaffPicksPane}) is the default: a short catalog Thuki
 *   hand-picks and groups by family, one chosen quant per model.
 * - "Browse all" ({@link BrowseAllPane}) is the full Hugging Face GGUF browser,
 *   the escape hatch for users who want anything beyond the curated set.
 *
 * This file only routes between the two panes; each owns its own data and
 * download flow. The tablist mirrors the Models segmented control's roving
 * arrow-key pattern.
 */

import { useState } from 'react';

import { StaffPicksPane } from './StaffPicksPane';
import { BrowseAllPane } from './BrowseAllPane';
import styles from './DiscoverPane.module.css';
import type { RawAppConfig } from '../../types';

type Pathway = 'staff' | 'browse';

const TABS: ReadonlyArray<{ id: Pathway; label: string }> = [
  { id: 'staff', label: 'Staff picks' },
  { id: 'browse', label: 'Browse all' },
];

interface DiscoverPaneProps {
  /** Lift a fresh config snapshot after a successful install. */
  onSaved: (next: RawAppConfig) => void;
}

export function DiscoverPane({ onSaved }: DiscoverPaneProps) {
  const [pathway, setPathway] = useState<Pathway>('staff');

  return (
    <div className={styles.host}>
      <div
        className={styles.tabs}
        role="tablist"
        aria-label="Discover pathways"
      >
        {TABS.map((tab) => {
          const active = tab.id === pathway;
          return (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={active}
              tabIndex={active ? 0 : -1}
              className={`${styles.tab} ${active ? styles.tabActive : ''}`}
              onClick={() => setPathway(tab.id)}
              onKeyDown={(e) => {
                const isNext = e.key === 'ArrowRight';
                const isPrev = e.key === 'ArrowLeft';
                if (isNext || isPrev) {
                  e.preventDefault();
                  const idx = TABS.findIndex((t) => t.id === pathway);
                  const next = isNext
                    ? TABS[(idx + 1) % TABS.length]
                    : TABS[(idx - 1 + TABS.length) % TABS.length];
                  setPathway(next.id);
                }
              }}
            >
              <span className={styles.tabLabel}>{tab.label}</span>
            </button>
          );
        })}
      </div>

      {pathway === 'staff' ? (
        <StaffPicksPane onSaved={onSaved} />
      ) : (
        <BrowseAllPane onSaved={onSaved} />
      )}
    </div>
  );
}
