import { render, screen, fireEvent } from '@testing-library/react';
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

  it('uses pointer cursor so users recognize it as interactive', () => {
    render(<SearchWarningIcon warnings={['reader_unavailable']} />);
    const el = screen.getByRole('img', { name: /warning/i });
    expect(el.style.cursor).toBe('pointer');
  });

  it('does not use the native title attribute (Tooltip handles hover)', () => {
    render(<SearchWarningIcon warnings={['reader_unavailable']} />);
    const el = screen.getByRole('img', { name: /warning/i });
    expect(el).not.toHaveAttribute('title');
  });

  it('shows the Tooltip with all warning copy lines on hover', () => {
    const warnings: SearchWarning[] = [
      'reader_unavailable',
      'iteration_cap_exhausted',
    ];
    render(<SearchWarningIcon warnings={warnings} />);
    const el = screen.getByRole('img');
    fireEvent.mouseEnter(el.parentElement!);
    expect(
      screen.getByText(/Couldn't read full pages.*limited information/s),
    ).toBeInTheDocument();
  });

  it('renders single-warning copy verbatim in the tooltip', () => {
    render(<SearchWarningIcon warnings={['no_results_initial']} />);
    const el = screen.getByRole('img', { name: /error/i });
    fireEvent.mouseEnter(el.parentElement!);
    expect(
      screen.getByText(
        'No search results found. Try rephrasing your question.',
      ),
    ).toBeInTheDocument();
  });
});
