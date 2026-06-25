import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import {
  DownloadStatusStrip,
  isDownloadActive,
  type DownloadStripStatus,
} from '../DownloadStatusStrip';

describe('DownloadStatusStrip', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows the model name, percent and ETA while downloading', () => {
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{
          kind: 'downloading',
          modelName: 'Qwen3.5 9B',
          percent: 62,
          etaSeconds: 90,
          onPause: vi.fn(),
        }}
      />,
    );
    expect(screen.getByText('Downloading Qwen3.5 9B')).toBeInTheDocument();
    expect(screen.getByText('62% · 1m left')).toBeInTheDocument();
  });

  it('alternates the label with the background hint on the ask bar', () => {
    vi.useFakeTimers();
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{
          kind: 'downloading',
          modelName: 'Qwen3.5 9B',
          percent: 30,
          etaSeconds: 120,
          onPause: vi.fn(),
        }}
      />,
    );
    expect(screen.getByText('Downloading Qwen3.5 9B')).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.getByText(/Safe to close/)).toBeInTheDocument();
    // The Control glyph renders as a keycap, not a bare caret.
    expect(screen.getByText('⌃')).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.getByText('Downloading Qwen3.5 9B')).toBeInTheDocument();
  });

  it('does not alternate the label during onboarding', () => {
    vi.useFakeTimers();
    render(
      <DownloadStatusStrip
        surface="onboarding"
        status={{
          kind: 'downloading',
          modelName: 'Qwen3.5 9B',
          percent: 30,
          etaSeconds: 120,
          onPause: vi.fn(),
        }}
      />,
    );
    expect(screen.getByText('Downloading Qwen3.5 9B')).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.getByText('Downloading Qwen3.5 9B')).toBeInTheDocument();
    expect(screen.queryByText(/Safe to close/)).not.toBeInTheDocument();
  });

  it('omits the ETA when it is not yet measurable', () => {
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{
          kind: 'downloading',
          modelName: 'Qwen3.5 9B',
          percent: 5,
          etaSeconds: null,
          onPause: vi.fn(),
        }}
      />,
    );
    expect(screen.getByText('5%')).toBeInTheDocument();
  });

  it('formats hour-scale and second-scale ETAs', () => {
    const { rerender } = render(
      <DownloadStatusStrip
        surface="askbar"
        status={{
          kind: 'downloading',
          modelName: 'Qwen3.5 9B',
          percent: 1,
          etaSeconds: 3700,
          onPause: vi.fn(),
        }}
      />,
    );
    expect(screen.getByText('1% · 1h 1m left')).toBeInTheDocument();
    rerender(
      <DownloadStatusStrip
        surface="askbar"
        status={{
          kind: 'downloading',
          modelName: 'Qwen3.5 9B',
          percent: 99,
          etaSeconds: 30,
          onPause: vi.fn(),
        }}
      />,
    );
    expect(screen.getByText('99% · 30s left')).toBeInTheDocument();
  });

  it('pauses the download from the downloading state', () => {
    const onPause = vi.fn();
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{
          kind: 'downloading',
          modelName: 'Qwen3.5 9B',
          percent: 40,
          etaSeconds: 60,
          onPause,
        }}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Pause download' }));
    expect(onPause).toHaveBeenCalledTimes(1);
  });

  it('shows a pausing state (no controls) while the cancel lands', () => {
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{ kind: 'pausing', percent: 40 }}
      />,
    );
    expect(screen.getByText('Pausing…')).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('shows a paused state with Resume but no Discard', () => {
    const onResume = vi.fn();
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{ kind: 'paused', percent: 58, onResume }}
      />,
    );
    expect(screen.getByText('Paused · 58%')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Resume download' }));
    expect(onResume).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByRole('button', { name: 'Discard download' }),
    ).not.toBeInTheDocument();
  });

  it('reassures that verifying can take a while during the re-hash', () => {
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{ kind: 'verifying', percent: 40 }}
      />,
    );
    expect(screen.getByText('Verifying…')).toBeInTheDocument();
    expect(
      screen.getByText('This can take a minute for large models'),
    ).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('invites the first message when ready on the ask bar', () => {
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{ kind: 'ready', modelName: 'Qwen3.5 9B' }}
      />,
    );
    expect(
      screen.getByText('Qwen3.5 9B ready. Send your first message!'),
    ).toBeInTheDocument();
  });

  it('points to Get Started when ready during onboarding', () => {
    render(
      <DownloadStatusStrip
        surface="onboarding"
        status={{ kind: 'ready', modelName: 'Qwen3.5 9B' }}
      />,
    );
    expect(
      screen.getByText('Qwen3.5 9B ready. Hit Get Started to start chatting!'),
    ).toBeInTheDocument();
  });

  it('confirms readiness without "Get Started" on the onboarding roadmap', () => {
    render(
      <DownloadStatusStrip
        surface="onboarding-roadmap"
        status={{ kind: 'ready', modelName: 'Qwen3.5 9B' }}
      />,
    );
    expect(
      screen.getByText("Qwen3.5 9B ready. You're good to go!"),
    ).toBeInTheDocument();
  });

  it('shows a failure message with a Retry button', () => {
    const onRetry = vi.fn();
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{ kind: 'failed', message: 'Download failed', onRetry }}
      />,
    );
    expect(screen.getByText('Download failed')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry download' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });
});

describe('isDownloadActive', () => {
  const cases: Array<[DownloadStripStatus | null, boolean]> = [
    [null, false],
    [
      {
        kind: 'downloading',
        modelName: 'X',
        percent: 1,
        etaSeconds: null,
        onPause: () => {},
      },
      true,
    ],
    [{ kind: 'paused', percent: 1, onResume: () => {} }, true],
    [{ kind: 'pausing', percent: 1 }, true],
    [{ kind: 'verifying', percent: 1 }, true],
    [{ kind: 'ready', modelName: 'X' }, false],
    [{ kind: 'failed', message: 'x', onRetry: () => {} }, false],
  ];

  it.each(cases)('returns %o -> %s', (status, expected) => {
    expect(isDownloadActive(status)).toBe(expected);
  });
});
