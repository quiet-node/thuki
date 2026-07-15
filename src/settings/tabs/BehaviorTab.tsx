/**
 * Behavior tab.
 *
 * Web search mode (auto vs on-demand), Text Replacement for `/rewrite` /
 * `/refine` (auto-replace and auto-close; the per-result Replace button is
 * always available regardless of those toggles), and a collapsible
 * Diagnostics block (trace recording plus open / free actions for the
 * on-disk trace folder).
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import {
  PointingWiggle,
  POINTING_WIGGLE_MS,
} from '../../components/PointingWiggle';
import { ConfirmDialog, Section, SettingRow, Toggle } from '../components';
import { SaveField } from '../components/SaveField';
import { configHelp } from '../configHelpers';
import { formatTracesSubtext } from '../../utils/formatTracesSubtext';
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

/** Serde shape returned by the `traces_stats` Tauri command. */
interface TracesStats {
  count: number;
  bytes: number;
}

/**
 * Section-level "?" copy: what the Text Replacement group is and which commands
 * it covers. The individual toggles explain their own behavior in their own
 * tooltips, so this stays scoped to "what is this and what does it apply to".
 */
const TEXT_REPLACEMENT_HELP =
  'Applies only to /rewrite and /refine: writing their result back into the app you were using, replacing your highlighted text.';

/** Row "?" copy for the Traces action bar: what the two actions do. */
const TRACES_HELP =
  'Open the folder where trace recordings are written, or permanently delete every recorded trace from disk (this cannot be undone).';

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
  // On-disk footprint subtext. `null` hides the line (before the first load
  // resolves, and on any invoke failure, so a backend error never throws into
  // render).
  const [tracesSubtext, setTracesSubtext] = useState<string | null>(null);

  // Reads the live trace footprint and formats it for the subtext. Kept in a
  // callback so both the "on expand" effect and the post-delete refresh share
  // one code path.
  const loadTracesStats = useCallback(async () => {
    try {
      const stats = await invoke<TracesStats>('traces_stats');
      setTracesSubtext(formatTracesSubtext(stats.count, stats.bytes));
    } catch {
      // Hide the subtext rather than surfacing a raw error in the UI.
      setTracesSubtext(null);
    }
  }, []);

  // Fetch the footprint only once the Diagnostics block is opened; a collapsed
  // block never touches disk.
  useEffect(() => {
    if (devOpen) void loadTracesStats();
  }, [devOpen, loadTracesStats]);

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
              label="Traces"
              helper={TRACES_HELP}
              tooltipPlacement="top"
              rightAlign
            >
              <div className={styles.tracesActions}>
                <button
                  type="button"
                  className={`${styles.button} ${styles.buttonGhost}`}
                  onClick={() => void invoke('open_traces_in_finder')}
                >
                  Open traces folder
                </button>
                <button
                  type="button"
                  className={`${styles.button} ${styles.buttonDestructive}`}
                  onClick={() => setConfirmFree(true)}
                >
                  Free traces…
                </button>
              </div>
              {tracesSubtext !== null ? (
                <div className={styles.tracesSubtext}>{tracesSubtext}</div>
              ) : null}
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
          // Delete, then refresh the footprint so the subtext drops to the
          // empty state. Swallow a delete failure and reload regardless, so the
          // subtext always resyncs to the true on-disk state and no rejection
          // escapes into an unhandled promise.
          void (async () => {
            try {
              await invoke('free_traces');
            } catch {
              // Deletion failed; the reload below reflects the real state.
            }
            await loadTracesStats();
          })();
        }}
        onCancel={() => setConfirmFree(false)}
      />
    </>
  );
}
