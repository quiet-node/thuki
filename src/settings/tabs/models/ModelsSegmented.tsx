/**
 * Segmented control that switches the Models surface between its three
 * sub-views. Rendered at the top of the Models section; the chosen view
 * swaps the pane below it.
 *
 * A nested tablist (the left sidebar is the outer one): the views are
 * mutually exclusive panes, so tab semantics + roving arrow keys are the
 * right pattern. Labelled distinctly so queries never collide with the
 * sidebar's section tabs.
 */

import type { ReactNode } from 'react';

import { blurOnProgrammaticFocus } from '../../../utils/blurOnProgrammaticFocus';
import styles from '../../../styles/settings.module.css';

export type ModelsSubview = 'library' | 'discover' | 'providers';

// Line-art icons in the same family as the sidebar section tabs (1.6 rounded
// stroke, currentColor): Library = stacked layers, Discover = compass,
// Providers = server stack. Decorative, so each is aria-hidden and the button's
// text label remains the accessible name.
const LIBRARY_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M12 3l9 4.8-9 4.8-9-4.8 9-4.8z" />
    <path d="M3 12.2l9 4.8 9-4.8" />
    <path d="M3 16.6l9 4.8 9-4.8" />
  </svg>
);
const DISCOVER_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <circle cx="12" cy="12" r="9" />
    <path d="M15.6 8.4l-2.3 5.2-5.2 2.3 2.3-5.2 5.2-2.3z" />
  </svg>
);
const PROVIDERS_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <rect x="3" y="4.5" width="18" height="6.5" rx="1.8" />
    <rect x="3" y="13" width="18" height="6.5" rx="1.8" />
    <path d="M6.5 7.75h.01M6.5 16.25h.01" />
  </svg>
);

const VIEWS: ReadonlyArray<{
  id: ModelsSubview;
  label: string;
  icon: ReactNode;
}> = [
  { id: 'library', label: 'Library', icon: LIBRARY_ICON },
  { id: 'discover', label: 'Discover', icon: DISCOVER_ICON },
  { id: 'providers', label: 'Providers', icon: PROVIDERS_ICON },
];

interface ModelsSegmentedProps {
  value: ModelsSubview;
  onChange: (next: ModelsSubview) => void;
}

export function ModelsSegmented({ value, onChange }: ModelsSegmentedProps) {
  return (
    <div className={styles.seg} role="tablist" aria-label="Model views">
      {VIEWS.map((view) => {
        const active = view.id === value;
        return (
          <button
            key={view.id}
            type="button"
            role="tab"
            aria-selected={active}
            tabIndex={active ? 0 : -1}
            className={`${styles.segItem} ${active ? styles.segItemActive : ''}`}
            onClick={() => onChange(view.id)}
            onFocus={blurOnProgrammaticFocus}
            onKeyDown={(e) => {
              const isNext = e.key === 'ArrowRight';
              const isPrev = e.key === 'ArrowLeft';
              if (isNext || isPrev) {
                e.preventDefault();
                const idx = VIEWS.findIndex((v) => v.id === value);
                const next = isNext
                  ? VIEWS[(idx + 1) % VIEWS.length]
                  : VIEWS[(idx - 1 + VIEWS.length) % VIEWS.length];
                onChange(next.id);
              }
            }}
          >
            {view.icon}
            <span className={styles.segItemLabel}>{view.label}</span>
          </button>
        );
      })}
    </div>
  );
}
