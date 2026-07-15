/**
 * Behavior tab.
 *
 * Web search mode (auto vs on-demand), Text Replacement for `/rewrite` /
 * `/refine` (auto-replace and auto-close; the per-result Replace button is
 * always available regardless of those toggles), and a collapsible
 * Diagnostics block (trace recording plus open / free actions for the
 * on-disk trace folder).
 */

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import {
  PointingWiggle,
  POINTING_WIGGLE_MS,
} from '../../components/PointingWiggle';
import { ConfirmDialog, Section, SettingRow, Toggle } from '../components';
import { SaveField } from '../components/SaveField';
import { configHelp } from '../configHelpers';
import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';

interface BehaviorTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
  /**
   * Deep-link highlight generation. 0 = no wiggle; any positive value shows
   * PointingWiggle keyed by this nonce so each deep-link restarts the CSS
   * animation even if a previous highlight had not finished.
   */
  highlightAutoSearchNonce?: number;
  /** Called after the highlight animation completes so the parent can clear. */
  onHighlightAutoSearchDone?: () => void;
}

/**
 * Section-level "?" copy: what the Text Replacement group is and which commands
 * it covers. The individual toggles explain their own behavior in their own
 * tooltips, so this stays scoped to "what is this and what does it apply to".
 */
const TEXT_REPLACEMENT_HELP =
  'Applies only to /rewrite and /refine: writing their result back into the app you were using, replacing your highlighted text.';

/** Row "?" copy: what "Open traces folder" does. */
const OPEN_TRACES_HELP =
  'Opens the folder where trace recordings are written, creating it if no trace has been recorded yet.';

/** Row "?" copy: what "Free traces" does and that it cannot be undone. */
const FREE_TRACES_HELP =
  'Permanently deletes every recorded trace from disk. This cannot be undone.';

/**
 * Renders Behavior settings: Auto search, Text Replacement toggles, then the
 * Diagnostics block.
 *
 * @param config Current raw app config from the Settings host.
 * @param resyncToken Bumps when the host reloads config from disk so fields re-seed.
 * @param onSaved Called with the resolved config after a successful field write.
 * @param highlightAutoSearchNonce Deep-link generation; 0 off, else wiggle + key.
 * @param onHighlightAutoSearchDone Fired when the highlight timeline ends.
 */
export function BehaviorTab({
  config,
  resyncToken,
  onSaved,
  highlightAutoSearchNonce = 0,
  onHighlightAutoSearchDone,
}: BehaviorTabProps) {
  const highlightActive = highlightAutoSearchNonce > 0;

  useEffect(() => {
    if (!highlightActive) return;
    const t = window.setTimeout(() => {
      onHighlightAutoSearchDone?.();
    }, POINTING_WIGGLE_MS);
    return () => window.clearTimeout(t);
  }, [highlightAutoSearchNonce, highlightActive, onHighlightAutoSearchDone]);

  // Collapsed by default: Diagnostics is a developer affordance, not part of
  // the everyday flow, so it stays folded until explicitly opened.
  const [devOpen, setDevOpen] = useState(false);
  // Two-stage guard for the destructive "Free traces" action: the button only
  // arms the modal; the actual delete runs from the modal's confirm.
  const [confirmFree, setConfirmFree] = useState(false);

  return (
    <>
      <Section heading="Web search">
        <div
          data-testid="auto-search-row"
          data-highlight={highlightActive ? 'true' : undefined}
        >
          <SaveField
            section="behavior"
            fieldKey="auto_search"
            label="Auto search"
            labelAccessory={
              <PointingWiggle
                key={highlightAutoSearchNonce}
                active={highlightActive}
                testId="auto-search-wiggle"
              />
            }
            helper={configHelp('behavior', 'auto_search')}
            initialValue={config.behavior.auto_search}
            resyncToken={resyncToken}
            onSaved={onSaved}
            rightAlign
            // Top of the panel: open the help below the "?" so it is not clipped
            // by the traffic-lights / window edge (Text Replacement uses "top").
            tooltipPlacement="bottom"
            render={(value, setValue) => (
              <Toggle
                checked={value}
                onChange={setValue}
                ariaLabel="Auto search the web when needed without /search"
              />
            )}
          />
        </div>
      </Section>

      <Section heading="Text Replacement" helper={TEXT_REPLACEMENT_HELP}>
        <SaveField
          section="behavior"
          fieldKey="auto_replace"
          label="Auto-replace"
          helper={configHelp('behavior', 'auto_replace')}
          initialValue={config.behavior.auto_replace}
          resyncToken={resyncToken}
          onSaved={onSaved}
          rightAlign
          // The tab is short, so its rows sit near the window bottom; anchor the
          // long help tooltips above the "?" so they are not clipped by the edge.
          tooltipPlacement="top"
          render={(value, setValue) => (
            <Toggle
              checked={value}
              onChange={setValue}
              ariaLabel="Auto-replace selected text after /rewrite or /refine"
            />
          )}
        />
        <SaveField
          section="behavior"
          fieldKey="auto_close"
          label="Auto-close"
          helper={configHelp('behavior', 'auto_close')}
          initialValue={config.behavior.auto_close}
          resyncToken={resyncToken}
          onSaved={onSaved}
          rightAlign
          tooltipPlacement="top"
          render={(value, setValue) => (
            <Toggle
              checked={value}
              onChange={setValue}
              ariaLabel="Close Thuki after replacing selected text"
            />
          )}
        />
      </Section>

      <div className={styles.devSection}>
        <button
          type="button"
          className={styles.devTrigger}
          aria-expanded={devOpen}
          aria-controls="dev-diagnostics"
          onClick={() => setDevOpen((o) => !o)}
        >
          <span className={styles.devTriggerLabel}>Diagnostics</span>
          <span className={styles.devTag}>DEV</span>
          <svg
            className={`${styles.devChevron} ${devOpen ? styles.devChevronOpen : ''}`}
            viewBox="0 0 10 10"
            fill="currentColor"
            aria-hidden
          >
            <path
              d="M3 2l4 3-4 3"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
              fill="none"
            />
          </svg>
        </button>
        {devOpen && (
          <div id="dev-diagnostics">
            <SaveField
              section="debug"
              fieldKey="trace_enabled"
              label="Trace recording"
              helper={configHelp('debug', 'trace_enabled')}
              initialValue={config.debug.trace_enabled}
              resyncToken={resyncToken}
              onSaved={onSaved}
              tooltipPlacement="top"
              rightAlign
              render={(value, setValue) => (
                <Toggle
                  checked={value}
                  onChange={setValue}
                  ariaLabel="Enable trace recording"
                />
              )}
            />
            <SettingRow
              label="Traces folder"
              helper={OPEN_TRACES_HELP}
              tooltipPlacement="top"
              rightAlign
            >
              <button
                type="button"
                className={`${styles.button} ${styles.buttonGhost}`}
                onClick={() => void invoke('open_traces_in_finder')}
              >
                Open traces folder
              </button>
            </SettingRow>
            <SettingRow
              label="Free traces"
              helper={FREE_TRACES_HELP}
              tooltipPlacement="top"
              rightAlign
            >
              <button
                type="button"
                className={`${styles.button} ${styles.buttonDestructive}`}
                onClick={() => setConfirmFree(true)}
              >
                Free traces…
              </button>
            </SettingRow>
          </div>
        )}
      </div>

      <ConfirmDialog
        open={confirmFree}
        title="Free all recorded traces?"
        message="Every trace recording on disk will be permanently deleted. This cannot be undone."
        confirmLabel="Free traces"
        destructive
        onConfirm={() => {
          setConfirmFree(false);
          void invoke('free_traces');
        }}
        onCancel={() => setConfirmFree(false)}
      />
    </>
  );
}
