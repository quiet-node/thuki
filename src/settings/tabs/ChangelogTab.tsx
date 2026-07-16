/**
 * Changelog tab: full release history from the bundled Keep-a-Changelog
 * file, rendered with the same accordion used by What's New.
 *
 * Always shows every released version (not update-gated). What's New uses
 * `selectSections` to filter to versions newer than the install; this tab
 * deliberately skips that filter so Settings remains a permanent history.
 */

import { useMemo } from 'react';

import changelogMd from '../../../CHANGELOG.md?raw';
import { InlineLink } from '../../components/InlineLink';
import styles from '../../styles/settings.module.css';
import { ChangelogAccordion } from '../../view/update/ChangelogAccordion';
import {
  compareSemver,
  parseChangelogSections,
  type ChangelogSection,
} from '../../view/update/changelog';
import { Section } from '../components';

/** GitHub releases index used when the bundled changelog parses empty. */
const GITHUB_RELEASES_URL = 'https://github.com/quiet-node/thuki/releases';

export interface ChangelogTabProps {
  /**
   * Markdown override for tests. Defaults to the Vite `?raw` import of
   * repo-root `CHANGELOG.md`.
   */
  markdown?: string;
}

/**
 * Parses changelog markdown into accordion sections, newest first.
 * Full history: does not call `selectSections`.
 */
export function sectionsFromChangelogMarkdown(
  markdown: string,
): ChangelogSection[] {
  return parseChangelogSections(markdown).sort((a, b) =>
    compareSemver(b.version, a.version),
  );
}

/**
 * Settings panel listing every released version as a collapsible accordion.
 */
export function ChangelogTab({
  markdown = changelogMd,
}: ChangelogTabProps = {}) {
  const sections = useMemo(
    () => sectionsFromChangelogMarkdown(markdown),
    [markdown],
  );

  return (
    <div className={styles.aboutBody}>
      <Section heading="Release history">
        <div className={styles.changelogNotes} data-testid="changelog-notes">
          {sections.length > 0 ? (
            <ChangelogAccordion sections={sections} showLatestPill />
          ) : (
            <p className={styles.changelogEmpty} data-testid="changelog-empty">
              No release notes are bundled with this build.{' '}
              <InlineLink
                url={GITHUB_RELEASES_URL}
                ariaLabel="View releases on GitHub"
              >
                View releases on GitHub
              </InlineLink>
            </p>
          )}
        </div>
      </Section>
    </div>
  );
}
