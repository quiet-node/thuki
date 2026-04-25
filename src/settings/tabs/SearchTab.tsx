/**
 * Search tab — sandbox service URLs, pipeline tuning, and timeouts.
 *
 * Sub-grouped: SERVICES (URLs + security warning), PIPELINE (knobs),
 * TIMEOUTS (per-stage seconds).
 */

import { useState } from 'react';

import { invoke } from '@tauri-apps/api/core';

import {
  Section,
  ResetSectionLink,
  NumberSlider,
  NumberStepper,
  TextField,
  ConfirmDialog,
} from '../components';
import { SaveField } from '../components/SaveField';
import styles from '../../styles/settings.module.css';
import type { RawAppConfig } from '../types';

interface SearchTabProps {
  config: RawAppConfig;
  resyncToken: number;
  onSaved: (next: RawAppConfig) => void;
}

export function SearchTab({ config, resyncToken, onSaved }: SearchTabProps) {
  const [confirmReset, setConfirmReset] = useState(false);

  return (
    <>
      <Section heading="Services">
        <div className={styles.warning}>
          <span aria-hidden>⚠</span>
          <span>
            Both URLs default to <code>localhost</code>. Pointing them at remote
            servers breaks Thuki’s sandbox isolation: the page reader would
            fetch arbitrary URLs from a host that may have access to private
            networks.
          </span>
        </div>
        <SaveField
          section="search"
          fieldKey="searxng_url"
          label="SearXNG URL"
          helper="Local search engine endpoint. Match the binding in sandbox/docker-compose.yml."
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
          helper="Local web-page reader endpoint. Match the binding in sandbox/docker-compose.yml."
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
          helper="Search-refine rounds before the AI gives up. Raise for hard, multi-step questions."
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
          helper="Pages opened and read after reranking. Raise for more sources, lower for faster searches."
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
          helper="Results SearXNG returns per query before reranking."
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
          helper="Seconds before a SearXNG query is abandoned."
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
          helper="Seconds per single page fetch."
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
          helper="Seconds for the full parallel reader batch. Loader auto-corrects to per-URL + 5 if too low."
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
          helper="Seconds for the AI to decide whether results are sufficient."
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
          helper="Seconds for the AI to plan initial queries."
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

      <ResetSectionLink
        label="Reset Search to defaults"
        onClick={() => setConfirmReset(true)}
      />
      <ConfirmDialog
        open={confirmReset}
        title="Reset Search to defaults?"
        message="Your current Search settings will be replaced with the defaults. This cannot be undone."
        confirmLabel="Reset"
        destructive
        onConfirm={() => {
          setConfirmReset(false);
          void invoke<RawAppConfig>('reset_config', { section: 'search' }).then(
            onSaved,
          );
        }}
        onCancel={() => setConfirmReset(false)}
      />
    </>
  );
}
