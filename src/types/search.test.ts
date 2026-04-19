import { describe, it, expect } from 'vitest';
import type { SearchEvent, SearchStage, SearchWarning } from './search';

describe('search types', () => {
  it('SearchEvent allows AnalyzingQuery variant', () => {
    const e: SearchEvent = { type: 'AnalyzingQuery' };
    expect(e.type).toBe('AnalyzingQuery');
  });
  it('SearchEvent RefiningSearch carries attempt and total', () => {
    const e: SearchEvent = { type: 'RefiningSearch', attempt: 2, total: 3 };
    expect(e.attempt).toBe(2);
    expect(e.total).toBe(3);
  });
  it('SearchEvent Warning carries a SearchWarning value', () => {
    const e: SearchEvent = { type: 'Warning', warning: 'reader_unavailable' };
    expect(e.warning).toBe('reader_unavailable');
  });
  it('SearchWarning union includes all six backend variants', () => {
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
  it('SearchStage refining_search carries attempt and total', () => {
    const s: SearchStage = { kind: 'refining_search', attempt: 2, total: 3 };
    if (s && s.kind === 'refining_search') {
      expect(s.attempt).toBe(2);
      expect(s.total).toBe(3);
    }
  });
  it('SearchStage may be null for idle', () => {
    const s: SearchStage = null;
    expect(s).toBeNull();
  });
  it('SearchEvent allows SandboxUnavailable variant', () => {
    const e: SearchEvent = { type: 'SandboxUnavailable' };
    expect(e.type).toBe('SandboxUnavailable');
  });
});
