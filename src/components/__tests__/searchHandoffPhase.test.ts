import { describe, expect, it } from 'vitest';
import {
  completeSearchHandoffExit,
  nextSearchHandoffPhase,
  SEARCH_HANDOFF_COLLAPSE_LEAD_MS,
  SEARCH_HANDOFF_EXIT_FALLBACK_MS,
  type SearchHandoffPhase,
} from '../searchHandoffPhase';

describe('nextSearchHandoffPhase', () => {
  const cases: Array<{
    name: string;
    prev: SearchHandoffPhase;
    showLiveSearch: boolean;
    handedOff: boolean;
    want: SearchHandoffPhase;
  }> = [
    {
      name: 'idle + live search → live',
      prev: 'idle',
      showLiveSearch: true,
      handedOff: false,
      want: 'live',
    },
    {
      name: 'done + live search (new turn) → live',
      prev: 'done',
      showLiveSearch: true,
      handedOff: false,
      want: 'live',
    },
    {
      name: 'exiting + live search → live',
      prev: 'exiting',
      showLiveSearch: true,
      handedOff: false,
      want: 'live',
    },
    {
      name: 'live + still pure search → live',
      prev: 'live',
      showLiveSearch: true,
      handedOff: false,
      want: 'live',
    },
    {
      name: 'live + handedOff → exiting',
      prev: 'live',
      showLiveSearch: false,
      handedOff: true,
      want: 'exiting',
    },
    {
      name: 'exiting + still handedOff → exiting (sticky)',
      prev: 'exiting',
      showLiveSearch: false,
      handedOff: true,
      want: 'exiting',
    },
    {
      name: 'idle + handedOff without ever live → done',
      prev: 'idle',
      showLiveSearch: false,
      handedOff: true,
      want: 'done',
    },
    {
      name: 'done + still handedOff → done',
      prev: 'done',
      showLiveSearch: false,
      handedOff: true,
      want: 'done',
    },
    {
      name: 'live + neither live nor handoff (cancel) → idle',
      prev: 'live',
      showLiveSearch: false,
      handedOff: false,
      want: 'idle',
    },
    {
      name: 'exiting + cancel (no handoff) → idle',
      prev: 'exiting',
      showLiveSearch: false,
      handedOff: false,
      want: 'idle',
    },
    {
      name: 'idle + idle → idle',
      prev: 'idle',
      showLiveSearch: false,
      handedOff: false,
      want: 'idle',
    },
    {
      name: 'done + cancel without content → idle',
      prev: 'done',
      showLiveSearch: false,
      handedOff: false,
      want: 'idle',
    },
  ];

  for (const c of cases) {
    it(c.name, () => {
      expect(
        nextSearchHandoffPhase(c.prev, {
          showLiveSearch: c.showLiveSearch,
          handedOff: c.handedOff,
        }),
      ).toBe(c.want);
    });
  }
});

describe('completeSearchHandoffExit', () => {
  it('moves exiting → done', () => {
    expect(completeSearchHandoffExit('exiting')).toBe('done');
  });

  it('is a no-op for other phases', () => {
    expect(completeSearchHandoffExit('idle')).toBe('idle');
    expect(completeSearchHandoffExit('live')).toBe('live');
    expect(completeSearchHandoffExit('done')).toBe('done');
  });
});

describe('handoff timing constants', () => {
  it('collapse lead is under body height duration and below fallback', () => {
    expect(SEARCH_HANDOFF_COLLAPSE_LEAD_MS).toBe(160);
    expect(SEARCH_HANDOFF_COLLAPSE_LEAD_MS).toBeLessThan(
      SEARCH_HANDOFF_EXIT_FALLBACK_MS,
    );
  });

  it('caps stuck exit under half a second', () => {
    expect(SEARCH_HANDOFF_EXIT_FALLBACK_MS).toBe(500);
  });
});
