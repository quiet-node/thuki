/**
 * About tab — app identity, social links, permission pills, and the
 * file-level escape hatches (Reveal config.toml, Refresh from disk,
 * Reset all to defaults).
 *
 * Info-only: no `set_config_field` calls. Reset-all and Refresh-from-disk
 * are the two write actions; both are gated by explicit confirms or
 * targeted at the on-disk snapshot rather than a live editor field.
 */

import { useEffect, useState } from 'react';

import { invoke } from '@tauri-apps/api/core';

import thukiLogo from '../../../src-tauri/icons/128x128.png';
import pkg from '../../../package.json';
import { Section, ConfirmDialog } from '../components';
import { Tooltip } from '../../components/Tooltip';
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
  // Evaluated per-render so vi.stubEnv works in tests.
  // VITE_GIT_COMMIT_SHA is injected by the nightly CI workflow; absent in
  // local dev and stable release builds. When present, the version is
  // suffixed with semver build metadata: 0.6.1+nightly.abc1234
  const sha = import.meta.env.VITE_GIT_COMMIT_SHA?.slice(0, 7);
  const APP_VERSION = sha ? `${pkg.version}+nightly.${sha}` : pkg.version;
  const releaseUrl = sha
    ? 'https://github.com/quiet-node/thuki/releases/tag/nightly'
    : `https://github.com/quiet-node/thuki/releases/tag/v${pkg.version}`;
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
    <div className={styles.aboutBody}>
      <div className={styles.aboutHero}>
        <img
          src={thukiLogo}
          alt="Thuki"
          className={styles.aboutHeroLogo}
          draggable={false}
        />
        <div className={styles.aboutHeroTitle}>Thuki</div>
        <Tooltip label={`View v${APP_VERSION} release notes on GitHub`}>
          <button
            type="button"
            className={styles.aboutHeroVersion}
            aria-label={`View v${APP_VERSION} release notes on GitHub`}
            onClick={() =>
              void invoke('open_url', { url: releaseUrl })
            }
          >
            v{APP_VERSION}
          </button>
        </Tooltip>
        <div className={styles.aboutHeroTagline}>
          A floating, local-first AI secretary for macOS.
          <br />
          <span className={styles.aboutHeroMantra}>
            No cloud. No clutter. Just answers.
          </span>
        </div>
        <div className={styles.aboutHeroActions}>
          <Tooltip label="View Thuki on GitHub">
            <button
              type="button"
              className={styles.iconLinkBtn}
              aria-label="View Thuki on GitHub"
              onClick={() =>
                void invoke('open_url', {
                  url: 'https://github.com/quiet-node/thuki',
                })
              }
            >
              <GitHubIcon />
            </button>
          </Tooltip>
          <Tooltip label="Reach out to Logan on X for questions or ideas.">
            <button
              type="button"
              className={styles.iconLinkBtn}
              aria-label="Reach out to Logan on X for questions or ideas."
              onClick={() =>
                void invoke('open_url', {
                  url: 'https://x.com/quiet_node',
                })
              }
            >
              <XIcon />
            </button>
          </Tooltip>
          <Tooltip label="Report a bug or share feedback on GitHub Issues.">
            <button
              type="button"
              className={styles.iconLinkBtn}
              aria-label="Open an issue or share feedback on GitHub"
              onClick={() =>
                void invoke('open_url', {
                  url: 'https://github.com/quiet-node/thuki/issues',
                })
              }
            >
              <FeedbackIcon />
            </button>
          </Tooltip>
        </div>
      </div>

      <Section heading="Permissions">
        <div className={styles.row}>
          <div className={styles.rowLabelGroup}>
            <span className={styles.rowLabel}>Accessibility</span>
          </div>
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
          <div className={styles.rowLabelGroup}>
            <span className={styles.rowLabel}>Screen Recording</span>
          </div>
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

      <Section heading="File">
        <div className={styles.aboutLinkRow}>
          <button
            type="button"
            className={`${styles.button} ${styles.buttonGhost}`}
            onClick={() => void invoke('reveal_config_in_finder')}
          >
            Reveal Thuki app data
          </button>
          <Tooltip
            label="Re-read config.toml from disk and refresh this window. Use after editing the file by hand outside Thuki."
            multiline
            placement="top"
          >
            <button
              type="button"
              className={`${styles.button} ${styles.buttonGhost}`}
              onClick={() => void onReload()}
            >
              Refresh config.toml
            </button>
          </Tooltip>
          <button
            type="button"
            className={`${styles.button} ${styles.buttonDestructive}`}
            onClick={() => setConfirmResetAll(true)}
          >
            Reset all to defaults…
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
    </div>
  );
}

// ─── Inline brand icons ───────────────────────────────────────────────────
//
// Vendored as small inline SVG components so we do not pull a fresh icon
// dependency for two glyphs. Both paths are the official simple-icons
// monochrome marks (CC0). currentColor lets the button tint them.

function GitHubIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="18"
      height="18"
      fill="currentColor"
      aria-hidden
    >
      <path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.56 0-.27-.01-1-.02-1.96-3.2.7-3.87-1.54-3.87-1.54-.52-1.32-1.27-1.67-1.27-1.67-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.18 1.76 1.18 1.02 1.75 2.68 1.24 3.34.95.1-.74.4-1.24.72-1.53-2.55-.29-5.24-1.28-5.24-5.69 0-1.26.45-2.29 1.18-3.1-.12-.29-.51-1.46.11-3.05 0 0 .96-.31 3.15 1.18a10.96 10.96 0 0 1 5.74 0c2.19-1.49 3.15-1.18 3.15-1.18.62 1.59.23 2.76.11 3.05.74.81 1.18 1.84 1.18 3.1 0 4.42-2.7 5.4-5.27 5.68.41.36.78 1.06.78 2.13 0 1.54-.01 2.78-.01 3.16 0 .31.21.68.8.56C20.21 21.39 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z" />
    </svg>
  );
}

function XIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="16"
      height="16"
      fill="currentColor"
      aria-hidden
    >
      <path d="M18.244 2H21l-6.56 7.5L22 22h-6.83l-4.74-6.2L4.8 22H2l7.04-8.06L2 2h6.92l4.28 5.66L18.244 2zm-2.4 18.5h1.74L7.27 3.4H5.4l10.444 17.1z" />
    </svg>
  );
}

// Speech-bubble glyph for the "open an issue / give feedback" action.
// Outlined to match the conversational tone of the destination
// (GitHub Issues), distinct from the solid github/x marks.
function FeedbackIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="18"
      height="18"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z" />
    </svg>
  );
}
