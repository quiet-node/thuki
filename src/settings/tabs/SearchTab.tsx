/**
 * Web tab — sandbox service URLs, pipeline tuning, and timeouts for the
 * `/search` slash command.
 *
 * Sub-grouped: SERVICES (URLs), PIPELINE (knobs), TIMEOUTS (per-stage
 * seconds). The cross-section "reset to defaults" affordance lives only
 * in the About tab to keep this surface focused on tuning.
 */

import { Section, NumberSlider, NumberStepper, TextField } from '../components';
import { SaveField } from '../components/SaveField';
import { configHelp } from '../configHelpers';
import type { RawAppConfig } from '../types';

interface SearchTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

export function SearchTab({ config, resyncToken, onSaved }: SearchTabProps) {
  return (
    <>
      <Section heading="Services">
        <SaveField
          section="search"
          fieldKey="searxng_url"
          label="SearXNG URL"
          helper={configHelp('search', 'searxng_url')}
          initialValue={config.search.searxng_url}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue, errored) => (
            <TextField
              value={value}
              onChange={setValue}
              placeholder="http://127.0.0.1:25017"
              errored={errored}
              ariaLabel="SearXNG URL"
            />
          )}
        />
        <SaveField
          section="search"
          fieldKey="reader_url"
          label="Reader URL"
          helper={configHelp('search', 'reader_url')}
          initialValue={config.search.reader_url}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue, errored) => (
            <TextField
              value={value}
              onChange={setValue}
              placeholder="http://127.0.0.1:25018"
              errored={errored}
              ariaLabel="Reader URL"
            />
          )}
        />
      </Section>

      <Section heading="Pipeline">
        <SaveField
          section="search"
          fieldKey="max_iterations"
          label="Max iterations"
          helper={configHelp('search', 'max_iterations')}
          initialValue={config.search.max_iterations}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberStepper
              value={value}
              min={1}
              max={10}
              onChange={setValue}
              ariaLabel="Max iterations"
            />
          )}
        />
        <SaveField
          section="search"
          fieldKey="top_k_urls"
          label="Top-K URLs"
          helper={configHelp('search', 'top_k_urls')}
          initialValue={config.search.top_k_urls}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberStepper
              value={value}
              min={1}
              max={20}
              onChange={setValue}
              ariaLabel="Top-K URLs"
            />
          )}
        />
        <SaveField
          section="search"
          fieldKey="searxng_max_results"
          label="Max SearXNG results"
          helper={configHelp('search', 'searxng_max_results')}
          initialValue={config.search.searxng_max_results}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberStepper
              value={value}
              min={1}
              max={20}
              onChange={setValue}
              ariaLabel="Max SearXNG results"
            />
          )}
        />
      </Section>

      <Section heading="Timeouts">
        <SaveField
          section="search"
          fieldKey="search_timeout_s"
          label="Search timeout"
          helper={configHelp('search', 'search_timeout_s')}
          initialValue={config.search.search_timeout_s}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={1}
              max={300}
              unit="s"
              onChange={setValue}
              ariaLabel="Search timeout"
            />
          )}
        />
        <SaveField
          section="search"
          fieldKey="reader_per_url_timeout_s"
          label="Reader per-URL timeout"
          helper={configHelp('search', 'reader_per_url_timeout_s')}
          initialValue={config.search.reader_per_url_timeout_s}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={1}
              max={300}
              unit="s"
              onChange={setValue}
              ariaLabel="Reader per-URL timeout"
            />
          )}
        />
        <SaveField
          section="search"
          fieldKey="reader_batch_timeout_s"
          label="Reader batch timeout"
          helper={configHelp('search', 'reader_batch_timeout_s')}
          initialValue={config.search.reader_batch_timeout_s}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={1}
              max={300}
              unit="s"
              onChange={setValue}
              ariaLabel="Reader batch timeout"
            />
          )}
        />
        <SaveField
          section="search"
          fieldKey="judge_timeout_s"
          label="Judge timeout"
          helper={configHelp('search', 'judge_timeout_s')}
          initialValue={config.search.judge_timeout_s}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={1}
              max={300}
              unit="s"
              onChange={setValue}
              ariaLabel="Judge timeout"
            />
          )}
        />
        <SaveField
          section="search"
          fieldKey="router_timeout_s"
          label="Router timeout"
          helper={configHelp('search', 'router_timeout_s')}
          initialValue={config.search.router_timeout_s}
          resyncToken={resyncToken}
          onSaved={onSaved}
          render={(value, setValue) => (
            <NumberSlider
              value={value}
              min={1}
              max={300}
              unit="s"
              onChange={setValue}
              ariaLabel="Router timeout"
            />
          )}
        />
      </Section>
    </>
  );
}
