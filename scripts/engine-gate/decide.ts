// Entrypoint for the engine regression gate. Thin I/O: it reads the signals the
// workflow collected (chat responses + observed throughput from collect.ts, and
// the codesign result), runs the responses through the unit-tested assertion
// core, reports the throughput as an informational number, prints the markdown
// verdict to the step summary, and exits non-zero if a blocking check failed.

import { appendFileSync, readFileSync } from 'node:fs';

import { evaluateAll } from './assertions';
import { CORRECTNESS_PROMPTS } from './prompts';
import { overallPass, renderGateSummary, type GateSection } from './report';

function required(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`engine-gate decide: missing required env ${name}`);
  }
  return value;
}

function readJson<T>(name: string): T {
  return JSON.parse(readFileSync(required(name), 'utf8')) as T;
}

function main(): void {
  const result = readJson<{ responses: Record<string, string>; tps: number }>(
    'GATE_RESULT',
  );
  const codesign = readJson<{ pass: boolean; detail: string }>('GATE_CODESIGN');

  const correctness = evaluateAll(CORRECTNESS_PROMPTS, result.responses);
  const passed = CORRECTNESS_PROMPTS.length - correctness.failures.length;

  const sections: GateSection[] = [
    {
      name: 'Loads + inference',
      pass: correctness.pass,
      detail: correctness.pass
        ? `${passed}/${CORRECTNESS_PROMPTS.length} prompts correct`
        : `failed: ${correctness.failures.join(', ')}`,
    },
    { name: 'Codesign', pass: codesign.pass, detail: codesign.detail },
    {
      name: 'Throughput',
      pass: true,
      informational: true,
      detail: `${result.tps.toFixed(1)} tok/s: reported for review, not a pass/fail gate; noisy on shared CI runners`,
    },
  ];

  const summary = renderGateSummary('Engine regression gate', sections);
  process.stdout.write(`${summary}\n`);

  const summaryPath = process.env.GITHUB_STEP_SUMMARY;
  if (summaryPath) {
    appendFileSync(summaryPath, `${summary}\n`);
  }

  if (!overallPass(sections)) {
    process.exitCode = 1;
  }
}

main();
