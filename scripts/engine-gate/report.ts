// Renders the engine-gate verdict as markdown. Shared by the CI job summary and
// the auto-bump PR body so a reviewer reads the exact same pass/fail breakdown in
// both places.

export interface GateSection {
  name: string;
  pass: boolean;
  detail?: string;
  /** Reported for the reviewer but never gates: excluded from the verdict. */
  informational?: boolean;
}

// True only when there is at least one section and every blocking section passed.
// Informational sections are reported, not judged. An empty list is a failure: it
// means no checks ran, which must never read as a pass.
export function overallPass(sections: GateSection[]): boolean {
  return sections.length > 0 && sections.every((s) => s.informational || s.pass);
}

// Markdown: a verdict header plus one checklist line per section.
export function renderGateSummary(title: string, sections: GateSection[]): string {
  const verdict = overallPass(sections) ? 'PASS' : 'FAIL';
  const lines = sections.map((s) => {
    const mark = s.informational ? 'ℹ️' : s.pass ? '✅' : '❌';
    const detail = s.detail ? ` — ${s.detail}` : '';
    return `- ${mark} ${s.name}${detail}`;
  });
  return [`### ${title}: ${verdict}`, '', ...lines, ''].join('\n');
}
