/**
 * About tab — version, activation info, permission pills, system limits,
 * and the file-level escape hatches (Reveal config.toml, Reset all,
 * Refresh from disk).
 *
 * Info-only: no `set_config_field` calls. Reset-all and Refresh-from-disk
 * are the two write actions; both are gated by explicit confirms.
 */

import { useEffect, useState } from 'react';

import { invoke } from '@tauri-apps/api/core';

import { Section, ConfirmDialog } from '../components';
import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';

interface AboutTabProps {
  onSaved: (next: RawAppConfig) => void;
  onReload: () => Promise<void>;
}

interface PermissionsState {
  accessibility: boolean;
  screenRecording: boolean;
}

export function AboutTab({ onSaved, onReload }: AboutTabProps) {
  const [confirmResetAll, setConfirmResetAll] = useState(false);
  const [perms, setPerms] = useState<PermissionsState>({
    accessibility: false,
    screenRecording: false,
  });

  // Refresh permissions on mount and on every window focus.
  useEffect(() => {
    let mounted = true;
    const refresh = async () => {
      try {
        const [a, s] = await Promise.all([
          invoke<boolean>('check_accessibility_permission'),
          invoke<boolean>('check_screen_recording_permission'),
        ]);
        if (mounted) setPerms({ accessibility: a, screenRecording: s });
      } catch {
        // Permission probes are diagnostic; failure leaves the previous
        // pill state in place.
      }
    };
    void refresh();
    const handler = () => void refresh();
    window.addEventListener('focus', handler);
    return () => {
      mounted = false;
      window.removeEventListener('focus', handler);
    };
  }, []);

  return (
    <>
      <Section heading="App">
        <div className={styles.aboutInfoLine}>
          <strong>Thuki</strong> — local-first AI secretary for macOS.
        </div>
        <div className={styles.aboutLinkRow}>
          <button
            type="button"
            className={`${styles.button} ${styles.buttonGhost}`}
            onClick={() =>
              void invoke('open_url', {
                url: 'https://github.com/quiet-node/thuki',
              })
            }
          >
            GitHub
          </button>
          <button
            type="button"
            className={`${styles.button} ${styles.buttonGhost}`}
            onClick={() =>
              void invoke('open_url', { url: 'https://ollama.com/' })
            }
          >
            Ollama
          </button>
        </div>
      </Section>

      <Section heading="Activation">
        <div className={styles.aboutInfoLine}>
          Double-tap <strong>Control</strong> to summon Thuki. Double-tap
          window:
          <strong> 400 ms</strong>, cooldown <strong>600 ms</strong> (baked-in
          for thread-safety).
        </div>
      </Section>

      <Section heading="Permissions">
        <div className={styles.row}>
          <span className={styles.rowLabel}>Accessibility</span>
          <div className={styles.rowControl}>
            <div>
              <span
                className={`${styles.permissionPill} ${
                  perms.accessibility
                    ? styles.permissionGranted
                    : styles.permissionRequired
                }`}
              >
                {perms.accessibility ? '✓ Granted' : '✗ Required'}
              </span>
              {!perms.accessibility ? (
                <button
                  type="button"
                  className={`${styles.button} ${styles.buttonGhost}`}
                  style={{ marginLeft: 8 }}
                  onClick={() => void invoke('open_accessibility_settings')}
                >
                  Open System Settings
                </button>
              ) : null}
            </div>
            <div className={styles.rowHelper}>
              Required for the global double-tap-Control hotkey.
            </div>
          </div>
        </div>
        <div className={styles.row}>
          <span className={styles.rowLabel}>Screen Recording</span>
          <div className={styles.rowControl}>
            <div>
              <span
                className={`${styles.permissionPill} ${
                  perms.screenRecording
                    ? styles.permissionGranted
                    : styles.permissionRequired
                }`}
              >
                {perms.screenRecording ? '✓ Granted' : '✗ Required'}
              </span>
              {!perms.screenRecording ? (
                <button
                  type="button"
                  className={`${styles.button} ${styles.buttonGhost}`}
                  style={{ marginLeft: 8 }}
                  onClick={() => void invoke('open_screen_recording_settings')}
                >
                  Open System Settings
                </button>
              ) : null}
            </div>
            <div className={styles.rowHelper}>
              Required for the /screen command.
            </div>
          </div>
        </div>
      </Section>

      <Section heading="Limits">
        <div className={styles.aboutInfoLine}>
          <strong>Max images per message:</strong> 4 (3 attached + 1 /screen
          capture).
        </div>
        <div className={styles.aboutInfoLine}>
          <strong>Max image size:</strong> 30 MB, downscaled to 1920 px @ JPEG
          Q85.
        </div>
      </Section>

      <Section heading="File">
        <div className={styles.aboutLinkRow}>
          <button
            type="button"
            className={styles.button}
            onClick={() => void invoke('reveal_config_in_finder')}
          >
            📂 Reveal config.toml in Finder
          </button>
          <button
            type="button"
            className={`${styles.button} ${styles.buttonGhost}`}
            onClick={() => void onReload()}
          >
            ↻ Refresh from disk
          </button>
          <button
            type="button"
            className={`${styles.button} ${styles.buttonDestructive}`}
            onClick={() => setConfirmResetAll(true)}
          >
            ⚠ Reset all to defaults…
          </button>
        </div>
      </Section>

      <ConfirmDialog
        open={confirmResetAll}
        title="Reset all settings to defaults?"
        message="Your entire config.toml will be replaced with the defaults. This cannot be undone."
        confirmLabel="Reset all"
        destructive
        onConfirm={() => {
          setConfirmResetAll(false);
          void invoke<RawAppConfig>('reset_config', { section: null }).then(
            onSaved,
          );
        }}
        onCancel={() => setConfirmResetAll(false)}
      />
    </>
  );
}
