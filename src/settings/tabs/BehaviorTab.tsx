/**
 * Behavior tab.
 *
 * Web search mode (auto vs on-demand) and Text Replacement for `/rewrite` /
 * `/refine` (auto-replace and auto-close). The per-result Replace button is
 * always available regardless of those toggles.
 */

import { Section, Toggle } from '../components';
import { SaveField } from '../components/SaveField';
import { configHelp } from '../configHelpers';
import type { RawAppConfig } from '../types';

interface BehaviorTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

/**
 * Section-level "?" copy: what the Text Replacement group is and which commands
 * it covers. The individual toggles explain their own behavior in their own
 * tooltips, so this stays scoped to "what is this and what does it apply to".
 */
const TEXT_REPLACEMENT_HELP =
  'Applies only to /rewrite and /refine: writing their result back into the app you were using, replacing your highlighted text.';

/**
 * Section-level "?" for the Web search group: mode scope, not per-row copy.
 */
const WEB_SEARCH_HELP =
  'Controls whether the built-in engine may open the web on ordinary messages. Force a look-up anytime with /search.';

/**
 * Renders Behavior settings: Auto search, then Text Replacement toggles.
 *
 * @param config Current raw app config from the Settings host.
 * @param resyncToken Bumps when the host reloads config from disk so fields re-seed.
 * @param onSaved Called with the resolved config after a successful field write.
 */
export function BehaviorTab({
  config,
  resyncToken,
  onSaved,
}: BehaviorTabProps) {
  return (
    <>
      <Section heading="Web search" helper={WEB_SEARCH_HELP}>
        <SaveField
          section="behavior"
          fieldKey="auto_search"
          label="Auto search"
          helper={configHelp('behavior', 'auto_search')}
          initialValue={config.behavior.auto_search}
          resyncToken={resyncToken}
          onSaved={onSaved}
          rightAlign
          tooltipPlacement="top"
          render={(value, setValue) => (
            <Toggle
              checked={value}
              onChange={setValue}
              ariaLabel="Auto search the web when needed without /search"
            />
          )}
        />
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
