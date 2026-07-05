// The fixed correctness prompts the gate sends to the engine over the /v1 chat
// endpoint. Each has an unambiguous answer that survives benign numerical drift
// across an engine bump, so a passing set means "the engine loaded, ran the chat
// template, tokenized, and generated coherent, correct text" — not "the tokens
// are byte-identical to a frozen golden." Kept small so the gate stays fast on
// every PR.

import type { Assertion } from './assertions';

export const CORRECTNESS_PROMPTS: Assertion[] = [
  {
    id: 'capital',
    prompt: 'What is the capital of France? Reply with just the city name.',
    kind: 'contains',
    needle: 'Paris',
  },
  {
    id: 'arithmetic',
    prompt: 'What is 17 + 25? Reply with only the number.',
    kind: 'contains',
    needle: '42',
  },
  {
    id: 'json',
    prompt: 'Reply with exactly this JSON and nothing else: {"ok": true}',
    kind: 'jsonEquals',
    expected: { ok: true },
  },
];
