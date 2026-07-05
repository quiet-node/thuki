// Semantic correctness assertions for engine-gate chat responses. Deliberately
// tolerant of real model output (code fences, surrounding prose) and asserts
// meaning, not bytes: greedy decoding is only reproducible on the same build and
// hardware, so a byte-exact golden would false-fail across an engine bump while
// genuine breakage (crash, garbage, broken chat template) still trips these.

// A single correctness check: the prompt to send and how its response is judged.
// `contains` asserts a substring (case-insensitive); `jsonEquals` asserts the
// response embeds a JSON object deep-equal to `expected`.
export type Assertion =
  | { id: string; prompt: string; kind: 'contains'; needle: string }
  | { id: string; prompt: string; kind: 'jsonEquals'; expected: unknown };

// Order-insensitive canonical form of a JSON value, so object key order does not
// affect equality.
function canonicalize(value: unknown): string {
  if (value === null || typeof value !== 'object') {
    return JSON.stringify(value) ?? 'null';
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalize).join(',')}]`;
  }
  const entries = Object.entries(value as Record<string, unknown>).sort(
    ([a], [b]) => (a < b ? -1 : a > b ? 1 : 0),
  );
  return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${canonicalize(v)}`).join(',')}}`;
}

// Judges a single response against its assertion.
export function evaluateResponse(assertion: Assertion, response: string): boolean {
  if (assertion.kind === 'contains') {
    return response.toLowerCase().includes(assertion.needle.toLowerCase());
  }
  const parsed = extractJsonObject(response);
  if (parsed === null) {
    return false;
  }
  return canonicalize(parsed) === canonicalize(assertion.expected);
}

export interface CorrectnessResult {
  pass: boolean;
  failures: string[];
}

// Evaluates every assertion against a map of response text keyed by assertion id.
// A missing response counts as a failure. Returns the ids that failed, in order.
export function evaluateAll(
  assertions: Assertion[],
  responses: Record<string, string>,
): CorrectnessResult {
  const failures = assertions
    .filter((a) => !evaluateResponse(a, responses[a.id] ?? ''))
    .map((a) => a.id);
  return { pass: failures.length === 0, failures };
}

// Extracts the first balanced JSON object embedded in `text` and parses it,
// ignoring code fences and surrounding prose. Returns null when there is no
// parseable object. Brace matching is string-aware so a `}` inside a JSON string
// does not prematurely close the span.
export function extractJsonObject(text: string): unknown | null {
  const start = text.indexOf('{');
  if (start === -1) {
    return null;
  }

  let depth = 0;
  let inString = false;
  let escaped = false;

  for (let i = start; i < text.length; i++) {
    const ch = text[i];

    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (ch === '\\') {
        escaped = true;
      } else if (ch === '"') {
        inString = false;
      }
      continue;
    }

    if (ch === '"') {
      inString = true;
    } else if (ch === '{') {
      depth++;
    } else if (ch === '}') {
      depth--;
      if (depth === 0) {
        try {
          return JSON.parse(text.slice(start, i + 1));
        } catch {
          return null;
        }
      }
    }
  }

  return null;
}
