import { useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import type { SearchResultPreview, SearchTraceStep } from '../types/search';
import { LoadingStage } from './LoadingStage';

export interface SearchTraceBlockProps {
  traces: SearchTraceStep[];
  isSearching: boolean;
  /** Final curated source list used for the footer count after completion. */
  sources?: SearchResultPreview[];
}

function verdictLabel(verdict: SearchTraceStep['verdict']): string | null {
  if (!verdict) return null;
  if (verdict === 'sufficient') return 'Enough evidence';
  if (verdict === 'partial') return 'Needs more detail';
  return 'Still not enough';
}

function stepMetrics(step: SearchTraceStep): string[] {
  const counts = step.counts;
  const chips: string[] = [];

  if (counts?.found) chips.push(`${counts.found} found`);
  if (counts?.kept) chips.push(`${counts.kept} kept`);
  if (
    counts?.processed !== undefined &&
    counts?.total !== undefined &&
    counts.total > 0
  ) {
    chips.push(`${counts.processed}/${counts.total} read`);
  }
  if (counts?.pages) chips.push(`${counts.pages} pages`);
  if (counts?.chunks) chips.push(`${counts.chunks} passages`);
  if (counts?.empty) chips.push(`${counts.empty} empty`);
  if (counts?.failed) chips.push(`${counts.failed} failed`);
  if (counts?.sources) chips.push(`${counts.sources} sources`);

  const verdict = verdictLabel(step.verdict);
  if (verdict) chips.push(verdict);

  return chips;
}

function uniqueRoundCount(traces: SearchTraceStep[]): number {
  return new Set(
    traces
      .map((trace) => trace.round)
      .filter((round): round is number => round !== undefined),
  ).size;
}

function activeStepLabel(traces: SearchTraceStep[]): string {
  const active = [...traces]
    .reverse()
    .find((trace) => trace.status === 'running');
  return active?.title ?? 'Starting search';
}

function traceSummary(traces: SearchTraceStep[]): string {
  const parts = [
    `Search trace${traces.length > 0 ? ` · ${traces.length} ${traces.length === 1 ? 'step' : 'steps'}` : ''}`,
  ];
  const rounds = uniqueRoundCount(traces);
  if (rounds > 0) {
    parts.push(`${rounds} ${rounds === 1 ? 'round' : 'rounds'}`);
  }

  return parts.join(' · ');
}

function PlaceholderRow() {
  return (
    <div data-testid="search-trace-pending-step" className="flex gap-2.5">
      <div className="flex flex-col items-center flex-shrink-0 w-3">
        <div className="mt-1.5 h-1.5 w-1.5 rounded-full bg-text-secondary/55 animate-pulse" />
      </div>

      <div className="min-w-0 flex-1 pb-0.5">
        <div className="text-[12px] font-medium leading-none text-text-secondary/72">
          Starting search
        </div>
        <div className="mt-1 text-[11px] leading-[1.4] text-text-secondary/42">
          Spinning up the search pipeline.
        </div>
      </div>
    </div>
  );
}

function TraceRow({
  step,
  hasNext,
}: {
  step: SearchTraceStep;
  hasNext: boolean;
}) {
  const chips = stepMetrics(step);
  const extraDomains = step.domains ? Math.max(step.domains.length - 4, 0) : 0;
  const visibleDomains = step.domains?.slice(0, 4) ?? [];
  const visibleUrls = step.urls ?? [];
  const domainSummary =
    visibleDomains.length > 0
      ? `${visibleDomains.join(' · ')}${extraDomains > 0 ? ` · +${extraDomains}` : ''}`
      : null;

  return (
    <div data-testid={`search-trace-step-${step.id}`} className="flex gap-2.5">
      <div className="flex w-3 flex-col items-center flex-shrink-0">
        <div
          className={`mt-1.5 h-1.5 w-1.5 rounded-full ${
            step.status === 'running'
              ? 'bg-primary/80 animate-pulse'
              : 'bg-text-secondary/30'
          }`}
        />
        {hasNext && (
          <div className="mt-1 h-full min-h-[16px] w-px bg-white/[0.08]" />
        )}
      </div>

      <div className="min-w-0 flex-1 pb-0.5">
        <div className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
          {step.round !== undefined && (
            <span className="text-[9px] font-medium uppercase tracking-[0.14em] text-text-secondary/34">
              Round {step.round}
            </span>
          )}
          <span className="text-[12px] font-medium leading-none text-text-secondary/78">
            {step.title}
          </span>
        </div>

        <p className="mt-1 text-[12px] leading-[1.45] text-text-secondary/60">
          {step.summary}
        </p>

        {step.queries && step.queries.length > 0 && (
          <p className="mt-1 text-[11px] leading-[1.4] text-text-secondary/40">
            Searches: "{step.queries.join('" · "')}"
          </p>
        )}

        {step.detail && (
          <p className="mt-1 text-[11px] leading-[1.4] text-text-secondary/40">
            {step.detail}
          </p>
        )}

        {chips.length > 0 && (
          <p className="mt-1.5 text-[10px] leading-[1.35] text-text-secondary/36">
            {chips.join(' · ')}
          </p>
        )}

        {visibleUrls.length > 0 && (
          <div
            data-testid={`search-trace-urls-${step.id}`}
            className="mt-1.5 flex flex-col gap-1"
          >
            {visibleUrls.map((url) => (
              <button
                key={url}
                type="button"
                title={url}
                onClick={() => void invoke('open_url', { url })}
                className="w-full cursor-pointer break-all text-left text-[10px] leading-[1.4] text-text-secondary/30 transition-colors hover:text-text-secondary/50"
              >
                {url}
              </button>
            ))}
          </div>
        )}

        {!visibleUrls.length && domainSummary && (
          <div className="mt-1 text-[10px] leading-[1.35] text-text-secondary/30">
            {domainSummary}
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * Collapsible stage timeline rendered above `/search` answers.
 *
 * The disclosure stays collapsed by default, including while a search is
 * active, so the streaming header remains lightweight until the user chooses
 * to inspect the timeline.
 */
export function SearchTraceBlock({
  traces = [],
  isSearching,
}: SearchTraceBlockProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  if (!isSearching && traces.length === 0) {
    return null;
  }

  const summary = traceSummary(traces);
  const chevron = (
    <span
      data-testid="search-trace-chevron"
      className="inline-block shrink-0 text-[9px] text-text-secondary/55 transition-transform duration-150"
      style={{
        transform: isExpanded ? 'rotate(180deg)' : 'rotate(90deg)',
      }}
    >
      &#9650;
    </span>
  );

  return (
    <div data-testid="search-trace-block" className="mb-1.5">
      <button
        type="button"
        onClick={() => setIsExpanded((prev) => !prev)}
        className="w-full cursor-pointer border-none bg-transparent p-0 text-left"
        aria-expanded={isExpanded}
        aria-label="Toggle search trace"
      >
        {isSearching ? (
          <span
            data-testid="search-trace-loading"
            className="inline-flex min-w-0 items-center"
          >
            <span className="min-w-0">
              <LoadingStage
                compact
                label={activeStepLabel(traces)}
                labelPrefix={chevron}
              />
            </span>
          </span>
        ) : (
          <div className="inline-flex items-center gap-1.5">
            {chevron}
            <span className="text-[11px] font-medium tracking-[0.01em] text-text-secondary/58">
              {summary}
            </span>
          </div>
        )}
      </button>

      <AnimatePresence>
        {isExpanded && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.2 }}
            className="overflow-hidden"
          >
            <div
              data-testid="search-trace-timeline"
              className="mt-2 flex flex-col gap-3 pl-0.5"
            >
              {traces.length === 0 ? (
                <PlaceholderRow />
              ) : (
                traces.map((trace, index) => (
                  <TraceRow
                    key={trace.id}
                    step={trace}
                    hasNext={index < traces.length - 1}
                  />
                ))
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
