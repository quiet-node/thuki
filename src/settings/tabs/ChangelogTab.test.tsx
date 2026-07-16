/**
 * Tests for the Settings Changelog tab: full-history accordion wiring and
 * empty-state fallback when parse yields no version sections.
 */

import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect } from 'vitest';

import { ChangelogTab, sectionsFromChangelogMarkdown } from './ChangelogTab';

const SAMPLE = `# Changelog

## Unreleased

### Added

- not shipped yet

## [0.14.0](https://github.com/quiet-node/thuki/compare/v0.13.1...v0.14.0) (2026-06-07)

### Features

* allow drafting messages while streaming

## [0.13.1](https://github.com/quiet-node/thuki/compare/v0.13.0...v0.13.1) (2026-05-26)

### Bug Fixes

* fix caret drift
`;

describe('sectionsFromChangelogMarkdown', () => {
  it('returns every version section, newest first', () => {
    const sections = sectionsFromChangelogMarkdown(SAMPLE);
    expect(sections.map((s) => s.version)).toEqual(['0.14.0', '0.13.1']);
  });

  it('returns [] for markdown with no version headers', () => {
    expect(sectionsFromChangelogMarkdown('just a paragraph')).toEqual([]);
    expect(sectionsFromChangelogMarkdown('')).toEqual([]);
  });
});

describe('ChangelogTab', () => {
  it('renders the accordion with known version strings from markdown', () => {
    render(<ChangelogTab markdown={SAMPLE} />);
    expect(screen.getByTestId('changelog-notes')).toBeInTheDocument();
    expect(screen.getByTestId('changelog-accordion')).toBeInTheDocument();
    expect(screen.getByText('0.14.0')).toBeInTheDocument();
    expect(screen.getByText('0.13.1')).toBeInTheDocument();
    expect(screen.getByText('Release history')).toBeInTheDocument();
  });

  it('expands the newest version by default and toggles on click', () => {
    render(<ChangelogTab markdown={SAMPLE} />);
    const newest = screen.getByRole('button', { name: /0\.14\.0/ });
    expect(newest).toHaveAttribute('aria-expanded', 'true');
    expect(
      screen.getByText(/allow drafting messages while streaming/),
    ).toBeInTheDocument();

    fireEvent.click(newest);
    expect(newest).toHaveAttribute('aria-expanded', 'false');
  });

  it('shows empty state when markdown has no version sections', () => {
    render(<ChangelogTab markdown="## Unreleased\n\n- wip only" />);
    expect(screen.getByTestId('changelog-empty')).toBeInTheDocument();
    expect(screen.queryByTestId('changelog-accordion')).toBeNull();
    expect(
      screen.getByRole('button', { name: /view releases on github/i }),
    ).toBeInTheDocument();
  });

  it('loads the bundled CHANGELOG when markdown prop is omitted', () => {
    render(<ChangelogTab />);
    expect(screen.getByTestId('changelog-accordion')).toBeInTheDocument();
    // Bundled Keep-a-Changelog always has at least one version header.
    const versionButtons = screen
      .getAllByRole('button')
      .filter((el) => /^\d+\.\d+\.\d+/.test(el.textContent ?? ''));
    expect(versionButtons.length).toBeGreaterThan(0);
  });
});
