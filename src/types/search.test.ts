import { describe, expect, it } from 'vitest';
import type { SearchResultPreview, SearchStage } from './search';

describe('search types', () => {
  it('SearchResultPreview carries title and url', () => {
    const source: SearchResultPreview = {
      title: 'Tokio tutorial',
      url: 'https://tokio.rs/tokio/tutorial',
    };
    expect(source.title).toBe('Tokio tutorial');
    expect(source.url).toContain('tokio.rs');
  });

  it('SearchStage refining_search carries attempt and total', () => {
    const stage: SearchStage = {
      kind: 'refining_search',
      attempt: 2,
      total: 3,
    };
    if (stage && stage.kind === 'refining_search') {
      expect(stage.attempt).toBe(2);
      expect(stage.total).toBe(3);
    }
  });

  it('SearchStage verifying_sources marks post-stream citation audit', () => {
    const stage: SearchStage = { kind: 'verifying_sources' };
    expect(stage?.kind).toBe('verifying_sources');
  });

  it('SearchStage idle is null', () => {
    const stage: SearchStage = null;
    expect(stage).toBeNull();
  });

  it('SearchEvent still allows InsufficientMemory', () => {
    const event: SearchEvent = { type: 'InsufficientMemory' };
    expect(event.type).toBe('InsufficientMemory');
  });
});
