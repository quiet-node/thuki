/**
 * Behavior tab.
 *
 * Web search mode (auto vs on-demand) and Text Replacement for `/rewrite` /
 * `/refine` (auto-replace and auto-close). The per-result Replace button is
 * always available regardless of those toggles.
 */

import { useEffect } from 'react';
import {
  PointingWiggle,
  POINTING_WIGGLE_MS,
} from '../../components/PointingWiggle';
import { Section, Toggle } from '../components';
import { SaveField } from '../components/SaveField';
import { configHelp } from '../configHelpers';
import type { RawAppConfig } from '../types';

/** @deprecated Prefer POINTING_WIGGLE_MS; kept for existing imports/tests. */
export const AUTO_SEARCH_HIGHLIGHT_MS = POINTING_WIGGLE_MS;

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

/**
 * Renders Behavior settings: Auto search, then Text Replacement toggles.
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
    </>
  );
}
