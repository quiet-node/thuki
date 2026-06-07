import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect } from 'vitest';

import { ChangelogAccordion } from '../ChangelogAccordion';
import type { ChangelogSection } from '../changelog';

const SECTIONS: ChangelogSection[] = [
  {
    version: '0.14.0',
    date: '2026-06-07',
    body: '### Features\n\n* newest thing',
  },
  { version: '0.13.0', date: null, body: '### Bug Fixes\n\n* older fix' },
];

describe('ChangelogAccordion', () => {
  it('renders a row per version with the version label', () => {
    render(<ChangelogAccordion sections={SECTIONS} />);
    expect(screen.getByText('0.14.0')).toBeInTheDocument();
    expect(screen.getByText('0.13.0')).toBeInTheDocument();
  });

  it('shows the date when present and omits it when null', () => {
    render(<ChangelogAccordion sections={SECTIONS} />);
    expect(screen.getByText('2026-06-07')).toBeInTheDocument();
  });

  it('expands the newest version by default and collapses the rest', () => {
    render(<ChangelogAccordion sections={SECTIONS} />);
    expect(screen.getByText('newest thing')).toBeInTheDocument();
    expect(screen.queryByText('older fix')).not.toBeInTheDocument();
  });

  it('marks the open row aria-expanded and the closed row not', () => {
    render(<ChangelogAccordion sections={SECTIONS} />);
    expect(screen.getByRole('button', { name: /0\.14\.0/ })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    expect(screen.getByRole('button', { name: /0\.13\.0/ })).toHaveAttribute(
      'aria-expanded',
      'false',
    );
  });

  it('expands a collapsed version when its header is clicked', () => {
    render(<ChangelogAccordion sections={SECTIONS} />);
    fireEvent.click(screen.getByRole('button', { name: /0\.13\.0/ }));
    expect(screen.getByText('older fix')).toBeInTheDocument();
  });

  it('collapses the newest version when its header is clicked', () => {
    render(<ChangelogAccordion sections={SECTIONS} />);
    expect(screen.getByText('newest thing')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /0\.14\.0/ }));
    expect(screen.queryByText('newest thing')).not.toBeInTheDocument();
  });
});
