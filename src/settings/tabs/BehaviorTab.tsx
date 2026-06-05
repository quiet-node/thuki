/**
 * Behavior tab.
 *
 * Settings that control how Thuki acts after you invoke it. Currently the
 * Text Replacement group: whether a `/rewrite` or `/refine` result is written
 * straight back into the source app, replacing your selection (auto-replace).
 * The per-result Replace button is always available regardless of this toggle.
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

export function BehaviorTab({
  config,
  resyncToken,
  onSaved,
}: BehaviorTabProps) {
  return (
    <Section heading="Text Replacement">
      <SaveField
        section="behavior"
        fieldKey="auto_replace"
        label="Auto-replace"
        helper={configHelp('behavior', 'auto_replace')}
        initialValue={config.behavior.auto_replace}
        resyncToken={resyncToken}
        onSaved={onSaved}
        rightAlign
        // The tab is short (a single row near the window bottom); anchor the
        // long help tooltip above the "?" so it is not clipped by the edge.
        tooltipPlacement="top"
        render={(value, setValue) => (
          <Toggle
            checked={value}
            onChange={setValue}
            ariaLabel="Auto-replace selected text after /rewrite or /refine"
          />
        )}
      />
    </Section>
  );
}
