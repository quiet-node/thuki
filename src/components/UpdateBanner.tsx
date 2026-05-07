import { invoke } from '@tauri-apps/api/core';
import styles from '../styles/settings.module.css';

interface UpdateBannerProps {
  version: string;
  notesUrl: string | null;
  onInstall: () => void;
  onLater: () => void;
}

export function UpdateBanner({
  version,
  notesUrl,
  onInstall,
  onLater,
}: UpdateBannerProps) {
  return (
    <div className={styles.updateBanner} role="status" aria-live="polite">
      <span className={styles.updateBannerPulse} aria-hidden />
      <div className={styles.updateBannerBody}>
        <div
          className={styles.updateBannerTitle}
        >{`Thuki ${version} is ready`}</div>
        <div className={styles.updateBannerMeta}>
          {notesUrl ? (
            <button
              type="button"
              onClick={() => void invoke('open_url', { url: notesUrl })}
              style={{
                background: 'none',
                border: 0,
                padding: 0,
                color: 'var(--color-primary)',
                textDecoration: 'underline',
                cursor: 'pointer',
                font: 'inherit',
              }}
            >
              Release notes
            </button>
          ) : (
            <span>Update ready</span>
          )}
        </div>
      </div>
      <div className={styles.updateBannerActions}>
        <button
          type="button"
          className={styles.updateBannerInstall}
          onClick={onInstall}
        >
          Install &amp; restart
        </button>
        <button
          type="button"
          className={styles.updateBannerLater}
          onClick={onLater}
        >
          Later
        </button>
      </div>
    </div>
  );
}
