import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
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

  it('rotates the label through download, safe-to-close, and browse on the ask bar', () => {
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
    expect(
      screen.getByRole('button', { name: /browse more models in settings/i }),
    ).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.getByText('Downloading Qwen3.5 9B')).toBeInTheDocument();
  });

  it('opens Settings on the browse label click', () => {
    vi.useFakeTimers();
    vi.mocked(invoke).mockClear();
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
    // Rotate past the model name and the safe-to-close hint to the browse label.
    act(() => vi.advanceTimersByTime(10000));
    fireEvent.click(
      screen.getByRole('button', { name: /browse more models in settings/i }),
    );
    expect(invoke).toHaveBeenCalledWith('open_settings_window');
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
    act(() => vi.advanceTimersByTime(10000));
    expect(screen.getByText('Downloading Qwen3.5 9B')).toBeInTheDocument();
    expect(screen.queryByText(/Safe to close/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /browse more models in settings/i }),
    ).not.toBeInTheDocument();
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

  it('shows a queued state with only Cancel and no queue badge below the threshold', () => {
    const onCancel = vi.fn();
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{ kind: 'queued', onCancel }}
      />,
    );
    expect(
      screen.getByText('Waiting for a download slot…'),
    ).toBeInTheDocument();
    expect(screen.queryByText(/in queue/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel download' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('omits the queue badge when only one other download is queued', () => {
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{ kind: 'queued', onCancel: () => {}, queueDepth: 1 }}
      />,
    );
    expect(screen.queryByText(/in queue/)).not.toBeInTheDocument();
  });

  it('shows the queue badge once at least one other download is also queued', () => {
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{ kind: 'queued', onCancel: () => {}, queueDepth: 2 }}
      />,
    );
    expect(screen.getByText('· #2 in queue')).toBeInTheDocument();
  });

  it('shows a paused state with both Resume and Discard', () => {
    const onResume = vi.fn();
    const onDiscard = vi.fn();
    render(
      <DownloadStatusStrip
        surface="askbar"
        status={{ kind: 'paused', percent: 58, onResume, onDiscard }}
      />,
    );
    expect(screen.getByText('Paused · 58%')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Resume download' }));
    expect(onResume).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'Discard download' }));
    expect(onDiscard).toHaveBeenCalledTimes(1);
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
    [
      { kind: 'paused', percent: 1, onResume: () => {}, onDiscard: () => {} },
      true,
    ],
    [{ kind: 'pausing', percent: 1 }, true],
    [{ kind: 'verifying', percent: 1 }, true],
    [{ kind: 'ready', modelName: 'X' }, false],
    [{ kind: 'failed', message: 'x', onRetry: () => {} }, false],
  ];

  it.each(cases)('returns %o -> %s', (status, expected) => {
    expect(isDownloadActive(status)).toBe(expected);
  });
});
