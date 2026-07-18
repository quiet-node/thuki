/**
 * Behavior-tab section listing the models the user chose to load over the mild
 * memory-fit limit (via the "Always allow this model" action on the warning).
 * Each remembered entry is keyed by its weights SHA-256; this section resolves
 * that digest back
 * to the installed model's display name (falling back to a short sha for an
 * orphaned entry whose model was since uninstalled) and offers a per-row Remove
 * that re-arms the warning for that model.
 *
 * The whole section is hidden when nothing is remembered, so it never adds
 * empty-state clutter. A freeze-band load (estimate at or above available
 * memory) still warns regardless of this list, by backend design.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AnimatePresence, motion } from 'framer-motion';

import { Section } from '../components';
import styles from '../../styles/settings.module.css';
import type { InstalledModel } from '../../types/starter';
import type { RawAppConfig } from '../types';

/** House decelerate curve, matching the app-wide motion language. */
const HOUSE_EASE = [0.16, 1, 0.3, 1] as const;

/** Scope description shown in the section heading's `?` tooltip. */
const SECTION_HELP =
  'Models you chose to load past the "may not fit in memory" warning. Thuki will not warn you about these again, except when a model is at serious risk of freezing your Mac.';

interface DismissedMemoryFitSectionProps {
  /** Current resolved config; its `behavior.dismissed_memory_fit_models` drives
   *  the list, refreshed by the parent on `config-updated`. */
  config: RawAppConfig;
  /** Lifts the resolved config returned by `forget_model_memory_fit` so the
   *  Settings window's shared state updates in place after a removal. */
  onSaved: (next: RawAppConfig) => void;
}

/**
 * Resolves a remembered weights SHA to a human label, or a short-sha fallback
 * when no installed model matches (an orphaned entry).
 */
function labelForSha(sha: string, installed: InstalledModel[]): string {
  const match = installed.find((model) => model.sha256 === sha);
  return match ? match.display_name : `${sha.slice(0, 8)}…`;
}

export function DismissedMemoryFitSection({
  config,
  onSaved,
}: DismissedMemoryFitSectionProps) {
  const shas = config.behavior.dismissed_memory_fit_models;
  const [installed, setInstalled] = useState<InstalledModel[]>([]);

  // Key the manifest fetch on the remembered digests themselves, not on the
  // array identity: the parent replaces the whole config object on every
  // `config-updated`, so depending on `shas` would re-issue this IPC on
  // unrelated setting changes.
  const shaKey = shas.join(',');

  // Load the installed manifest so a remembered sha can be shown as a model
  // name. Re-run when the remembered set changes (a newly remembered model may
  // not have been in the first fetch); a failed or non-array read leaves the
  // fallback short-sha labels in place.
  useEffect(() => {
    void invoke<InstalledModel[]>('list_installed_models')
      .then((rows) => setInstalled(Array.isArray(rows) ? rows : []))
      .catch(() => setInstalled([]));
  }, [shaKey]);

  /**
   * Re-arms the memory warning for one model by removing its sha, then lifts
   * the resolved config the backend returns. Best-effort: a persistence
   * failure leaves the row in place rather than crashing the panel.
   */
  const handleRemove = useCallback(
    async (sha: string) => {
      try {
        const next = await invoke<RawAppConfig>('forget_model_memory_fit', {
          modelSha: sha,
        });
        onSaved(next);
      } catch {
        // Swallow: a failed removal simply leaves the entry listed.
      }
    },
    [onSaved],
  );

  // Hidden entirely when nothing is remembered: no empty-state clutter.
  if (shas.length === 0) return null;

  return (
    <Section
      heading="Models allowed over the memory limit"
      helper={SECTION_HELP}
    >
      <div className={styles.listcard}>
        <AnimatePresence initial={false}>
          {shas.map((sha) => (
            <motion.div
              key={sha}
              className={styles.providerRow}
              // Rows added while Settings is open animate open as well as
              // closed; `initial={false}` on the AnimatePresence above keeps
              // the first paint of an existing list from animating.
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: 'auto', opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              transition={{ duration: 0.2, ease: HOUSE_EASE }}
              style={{ overflow: 'hidden' }}
            >
              <span className={styles.providerRowName}>
                {labelForSha(sha, installed)}
              </span>
              <span className={styles.providerRowSub}>
                Loads without the memory-fit warning
              </span>
              <span className={styles.grow} />
              <button
                type="button"
                className={styles.switchBtn}
                onClick={() => void handleRemove(sha)}
              >
                Remove
              </button>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>
    </Section>
  );
}
