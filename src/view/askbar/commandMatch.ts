import type { EntityMatch } from '@lexical/text';
import { COMMANDS } from '../../config/commands';

/** All known slash-command triggers, e.g. "/search". */
const TRIGGERS: readonly string[] = COMMANDS.map((c) => c.trigger);

/** True when `ch` is undefined (string edge) or a whitespace character. */
function isBoundary(ch: string | undefined): boolean {
  return ch === undefined || /\s/.test(ch);
}

/**
 * Finds the earliest known slash-command token in `text` that is bounded by
 * whitespace (or the string edges) on both sides, returning its character
 * range. `registerLexicalTextEntity` calls this to wrap command triggers in a
 * styled CommandNode so they render violet while the caret stays native.
 *
 * Word-boundary aware: "/searching" does not match "/search". Pure and
 * exported for direct unit testing.
 */
export function getCommandMatch(text: string): EntityMatch | null {
  let best: EntityMatch | null = null;
  for (const trigger of TRIGGERS) {
    let from = 0;
    // Find the first boundary-valid occurrence of this trigger.
    for (;;) {
      const idx = text.indexOf(trigger, from);
      if (idx === -1) break;
      const beforeOk = idx === 0 || isBoundary(text[idx - 1]);
      const afterOk = isBoundary(text[idx + trigger.length]);
      if (beforeOk && afterOk) {
        if (best === null || idx < best.start) {
          best = { start: idx, end: idx + trigger.length };
        }
        break;
      }
      from = idx + trigger.length;
    }
  }
  return best;
}
