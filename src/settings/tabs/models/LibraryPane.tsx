/**
 * Library pane of the Models surface: the user's installed local models.
 *
 * Each downloaded model shows as a quiet row: its name, an Active state, the
 * Hugging Face repo / quantisation / size, capability text tags (Vision /
 * Reasoning, detected automatically), and a RAM-fit hint for this Mac. A ⋮
 * button opens a floating popover (Set as active / View on Hugging Face /
 * Delete) instead of expanding the card. Delete routes through a confirm
 * dialog. When nothing is installed the pane invites the user over to
 * Discover; a footer reports the model count and free disk space.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { useModelCapabilities } from '../../../hooks/useModelCapabilities';
import { ConfirmDialog } from '../../components';
import styles from './LibraryPane.module.css';
import type { RawAppConfig } from '../../types';
import type { InstalledModel, RamFit } from '../../../types/starter';

const HF_BASE_URL = 'https://huggingface.co';

/** RAM-fit hint label shown next to a model. */
const FIT_LABEL: Record<RamFit, string> = {
  fits: 'Comfortable',
  tight: 'Tight',
  too_big: 'Heavy',
};

/** RAM-fit hint colour class on this pane's stylesheet. */
const FIT_CLASS: Record<RamFit, string> = {
  fits: styles.fitOk,
  tight: styles.fitTight,
  too_big: styles.fitHeavy,
};

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
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
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

  // Close the popover on an outside click or Escape so it behaves like a real
  // menu rather than a sticky panel.
  useEffect(() => {
    if (openMenu === null) return;
    const onDown = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest('[data-menu-root]')) {
        setOpenMenu(null);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpenMenu(null);
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [openMenu]);

  // The backend writes the builtin provider's model field; lift the fresh
  // snapshot so the active row moves without a tab remount.
  function selectModel(id: string) {
    setOpenMenu(null);
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

  function openHuggingFace(id: string) {
    setOpenMenu(null);
    void invoke('open_url', { url: `${HF_BASE_URL}/${id.split(':')[0]}` });
  }

  // Deletion is refcounted server-side; the backend also clears the builtin
  // provider's model field when the deleted model was the selected one, so
  // the lifted snapshot is the source of truth.
  async function handleDelete(id: string) {
    setConfirmDelete(null);
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

  const confirmModel = installed.find((m) => m.id === confirmDelete);

  return (
    <div className={styles.pane}>
      <div className={styles.bar}>
        <button type="button" className={styles.addButton} onClick={onAddModel}>
          <svg
            viewBox="0 0 24 24"
            aria-hidden="true"
            className={styles.addIcon}
          >
            <path d="M12 5v14M5 12h14" />
          </svg>
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
                {active ? <span className={styles.activeEdge} /> : null}
                <div className={styles.row}>
                  <div className={styles.mid}>
                    <div className={styles.name}>
                      {m.display_name}
                      {active ? (
                        <span className={styles.activeBadge}>Active</span>
                      ) : null}
                    </div>
                    <div className={styles.org}>
                      {repo}
                      {m.quant !== '' ? ` · ${m.quant}` : ''} ·{' '}
                      {gb(m.size_bytes)} GB
                    </div>
                  </div>
                  <div className={styles.right}>
                    {m.fit ? (
                      <span className={`${styles.fit} ${FIT_CLASS[m.fit]}`}>
                        {FIT_LABEL[m.fit]}
                      </span>
                    ) : null}
                    {caps?.vision ? (
                      <span className={styles.tagVision}>Vision</span>
                    ) : null}
                    {caps?.thinking ? (
                      <span className={styles.tagReason}>Reasoning</span>
                    ) : null}
                    <div className={styles.menuWrap} data-menu-root>
                      <button
                        type="button"
                        className={styles.manageButton}
                        aria-label={`Manage ${m.display_name}`}
                        aria-haspopup="menu"
                        aria-expanded={openMenu === m.id}
                        onClick={() =>
                          setOpenMenu((cur) => (cur === m.id ? null : m.id))
                        }
                      >
                        ⋮
                      </button>
                      {openMenu === m.id ? (
                        <div className={styles.menu} role="menu">
                          {active ? null : (
                            <button
                              type="button"
                              role="menuitem"
                              className={styles.menuItem}
                              onClick={() => selectModel(m.id)}
                            >
                              Set as active
                            </button>
                          )}
                          <button
                            type="button"
                            role="menuitem"
                            className={styles.menuItem}
                            onClick={() => openHuggingFace(m.id)}
                          >
                            View on Hugging Face
                          </button>
                          <div className={styles.menuSep} />
                          <button
                            type="button"
                            role="menuitem"
                            className={`${styles.menuItem} ${styles.menuItemDanger}`}
                            aria-label={`Delete ${m.display_name}`}
                            onClick={() => {
                              setOpenMenu(null);
                              setConfirmDelete(m.id);
                            }}
                          >
                            Delete
                          </button>
                        </div>
                      ) : null}
                    </div>
                  </div>
                </div>
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
          {installed.length} model{installed.length === 1 ? '' : 's'} installed
        </span>
        <span>
          {freeDiskBytes !== null ? `${gb(freeDiskBytes)} GB free` : ''}
        </span>
      </div>

      {confirmModel ? (
        <ConfirmDialog
          open
          title={`Delete ${confirmModel.display_name}?`}
          message="Its files are removed from disk."
          confirmLabel="Delete"
          destructive
          onConfirm={() => void handleDelete(confirmModel.id)}
          onCancel={() => setConfirmDelete(null)}
        />
      ) : null}
    </div>
  );
}
