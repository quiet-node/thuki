/**
 * Behavior tab.
 *
 * Web search mode (auto vs on-demand) and Text Replacement for `/rewrite` /
 * `/refine` (auto-replace and auto-close). The per-result Replace button is
 * always available regardless of those toggles.
 */

import { useEffect } from 'react';
import { Section, Toggle } from '../components';
import { SaveField } from '../components/SaveField';
import { configHelp } from '../configHelpers';
import type { RawAppConfig } from '../types';
import styles from '../../styles/settings.module.css';

/**
 * Full deep-link highlight timeline for Auto search (design D): draw, settle,
 * three soft breaths, fade. Must match `autoSearchWiggleLife` in CSS.
 */
export const AUTO_SEARCH_HIGHLIGHT_MS = 7200;

interface BehaviorTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
  /** When true, play the Auto search label wiggle highlight. */
  highlightAutoSearch?: boolean;
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
 * Hand-drawn squiggle path under the Auto search label (viewBox 0 0 100 10).
 * pathLength is set in SVG so stroke-dash animation is length-independent.
 */
const WIGGLE_PATH_D =
  'M1.5 6.2 C 8 3.8, 12 7.5, 18 5.5 S 28 2.8, 34 5.8 S 44 8.2, 50 5.2 S 60 2.5, 68 5.9 S 80 8.5, 88 4.8 S 95 6.5, 98.5 5.2';

/**
 * Renders Behavior settings: Auto search, then Text Replacement toggles.
 *
 * @param config Current raw app config from the Settings host.
 * @param resyncToken Bumps when the host reloads config from disk so fields re-seed.
 * @param onSaved Called with the resolved config after a successful field write.
 * @param highlightAutoSearch When true, draws the design-D wiggle under Auto search.
 * @param onHighlightAutoSearchDone Fired when the highlight timeline ends.
 */
export function BehaviorTab({
  config,
  resyncToken,
  onSaved,
  highlightAutoSearch = false,
  onHighlightAutoSearchDone,
}: BehaviorTabProps) {
  useEffect(() => {
    if (!highlightAutoSearch) return;
    const t = window.setTimeout(() => {
      onHighlightAutoSearchDone?.();
    }, AUTO_SEARCH_HIGHLIGHT_MS);
    return () => window.clearTimeout(t);
  }, [highlightAutoSearch, onHighlightAutoSearchDone]);

  /**
   * Label accessory: animated squiggle only while deep-link highlight is on.
   */
  const autoSearchLabelAccessory = highlightAutoSearch ? (
    <svg
      className={styles.autoSearchWiggle}
      viewBox="0 0 100 10"
      preserveAspectRatio="none"
      aria-hidden="true"
      data-testid="auto-search-wiggle"
    >
      <path
        className={styles.autoSearchWigglePath}
        pathLength={1}
        d={WIGGLE_PATH_D}
      />
    </svg>
  ) : null;

  return (
    <>
      <Section heading="Web search">
        <div
          data-testid="auto-search-row"
          data-highlight={highlightAutoSearch ? 'true' : undefined}
        >
          <SaveField
            section="behavior"
            fieldKey="auto_search"
            label="Auto search"
            labelAccessory={autoSearchLabelAccessory}
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
