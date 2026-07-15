/**
 * Behavior tab.
 *
 * Web search mode (auto vs on-demand), Text Replacement for `/rewrite` /
 * `/refine` (auto-replace and auto-close; the per-result Replace button is
 * always available regardless of those toggles), and a collapsible
 * Diagnostics block (trace recording, on-disk retention, plus open / free
 * actions for the on-disk trace folder).
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import {
  PointingWiggle,
  POINTING_WIGGLE_MS,
} from '../../components/PointingWiggle';
import {
  ConfirmDialog,
  prefersReducedMotion,
  Section,
  SettingRow,
  Toggle,
} from '../components';
import { DrawCheckIcon } from '../../components/DrawCheckIcon';
import { SaveField } from '../components/SaveField';
import { useDebouncedSave } from '../hooks/useDebouncedSave';
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
 * How long the "Free traces" button shows its drawn green tick after a
 * successful delete before settling into the disabled/grey empty state.
 *
 * Sized to clear the full `DrawCheckIcon` draw and add a brief dwell: the ring
 * animates 0..550ms and the check 450..750ms (see `keepWarmCircleAnim` /
 * `keepWarmCheckAnim` in `settings.module.css`), so anything under ~750ms would
 * cut the checkmark mid-stroke. This holds the completed tick for ~450ms more.
 */
export const FREE_SUCCESS_HOLD_MS = 1200;

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
  // Numeric trace count driving the "Free traces" disabled state. `null` means
  // unknown (still loading, or the stats fetch failed): the button stays
  // enabled then, since `free_traces` is a safe no-op on an empty dir. `0`
  // greys the button; any positive value keeps it active.
  const [tracesCount, setTracesCount] = useState<number | null>(null);
  // Transient success flag: true while the "Free traces" button draws its green
  // tick after a successful delete, then cleared so it settles into the grey
  // empty state (count is 0 by then).
  const [freeSuccess, setFreeSuccess] = useState(false);
  const freeSuccessTimerRef = useRef<number | null>(null);

  // Trace retention days (debounced save). Mirrors the Keep Warm "Release
  // after" numeric input: a raw string backs the field so a partial "-" or a
  // literal "-1" can be typed, and the committed integer is written through
  // the same `set_config_field` path every other row uses. The backend loader
  // is the authoritative clamp (`-1` kept, `1..=3650` kept, anything else
  // reset to the default); the frontend only bounds the value to the input's
  // advertised range.
  const [retentionDays, setRetentionDays] = useState(
    config.debug.trace_retention_days,
  );
  const [rawRetention, setRawRetention] = useState(
    String(config.debug.trace_retention_days),
  );
  const retentionFocusedRef = useRef(false);
  const { resetTo: resetRetention } = useDebouncedSave(
    'debug',
    'trace_retention_days',
    retentionDays,
    { onSaved },
  );

  // Re-seed the retention field from an external resync (e.g. after the loader
  // clamps a saved value back to a valid one), but never while the user is
  // mid-edit, so a background reload cannot clobber an in-progress keystroke.
  const prevTokenRef = useRef(resyncToken);
  if (prevTokenRef.current !== resyncToken) {
    prevTokenRef.current = resyncToken;
    if (!retentionFocusedRef.current) {
      setRetentionDays(config.debug.trace_retention_days);
      setRawRetention(String(config.debug.trace_retention_days));
      resetRetention(config.debug.trace_retention_days);
    }
  }

  // Reads the live trace footprint and formats it for the subtext. Kept in a
  // callback so both the "on expand" effect and the post-delete refresh share
  // one code path.
  const loadTracesStats = useCallback(async () => {
    try {
      const stats = await invoke<TracesStats>('traces_stats');
      setTracesSubtext(formatTracesSubtext(stats.count, stats.bytes));
      setTracesCount(stats.count);
    } catch {
      // Hide the subtext rather than surfacing a raw error in the UI, and drop
      // the count back to unknown so the button re-enables (never trap the user
      // on a stale zero after a failed probe).
      setTracesSubtext(null);
      setTracesCount(null);
    }
  }, []);

  // Runs the Free-traces success draw: a brief green tick, then settle. Skipped
  // entirely under reduced motion so the button jumps straight to grey rather
  // than flashing a half-drawn tick.
  const startFreeSuccess = useCallback(() => {
    if (prefersReducedMotion()) return;
    setFreeSuccess(true);
    freeSuccessTimerRef.current = window.setTimeout(() => {
      setFreeSuccess(false);
    }, FREE_SUCCESS_HOLD_MS);
  }, []);

  // Clear a pending success-hold timer if the tab unmounts mid-animation.
  useEffect(() => {
    return () => {
      if (freeSuccessTimerRef.current !== null) {
        window.clearTimeout(freeSuccessTimerRef.current);
      }
    };
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
              label="Retention"
              helper={configHelp('debug', 'trace_retention_days')}
              tooltipPlacement="top"
              rightAlign
            >
              <span className={styles.genWarmControls}>
                <input
                  type="number"
                  className={styles.keepWarmNumberInput}
                  value={rawRetention}
                  min={-1}
                  max={3650}
                  aria-label="Days to keep recorded traces"
                  onFocus={() => {
                    retentionFocusedRef.current = true;
                  }}
                  onChange={(e) => {
                    const n = parseInt(e.target.value, 10);
                    if (Number.isNaN(n)) {
                      setRawRetention(e.target.value);
                    } else {
                      const clamped = Math.max(-1, Math.min(3650, n));
                      setRawRetention(String(clamped));
                      setRetentionDays(clamped);
                    }
                  }}
                  onBlur={() => {
                    retentionFocusedRef.current = false;
                    // A left-empty or otherwise unparseable field reverts to the
                    // last committed value rather than writing a stray number.
                    if (Number.isNaN(parseInt(rawRetention, 10))) {
                      setRawRetention(String(retentionDays));
                    }
                  }}
                />
                <span className={styles.keepWarmUnit}>days</span>
              </span>
            </SettingRow>
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
                  data-freed={freeSuccess ? 'true' : undefined}
                  disabled={freeSuccess || tracesCount === 0}
                  aria-label={freeSuccess ? 'Traces freed' : undefined}
                  onClick={() => setConfirmFree(true)}
                >
                  {freeSuccess ? <DrawCheckIcon /> : 'Free traces…'}
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
          // escapes into an unhandled promise. Only a clean delete plays the
          // success tick; a failure just resyncs quietly.
          void (async () => {
            let freed = false;
            try {
              await invoke('free_traces');
              freed = true;
            } catch {
              // Deletion failed; the reload below reflects the real state.
            }
            await loadTracesStats();
            if (freed) startFreeSuccess();
          })();
        }}
        onCancel={() => setConfirmFree(false)}
      />
    </>
  );
}
