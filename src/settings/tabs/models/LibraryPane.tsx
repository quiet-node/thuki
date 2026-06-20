/**
 * Library pane of the Models surface: the user's installed local models.
 *
 * Each downloaded model shows as a quiet row: its name with capability pills
 * (Text always, plus Vision / Thinking when applicable), the Hugging Face repo
 * / quantisation / size, and a RAM-fit hint (hover for a one-line explanation).
 * The active model is marked by the accent edge alone, not a textual pill.
 * A ⋮ button opens a floating popover (Set as active / View on Hugging Face /
 * Reveal in Finder / Delete) instead of expanding the card; Delete routes
 * through a confirm dialog. When nothing is installed the pane invites the
 * user over to Discover.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { useModelCapabilities } from '../../../hooks/useModelCapabilities';
import { ConfirmDialog } from '../../components';
import { Tooltip } from '../../../components/Tooltip';
import { formatContextWindow } from '../../../utils/contextWindow';
import { RAM_FIT_LABEL, RAM_FIT_TOOLTIP } from '../../../utils/ramFit';
import styles from './LibraryPane.module.css';
import type { RawAppConfig } from '../../types';
import type { InstalledModel, RamFit } from '../../../types/starter';

const HF_BASE_URL = 'https://huggingface.co';

/**
 * Approximate height (px) the popover needs below the ⋮ trigger. When the space
 * beneath it is tighter than this, the menu flips above the button so it is
 * never clipped: the Settings window auto-hugs its content and `.body` hides
 * overflow, so a downward menu on the last row would spill past the window.
 */
const MENU_DROP_ESTIMATE_PX = 210;

/** RAM-fit hint colour class on this pane's stylesheet (labels are shared). */
const FIT_CLASS: Record<RamFit, string> = {
  fits: styles.fitOk,
  tight: styles.fitTight,
  too_big: styles.fitHeavy,
};

// Popover icons (line-art, currentColor), matching the locked menu layout.
const SET_ACTIVE_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M5 13l4 4L19 7" />
  </svg>
);
const HF_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M14 3h7v7M21 3l-9 9M19 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h5" />
  </svg>
);
const FINDER_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M3 7h6l2 2h10v9a2 2 0 0 1-2 2H3z" />
  </svg>
);
const TRASH_ICON = (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14" />
  </svg>
);

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
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const [menuDropUp, setMenuDropUp] = useState(false);
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

  // Open the popover for `id` (or close it if already open). On open, flip the
  // menu above the trigger when there is not enough room below: rows near the
  // window's bottom edge would otherwise be clipped by the hidden body overflow.
  function toggleMenu(id: string, trigger: HTMLElement) {
    if (openMenu === id) {
      setOpenMenu(null);
      return;
    }
    const spaceBelow =
      window.innerHeight - trigger.getBoundingClientRect().bottom;
    setMenuDropUp(spaceBelow < MENU_DROP_ESTIMATE_PX);
    setOpenMenu(id);
  }

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

  function revealInFinder(id: string) {
    setOpenMenu(null);
    void invoke('reveal_model_in_finder', { id }).catch(() => {
      // Best-effort: a missing blob just means nothing to reveal.
    });
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
            // Empty when the model carries no context window, which skips it.
            const contextLabel = formatContextWindow(m.context_length ?? 0);
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
                      <span className={`${styles.pill} ${styles.pillText}`}>
                        Text
                      </span>
                      {caps?.vision ? (
                        <span className={`${styles.pill} ${styles.pillVision}`}>
                          Vision
                        </span>
                      ) : null}
                      {caps?.thinking ? (
                        <span
                          className={`${styles.pill} ${styles.pillThinking}`}
                        >
                          Thinking
                        </span>
                      ) : null}
                    </div>
                    <div className={styles.org}>
                      {repo}
                      {m.quant !== '' ? ` · ${m.quant}` : ''} ·{' '}
                      {gb(m.size_bytes)} GB
                      {contextLabel ? ` · ${contextLabel}` : ''}
                    </div>
                  </div>
                  <div className={styles.right}>
                    {m.fit ? (
                      <Tooltip label={RAM_FIT_TOOLTIP[m.fit]} placement="top">
                        <span className={`${styles.fit} ${FIT_CLASS[m.fit]}`}>
                          {RAM_FIT_LABEL[m.fit]}
                        </span>
                      </Tooltip>
                    ) : null}
                    <div className={styles.menuWrap} data-menu-root>
                      <button
                        type="button"
                        className={styles.manageButton}
                        aria-label={`Manage ${m.display_name}`}
                        aria-haspopup="menu"
                        aria-expanded={openMenu === m.id}
                        onClick={(e) => toggleMenu(m.id, e.currentTarget)}
                      >
                        ⋮
                      </button>
                      {openMenu === m.id ? (
                        <div
                          className={styles.menu}
                          role="menu"
                          data-side={menuDropUp ? 'top' : 'bottom'}
                        >
                          {active ? null : (
                            <button
                              type="button"
                              role="menuitem"
                              className={styles.menuItem}
                              onClick={() => selectModel(m.id)}
                            >
                              {SET_ACTIVE_ICON}
                              <span>Set as active</span>
                            </button>
                          )}
                          <button
                            type="button"
                            role="menuitem"
                            className={styles.menuItem}
                            onClick={() => openHuggingFace(m.id)}
                          >
                            {HF_ICON}
                            <span>View on Hugging Face</span>
                            <span className={styles.menuExt} aria-hidden="true">
                              ↗
                            </span>
                          </button>
                          <button
                            type="button"
                            role="menuitem"
                            className={styles.menuItem}
                            onClick={() => revealInFinder(m.id)}
                          >
                            {FINDER_ICON}
                            <span>Reveal in Finder</span>
                          </button>
                          <div className={styles.menuSep} />
                          <button
                            type="button"
                            role="menuitem"
                            className={`${styles.menuItem} ${styles.menuItemDanger}`}
                            onClick={() => {
                              setOpenMenu(null);
                              setConfirmDelete(m.id);
                            }}
                          >
                            {TRASH_ICON}
                            <span>Delete model</span>
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
