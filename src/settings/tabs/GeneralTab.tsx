/**
 * General tab — model, prompt, window, and quote settings.
 *
 * Renders the four sub-sections (MODEL / PROMPT / WINDOW / QUOTE) defined
 * in the design doc. Section order is intentional: MODEL above the fold
 * because model swap is the most-frequent reason to open Settings (P1
 * frequency principle).
 */

import { useState } from 'react';

import { invoke } from '@tauri-apps/api/core';

import {
  Section,
  ResetSectionLink,
  NumberSlider,
  NumberStepper,
  TextField,
  Textarea,
  OrderedListEditor,
  ConfirmDialog,
} from '../components';
import { SaveField } from '../components/SaveField';
import type { RawAppConfig } from '../types';

interface GeneralTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

const PROMPT_MAX_CHARS = 8000;

export function GeneralTab({ config, resyncToken, onSaved }: GeneralTabProps) {
  const [confirmReset, setConfirmReset] = useState(false);

  const promptCharCount = config.prompt.system.length;

  return (
    <>
      <Section heading="Model">
        <SaveField
          section="model"
          fieldKey="available"
          label="Active Ollama model"
          helper="First entry is active. Reorder to switch. Click ✕ to remove."
          vertical
          initialValue={config.model.available}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <OrderedListEditor
              items={value}
              onChange={setValue}
              emptyMessage="No models. Run `ollama pull <name>` to add one."
            />
          )}
        />
        <SaveField
          section="model"
          fieldKey="ollama_url"
          label="Ollama URL"
          helper="Web address of your local Ollama server. Default works if Ollama runs on this machine on its standard port."
          initialValue={config.model.ollama_url}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue, errored) => (
            <TextField
              value={value}
              onChange={setValue}
              placeholder="http://127.0.0.1:11434"
              errored={errored}
              ariaLabel="Ollama URL"
            />
          )}
        />
      </Section>

      <Section heading="Prompt">
        <SaveField
          section="prompt"
          fieldKey="system"
          label="System prompt"
          helper={
            <>
              Custom personality or instructions. Leave empty to use the
              built-in secretary persona. <strong>{promptCharCount}</strong> /{' '}
              {PROMPT_MAX_CHARS} characters.
            </>
          }
          vertical
          initialValue={config.prompt.system}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <Textarea
              value={value}
              onChange={setValue}
              placeholder="Use built-in secretary persona…"
              maxLength={PROMPT_MAX_CHARS}
              ariaLabel="System prompt"
            />
          )}
        />
      </Section>

      <Section heading="Window">
        <SaveField
          section="window"
          fieldKey="overlay_width"
          label="Overlay width"
          helper="Width of the floating Thuki window."
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
          helper="Height of the input bar before chat starts."
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
          helper="Tallest the chat window grows during long conversations."
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
          helper="How long the close animation plays before the window disappears."
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
          helper="Lines of quoted text shown in the input bar preview."
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
          helper="Characters of quoted text shown in the preview. Full text is still sent to the AI."
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
          helper="Characters of quoted text actually sent to the AI."
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

      <ResetSectionLink
        label="Reset General to defaults"
        onClick={() => setConfirmReset(true)}
      />
      <ConfirmDialog
        open={confirmReset}
        title="Reset General to defaults?"
        message="Your current Model, Prompt, Window, and Quote settings will be replaced with the defaults. This cannot be undone."
        confirmLabel="Reset"
        destructive
        onConfirm={() => {
          setConfirmReset(false);
          // Reset each section sequentially — keeps API surface minimal.
          // Cross-section reset combines: model + prompt + window + quote.
          void Promise.all(
            (['model', 'prompt', 'window', 'quote'] as const).map((s) =>
              invoke<RawAppConfig>('reset_config', { section: s }),
            ),
          ).then((results) => {
            const last = results[results.length - 1];
            if (last) onSaved(last);
          });
        }}
        onCancel={() => setConfirmReset(false)}
      />
    </>
  );
}
