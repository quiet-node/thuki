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

import styles from '../../../styles/settings.module.css';

export type ModelsSubview = 'library' | 'discover' | 'providers';

const VIEWS: ReadonlyArray<{ id: ModelsSubview; label: string }> = [
  { id: 'library', label: 'Library' },
  { id: 'discover', label: 'Discover' },
  { id: 'providers', label: 'Providers' },
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
            {view.label}
          </button>
        );
      })}
    </div>
  );
}
