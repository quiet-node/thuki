/**
 * User-facing copy and severity mapping for the agentic search pipeline's
 * warnings. Kept in a const map rather than inline so the wording can be
 * tuned without editing component code.
 *
 * Severity drives the warning icon's appearance:
 * - "warn"  -> amber triangle, the pipeline still produced an answer.
 * - "error" -> red circle, the pipeline failed or the answer is unreliable.
 */
import type { SearchWarning } from '../types/search';

export const SEARCH_WARNING_COPY: Record<SearchWarning, string> = {
  reader_unavailable:
    "Couldn't read full pages. Showing results from search snippets only.",
  reader_partial_failure: "Some pages couldn't be loaded.",
  no_results_initial: 'No search results found. Try rephrasing your question.',
  iteration_cap_exhausted:
    'Answer based on limited information. Try a more specific question for better results.',
  router_failure:
    'Something went wrong while analyzing your question. Try again.',
  judge_failure:
    'Evidence check had a hiccup. Answer is based on whatever Thuki gathered so far.',
  budget_exhausted:
    'Search ran out of time. Answer is based on the evidence gathered before the budget hit.',
  no_progress:
    'Search stopped early because follow-up rounds were not surfacing new information.',
  synthesis_interrupted: 'Answer was cut off. Try again.',
};

export const SEARCH_WARNING_SEVERITY: Record<SearchWarning, 'warn' | 'error'> =
  {
    reader_unavailable: 'warn',
    reader_partial_failure: 'warn',
    no_results_initial: 'error',
    iteration_cap_exhausted: 'warn',
    router_failure: 'error',
    judge_failure: 'warn',
    budget_exhausted: 'warn',
    no_progress: 'warn',
    synthesis_interrupted: 'error',
  };
