import { describe, expect, it } from 'vitest';
import type {
  IterationTrace,
  SearchEvent,
  SearchStage,
  SearchTraceStep,
  SearchWarning,
} from './search';

describe('search types', () => {
  it('SearchEvent allows the new Trace variant', () => {
    const step: SearchTraceStep = {
      id: 'analyze',
      kind: 'analyze',
      status: 'running',
      title: 'Understanding the question',
      summary: 'Deciding whether to search.',
    };

    const event: SearchEvent = { type: 'Trace', step };
    expect(event.type).toBe('Trace');
    if (event.type === 'Trace') {
      expect(event.step.kind).toBe('analyze');
    }
  });

  it('SearchTraceStep supports verdicts, counts, queries, urls, and domains', () => {
    const step: SearchTraceStep = {
      id: 'round-1-snippet-judge',
      kind: 'snippet_judge',
      status: 'completed',
      round: 1,
      title: 'Checking what the results already cover',
      summary:
        'The results point in the right direction, but a few details are still missing.',
      detail: 'Still missing the exact version number.',
      queries: ['tokio runtime version'],
      urls: ['https://tokio.rs/tokio/tutorial'],
      domains: ['tokio.rs', 'docs.rs'],
      verdict: 'partial',
      counts: {
        sources: 2,
        kept: 2,
      },
    };

    expect(step.round).toBe(1);
    expect(step.verdict).toBe('partial');
    expect(step.counts?.sources).toBe(2);
    expect(step.urls).toEqual(['https://tokio.rs/tokio/tutorial']);
    expect(step.domains).toEqual(['tokio.rs', 'docs.rs']);
  });

  it('SearchStage refining_search carries attempt and total', () => {
    const stage: SearchStage = {
      kind: 'refining_search',
      attempt: 2,
      total: 3,
    };
    if (stage && stage.kind === 'refining_search') {
      expect(stage.attempt).toBe(2);
      expect(stage.total).toBe(3);
    }
  });

  it('SearchWarning union still includes the backend warning variants', () => {
    const variants: SearchWarning[] = [
      'reader_unavailable',
      'reader_partial_failure',
      'no_results_initial',
      'iteration_cap_exhausted',
      'router_failure',
      'synthesis_interrupted',
    ];

    expect(variants).toHaveLength(6);
  });

  it('legacy IterationComplete payloads still type-check for compatibility', () => {
    const trace: IterationTrace = {
      stage: { kind: 'initial' },
      queries: ['legacy query'],
      urls_fetched: ['https://example.com'],
      reader_empty_urls: [],
      judge_verdict: 'sufficient',
      judge_reasoning: 'covers the topic',
      duration_ms: 200,
    };

    const event: SearchEvent = { type: 'IterationComplete', trace };
    expect(event.type).toBe('IterationComplete');
    if (event.type === 'IterationComplete') {
      expect(event.trace.duration_ms).toBe(200);
    }
  });

  it('SearchEvent still allows SandboxUnavailable', () => {
    const event: SearchEvent = { type: 'SandboxUnavailable' };
    expect(event.type).toBe('SandboxUnavailable');
  });
});
