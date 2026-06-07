import { describe, it, expect } from 'vitest';

import {
  parseChangelogSections,
  selectSections,
  compareSemver,
  type ChangelogSection,
} from '../changelog';

const FULL = `# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

### Added

- something not yet shipped

## [0.14.0](https://github.com/quiet-node/thuki/compare/v0.13.1...v0.14.0) (2026-06-07)

### Features

* allow drafting messages while streaming ([#200](https://x/200))

## [0.13.1](https://github.com/quiet-node/thuki/compare/v0.13.0...v0.13.1) (2026-05-26)

### Bug Fixes

* fix caret drift

## [0.13.0](https://github.com/quiet-node/thuki/compare/v0.12.0...v0.13.0) (2026-05-25)

### Features

* pre-load conversation list
`;

describe('parseChangelogSections', () => {
  it('returns one section per version header, newest-source order preserved', () => {
    const sections = parseChangelogSections(FULL);
    expect(sections.map((s) => s.version)).toEqual([
      '0.14.0',
      '0.13.1',
      '0.13.0',
    ]);
  });

  it('captures the date from the header', () => {
    const [latest] = parseChangelogSections(FULL);
    expect(latest.date).toBe('2026-06-07');
  });

  it('captures the body markdown beneath the header, header line stripped', () => {
    const [latest] = parseChangelogSections(FULL);
    expect(latest.body).toContain('### Features');
    expect(latest.body).toContain('allow drafting messages while streaming');
    expect(latest.body).not.toContain('## [0.14.0]');
  });

  it('excludes the document preamble and the Unreleased section', () => {
    const sections = parseChangelogSections(FULL);
    const joined = sections.map((s) => s.body).join('\n');
    expect(joined).not.toContain('something not yet shipped');
    expect(joined).not.toContain('All notable changes');
  });

  it('treats a version header without a date as date: null', () => {
    const sections = parseChangelogSections(
      '## [1.2.3]\n\n### Features\n\n* no date here',
    );
    expect(sections).toHaveLength(1);
    expect(sections[0].date).toBeNull();
    expect(sections[0].body).toContain('no date here');
  });

  it('drops content under a non-version level-2 header', () => {
    // A `## Notes` header is not a version, so its body must not attach to any
    // section (and must not start one).
    const sections = parseChangelogSections(
      '## [1.0.0] (2026-01-01)\n\n* shipped\n\n## Notes\n\n* internal aside',
    );
    expect(sections).toHaveLength(1);
    expect(sections[0].body).toContain('shipped');
    expect(sections[0].body).not.toContain('internal aside');
  });

  it('returns [] when no version header is present', () => {
    expect(parseChangelogSections('just a paragraph, no headers')).toEqual([]);
    expect(parseChangelogSections('## Unreleased\n\n- wip only')).toEqual([]);
    expect(parseChangelogSections('')).toEqual([]);
  });
});

describe('compareSemver', () => {
  it('orders by major, then minor, then patch', () => {
    expect(compareSemver('1.0.0', '0.9.9')).toBeGreaterThan(0);
    expect(compareSemver('0.14.0', '0.13.9')).toBeGreaterThan(0);
    expect(compareSemver('0.13.1', '0.13.2')).toBeLessThan(0);
  });

  it('returns 0 for equal versions', () => {
    expect(compareSemver('0.13.1', '0.13.1')).toBe(0);
  });
});

describe('selectSections', () => {
  const sections: ChangelogSection[] = [
    { version: '0.14.0', date: null, body: 'a' },
    { version: '0.13.1', date: null, body: 'b' },
    { version: '0.13.0', date: null, body: 'c' },
  ];

  it('keeps only versions newer than the installed one, newest first', () => {
    const out = selectSections(sections, '0.13.0');
    expect(out.map((s) => s.version)).toEqual(['0.14.0', '0.13.1']);
  });

  it('sorts newest-first even when the source order is shuffled', () => {
    const shuffled = [sections[1], sections[2], sections[0]];
    expect(selectSections(shuffled, '0.13.0').map((s) => s.version)).toEqual([
      '0.14.0',
      '0.13.1',
    ]);
  });

  it('keeps every section when current is null (lookup not resolved)', () => {
    expect(selectSections(sections, null).map((s) => s.version)).toEqual([
      '0.14.0',
      '0.13.1',
      '0.13.0',
    ]);
  });

  it('keeps every section when current is not semver-shaped', () => {
    expect(selectSections(sections, 'garbage').map((s) => s.version)).toEqual([
      '0.14.0',
      '0.13.1',
      '0.13.0',
    ]);
  });

  it('returns [] when the installed version is at or above every section', () => {
    expect(selectSections(sections, '0.14.0')).toEqual([]);
  });
});
