/**
 * Behavior tab.
 *
 * Web search mode (auto vs on-demand), Text Replacement for `/rewrite` /
 * `/refine` (auto-replace and auto-close; the per-result Replace button is
 * always available regardless of those toggles), chat History (auto-save,
 * retention with confirm, Free chats), and a collapsible Diagnostics block
 * (trace recording, on-disk retention, plus open / free actions for the
 * on-disk trace folder).
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emit } from '@tauri-apps/api/event';

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
import { formatHistorySubtext } from '../../utils/formatHistorySubtext';
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
  /**
   * Deep-link highlight for Auto-save chats. Same nonce contract as
   * `highlightAutoSearchNonce`.
   */
  highlightAutoSaveNonce?: number;
  /** Called after the Auto-save highlight animation completes. */
  onHighlightAutoSaveDone?: () => void;
}

/** Serde shape returned by `traces_stats` / `history_stats` Tauri commands. */
interface FootprintStats {
  count: number;
  bytes: number;
}

/**
 * How long Free traces / Free chats show the drawn green tick after a
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

/** Section "?" for chat History (auto-save + retention + Free chats). */
const HISTORY_SECTION_HELP =
  'Local SQLite history for chats you keep. Auto-save writes completed turns without a bookmark click; retention prunes by last activity; Free chats wipes every saved chat.';

/** Row "?" for the Free chats action: irreversible wipe of local history. */
const FREE_CHATS_HELP =
  'Permanently delete every chat stored in local history. This cannot be undone.';

/** Broadcast after Free chats succeeds so the overlay drops its saved identity. */
export const HISTORY_CLEARED_EVENT = 'thuki://history-cleared';

/**
 * Inline helper when the history retention field holds invalid `0` (backend
 * would reset; forever is `-1`, finite is `1..=3650`).
 */
export const HISTORY_RETENTION_ZERO_ERROR =
  'Use -1 for forever, or 1–3650 days.';

/**
 * Renders Behavior settings: Auto search, Text Replacement, History, then the
 * Diagnostics block.
 *
 * @param config Current raw app config from the Settings host.
 * @param resyncToken Bumps when the host reloads config from disk so fields re-seed.
 * @param onSaved Called with the resolved config after a successful field write.
 * @param highlightAutoSearchNonce Deep-link generation; 0 off, else wiggle + key.
 * @param onHighlightAutoSearchDone Fired when the highlight timeline ends.
 * @param highlightAutoSaveNonce Deep-link for Auto-save row; 0 off.
 * @param onHighlightAutoSaveDone Fired when the Auto-save highlight ends.
 */
export function BehaviorTab({
  config,
  resyncToken,
  onSaved,
  highlightAutoSearchNonce = 0,
  onHighlightAutoSearchDone,
  highlightAutoSaveNonce = 0,
  onHighlightAutoSaveDone,
}: BehaviorTabProps) {
  const highlightActive = highlightAutoSearchNonce > 0;
  const highlightAutoSaveActive = highlightAutoSaveNonce > 0;

  useEffect(() => {
    if (!highlightActive) return;
    const t = window.setTimeout(() => {
      onHighlightAutoSearchDone?.();
    }, POINTING_WIGGLE_MS);
    return () => window.clearTimeout(t);
  }, [highlightAutoSearchNonce, highlightActive, onHighlightAutoSearchDone]);

  useEffect(() => {
    if (!highlightAutoSaveActive) return;
    const t = window.setTimeout(() => {
      onHighlightAutoSaveDone?.();
    }, POINTING_WIGGLE_MS);
    return () => window.clearTimeout(t);
  }, [
    highlightAutoSaveNonce,
    highlightAutoSaveActive,
    onHighlightAutoSaveDone,
  ]);

  // Collapsed by default: Diagnostics is a developer affordance, not part of
  // the everyday flow, so it stays folded until explicitly opened.
  const [devOpen, setDevOpen] = useState(false);
  // Two-stage guard for the destructive "Free traces" action: the button only
  // arms the modal; the actual delete runs from the modal's confirm.
  const [confirmFree, setConfirmFree] = useState(false);
  // Free chats: same confirm-before-mutate pattern as Free traces.
  const [confirmFreeChats, setConfirmFreeChats] = useState(false);
  // Pending finite retention change awaiting ConfirmDialog; null when idle.
  const [pendingHistoryRetention, setPendingHistoryRetention] = useState<
    number | null
  >(null);
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

  // History footprint subtext + count (same null/unknown contract as traces).
  const [historySubtext, setHistorySubtext] = useState<string | null>(null);
  const [historyCount, setHistoryCount] = useState<number | null>(null);
  // Transient success flag for Free chats (green tick hold, then grey empty).
  const [freeChatsSuccess, setFreeChatsSuccess] = useState(false);
  const freeChatsSuccessTimerRef = useRef<number | null>(null);

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

  // History retention: committed value only writes after Confirm when the new
  // value is finite (would delete). Forever (-1) applies immediately. Draft
  // state reverts on Cancel so Cancel never mutates config or prunes.
  const [historyRetentionCommitted, setHistoryRetentionCommitted] = useState(
    config.behavior.history_retention_days,
  );
  const [rawHistoryRetention, setRawHistoryRetention] = useState(
    String(config.behavior.history_retention_days),
  );
  // Validation message for invalid drafts (currently only `0`); null when clean.
  const [historyRetentionError, setHistoryRetentionError] = useState<
    string | null
  >(null);
  const historyRetentionFocusedRef = useRef(false);
  // Enter already applied the draft; skip the synthetic blur re-apply.
  const skipHistoryRetentionBlurRef = useRef(false);

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
    if (!historyRetentionFocusedRef.current) {
      setHistoryRetentionCommitted(config.behavior.history_retention_days);
      setRawHistoryRetention(String(config.behavior.history_retention_days));
      setHistoryRetentionError(null);
    }
  }

  /**
   * Loads saved-chat count + content/image footprint for the History subtext.
   * Shared by mount, post-free, and post-retention prune refresh paths.
   */
  const loadHistoryStats = useCallback(async () => {
    try {
      const stats = await invoke<FootprintStats>('history_stats');
      setHistorySubtext(formatHistorySubtext(stats.count, stats.bytes));
      setHistoryCount(stats.count);
    } catch {
      // Hide subtext; drop count to unknown so Free chats stays enabled (same
      // as Free traces when the probe fails).
      setHistorySubtext(null);
      setHistoryCount(null);
    }
  }, []);

  /**
   * Writes a history retention value and, for finite windows, prunes immediately.
   * Forever (`-1`) only updates config (nothing is deleted).
   *
   * @param days Clamped retention days (`-1` or `1..=3650`).
   */
  const commitHistoryRetention = useCallback(
    async (days: number) => {
      try {
        const next = await invoke<RawAppConfig>('set_config_field', {
          section: 'behavior',
          key: 'history_retention_days',
          value: days,
        });
        setHistoryRetentionCommitted(next.behavior.history_retention_days);
        setRawHistoryRetention(String(next.behavior.history_retention_days));
        onSaved(next);
        if (next.behavior.history_retention_days >= 1) {
          await invoke('prune_conversation_history');
          // Prune may have deleted chats; resync footprint for Free chats.
          await loadHistoryStats();
        }
      } catch {
        // Revert draft to last committed so a failed write never looks applied.
        setRawHistoryRetention(String(historyRetentionCommitted));
      }
    },
    [historyRetentionCommitted, loadHistoryStats, onSaved],
  );

  /**
   * Applies a draft retention value from blur or Enter.
   *
   * Forever (`-1`) writes immediately. Finite values that are stricter than
   * the committed window (including forever → finite, or fewer days) probe
   * `history_retention_prune_count`: open ConfirmDialog only when ≥1 chat
   * would be deleted; otherwise commit + prune immediately (no empty
   * destructive confirm). Probe failure fails closed (still confirm).
   * Dialog open stays on a microtask so the same Enter keystroke cannot
   * activate Acknowledge. Lengthening a finite window (e.g. 7 → 30) applies
   * without the prune dialog; prune still runs and is a no-op delete.
   * Equal-to-committed values are no-ops. Invalid `0` keeps the raw field and
   * shows an inline error; it never commits or opens confirm.
   *
   * @param days Parsed integer from the retention input.
   */
  const applyHistoryRetentionDraft = useCallback(
    (days: number) => {
      const clamped = Math.max(-1, Math.min(3650, days));
      if (clamped === 0) {
        // 0 is invalid (loader would reset). Keep "0" visible + error; no write.
        setRawHistoryRetention('0');
        setHistoryRetentionError(HISTORY_RETENTION_ZERO_ERROR);
        return;
      }
      setHistoryRetentionError(null);
      if (clamped === historyRetentionCommitted) {
        setRawHistoryRetention(String(clamped));
        return;
      }
      if (clamped === -1) {
        void commitHistoryRetention(-1);
        return;
      }
      // Stricter finite window (or forever → finite): confirm only if prune would delete.
      const shortening =
        historyRetentionCommitted === -1 || clamped < historyRetentionCommitted;
      setRawHistoryRetention(String(clamped));
      if (shortening) {
        void (async () => {
          try {
            const n = await invoke<number>('history_retention_prune_count', {
              days: clamped,
            });
            if (n > 0) {
              // Defer open so Enter that submitted the field cannot hit the new button.
              queueMicrotask(() => setPendingHistoryRetention(clamped));
            } else {
              await commitHistoryRetention(clamped);
            }
          } catch {
            // Fail closed: still confirm if we cannot probe.
            queueMicrotask(() => setPendingHistoryRetention(clamped));
          }
        })();
        return;
      }
      // Lengthening: write + prune without ConfirmDialog.
      void commitHistoryRetention(clamped);
    },
    [commitHistoryRetention, historyRetentionCommitted],
  );

  // Reads the live trace footprint and formats it for the subtext. Kept in a
  // callback so both the "on expand" effect and the post-delete refresh share
  // one code path.
  const loadTracesStats = useCallback(async () => {
    try {
      const stats = await invoke<FootprintStats>('traces_stats');
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

  /**
   * Green tick hold for Free chats after a successful wipe (same timing as
   * Free traces). Skipped under reduced motion.
   */
  const startFreeChatsSuccess = useCallback(() => {
    if (prefersReducedMotion()) return;
    setFreeChatsSuccess(true);
    freeChatsSuccessTimerRef.current = window.setTimeout(() => {
      setFreeChatsSuccess(false);
    }, FREE_SUCCESS_HOLD_MS);
  }, []);

  /**
   * Deletes on-disk traces, resyncs the footprint subtext, and plays the
   * success tick only when the delete itself succeeded.
   */
  const freeAllTraces = useCallback(async () => {
    let freed = false;
    try {
      await invoke('free_traces');
      freed = true;
    } catch {
      // Deletion failed; the reload below reflects the real state.
    }
    await loadTracesStats();
    if (freed) startFreeSuccess();
  }, [loadTracesStats, startFreeSuccess]);

  /**
   * Wipes every saved chat after ConfirmDialog. On success: refresh stats,
   * play the success tick first (so a failed `emit` never steals the green
   * check), then broadcast `HISTORY_CLEARED_EVENT` so the live chat drops its
   * saved identity without a second delete.
   */
  const freeAllChats = useCallback(async () => {
    let freed = false;
    try {
      await invoke('clear_all_conversations');
      freed = true;
    } catch {
      // Surface nothing destructive-looking; user can retry.
    }
    await loadHistoryStats();
    if (freed) {
      // Visual success first (same priority as Free traces). Emit is best-effort.
      startFreeChatsSuccess();
      try {
        await emit(HISTORY_CLEARED_EVENT);
      } catch {
        // Overlay may miss the clear signal; wipe itself already succeeded.
      }
    }
  }, [loadHistoryStats, startFreeChatsSuccess]);

  // Clear pending success-hold timers if the tab unmounts mid-animation.
  useEffect(() => {
    return () => {
      if (freeSuccessTimerRef.current !== null) {
        window.clearTimeout(freeSuccessTimerRef.current);
      }
      if (freeChatsSuccessTimerRef.current !== null) {
        window.clearTimeout(freeChatsSuccessTimerRef.current);
      }
    };
  }, []);

  // History is always visible on Behavior: load footprint on mount.
  useEffect(() => {
    void loadHistoryStats();
  }, [loadHistoryStats]);

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
          label="Auto replace"
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
          label="Auto close"
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

      <Section heading="History" helper={HISTORY_SECTION_HELP}>
        <div
          data-testid="auto-save-conversations-row"
          data-highlight={highlightAutoSaveActive ? 'true' : undefined}
        >
          <SaveField
            section="behavior"
            fieldKey="auto_save_conversations"
            label="Auto save"
            labelAccessory={
              <PointingWiggle
                key={highlightAutoSaveNonce}
                active={highlightAutoSaveActive}
                testId="auto-save-wiggle"
              />
            }
            helper={configHelp('behavior', 'auto_save_conversations')}
            initialValue={config.behavior.auto_save_conversations}
            resyncToken={resyncToken}
            onSaved={onSaved}
            rightAlign
            tooltipPlacement="top"
            render={(value, setValue) => (
              <Toggle
                checked={value}
                onChange={setValue}
                ariaLabel="Auto-save completed chats to history"
              />
            )}
          />
        </div>
        <SettingRow
          label="Retention"
          helper={configHelp('behavior', 'history_retention_days')}
          tooltipPlacement="top"
          rightAlign
        >
          <span className={styles.genWarmControls}>
            <input
              type="number"
              className={`${styles.keepWarmNumberInput}${
                historyRetentionError ? ` ${styles.inputError}` : ''
              }`}
              value={rawHistoryRetention}
              min={-1}
              max={3650}
              aria-label="Days to keep saved chats"
              aria-invalid={historyRetentionError ? true : undefined}
              data-testid="history-retention-input"
              onFocus={() => {
                historyRetentionFocusedRef.current = true;
              }}
              onChange={(e) => {
                const n = parseInt(e.target.value, 10);
                if (Number.isNaN(n)) {
                  setRawHistoryRetention(e.target.value);
                  return;
                }
                const clamped = Math.max(-1, Math.min(3650, n));
                setRawHistoryRetention(String(clamped));
                // Clear stale error as soon as the draft is a valid number.
                if (clamped !== 0) setHistoryRetentionError(null);
              }}
              onBlur={(e) => {
                historyRetentionFocusedRef.current = false;
                if (skipHistoryRetentionBlurRef.current) {
                  skipHistoryRetentionBlurRef.current = false;
                  return;
                }
                // Parse from the DOM value, not React state: blur can race the
                // last onChange setState and commit a stale draft otherwise.
                const n = parseInt(e.currentTarget.value, 10);
                if (Number.isNaN(n)) {
                  setRawHistoryRetention(String(historyRetentionCommitted));
                  setHistoryRetentionError(null);
                  return;
                }
                applyHistoryRetentionDraft(n);
              }}
              onKeyDown={(e) => {
                if (e.key !== 'Enter') return;
                // PreventDefault so Enter cannot also activate a newly focused
                // dialog button; apply from currentTarget (not blur-only hope).
                e.preventDefault();
                e.stopPropagation();
                historyRetentionFocusedRef.current = false;
                const n = parseInt(e.currentTarget.value, 10);
                if (Number.isNaN(n)) {
                  setRawHistoryRetention(String(historyRetentionCommitted));
                  setHistoryRetentionError(null);
                  skipHistoryRetentionBlurRef.current = true;
                  e.currentTarget.blur();
                  return;
                }
                applyHistoryRetentionDraft(n);
                // Blur for chrome/focus only; commit already ran above.
                skipHistoryRetentionBlurRef.current = true;
                e.currentTarget.blur();
              }}
            />
            <span className={styles.keepWarmUnit}>days</span>
          </span>
          {historyRetentionError ? (
            <div
              className={styles.rowError}
              role="alert"
              data-testid="history-retention-error"
            >
              {historyRetentionError}
            </div>
          ) : null}
        </SettingRow>
        <SettingRow
          label="Chats"
          helper={FREE_CHATS_HELP}
          tooltipPlacement="top"
          rightAlign
        >
          <div className={styles.tracesActions}>
            <button
              type="button"
              className={`${styles.button} ${styles.buttonDestructive}`}
              data-testid="clear-all-history"
              data-freed={freeChatsSuccess ? 'true' : undefined}
              disabled={freeChatsSuccess || historyCount === 0}
              aria-label={freeChatsSuccess ? 'Chats freed' : undefined}
              onClick={() => setConfirmFreeChats(true)}
            >
              {freeChatsSuccess ? <DrawCheckIcon /> : 'Free chats…'}
            </button>
          </div>
          {historySubtext !== null ? (
            <div className={styles.tracesSubtext}>{historySubtext}</div>
          ) : null}
        </SettingRow>
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
              label="Auto record"
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
          void freeAllTraces();
        }}
        onCancel={() => setConfirmFree(false)}
      />

      <ConfirmDialog
        open={pendingHistoryRetention !== null}
        title="Shorten chat history retention?"
        message={
          pendingHistoryRetention === null
            ? ''
            : `Chats with last activity older than ${pendingHistoryRetention} day${pendingHistoryRetention === 1 ? '' : 's'} will be permanently deleted. This cannot be undone. Back up anything you need first.`
        }
        confirmLabel="Acknowledge"
        destructive
        onConfirm={() => {
          // Dialog only opens when pending is set; capture then clear before
          // the async write so a second click cannot double-confirm.
          const days = pendingHistoryRetention as number;
          setPendingHistoryRetention(null);
          setHistoryRetentionError(null);
          void commitHistoryRetention(days);
        }}
        onCancel={() => {
          setPendingHistoryRetention(null);
          setRawHistoryRetention(String(historyRetentionCommitted));
          setHistoryRetentionError(null);
        }}
      />

      <ConfirmDialog
        open={confirmFreeChats}
        title="Free all saved chats?"
        message="Every saved chat will be permanently deleted from this Mac. This cannot be undone."
        confirmLabel="Free chats"
        destructive
        onConfirm={() => {
          setConfirmFreeChats(false);
          // Wipe, then refresh footprint so the subtext drops to empty. Only a
          // clean delete plays the success tick and emits history-cleared.
          void freeAllChats();
        }}
        onCancel={() => setConfirmFreeChats(false)}
      />
    </>
  );
}
