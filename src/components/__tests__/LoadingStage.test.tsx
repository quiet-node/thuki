import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { LoadingStage } from '../LoadingStage';

describe('LoadingStage', () => {
  beforeEach(() => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn().mockReturnValue({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('delegates to RequestStatusStrip with the same label', () => {
    render(<LoadingStage label="Reading model weights…" />);
    expect(screen.getByTestId('request-status-strip')).toBeInTheDocument();
    expect(screen.getByTestId('loading-label').textContent).toBe(
      'Reading model weights…',
    );
  });

  it('passes compact and labelPrefix through', () => {
    render(
      <LoadingStage
        compact
        label="Reasoning..."
        labelPrefix={<span data-testid="chev">▾</span>}
      />,
    );
    expect(screen.getByTestId('chev')).toBeInTheDocument();
    expect(screen.getByTestId('loading-stage-title').className).toContain(
      'text-[11px]',
    );
  });
});
