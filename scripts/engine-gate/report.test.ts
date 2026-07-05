import { describe, expect, test } from 'vitest';

import { overallPass, renderGateSummary, type GateSection } from './report';

const sections: GateSection[] = [
  { name: 'Loads + inference', pass: true, detail: '3/3 prompts correct' },
  { name: 'Codesign', pass: true, detail: 'binary + 9 dylibs verified' },
];

describe('overallPass', () => {
  test('is true only when every section passes', () => {
    expect(overallPass(sections)).toBe(true);
    expect(overallPass([...sections, { name: 'Perf', pass: false }])).toBe(false);
  });

  test('is false for an empty section list', () => {
    expect(overallPass([])).toBe(false);
  });
});

describe('renderGateSummary', () => {
  test('renders an overall verdict and a checklist line per section', () => {
    const md = renderGateSummary('Engine gate', sections);

    expect(md).toContain('### Engine gate: PASS');
    expect(md).toContain('- ✅ Loads + inference — 3/3 prompts correct');
    expect(md).toContain('- ✅ Codesign — binary + 9 dylibs verified');
  });

  test('marks the verdict FAIL and flags failing sections', () => {
    const md = renderGateSummary('Engine gate', [
      { name: 'Perf', pass: false, detail: 'ratio 0.60 < 0.75' },
    ]);

    expect(md).toContain('### Engine gate: FAIL');
    expect(md).toContain('- ❌ Perf — ratio 0.60 < 0.75');
  });

  test('omits the trailing detail when a section has none', () => {
    const md = renderGateSummary('Engine gate', [{ name: 'Perf', pass: true }]);

    expect(md).toContain('- ✅ Perf\n');
    expect(md).not.toContain('Perf —');
  });
});
