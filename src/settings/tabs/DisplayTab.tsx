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
import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';

const FONT_WEIGHT_OPTIONS: readonly {
  value: 400 | 500 | 600 | 700;
  label: string;
}[] = [
  { value: 400, label: 'Regular' },
  { value: 500, label: 'Medium' },
  { value: 600, label: 'Semi-bold' },
  { value: 700, label: 'Bold' },
];

/**
 * Numeric font-weight dropdown. Surfaces the four loaded Nunito weights
 * with descriptive labels (Regular / Medium / Semi-bold / Bold) while
 * keeping the underlying value the numeric CSS `font-weight` the schema
 * expects. Lives in this file rather than the shared components module
 * because no other settings row needs a label-decoupled enum dropdown.
 */
function FontWeightSelect({
  value,
  onChange,
  ariaLabel,
}: {
  value: number;
  onChange: (next: number) => void;
  ariaLabel?: string;
}) {
  return (
    <select
      className={styles.dropdown}
      value={String(value)}
      aria-label={ariaLabel}
      onChange={(e) => onChange(Number(e.target.value))}
    >
      {FONT_WEIGHT_OPTIONS.map((opt) => (
        <option key={opt.value} value={String(opt.value)}>
          {opt.label}
        </option>
      ))}
    </select>
  );
}

interface DisplayTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

export function DisplayTab({ config, resyncToken, onSaved }: DisplayTabProps) {
  return (
    <>
      <Section heading="Text">
        <SaveField
          section="window"
          fieldKey="text_base_px"
          label="Text size"
          helper={configHelp('window', 'text_base_px')}
          initialValue={config.window.text_base_px}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={11}
              max={22}
              step={0.5}
              unit="px"
              onChange={setValue}
              ariaLabel="Text size"
            />
          )}
        />
        <SaveField
          section="window"
          fieldKey="text_line_height"
          label="Line height"
          helper={configHelp('window', 'text_line_height')}
          initialValue={config.window.text_line_height}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={1}
              max={2.5}
              step={0.05}
              onChange={setValue}
              ariaLabel="Line height"
            />
          )}
        />
        <SaveField
          section="window"
          fieldKey="text_letter_spacing_px"
          label="Letter spacing"
          helper={configHelp('window', 'text_letter_spacing_px')}
          initialValue={config.window.text_letter_spacing_px}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={-0.5}
              max={2}
              step={0.05}
              unit="px"
              onChange={setValue}
              ariaLabel="Letter spacing"
            />
          )}
        />
        <SaveField
          section="window"
          fieldKey="text_font_weight"
          label="Font weight"
          helper={configHelp('window', 'text_font_weight')}
          initialValue={config.window.text_font_weight}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <FontWeightSelect
              value={value}
              onChange={setValue}
              ariaLabel="Font weight"
            />
          )}
        />
      </Section>

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
      </Section>

      <Section heading="Input">
        <SaveField
          section="window"
          fieldKey="max_images"
          label="Max images"
          helper={configHelp('window', 'max_images')}
          initialValue={config.window.max_images}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberStepper
              value={value}
              min={1}
              max={20}
              onChange={setValue}
              ariaLabel="Max images"
            />
          )}
        />
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
