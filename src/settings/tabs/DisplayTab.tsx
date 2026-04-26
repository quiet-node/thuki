/**
 * Display tab — appearance + presentation knobs.
 *
 * Holds the floating-window dimensions, the close-animation timing,
 * and the quoted-text preview limits. These were split out of the old
 * "General" tab so the AI tab can focus on the AI brain.
 */

import { Section, NumberSlider, NumberStepper } from '../components';
import { SaveField } from '../components/SaveField';
import { configHelp } from '../configHelpers';
import type { RawAppConfig } from '../types';

interface DisplayTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

export function DisplayTab({ config, resyncToken, onSaved }: DisplayTabProps) {
  return (
    <>
      <Section heading="Window">
        <SaveField
          section="window"
          fieldKey="overlay_width"
          label="Overlay width"
          helper={configHelp('window', 'overlay_width')}
          initialValue={config.window.overlay_width}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={200}
              max={2000}
              step={10}
              unit="px"
              onChange={setValue}
              ariaLabel="Overlay width"
            />
          )}
        />
        <SaveField
          section="window"
          fieldKey="collapsed_height"
          label="Collapsed height"
          helper={configHelp('window', 'collapsed_height')}
          initialValue={config.window.collapsed_height}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={40}
              max={400}
              step={5}
              unit="px"
              onChange={setValue}
              ariaLabel="Collapsed height"
            />
          )}
        />
        <SaveField
          section="window"
          fieldKey="max_chat_height"
          label="Max chat height"
          helper={configHelp('window', 'max_chat_height')}
          initialValue={config.window.max_chat_height}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={200}
              max={2000}
              step={10}
              unit="px"
              onChange={setValue}
              ariaLabel="Max chat height"
            />
          )}
        />
        <SaveField
          section="window"
          fieldKey="hide_commit_delay_ms"
          label="Hide-commit delay"
          helper={configHelp('window', 'hide_commit_delay_ms')}
          initialValue={config.window.hide_commit_delay_ms}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={0}
              max={5000}
              step={50}
              unit="ms"
              onChange={setValue}
              ariaLabel="Hide-commit delay"
            />
          )}
        />
      </Section>

      <Section heading="Quote">
        <SaveField
          section="quote"
          fieldKey="max_display_lines"
          label="Max display lines"
          helper={configHelp('quote', 'max_display_lines')}
          initialValue={config.quote.max_display_lines}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberStepper
              value={value}
              min={1}
              max={100}
              onChange={setValue}
              ariaLabel="Max display lines"
            />
          )}
        />
        <SaveField
          section="quote"
          fieldKey="max_display_chars"
          label="Max display chars"
          helper={configHelp('quote', 'max_display_chars')}
          initialValue={config.quote.max_display_chars}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberStepper
              value={value}
              min={1}
              max={10000}
              step={50}
              onChange={setValue}
              ariaLabel="Max display chars"
            />
          )}
        />
        <SaveField
          section="quote"
          fieldKey="max_context_length"
          label="Max context length"
          helper={configHelp('quote', 'max_context_length')}
          initialValue={config.quote.max_context_length}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberStepper
              value={value}
              min={1}
              max={65536}
              step={256}
              onChange={setValue}
              ariaLabel="Max context length"
            />
          )}
        />
      </Section>
    </>
  );
}
