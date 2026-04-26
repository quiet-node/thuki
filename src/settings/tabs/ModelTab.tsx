/**
 * Model tab — the AI brain.
 *
 * Holds the active Ollama model list, the local Ollama URL, and the
 * custom system prompt. The Window/Quote knobs that used to live in
 * the old "General" tab now live in the Display tab.
 */

import { Section, TextField, Textarea, OrderedListEditor } from '../components';
import { SaveField } from '../components/SaveField';
import { configHelp } from '../configHelpers';
import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';

interface ModelTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

const PROMPT_MAX_CHARS = 8000;

export function ModelTab({ config, resyncToken, onSaved }: ModelTabProps) {
  return (
    <>
      <Section heading="Model">
        <SaveField
          section="model"
          fieldKey="available"
          label="Active Ollama model"
          helper={configHelp('model', 'available')}
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
          helper={configHelp('model', 'ollama_url')}
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
          helper={configHelp('prompt', 'system')}
          vertical
          initialValue={config.prompt.system}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <>
              <Textarea
                value={value}
                onChange={setValue}
                placeholder="Use built-in secretary persona…"
                maxLength={PROMPT_MAX_CHARS}
                ariaLabel="System prompt"
              />
              <div className={styles.charCounter}>
                {value.length} / {PROMPT_MAX_CHARS}
              </div>
            </>
          )}
        />
      </Section>
    </>
  );
}
