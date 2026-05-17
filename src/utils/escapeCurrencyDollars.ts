/**
 * Disambiguates currency from LaTeX math before markdown parsing.
 *
 * Streamdown's math plugin uses `remark-math`, which treats `$...$` as inline
 * math. `micromark-extension-math` has no currency awareness (its only knob is
 * the all-or-nothing `singleDollarTextMath`), so a message like
 * "raise $1M ... reach $1M" has everything between the two `$` parsed as one
 * giant math expression: KaTeX renders it in a nowrap serif run that blows the
 * chat window out horizontally.
 *
 * The fix is the one deterministic, low-false-positive rule that exists at the
 * syntax level: a `$` immediately followed by a digit is currency, not a math
 * opener (`$5`, `$1M`, `$1,000`, `$10.50`). Such dollars are backslash-escaped
 * so `remark-math` treats them as literal text. Genuine inline math virtually
 * always opens with a letter or backslash (`$E=mc^2$`, `$\alpha$`) and is left
 * untouched, as is all `$$...$$` block math.
 *
 * Documented trade-off: digit-led math like `$2x$` or `$3.14$` is
 * indistinguishable from currency at the syntax level and is therefore treated
 * as currency (rendered as readable literal text, not typeset). There is no
 * reliable syntactic rule that separates the two; `.katex-display` overflow
 * containment in App.css is the structural backstop that guarantees layout can
 * never break regardless.
 *
 * Code is never altered: `$` inside fenced code blocks (``` / ~~~) and inline
 * code spans is left exactly as written. Known minor limitation: 4-space
 * indented code blocks are not detected (they are rare in chat and require a
 * full block parser to identify correctly); a `$<digit>` in one would be
 * escaped. This is acceptable given the dominant failure mode is prose.
 *
 * Pure and idempotent on already-safe input. Applied at the single markdown
 * chokepoint (MarkdownRenderer) so every caller (user messages, assistant
 * messages, thinking blocks, update notes) is protected uniformly.
 */

/** A `$` that is currency: preceded by start-of-segment or any char that is
 *  not a backslash (already-escaped) or another `$` (`$$` block delimiter),
 *  and immediately followed by a digit. */
const CURRENCY_DOLLAR = /(^|[^\\$])\$(?=\d)/g;

/** Open fence: up to 3 spaces of indent then a run of 3+ backticks or tildes. */
const FENCE_OPEN = /^\s{0,3}(`{3,}|~{3,})/;

function escapeInText(text: string): string {
  return text.replace(
    CURRENCY_DOLLAR,
    (_match, prefix: string) => prefix + '\\$',
  );
}

/**
 * Finds the index of a closing inline-code backtick run of exactly `len`
 * backticks at or after `from`, or -1 if there is none. A run longer than
 * `len` cannot close the span (CommonMark), so it is skipped wholesale.
 */
function findClosingBackticks(line: string, from: number, len: number): number {
  let i = from;
  while (i < line.length) {
    if (line[i] === '`') {
      let run = 0;
      while (i + run < line.length && line[i + run] === '`') run++;
      if (run === len) return i;
      i += run;
    } else {
      i++;
    }
  }
  return -1;
}

/**
 * Escapes currency dollars in a single non-fenced line while leaving every
 * inline code span (any backtick run length) byte-for-byte intact.
 */
function escapeOutsideInlineCode(line: string): string {
  if (line.indexOf('$') === -1) return line;
  let result = '';
  let i = 0;
  while (i < line.length) {
    if (line[i] === '`') {
      let len = 0;
      while (i + len < line.length && line[i + len] === '`') len++;
      const close = findClosingBackticks(line, i + len, len);
      if (close === -1) {
        // Unbalanced backticks: the rest of the line is ordinary text.
        result += escapeInText(line.slice(i));
        break;
      }
      result += line.slice(i, close + len); // code span, verbatim
      i = close + len;
    } else {
      let j = i;
      while (j < line.length && line[j] !== '`') j++;
      result += escapeInText(line.slice(i, j));
      i = j;
    }
  }
  return result;
}

/**
 * Escapes `$` used as currency so it is not misparsed as LaTeX inline math,
 * leaving genuine math and all code regions untouched. See the module comment
 * for the rule and its documented trade-off.
 */
export function escapeCurrencyDollars(md: string): string {
  if (!md || md.indexOf('$') === -1) return md;

  const lines = md.split('\n');
  let inFence = false;
  let fenceChar = '';
  let fenceLen = 0;
  const out: string[] = [];

  for (const line of lines) {
    if (inFence) {
      out.push(line);
      const close = new RegExp(`^\\s{0,3}${fenceChar}{${fenceLen},}\\s*$`);
      if (close.test(line)) inFence = false;
      continue;
    }
    const open = FENCE_OPEN.exec(line);
    if (open) {
      inFence = true;
      fenceChar = open[1][0];
      fenceLen = open[1].length;
      out.push(line);
      continue;
    }
    out.push(escapeOutsideInlineCode(line));
  }
  return out.join('\n');
}
