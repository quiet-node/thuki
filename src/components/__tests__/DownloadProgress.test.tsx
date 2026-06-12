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

  describe('downloading', () => {
    const progress: DownloadProgressInfo = {
      file: 'weights.gguf',
      bytes: 2_500_000_000,
      totalBytes: 8_200_000_000,
    };

    it('shows percent, byte counts, ETA, and a working Cancel', () => {
      const { onCancel } = renderProgress(
        { phase: 'downloading' },
        { progress, etaSeconds: 300 },
      );
      expect(screen.getByText('Downloading model')).toBeInTheDocument();
      expect(screen.getByText('30%')).toBeInTheDocument();
      expect(screen.getByText('2.5 GB of 8.2 GB')).toBeInTheDocument();
      expect(screen.getByText('About 5m left')).toBeInTheDocument();

      fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
      expect(onCancel).toHaveBeenCalledTimes(1);
    });

    it('labels the mmproj phase as the vision companion', () => {
      renderProgress(
        { phase: 'downloading_mmproj' },
        { progress, etaSeconds: null },
      );
      expect(
        screen.getByText('Downloading vision companion'),
      ).toBeInTheDocument();
      expect(screen.queryByText(/left$/)).not.toBeInTheDocument();
    });

    it('falls back to 0% before the first Started event lands', () => {
      renderProgress({ phase: 'downloading' });
      expect(screen.getByText('0%')).toBeInTheDocument();
      expect(screen.queryByText(/GB of/)).not.toBeInTheDocument();
    });

    it('guards the percent math against a zero total', () => {
      renderProgress(
        { phase: 'downloading' },
        { progress: { file: 'w.gguf', bytes: 10, totalBytes: 0 } },
      );
      expect(screen.getByText('0%')).toBeInTheDocument();
    });

    it('formats sub-minute and multi-hour ETAs', () => {
      renderProgress({ phase: 'downloading' }, { progress, etaSeconds: 45 });
      expect(screen.getByText('About 45s left')).toBeInTheDocument();

      renderProgress({ phase: 'downloading' }, { progress, etaSeconds: 7300 });
      expect(screen.getByText('About 2h 1m left')).toBeInTheDocument();
    });
  });

  it('renders an indeterminate verifying state', () => {
    const { container } = renderProgress({ phase: 'verifying' });
    expect(screen.getByText('Verifying download')).toBeInTheDocument();
    expect(
      container.querySelector('[data-indeterminate="true"]'),
    ).not.toBeNull();
  });

  it('renders the installing state', () => {
    renderProgress({ phase: 'installing' });
    expect(screen.getByText('Installing')).toBeInTheDocument();
  });

  it('renders the warming up state', () => {
    renderProgress({ phase: 'warming_up' });
    expect(screen.getByText('Starting the engine')).toBeInTheDocument();
  });

  it('renders the ready checkmark', () => {
    const { container } = renderProgress({ phase: 'ready' });
    expect(screen.getByText('Ready')).toBeInTheDocument();
    expect(container.querySelector('svg')).not.toBeNull();
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

    it('shows the disk_full copy', () => {
      renderProgress({
        phase: 'failed',
        kind: 'disk_full',
        message: 'write failed: no space left',
      });
      expect(
        screen.getByText('Not enough disk space. Free up space and retry.'),
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
