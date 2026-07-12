import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { SearchProgressBlock } from '../SearchProgressBlock';
import { invoke } from '../../testUtils/mocks/tauri';
import type { SearchResultPreview } from '../../types/search';

const SOURCES: SearchResultPreview[] = [
  {
    title: 'Tom Cruise – Wikipedia',
    url: 'https://en.wikipedia.org/wiki/Tom_Cruise',
  },
  {
    title: 'Britannica',
    url: 'https://www.britannica.com/biography/Tom-Cruise',
  },
  { title: 'IMDb', url: 'https://www.imdb.com/name/nm0000129/' },
];

describe('SearchProgressBlock', () => {
  beforeEach(() => {
    invoke.mockClear();
  });

  it('renders nothing when idle with no sources', () => {
    const { container } = render(
      <SearchProgressBlock stage={null} isSearching={false} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('renders nothing when search finished even if sources exist (footer owns list)', () => {
    const { container } = render(
      <SearchProgressBlock
        stage={null}
        sources={SOURCES}
        isSearching={false}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('shows live phase label without a body when searching with no sources yet', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'searching' }}
        isSearching
        sources={[]}
      />,
    );

    expect(screen.getByTestId('search-progress-block')).toBeInTheDocument();
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Searching the web',
    );
    expect(
      screen.queryByTestId('search-progress-body'),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByTestId('search-progress-toggle'),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByTestId('search-progress-chevron'),
    ).not.toBeInTheDocument();
  });

  it('maps analyzing and reading stages to the correct labels', () => {
    const { rerender } = render(
      <SearchProgressBlock stage={{ kind: 'analyzing_query' }} isSearching />,
    );
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Analyzing query',
    );

    rerender(
      <SearchProgressBlock
        stage={{ kind: 'reading_sources' }}
        isSearching
        sources={SOURCES}
      />,
    );
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Reading sources (3)',
    );
  });

  it('maps verifying_sources to the C3 label', () => {
    render(
      <SearchProgressBlock stage={{ kind: 'verifying_sources' }} isSearching />,
    );
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Verifying sources...',
    );
  });

  it('auto-expands the collapsible source list when sources arrive while searching', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'reading_sources' }}
        isSearching
        sources={SOURCES}
      />,
    );

    expect(screen.getByTestId('search-progress-body')).toBeInTheDocument();
    expect(screen.getAllByTestId('search-progress-source-row')).toHaveLength(3);
    expect(screen.getByTestId('search-progress-toggle')).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Reading sources (3)',
    );
    // Live row: dots → chevron accessory → label (chevron inside strip).
    const toggle = screen.getByTestId('search-progress-toggle');
    const strip = screen.getByTestId('request-status-strip');
    const chevron = screen.getByTestId('search-progress-chevron');
    expect(toggle.firstElementChild).toBe(strip);
    const stripChildren = Array.from(strip.children);
    expect(stripChildren[0].className).toContain('request-status-strip__dots');
    expect(stripChildren[1]).toBe(chevron);
    expect(stripChildren[2]).toHaveAttribute(
      'data-testid',
      'loading-stage-title',
    );
    expect(chevron).toHaveStyle({ transform: 'rotate(180deg)' });
  });

  it('lets the user collapse an auto-expanded list while keeping stage label', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'reading_sources' }}
        isSearching
        sources={SOURCES}
      />,
    );

    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Reading sources (3)',
    );
    fireEvent.click(screen.getByTestId('search-progress-toggle'));
    expect(
      screen.queryByTestId('search-progress-body'),
    ).not.toBeInTheDocument();
    // Still live strip + stage label with count; never bare "3 sources".
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Reading sources (3)',
    );
    expect(screen.getByTestId('search-progress-chevron')).toHaveStyle({
      transform: 'rotate(90deg)',
    });
  });

  it('lets the user re-expand after collapsing', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'reading_sources' }}
        isSearching
        sources={SOURCES}
      />,
    );

    fireEvent.click(screen.getByTestId('search-progress-toggle'));
    expect(
      screen.queryByTestId('search-progress-body'),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId('search-progress-toggle'));
    expect(screen.getByTestId('search-progress-body')).toBeInTheDocument();
    expect(screen.getByTestId('search-progress-toggle')).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    // Label stable across expand/collapse.
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Reading sources (3)',
    );
  });

  it('opens a source URL through the open_url Tauri command', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'reading_sources' }}
        isSearching
        sources={SOURCES}
      />,
    );

    fireEvent.click(screen.getAllByTestId('search-progress-source-row')[0]);
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://en.wikipedia.org/wiki/Tom_Cruise',
    });
  });

  it('appends (1) for a single source; never bare singular source label', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'reading_sources' }}
        isSearching
        sources={[SOURCES[0]]}
      />,
    );
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Reading sources (1)',
    );
  });

  it('falls back to the raw URL when title is empty and domain parse fails', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'reading_sources' }}
        isSearching
        sources={[{ title: '', url: 'not-a-url' }]}
      />,
    );
    const row = screen.getByTestId('search-progress-source-row');
    expect(row).toHaveTextContent('not-a-url');
  });

  it('uses a ? letter avatar when the domain string is empty', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'reading_sources' }}
        isSearching
        sources={[{ title: 'Empty host', url: '' }]}
      />,
    );
    expect(screen.getByTestId('search-progress-source-row')).toHaveTextContent(
      '?',
    );
  });

  it('maps gap and compose stages to their live labels', () => {
    const { rerender } = render(
      <SearchProgressBlock
        stage={{ kind: 'searching', gap: true }}
        isSearching
      />,
    );
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Searching more angles',
    );

    rerender(
      <SearchProgressBlock
        stage={{ kind: 'reading_sources', gap: true }}
        isSearching
      />,
    );
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Reading additional pages',
    );

    rerender(
      <SearchProgressBlock
        stage={{ kind: 'refining_search', attempt: 2, total: 3 }}
        isSearching
      />,
    );
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Refining search (2/3)',
    );

    rerender(<SearchProgressBlock stage={{ kind: 'composing' }} isSearching />);
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Composing answer',
    );

    rerender(
      <SearchProgressBlock
        stage={{ kind: 'composing', gap: true }}
        isSearching
      />,
    );
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Composing refined answer',
    );

    rerender(<SearchProgressBlock stage={null} isSearching />);
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Searching the web',
    );
  });

  it('appends source count to gap/searching stage labels when sources present', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'searching', gap: true }}
        isSearching
        sources={SOURCES}
      />,
    );
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Searching more angles (3)',
    );
  });

  it('forces collapse and disables toggle while isExiting', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'reading_sources' }}
        isSearching
        sources={SOURCES}
        isExiting
      />,
    );

    const block = screen.getByTestId('search-progress-block');
    expect(block).toHaveAttribute('data-exiting', 'true');
    expect(block).toHaveAttribute('aria-busy', 'true');
    expect(
      screen.queryByTestId('search-progress-body'),
    ).not.toBeInTheDocument();
    const toggle = screen.getByTestId('search-progress-toggle');
    expect(toggle).toBeDisabled();
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
  });

  it('stays mounted while isExiting even if isSearching is false', () => {
    render(
      <SearchProgressBlock
        stage={{ kind: 'composing' }}
        isSearching={false}
        isExiting
      />,
    );
    expect(screen.getByTestId('search-progress-block')).toBeInTheDocument();
    expect(screen.getByTestId('loading-label')).toHaveAttribute(
      'data-label',
      'Composing answer',
    );
  });
});
