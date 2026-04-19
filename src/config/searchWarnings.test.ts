import { describe, it, expect } from 'vitest';
import { SEARCH_WARNING_COPY, SEARCH_WARNING_SEVERITY } from './searchWarnings';
import type { SearchWarning } from '../types/search';

const ALL: SearchWarning[] = [
  'reader_unavailable',
  'reader_partial_failure',
  'no_results_initial',
  'iteration_cap_exhausted',
  'router_failure',
  'synthesis_interrupted',
];

describe('searchWarnings', () => {
  it('every SearchWarning has friendly copy under 200 chars', () => {
    for (const w of ALL) {
      const copy = SEARCH_WARNING_COPY[w];
      expect(copy).toBeTruthy();
      expect(copy.length).toBeLessThan(200);
    }
  });

  it('every SearchWarning has warn or error severity', () => {
    for (const w of ALL) {
      expect(['warn', 'error']).toContain(SEARCH_WARNING_SEVERITY[w]);
    }
  });

  it('error-severity warnings cover the fatal cases', () => {
    expect(SEARCH_WARNING_SEVERITY.no_results_initial).toBe('error');
    expect(SEARCH_WARNING_SEVERITY.router_failure).toBe('error');
    expect(SEARCH_WARNING_SEVERITY.synthesis_interrupted).toBe('error');
  });

  it('warn-severity warnings cover the degraded-but-answered cases', () => {
    expect(SEARCH_WARNING_SEVERITY.reader_unavailable).toBe('warn');
    expect(SEARCH_WARNING_SEVERITY.reader_partial_failure).toBe('warn');
    expect(SEARCH_WARNING_SEVERITY.iteration_cap_exhausted).toBe('warn');
  });
});
