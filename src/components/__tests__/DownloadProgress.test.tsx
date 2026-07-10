import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { DownloadProgress } from '../DownloadProgress';
import type { ConfirmInfo, DownloadProgressProps } from '../DownloadProgress';
import type {
  DownloadProgressInfo,
  DownloadUiState,
} from '../../hooks/useDownloadModel';

function renderProgress(
  state: DownloadUiState,
  overrides?: Partial<DownloadProgressProps>,
) {
  const handlers = {
    onConfirm: vi.fn(),
    onCancelConfirm: vi.fn(),
    onCancel: vi.fn(),
    onRetry: vi.fn(),
  };
  const utils = render(
    <DownloadProgress
      state={state}
      progress={null}
      etaSeconds={null}
      {...handlers}
      {...overrides}
    />,
  );
  return { ...utils, ...handlers };
}

const confirmInfo = (overrides?: Partial<ConfirmInfo>): ConfirmInfo => ({
  sizeGb: 8.2,
  freeDiskGb: 50,
  ramWarning: null,
  ...overrides,
});

describe('DownloadProgress', () => {
  it('renders nothing for idle and resume_pending', () => {
    const idle = renderProgress({ phase: 'idle' });
    expect(idle.container).toBeEmptyDOMElement();
    const pending = renderProgress({ phase: 'resume_pending' });
    expect(pending.container).toBeEmptyDOMElement();
  });

  describe('confirming', () => {
    it('shows the size, free disk space, and the action buttons', () => {
      const { onConfirm, onCancelConfirm } = renderProgress(
        { phase: 'confirming', tier: 'balanced' },
        { confirmInfo: confirmInfo() },
      );
      expect(screen.getByText('8.2 GB download.')).toBeInTheDocument();
      expect(
        screen.getByText('50.0 GB free on this disk.'),
      ).toBeInTheDocument();
      expect(
        screen.queryByText('Low on disk space. The download may not fit.'),
      ).not.toBeInTheDocument();

      fireEvent.click(screen.getByRole('button', { name: 'Download' }));
      expect(onConfirm).toHaveBeenCalledTimes(1);
      fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
      expect(onCancelConfirm).toHaveBeenCalledTimes(1);
    });

    it('warns when free disk is below size + 2 GB but keeps Download enabled', () => {
      renderProgress(
        { phase: 'confirming', tier: 'balanced' },
        { confirmInfo: confirmInfo({ freeDiskGb: 10.19 }) },
      );
      expect(
        screen.getByText('Low on disk space. The download may not fit.'),
      ).toBeInTheDocument();
      // Warn, never block: the Download button stays clickable.
      expect(screen.getByRole('button', { name: 'Download' })).toBeEnabled();
    });

    it('hides the warning exactly at the size + 2 GB boundary', () => {
      renderProgress(
        { phase: 'confirming', tier: 'balanced' },
        { confirmInfo: confirmInfo({ freeDiskGb: 10.2 }) },
      );
      expect(
        screen.queryByText('Low on disk space. The download may not fit.'),
      ).not.toBeInTheDocument();
    });

    it('skips the disk line when free space is unknown', () => {
      renderProgress(
        { phase: 'confirming', tier: 'balanced' },
        { confirmInfo: confirmInfo({ freeDiskGb: null }) },
      );
      expect(screen.getByText('8.2 GB download.')).toBeInTheDocument();
      expect(screen.queryByText(/free on this disk/)).not.toBeInTheDocument();
    });

    it('passes the RAM warning through', () => {
      renderProgress(
        { phase: 'confirming', tier: 'smartest' },
        {
          confirmInfo: confirmInfo({
            ramWarning: "Will run, but close to this Mac's memory limit",
          }),
        },
      );
      expect(
        screen.getByText("Will run, but close to this Mac's memory limit"),
      ).toBeInTheDocument();
    });

    it('renders only the buttons when confirmInfo is absent', () => {
      renderProgress({ phase: 'confirming', tier: 'fast' });
      expect(screen.queryByText(/GB download/)).not.toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: 'Download' }),
      ).toBeInTheDocument();
    });
  });

  describe('queued', () => {
    it('shows Waiting for a slot with a working Cancel, no badge, and the sliding-fill animation class', () => {
      const { onCancel, container } = renderProgress({ phase: 'queued' });
      expect(
        screen.getByText('Waiting for a download slot…'),
      ).toBeInTheDocument();
      expect(screen.queryByText(/in queue/)).not.toBeInTheDocument();
      expect(
        container.querySelector('.download-indeterminate-fill'),
      ).not.toBeNull();
      fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
      expect(onCancel).toHaveBeenCalledTimes(1);
    });

    it('omits the badge when exactly one item is queued', () => {
      renderProgress({ phase: 'queued' }, { queuePosition: 1, queuedTotal: 1 });
      expect(
        screen.getByText('Waiting for a download slot…'),
      ).toBeInTheDocument();
      expect(screen.queryByText(/in queue/)).not.toBeInTheDocument();
    });

    it('shows badges on every item, including #1, once more than one is queued', () => {
      renderProgress({ phase: 'queued' }, { queuePosition: 1, queuedTotal: 2 });
      expect(screen.getByText(/#1 in queue/)).toBeInTheDocument();
    });

    it('shows the boundary at >1 rather than >=2: two queued items both get badges', () => {
      renderProgress({ phase: 'queued' }, { queuePosition: 2, queuedTotal: 2 });
      expect(screen.getByText(/#2 in queue/)).toBeInTheDocument();
    });

    it('numbers three queued items #1, #2, #3 in FIFO order', () => {
      const first = renderProgress(
        { phase: 'queued' },
        { queuePosition: 1, queuedTotal: 3 },
      );
      expect(first.getByText(/#1 in queue/)).toBeInTheDocument();
      first.unmount();

      const second = renderProgress(
        { phase: 'queued' },
        { queuePosition: 2, queuedTotal: 3 },
      );
      expect(second.getByText(/#2 in queue/)).toBeInTheDocument();
      second.unmount();

      const third = renderProgress(
        { phase: 'queued' },
        { queuePosition: 3, queuedTotal: 3 },
      );
      expect(third.getByText(/#3 in queue/)).toBeInTheDocument();
    });
  });

  describe('downloading', () => {
    const progress: DownloadProgressInfo = {
      file: 'weights.gguf',
      bytes: 2_500_000_000,
      totalBytes: 8_200_000_000,
    };

    it('shows the unified percent, byte figures, and a working Cancel', () => {
      const { onCancel } = renderProgress(
        { phase: 'downloading' },
        { combinedBytes: 1.2e9, grandTotalBytes: 2.0e9, etaSeconds: 240 },
      );
      expect(screen.getByTestId('download-figures')).toHaveTextContent(
        '60% · 1.2 / 2.0 GB · ~4m',
      );
      // The boxy "Downloading model" headline is gone in the hairline design.
      expect(screen.queryByText('Downloading model')).not.toBeInTheDocument();

      fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
      expect(onCancel).toHaveBeenCalledTimes(1);
    });

    it('falls back to per-file figures for a single-file repo download', () => {
      renderProgress({ phase: 'downloading' }, { progress, etaSeconds: 300 });
      expect(screen.getByTestId('download-figures')).toHaveTextContent(
        '30% · 2.5 / 8.2 GB · ~5m',
      );
    });

    it('keeps one continuous bar and notes the vision companion leg', () => {
      renderProgress(
        { phase: 'downloading_mmproj' },
        { combinedBytes: 1.8e9, grandTotalBytes: 2.0e9 },
      );
      expect(screen.getByTestId('download-figures')).toHaveTextContent(
        '90% · 1.8 / 2.0 GB · finishing vision',
      );
    });

    it('shows 0% with no byte figures before the first bytes land', () => {
      renderProgress({ phase: 'downloading' });
      expect(screen.getByTestId('download-figures')).toHaveTextContent('0%');
      expect(screen.queryByText(/GB/)).not.toBeInTheDocument();
    });

    it('renders the Part N of M subline for a multi-part download', () => {
      renderProgress(
        { phase: 'downloading' },
        {
          combinedBytes: 21.5e9,
          grandTotalBytes: 64.5e9,
          partLabel: 'Part 1 of 2',
        },
      );
      expect(screen.getByTestId('download-figures')).toHaveTextContent(
        '33% · 21.5 / 64.5 GB · Part 1 of 2',
      );
    });

    it('shows the part label instead of finishing vision on a later shard', () => {
      // A split model's later shards reuse the mmproj phase; the part label must
      // win so a text model never reads "finishing vision".
      renderProgress(
        { phase: 'downloading_mmproj' },
        {
          combinedBytes: 50e9,
          grandTotalBytes: 64.5e9,
          partLabel: 'Part 2 of 2',
        },
      );
      const figures = screen.getByTestId('download-figures');
      expect(figures).toHaveTextContent('77% · 50.0 / 64.5 GB · Part 2 of 2');
      expect(figures).not.toHaveTextContent('finishing vision');
    });
  });

  it('renders an indeterminate verifying state with the sliding-fill animation class', () => {
    const { container } = renderProgress({ phase: 'verifying' });
    expect(screen.getByText('Verifying download')).toBeInTheDocument();
    expect(
      container.querySelector('[data-indeterminate="true"]'),
    ).not.toBeNull();
    expect(
      container.querySelector('.download-indeterminate-fill'),
    ).not.toBeNull();
  });

  it('renders the installing state with the sliding-fill animation class', () => {
    const { container } = renderProgress({ phase: 'installing' });
    expect(screen.getByText('Installing')).toBeInTheDocument();
    expect(
      container.querySelector('.download-indeterminate-fill'),
    ).not.toBeNull();
  });

  it('renders the warming up state with the sliding-fill animation class', () => {
    const { container } = renderProgress({ phase: 'warming_up' });
    expect(screen.getByText('Starting the engine')).toBeInTheDocument();
    expect(
      container.querySelector('.download-indeterminate-fill'),
    ).not.toBeNull();
  });

  it('renders the ready checkmark without the indeterminate animation class', () => {
    const { container } = renderProgress({ phase: 'ready' });
    expect(screen.getByText('Ready')).toBeInTheDocument();
    expect(container.querySelector('svg')).not.toBeNull();
    // A determinate fill (percent-based) never carries the sliding class.
    expect(container.querySelector('.download-indeterminate-fill')).toBeNull();
  });

  describe('failed', () => {
    it('shows the offline copy with Retry', () => {
      const { onRetry } = renderProgress({
        phase: 'failed',
        kind: 'offline',
        message: 'connection failed: dns error',
      });
      expect(screen.getByText('You appear to be offline.')).toBeInTheDocument();
      fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
      expect(onRetry).toHaveBeenCalledTimes(1);
    });

    it('extracts the status from an http failure and passes the message through', () => {
      renderProgress({
        phase: 'failed',
        kind: 'http',
        message: 'server returned HTTP 403',
      });
      expect(
        screen.getByText('Hugging Face returned an error (status 403).'),
      ).toBeInTheDocument();
      expect(screen.getByText('server returned HTTP 403')).toBeInTheDocument();
    });

    it('falls back to a status-less http headline when no status is found', () => {
      renderProgress({
        phase: 'failed',
        kind: 'http',
        message: 'server returned a strange response',
      });
      expect(
        screen.getByText('Hugging Face returned an error.'),
      ).toBeInTheDocument();
      expect(
        screen.getByText('server returned a strange response'),
      ).toBeInTheDocument();
    });

    it('shows the checksum copy', () => {
      renderProgress({
        phase: 'failed',
        kind: 'checksum',
        message: 'checksum mismatch',
      });
      expect(
        screen.getByText("Download didn't verify. Retrying re-downloads it."),
      ).toBeInTheDocument();
    });

    it('shows the disk_full copy with the backend detail as a second line', () => {
      renderProgress({
        phase: 'failed',
        kind: 'disk_full',
        message: 'write failed: no space left',
      });
      // The locked headline never changes shape with the message.
      expect(
        screen.getByText('Not enough disk space. Free up space and retry.'),
      ).toBeInTheDocument();
      // The InsufficientDisk-formatted (or raw backend) detail now renders as
      // a second line, mirroring how the http case already surfaces `message`.
      expect(
        screen.getByText('write failed: no space left'),
      ).toBeInTheDocument();
    });

    it('shows the InsufficientDisk-formatted GB detail line', () => {
      renderProgress({
        phase: 'failed',
        kind: 'disk_full',
        message: 'Needs ~4.7 GB, ~1.4 GB free on disk.',
      });
      expect(
        screen.getByText('Needs ~4.7 GB, ~1.4 GB free on disk.'),
      ).toBeInTheDocument();
    });

    it('shows the engine copy', () => {
      renderProgress({
        phase: 'failed',
        kind: 'engine',
        message: 'spawn failed',
      });
      expect(
        screen.getByText("Thuki's engine could not start."),
      ).toBeInTheDocument();
    });

    it('shows the raw message for kind other', () => {
      renderProgress({
        phase: 'failed',
        kind: 'other',
        message: 'invalid sha256 in download spec',
      });
      expect(
        screen.getByText('invalid sha256 in download spec'),
      ).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
    });

    it('renders Choose a different model when onChooseAnother is wired', () => {
      const onChooseAnother = vi.fn();
      renderProgress(
        { phase: 'failed', kind: 'disk_full', message: 'no space left' },
        { onChooseAnother },
      );
      fireEvent.click(
        screen.getByRole('button', { name: 'Choose a different model' }),
      );
      expect(onChooseAnother).toHaveBeenCalledTimes(1);
    });

    it('omits Choose a different model when onChooseAnother is absent', () => {
      renderProgress({
        phase: 'failed',
        kind: 'disk_full',
        message: 'no space left',
      });
      expect(
        screen.queryByRole('button', { name: 'Choose a different model' }),
      ).not.toBeInTheDocument();
    });
  });
});
