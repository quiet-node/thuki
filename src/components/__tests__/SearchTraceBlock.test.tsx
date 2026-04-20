import { fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { SearchTraceBlock } from '../SearchTraceBlock';
import { invoke } from '../../testUtils/mocks/tauri';
import type { SearchTraceStep } from '../../types/search';

const SEARCH_URLS = Array.from({ length: 10 }, (_, index) => {
  const number = index + 1;
  return `https://example${number}.com/result-${number}`;
});

const SEARCH_STEPS: SearchTraceStep[] = [
  {
    id: 'analyze',
    kind: 'analyze',
    status: 'completed',
    title: 'Understanding the question',
    summary:
      'This needs fresh web results, so Thuki is switching into search mode.',
    detail: 'Using search query: tokio runtime release date',
    queries: ['tokio runtime release date'],
  },
  {
    id: 'round-1-search',
    kind: 'search',
    status: 'completed',
    round: 1,
    title: 'Searching the web',
    summary: 'Found 10 results across 7 sites.',
    urls: SEARCH_URLS,
    domains: ['tokio.rs', 'docs.rs', 'github.com'],
    counts: { found: 10 },
  },
  {
    id: 'round-1-rerank',
    kind: 'url_rerank',
    status: 'completed',
    round: 1,
    title: 'Rerank pages based on relevance',
    summary:
      'Rank the results based on their relevance to the question and kept the top 2.',
    urls: [
      'https://tokio.rs/tokio/tutorial',
      'https://docs.rs/tokio/latest/tokio/',
    ],
    domains: ['tokio.rs', 'docs.rs'],
    counts: { kept: 2 },
  },
  {
    id: 'round-1-read',
    kind: 'read',
    status: 'running',
    round: 1,
    title: 'Opening the shortlisted pages',
    summary: 'Opened 2 of 5 pages so far.',
    domains: ['tokio.rs', 'docs.rs'],
    counts: { processed: 2, total: 5 },
  },
];

describe('SearchTraceBlock', () => {
  beforeEach(() => {
    invoke.mockClear();
  });

  it('does not render when idle and there are no traces', () => {
    const { container } = render(
      <SearchTraceBlock traces={[]} isSearching={false} />,
    );

    expect(container.firstChild).toBeNull();
  });

  it('renders a loading header and placeholder row before trace steps arrive', () => {
    render(<SearchTraceBlock traces={[]} isSearching />);

    expect(screen.getByTestId('search-trace-block')).toBeInTheDocument();
    expect(screen.getByTestId('search-trace-loading')).toBeInTheDocument();
    expect(
      screen.queryByTestId('search-trace-pending-step'),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId('loading-label')).toHaveTextContent(
      'Starting search',
    );
  });

  it('renders the placeholder timeline row when expanded before traces arrive', () => {
    render(<SearchTraceBlock traces={[]} isSearching />);

    fireEvent.click(
      screen.getByRole('button', { name: /toggle search trace/i }),
    );

    expect(screen.getByTestId('search-trace-pending-step')).toHaveTextContent(
      'Spinning up the search pipeline.',
    );
  });

  it('starts collapsed while a search is live and expands on click', () => {
    render(<SearchTraceBlock traces={SEARCH_STEPS} isSearching />);

    expect(
      screen.queryByTestId('search-trace-timeline'),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId('loading-label')).toHaveTextContent(
      'Opening the shortlisted pages',
    );

    fireEvent.click(
      screen.getByRole('button', { name: /toggle search trace/i }),
    );

    expect(screen.getByTestId('search-trace-timeline')).toBeInTheDocument();
  });

  it('uses the active running step title in the loading header', () => {
    render(<SearchTraceBlock traces={SEARCH_STEPS} isSearching />);

    expect(screen.getByTestId('loading-label')).toHaveTextContent(
      'Opening the shortlisted pages',
    );
  });

  it('renders completed historical traces collapsed by default and expands on click', () => {
    render(<SearchTraceBlock traces={SEARCH_STEPS} isSearching={false} />);

    expect(
      screen.queryByTestId('search-trace-timeline'),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /toggle search trace/i }),
    ).toHaveTextContent('Search trace · 4 steps · 1 round');

    fireEvent.click(
      screen.getByRole('button', { name: /toggle search trace/i }),
    );

    expect(screen.getByTestId('search-trace-timeline')).toBeInTheDocument();
  });

  it('renders queries, detail text, chips, and transparent URL lists for each step', () => {
    render(<SearchTraceBlock traces={SEARCH_STEPS} isSearching={false} />);

    fireEvent.click(
      screen.getByRole('button', { name: /toggle search trace/i }),
    );

    const analyzeRow = screen.getByTestId('search-trace-step-analyze');
    expect(
      within(analyzeRow).getByText(/Using search query/i),
    ).toBeInTheDocument();
    expect(within(analyzeRow).getByText(/Searches:/)).toBeInTheDocument();

    const searchRow = screen.getByTestId('search-trace-step-round-1-search');
    expect(within(searchRow).getByText('Round 1')).toBeInTheDocument();
    expect(within(searchRow).getByText('10 found')).toBeInTheDocument();
    expect(
      within(searchRow).getByText('https://example1.com/result-1'),
    ).toBeInTheDocument();
    expect(within(searchRow).getAllByRole('button')).toHaveLength(10);

    const rerankRow = screen.getByTestId('search-trace-step-round-1-rerank');
    expect(within(rerankRow).getByText('2 kept')).toBeInTheDocument();
    expect(
      within(rerankRow).getByText('https://docs.rs/tokio/latest/tokio/'),
    ).toBeInTheDocument();

    const readRow = screen.getByTestId('search-trace-step-round-1-read');
    expect(within(readRow).getByText('2/5 opened')).toBeInTheDocument();
  });

  it('opens a trace URL in the browser when clicked', () => {
    render(<SearchTraceBlock traces={SEARCH_STEPS} isSearching={false} />);

    fireEvent.click(
      screen.getByRole('button', { name: /toggle search trace/i }),
    );
    fireEvent.click(screen.getByTitle('https://example1.com/result-1'));

    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://example1.com/result-1',
    });
  });

  it('uses the loading stage as the active disclosure title without a live badge', () => {
    render(<SearchTraceBlock traces={SEARCH_STEPS} isSearching />);

    const header = screen.getByTestId('search-trace-loading');

    expect(within(header).getByTestId('loading-label')).toHaveTextContent(
      'Opening the shortlisted pages',
    );
    expect(screen.queryByText('Live')).not.toBeInTheDocument();
  });

  it('renders the disclosure chevron after the loading stage for active searches', () => {
    render(<SearchTraceBlock traces={SEARCH_STEPS} isSearching />);

    const header = screen.getByTestId('search-trace-loading');
    const title = within(header).getByTestId('loading-stage-title');
    const prefix = within(header).getByTestId('loading-label-prefix');
    const chevron = within(header).getByTestId('search-trace-chevron');

    expect(title).toContainElement(prefix);
    expect(prefix).toContainElement(chevron);
  });

  it('renders the disclosure chevron before the loading label text for active searches', () => {
    render(<SearchTraceBlock traces={SEARCH_STEPS} isSearching />);

    const header = screen.getByTestId('search-trace-loading');
    const title = within(header).getByTestId('loading-stage-title');
    const prefix = within(header).getByTestId('loading-label-prefix');
    const chevron = within(header).getByTestId('search-trace-chevron');
    const labelText = within(header).getByTestId('loading-label');

    expect(title).toContainElement(prefix);
    expect(title).toContainElement(labelText);

    expect(
      prefix.compareDocumentPosition(chevron) &
        Node.DOCUMENT_POSITION_CONTAINED_BY,
    ).toBeTruthy();
    expect(
      chevron.compareDocumentPosition(labelText) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it('renders rich verdict chips, plural rounds, and overflowed domain summaries', () => {
    const richSteps: SearchTraceStep[] = [
      {
        id: 'judge-sufficient',
        kind: 'snippet_judge',
        status: 'completed',
        round: 1,
        title: 'Checked the snippets',
        summary: 'The first pass already covered the key facts.',
        verdict: 'sufficient',
        counts: {
          pages: 2,
          chunks: 3,
          empty: 1,
          failed: 1,
          sources: 4,
        },
        domains: ['a.com', 'b.com', 'c.com', 'd.com', 'e.com'],
      },
      {
        id: 'judge-partial',
        kind: 'chunk_judge',
        status: 'completed',
        round: 2,
        title: 'Checked the passages',
        summary: 'The answer is closer, but still incomplete.',
        verdict: 'partial',
      },
      {
        id: 'judge-insufficient',
        kind: 'chunk_judge',
        status: 'completed',
        round: 2,
        title: 'Checked the final evidence',
        summary: 'The available evidence is still too thin.',
        verdict: 'insufficient',
      },
    ];

    render(<SearchTraceBlock traces={richSteps} isSearching={false} />);

    const toggle = screen.getByRole('button', { name: /toggle search trace/i });
    expect(toggle).toHaveTextContent('Search trace · 3 steps · 2 rounds');

    fireEvent.click(toggle);

    expect(
      screen.getByTestId('search-trace-step-judge-sufficient'),
    ).toHaveTextContent(
      '2 pages · 3 passages · 1 empty · 1 failed · 4 sources · Enough evidence',
    );
    expect(
      screen.getByTestId('search-trace-step-judge-partial'),
    ).toHaveTextContent('Needs more detail');
    expect(
      screen.getByTestId('search-trace-step-judge-insufficient'),
    ).toHaveTextContent('Still not enough');
    expect(
      screen.getByText('a.com · b.com · c.com · d.com · +1'),
    ).toBeInTheDocument();
  });
});
