import { render, screen, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { RequestStatusStrip } from '../RequestStatusStrip';

describe('RequestStatusStrip', () => {
  beforeEach(() => {
    vi.useFakeTimers();
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
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('renders dots without a label when label is omitted', () => {
    render(<RequestStatusStrip />);
    expect(screen.getByTestId('request-status-strip')).toBeInTheDocument();
    expect(screen.getByTestId('three-dot-motion')).toBeInTheDocument();
    expect(screen.queryByTestId('loading-label')).toBeNull();
  });

  it('shows the label with loading-label test id for shimmer hooks', () => {
    render(<RequestStatusStrip label="Analyzing query" />);
    const label = screen.getByTestId('loading-label');
    expect(label.textContent).toBe('Analyzing query');
    expect(label).toHaveAttribute('data-label', 'Analyzing query');
  });

  it('uses the shared title contract for label typography', () => {
    render(<RequestStatusStrip label="Searching the web" />);
    expect(screen.getByTestId('loading-stage-title').className).toContain(
      'request-status-strip__title',
    );
  });

  it('runs tracking-settle when the label string changes', () => {
    const { rerender } = render(<RequestStatusStrip label="Analyzing query" />);
    expect(screen.getByTestId('loading-label').textContent).toBe(
      'Analyzing query',
    );
    rerender(<RequestStatusStrip label="Searching the web" />);
    // Outgoing phase still shows previous copy until TRACK_OUT_MS.
    expect(screen.getByTestId('loading-label').textContent).toBe(
      'Analyzing query',
    );
    act(() => {
      vi.advanceTimersByTime(400);
    });
    expect(screen.getByTestId('loading-label').textContent).toBe(
      'Searching the web',
    );
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(screen.getByTestId('loading-label').className).toContain(
      'loading-label-track-in',
    );
  });

  it('clears the label immediately when it becomes empty', () => {
    const { rerender } = render(<RequestStatusStrip label="Analyzing query" />);
    rerender(<RequestStatusStrip label={null} />);
    expect(screen.queryByTestId('loading-label')).toBeNull();
  });

  it('supersedes an in-flight tracking-settle when the label changes again', () => {
    const { rerender } = render(<RequestStatusStrip label="Analyzing query" />);
    rerender(<RequestStatusStrip label="Searching the web" />);
    act(() => {
      vi.advanceTimersByTime(100);
    });
    rerender(<RequestStatusStrip label="Reading sources" />);
    act(() => {
      vi.advanceTimersByTime(400);
    });
    expect(screen.getByTestId('loading-label').textContent).toBe(
      'Reading sources',
    );
  });

  it('ignores a no-op label update to the same string', () => {
    const { rerender } = render(<RequestStatusStrip label="Analyzing query" />);
    rerender(<RequestStatusStrip label="Analyzing query" />);
    expect(screen.getByTestId('loading-label').textContent).toBe(
      'Analyzing query',
    );
  });
});
