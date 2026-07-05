// Renders the engine-gate verdict as markdown. Shared by the CI job summary and
// the auto-bump PR body so a reviewer reads the exact same pass/fail breakdown in
// both places.

export interface GateSection {
  name: string;
  pass: boolean;
  detail?: string;
}

// True only when there is at least one section and all of them passed. An empty
// list is a failure: it means no checks ran, which must never read as a pass.
export function overallPass(sections: GateSection[]): boolean {
  return sections.length > 0 && sections.every((s) => s.pass);
}

// Markdown: a verdict header plus one checklist line per section.
export function renderGateSummary(title: string, sections: GateSection[]): string {
  const verdict = overallPass(sections) ? 'PASS' : 'FAIL';
  const lines = sections.map((s) => {
    const mark = s.pass ? '✅' : '❌';
    const detail = s.detail ? ` — ${s.detail}` : '';
    return `- ${mark} ${s.name}${detail}`;
  });
  return [`### ${title}: ${verdict}`, '', ...lines, ''].join('\n');
}
