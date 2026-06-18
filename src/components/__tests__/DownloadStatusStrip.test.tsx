import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { DownloadStatusStrip } from '../DownloadStatusStrip';

describe('DownloadStatusStrip', () => {
  it('shows the setup label, percent and ETA while downloading', () => {
    render(
      <DownloadStatusStrip
        status={{
          kind: 'downloading',
          percent: 62,
          etaSeconds: 90,
          onPause: vi.fn(),
        }}
      />,
    );
    expect(screen.getByText('Setting up your model')).toBeInTheDocument();
    expect(screen.getByText('62% · 1m left')).toBeInTheDocument();
  });

  it('omits the ETA when it is not yet measurable', () => {
    render(
      <DownloadStatusStrip
        status={{
          kind: 'downloading',
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
        status={{ kind: 'downloading', percent: 40, etaSeconds: 60, onPause }}
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

  it('shows a paused state with Resume and Discard', () => {
    const onResume = vi.fn();
    const onDiscard = vi.fn();
    render(
      <DownloadStatusStrip
        status={{ kind: 'paused', percent: 58, onResume, onDiscard }}
      />,
    );
    expect(screen.getByText('Paused · 58%')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Resume download' }));
    expect(onResume).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'Discard download' }));
    expect(onDiscard).toHaveBeenCalledTimes(1);
  });

  it('shows a ready state', () => {
    render(<DownloadStatusStrip status={{ kind: 'ready' }} />);
    expect(screen.getByText('Model ready')).toBeInTheDocument();
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
