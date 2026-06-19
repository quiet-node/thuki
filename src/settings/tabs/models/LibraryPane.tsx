/**
 * Library pane of the Models surface: the user's installed local models.
 *
 * Each downloaded model shows as a card with its name, capability badges
 * (Vision / Reasoning, detected automatically), and its Hugging Face repo,
 * quantisation, and size. The currently selected built-in model is marked
 * Active; any other model offers a Use button that makes it the active one.
 * A per-card Manage menu reveals an inline Delete confirm that removes the
 * model from disk. When nothing is installed the pane invites the user over
 * to Discover; a footer reports free disk space and the model count.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { useModelCapabilities } from '../../../hooks/useModelCapabilities';
import styles from './LibraryPane.module.css';
import type { RawAppConfig } from '../../types';
import type { InstalledModel } from '../../../types/starter';

/** Bytes rendered as decimal gigabytes with one decimal (e.g. "8.2"). */
function gb(bytes: number): string {
  return (bytes / 1e9).toFixed(1);
}

interface LibraryPaneProps {
  config: RawAppConfig;
  /** Lift a fresh config after a Use or Delete writes to disk. */
  onSaved: (next: RawAppConfig) => void;
  /** Navigate to the Discover view to download a new model. */
  onAddModel: () => void;
}

export function LibraryPane({ config, onSaved, onAddModel }: LibraryPaneProps) {
  const activeModel =
    config.inference.providers.find((p) => p.kind === 'builtin')?.model ?? '';

  const [installed, setInstalled] = useState<InstalledModel[]>([]);
  const [freeDiskBytes, setFreeDiskBytes] = useState<number | null>(null);
  const [managing, setManaging] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const { capabilities } = useModelCapabilities();

  const refreshInstalled = useCallback(async () => {
    try {
      const rows = await invoke<InstalledModel[]>('list_installed_models');
      setInstalled(Array.isArray(rows) ? rows : []);
    } catch {
      setInstalled([]);
    }
  }, []);

  useEffect(() => {
    void refreshInstalled();
    void invoke<number | null>('get_models_dir_free_bytes')
      .then((bytes) => {
        setFreeDiskBytes(typeof bytes === 'number' ? bytes : null);
      })
      .catch(() => {
        // Unknown free space just hides the disk line.
      });
  }, [refreshInstalled]);

  // The backend writes the builtin provider's model field; lift the fresh
  // snapshot so the active card moves without a tab remount.
  function selectModel(id: string) {
    void invoke('update_provider_field', {
      providerId: 'builtin',
      field: 'model',
      value: id,
    })
      .then(async () => {
        await refreshInstalled();
        onSaved(await invoke<RawAppConfig>('get_config'));
      })
      .catch(() => {
        // The focus-driven resync picks the change up on next activation.
      });
  }

  // Deletion is refcounted server-side; the backend also clears the builtin
  // provider's model field when the deleted model was the selected one, so
  // the lifted snapshot is the source of truth.
  async function handleDelete(id: string) {
    setConfirmingDelete(null);
    setManaging(null);
    try {
      await invoke('delete_installed_model', { id });
    } catch (err) {
      setDeleteError(String(err));
      return;
    }
    setDeleteError(null);
    await refreshInstalled();
    try {
      onSaved(await invoke<RawAppConfig>('get_config'));
    } catch {
      // The focus-driven resync picks the change up on next activation.
    }
  }

  return (
    <div className={styles.pane}>
      <div className={styles.bar}>
        <button type="button" className={styles.addButton} onClick={onAddModel}>
          Add model
        </button>
      </div>

      {installed.length === 0 ? (
        <div className={styles.empty}>
          <p className={styles.emptyText}>No models downloaded yet.</p>
          <button
            type="button"
            className={styles.browseButton}
            onClick={onAddModel}
          >
            Browse Discover
          </button>
        </div>
      ) : (
        <div className={styles.list}>
          {installed.map((m) => {
            const active = m.id === activeModel;
            const caps = capabilities[m.id];
            const repo = m.id.split(':')[0];
            return (
              <div
                key={m.id}
                className={`${styles.card} ${active ? styles.cardActive : ''}`}
              >
                <div className={styles.row}>
                  <div className={styles.avatar}>
                    {m.display_name.charAt(0).toUpperCase()}
                  </div>
                  <div className={styles.mid}>
                    <div className={styles.name}>
                      {m.display_name}
                      {active ? (
                        <span
                          className={`${styles.badge} ${styles.badgeActive}`}
                        >
                          Active
                        </span>
                      ) : null}
                      {caps?.vision ? (
                        <span
                          className={`${styles.badge} ${styles.badgeVision}`}
                        >
                          Vision
                        </span>
                      ) : null}
                      {caps?.thinking ? (
                        <span
                          className={`${styles.badge} ${styles.badgeReason}`}
                        >
                          Reasoning
                        </span>
                      ) : null}
                    </div>
                    <div className={styles.org}>
                      {repo}
                      {m.quant !== '' ? ` · ${m.quant}` : ''} ·{' '}
                      {gb(m.size_bytes)} GB
                    </div>
                  </div>
                  <div className={styles.actions}>
                    {active ? null : (
                      <button
                        type="button"
                        className={styles.useButton}
                        aria-label={`Use ${m.display_name}`}
                        onClick={() => selectModel(m.id)}
                      >
                        Use
                      </button>
                    )}
                    <button
                      type="button"
                      className={styles.manageButton}
                      aria-label={`Manage ${m.display_name}`}
                      onClick={() =>
                        setManaging((cur) => (cur === m.id ? null : m.id))
                      }
                    >
                      ⋮
                    </button>
                  </div>
                </div>

                {managing === m.id ? (
                  <div className={styles.manageRow}>
                    {confirmingDelete === m.id ? (
                      <>
                        <span className={styles.confirmText}>
                          Delete {m.display_name}? Its files are removed from
                          disk.
                        </span>
                        <button
                          type="button"
                          className={styles.deleteButton}
                          aria-label="Confirm delete"
                          onClick={() => void handleDelete(m.id)}
                        >
                          Delete
                        </button>
                        <button
                          type="button"
                          className={styles.ghostButton}
                          onClick={() => setConfirmingDelete(null)}
                        >
                          Cancel
                        </button>
                      </>
                    ) : (
                      <button
                        type="button"
                        className={styles.deleteButton}
                        aria-label={`Delete ${m.display_name}`}
                        onClick={() => setConfirmingDelete(m.id)}
                      >
                        Delete
                      </button>
                    )}
                  </div>
                ) : null}
              </div>
            );
          })}
        </div>
      )}

      {deleteError !== null ? (
        <p className={styles.error} role="alert">
          {deleteError}
        </p>
      ) : null}

      <div className={styles.footer}>
        <span>
          {freeDiskBytes !== null ? `${gb(freeDiskBytes)} GB free on disk` : ''}
        </span>
        <span>
          {installed.length} models · capabilities detected automatically
        </span>
      </div>
    </div>
  );
}
