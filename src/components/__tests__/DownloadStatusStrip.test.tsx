import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import { DownloadStatusStrip } from '../DownloadStatusStrip';

describe('DownloadStatusStrip', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows the model name, percent and ETA while downloading', () => {
    render(
      <DownloadStatusStrip
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

  it('alternates the label with the background hint every few seconds', () => {
    vi.useFakeTimers();
    render(
      <DownloadStatusStrip
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
    act(() => vi.advanceTimersByTime(7000));
    expect(
      screen.getByText('You can close and come back anytime'),
    ).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(7000));
    expect(screen.getByText('Downloading Qwen3.5 9B')).toBeInTheDocument();
  });

  it('omits the ETA when it is not yet measurable', () => {
    render(
      <DownloadStatusStrip
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
    render(<DownloadStatusStrip status={{ kind: 'pausing', percent: 40 }} />);
    expect(screen.getByText('Pausing…')).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('shows a paused state with Resume but no Discard', () => {
    const onResume = vi.fn();
    render(
      <DownloadStatusStrip
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
    render(<DownloadStatusStrip status={{ kind: 'verifying', percent: 40 }} />);
    expect(screen.getByText('Verifying…')).toBeInTheDocument();
    expect(
      screen.getByText('This can take a minute for large models'),
    ).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('names the model and invites the first message when ready', () => {
    render(
      <DownloadStatusStrip
        status={{ kind: 'ready', modelName: 'Qwen3.5 9B' }}
      />,
    );
    expect(
      screen.getByText('Qwen3.5 9B ready. Send your first message!'),
    ).toBeInTheDocument();
  });

  it('shows a failure message with a Retry button', () => {
    const onRetry = vi.fn();
    render(
      <DownloadStatusStrip
        status={{ kind: 'failed', message: 'Download failed', onRetry }}
      />,
    );
    expect(screen.getByText('Download failed')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry download' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });
});
