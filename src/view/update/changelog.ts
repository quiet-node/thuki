/**
 * Parsing + selection helpers for the multi-version "What's New" notes.
 *
 * The updater manifest's `notes` field carries the full release history (every
 * released section of CHANGELOG.md). The window slices that history down to the
 * versions the user skipped over and renders them as an accordion. Keeping the
 * logic here as pure functions makes it unit-testable without a webview and
 * keeps `UpdateWindow` declarative.
 */

export interface ChangelogSection {
  /** Semver string from the section header, e.g. "0.14.0". */
  version: string;
  /** ISO date from the header (YYYY-MM-DD), or null when the header omits it. */
  date: string | null;
  /** Markdown beneath the header (the "### Features" lists), trimmed. */
  body: string;
}

/** release-please version header: `## [0.14.0](compare-url) (2026-06-07)`. */
const VERSION_HEADER = /^##\s+\[?(\d+\.\d+\.\d+)\]?/;
/** Trailing date in a version header. */
const HEADER_DATE = /\((\d{4}-\d{2}-\d{2})\)/;
/** Any level-2 header, used to detect where one section ends and the next begins. */
const LEVEL_2 = /^##\s+/;
/** Bare semver, for validating the installed-version string before comparing. */
const SEMVER = /^\d+\.\d+\.\d+$/;

/**
 * Splits changelog markdown into one section per version. Content before the
 * first version header (document title, preamble, "## Unreleased") and content
 * under any non-version level-2 header is discarded. Returns [] when no version
 * header is present, which the window treats as "render the body as-is."
 */
export function parseChangelogSections(markdown: string): ChangelogSection[] {
  const sections: ChangelogSection[] = [];
  let current: {
    version: string;
    date: string | null;
    lines: string[];
  } | null = null;

  const flush = () => {
    if (current) {
      sections.push({
        version: current.version,
        date: current.date,
        body: current.lines.join('\n').trim(),
      });
      current = null;
    }
  };

  for (const line of markdown.split('\n')) {
    if (LEVEL_2.test(line)) {
      flush();
      const match = line.match(VERSION_HEADER);
      if (match) {
        const date = line.match(HEADER_DATE);
        current = { version: match[1], date: date ? date[1] : null, lines: [] };
      }
      continue;
    }
    if (current) current.lines.push(line);
  }
  flush();

  return sections;
}

/**
 * Compares two `x.y.z` strings: >0 when `a` is newer, <0 when older, 0 when
 * equal. Callers only pass semver-shaped strings (the parser emits them; the
 * selection guard validates the installed version first).
 */
export function compareSemver(a: string, b: string): number {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < 3; i += 1) {
    if (pa[i] !== pb[i]) return pa[i] - pb[i];
  }
  return 0;
}

/**
 * Keeps the sections newer than the user's installed version, newest first.
 * When `current` is null or not semver-shaped (the app-version lookup has not
 * resolved yet or failed), no lower bound is applied and every section is kept.
 */
export function selectSections(
  sections: ChangelogSection[],
  current: string | null,
): ChangelogSection[] {
  const lowerBound = current !== null && SEMVER.test(current) ? current : null;
  const kept =
    lowerBound === null
      ? [...sections]
      : sections.filter((s) => compareSemver(s.version, lowerBound) > 0);
  return kept.sort((a, b) => compareSemver(b.version, a.version));
}
