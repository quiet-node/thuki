import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { SearchWarningIcon } from './SearchWarningIcon';
import type { SearchWarning } from '../types/search';

describe('SearchWarningIcon', () => {
  it('renders nothing when warnings is empty', () => {
    const { container } = render(<SearchWarningIcon warnings={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('renders warn severity for warn-only warning list', () => {
    render(<SearchWarningIcon warnings={['reader_unavailable']} />);
    const el = screen.getByRole('img', { name: /warning/i });
    expect(el).toHaveAttribute('data-severity', 'warn');
  });

  it('escalates to error severity when any warning is error-severity', () => {
    const warnings: SearchWarning[] = ['reader_unavailable', 'router_failure'];
    render(<SearchWarningIcon warnings={warnings} />);
    const el = screen.getByRole('img', { name: /error/i });
    expect(el).toHaveAttribute('data-severity', 'error');
  });

  it('tooltip title concatenates all warning copy lines', () => {
    const warnings: SearchWarning[] = [
      'reader_unavailable',
      'iteration_cap_exhausted',
    ];
    render(<SearchWarningIcon warnings={warnings} />);
    const el = screen.getByRole('img');
    const title = el.getAttribute('title') ?? '';
    expect(title).toContain("Couldn't read full pages");
    expect(title).toContain('limited information');
    expect(title.split('\n').length).toBe(2);
  });

  it('renders a single warning copy without a newline', () => {
    render(<SearchWarningIcon warnings={['no_results_initial']} />);
    const el = screen.getByRole('img', { name: /error/i });
    expect(el.getAttribute('title')).toBe(
      'No search results found. Try rephrasing your question.',
    );
  });
});
