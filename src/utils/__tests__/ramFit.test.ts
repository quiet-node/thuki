import { describe, expect, it } from 'vitest';

import { RAM_FIT_LABEL, RAM_FIT_TOOLTIP } from '../ramFit';
import type { RamFit } from '../../types/starter';

const VERDICTS: RamFit[] = ['fits', 'tight', 'too_big'];

describe('RAM-fit copy', () => {
  it('keeps each tooltip a single short, clean sentence', () => {
    for (const verdict of VERDICTS) {
      const tip = RAM_FIT_TOOLTIP[verdict];
      // Short and clean: a handful of words ending in a period, no clauses.
      expect(tip.length).toBeLessThanOrEqual(30);
      expect(tip).toMatch(/^[^;]+\.$/);
    }
  });

  it('exposes a label for every verdict', () => {
    for (const verdict of VERDICTS) {
      expect(RAM_FIT_LABEL[verdict]).toBeTruthy();
    }
  });
});
