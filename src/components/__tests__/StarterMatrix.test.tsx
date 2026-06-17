import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { StarterMatrix } from '../StarterMatrix';
import { invoke } from '../../testUtils/mocks/tauri';
import type {
  DownloadProgressInfo,
  DownloadUiState,
} from '../../hooks/useDownloadModel';
import type { Starter, StarterOption, StarterTier } from '../../types/starter';

function makeStarter(tier: StarterTier, overrides?: Partial<Starter>): Starter {
  return {
    tier,
    display_name: `Model ${tier}`,
    repo: `org/${tier}-repo`,
    revision: 'a'.repeat(40),
    file_name: `${tier}.gguf`,
    sha256: `${tier}-sha`,
    size_bytes: 2_500_000_000,
    quant: 'Q4_K_M',
    vision: true,
    thinking: false,
    mmproj_file: null,
    mmproj_sha256: null,
    mmproj_bytes: 800_000_000,
    est_runtime_gb: 5,
    license_note: 'Gemma Terms of Use',
    origin: 'TestMaker',
    origin_repo: `maker/${tier}-repo`,
    ...overrides,
  };
}

function makeOption(
  tier: StarterTier,
  overrides?: Partial<StarterOption>,
  starterOverrides?: Partial<Starter>,
): StarterOption {
  return {
    starter: makeStarter(tier, starterOverrides),
    fit: 'fits',
    installed: false,
    partial_bytes: null,
    ...overrides,
  };
}

const THREE_TIERS: StarterOption[] = [
  makeOption('fast', { fit: 'fits' }, { vision: true }),
  makeOption('balanced', { fit: 'tight' }, { vision: true }),
  makeOption(
    'smartest',
    { fit: 'too_big' },
    { vision: false, license_note: 'MIT' },
  ),
];

function renderMatrix(
  options: StarterOption[],
  props?: Partial<Parameters<typeof StarterMatrix>[0]>,
) {
  const handlers = {
    onDownload: vi.fn(),
    onResume: vi.fn(),
    onDiscard: vi.fn(),
    onCancel: vi.fn(),
    onRetry: vi.fn(),
  };
  const utils = render(
    <StarterMatrix
      options={options}
      state={{ phase: 'idle' }}
      progress={null}
      etaSeconds={null}
      downloadingTier={null}
      {...handlers}
      {...props}
    />,
  );
  return { ...utils, ...handlers };
}

const PROGRESS: DownloadProgressInfo = {
  file: 'weights.gguf',
  bytes: 1_400_000_000,
  totalBytes: 2_500_000_000,
};

describe('StarterMatrix (picker)', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('renders the three tiers left to right with names, tiers and sizes', () => {
    const { container } = renderMatrix(THREE_TIERS);
    const cols = container.querySelectorAll('[data-tier-column]');
    expect(cols).toHaveLength(3);
    expect(cols[0].getAttribute('data-tier')).toBe('fast');
    expect(cols[1].getAttribute('data-tier')).toBe('balanced');
    expect(cols[2].getAttribute('data-tier')).toBe('smartest');
    expect(screen.getByText('Model fast')).toBeInTheDocument();
    expect(screen.getByText('Balanced ★')).toBeInTheDocument();
    expect(screen.getByText('Fast')).toBeInTheDocument();
    expect(screen.getByText('Smartest')).toBeInTheDocument();
    // (2_500_000_000 + 850_000_000) / 1e9 = 3.35 -> "3.3 GB", one per column.
    expect(screen.getAllByText('3.3 GB')).toHaveLength(3);
  });

  it('orders columns even when the backend returns them shuffled', () => {
    const { container } = renderMatrix([
      THREE_TIERS[2],
      THREE_TIERS[0],
      THREE_TIERS[1],
    ]);
    const cols = container.querySelectorAll('[data-tier-column]');
    expect([...cols].map((c) => c.getAttribute('data-tier'))).toEqual([
      'fast',
      'balanced',
      'smartest',
    ]);
  });

  it('marks only the Balanced column as recommended', () => {
    const { container } = renderMatrix(THREE_TIERS);
    const rec = (tier: string) =>
      container
        .querySelector(`[data-tier="${tier}"]`)
        ?.getAttribute('data-recommended');
    expect(rec('balanced')).toBe('true');
    expect(rec('fast')).toBe('false');
    expect(rec('smartest')).toBe('false');
  });

  it('renders Vision yes/no and the On-your-Mac fit copy', () => {
    renderMatrix(THREE_TIERS);
    expect(screen.getAllByText('Yes')).toHaveLength(2); // fast + balanced
    expect(screen.getByText('—')).toBeInTheDocument(); // smartest text-only
    expect(screen.getByText('Comfortable')).toBeInTheDocument();
    expect(screen.getByText('Tight')).toBeInTheDocument();
    expect(screen.getByText('Heavy')).toBeInTheDocument();
  });

  it('opens the Hugging Face repo from the license cell', () => {
    renderMatrix(THREE_TIERS);
    expect(screen.getAllByText('Gemma Terms of Use ↗')).toHaveLength(2);
    expect(screen.getByText('MIT ↗')).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole('button', {
        name: 'Open Model smartest on Hugging Face',
      }),
    );
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://huggingface.co/org/smartest-repo',
    });
  });

  it('opens the maker page from the origin cell', () => {
    renderMatrix(THREE_TIERS);
    // Origin defaults to 'TestMaker' for every tier; the link uses
    // origin_repo (the maker's page), distinct from the license repo.
    expect(screen.getAllByText('TestMaker ↗')).toHaveLength(3);
    fireEvent.click(
      screen.getByRole('button', {
        name: 'Verify Model smartest: open its maker TestMaker on Hugging Face',
      }),
    );
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://huggingface.co/maker/smartest-repo',
    });
  });

  it('fires onDownload from a tier with no partial', () => {
    const { onDownload } = renderMatrix([makeOption('smartest')]);
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    expect(onDownload).toHaveBeenCalledWith('smartest');
  });

  it('shows the installed line instead of a download button', () => {
    renderMatrix([makeOption('fast', { installed: true })]);
    expect(screen.getByText('Installed')).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Download' }),
    ).not.toBeInTheDocument();
  });

  it('shows the recommended download button with a hover state', () => {
    renderMatrix([makeOption('balanced')]);
    const btn = screen.getByRole('button', { name: 'Download' });
    fireEvent.mouseEnter(btn);
    fireEvent.mouseLeave(btn);
    expect(btn).toBeInTheDocument();
  });

  it('hovers a ghost (non-recommended) download button', () => {
    renderMatrix([makeOption('fast')]); // fast = ghost (not recommended)
    const dl = screen.getByRole('button', { name: 'Download' });
    fireEvent.mouseEnter(dl);
    fireEvent.mouseLeave(dl);
    expect(dl).toBeInTheDocument();
  });

  it('offers Resume (bytes at rest, "Resume" on hover) + Discard for a partial', () => {
    const { onResume, onDiscard } = renderMatrix([
      makeOption('fast', { partial_bytes: 1_200_000_000 }),
    ]);
    // 1.2 / 2.5 GB (size_bytes only, mirroring the download view).
    expect(screen.getByText('1.2 / 2.5 GB')).toBeInTheDocument();
    const resume = screen.getByRole('button', { name: 'Resume download' });
    fireEvent.mouseEnter(resume); // reveals "Resume", covers the hover branch
    fireEvent.click(resume);
    expect(onResume).toHaveBeenCalledWith('fast', 1_200_000_000, 2_500_000_000);
    fireEvent.mouseLeave(resume);

    fireEvent.click(screen.getByText('Discard partial'));
    expect(onDiscard).toHaveBeenCalledWith('fast-sha');
  });

  it('renders the active column download fill and cancels on click', () => {
    const { onCancel } = renderMatrix(THREE_TIERS, {
      state: { phase: 'downloading' },
      progress: PROGRESS,
      etaSeconds: 30,
      downloadingTier: 'fast',
    });
    expect(screen.getByText('1.4 / 2.5 GB · 30s left')).toBeInTheDocument();
    const pause = screen.getByRole('button', { name: 'Pause download' });
    fireEvent.mouseEnter(pause); // cross-fade to grey/"Pause download"
    fireEvent.click(pause);
    expect(onCancel).toHaveBeenCalledTimes(1);
    fireEvent.mouseLeave(pause);
  });

  it('dims and disables the other columns while one is downloading', () => {
    const { container, onDownload } = renderMatrix(THREE_TIERS, {
      state: { phase: 'downloading' },
      progress: PROGRESS,
      etaSeconds: null,
      downloadingTier: 'fast',
    });
    // No ETA -> just the byte counts.
    expect(screen.getByText('1.4 / 2.5 GB')).toBeInTheDocument();
    const balanced = container.querySelector('[data-tier="balanced"]');
    expect(balanced?.getAttribute('style')).toContain('opacity: 0.32');
    const downloads = screen.getAllByRole('button', { name: 'Download' });
    downloads.forEach((b) => expect(b).toBeDisabled());
    fireEvent.click(downloads[0]);
    expect(onDownload).not.toHaveBeenCalled();
  });

  it('formats minute- and hour-scale ETAs', () => {
    const { rerender } = renderMatrix([makeOption('fast')], {
      state: { phase: 'downloading' },
      progress: PROGRESS,
      etaSeconds: 90,
      downloadingTier: 'fast',
    });
    expect(screen.getByText('1.4 / 2.5 GB · 1m left')).toBeInTheDocument();
    rerender(
      <StarterMatrix
        options={[makeOption('fast')]}
        state={{ phase: 'downloading' }}
        progress={PROGRESS}
        etaSeconds={3700}
        downloadingTier="fast"
        onDownload={vi.fn()}
        onResume={vi.fn()}
        onDiscard={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />,
    );
    expect(screen.getByText('1.4 / 2.5 GB · 1h 1m left')).toBeInTheDocument();
  });

  it('shows "Starting…" before the first progress event', () => {
    renderMatrix([makeOption('fast')], {
      state: { phase: 'downloading' },
      progress: null,
      downloadingTier: 'fast',
    });
    expect(screen.getByText('Starting…')).toBeInTheDocument();
  });

  it('labels the vision-companion phase', () => {
    renderMatrix([makeOption('fast')], {
      state: { phase: 'downloading_mmproj' },
      progress: { file: 'mmproj', bytes: 400_000_000, totalBytes: 800_000_000 },
      etaSeconds: 5,
      downloadingTier: 'fast',
    });
    expect(screen.getByText('0.4 / 0.8 GB · 5s left')).toBeInTheDocument();
  });

  it('renders each post-download phase label', () => {
    const phases: Array<[DownloadUiState['phase'], string]> = [
      ['verifying', 'Verifying'],
      ['installing', 'Installing'],
      ['warming_up', 'Starting engine'],
      ['ready', 'Ready'],
    ];
    for (const [phase, label] of phases) {
      const { unmount } = renderMatrix([makeOption('fast')], {
        state: { phase } as DownloadUiState,
        downloadingTier: 'fast',
      });
      expect(screen.getByText(label)).toBeInTheDocument();
      unmount();
    }
  });

  it('shows a failed headline + Retry, and leaves other columns usable', () => {
    const { onRetry, onDownload } = renderMatrix(THREE_TIERS, {
      state: { phase: 'failed', kind: 'disk_full', message: 'ENOSPC' },
      downloadingTier: 'fast',
    });
    expect(screen.getByText('Not enough disk')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
    // A failure does not lock the other tiers.
    const downloads = screen.getAllByRole('button', { name: 'Download' });
    expect(downloads[0]).not.toBeDisabled();
    fireEvent.click(downloads[0]);
    expect(onDownload).toHaveBeenCalled();
  });

  it('renders every failure kind headline', () => {
    const kinds: Array<[string, string]> = [
      ['offline', "You're offline"],
      ['http', 'Download error'],
      ['checksum', 'Verify failed'],
      ['engine', 'Engine could not start'],
      ['other', 'Download failed'],
    ];
    for (const [kind, label] of kinds) {
      const { unmount } = renderMatrix([makeOption('fast')], {
        state: { phase: 'failed', kind, message: 'x' } as DownloadUiState,
        downloadingTier: 'fast',
      });
      expect(screen.getByText(label)).toBeInTheDocument();
      unmount();
    }
  });

  it('disables Resume and hides Discard while another tier downloads', () => {
    const { onResume } = renderMatrix(
      [
        makeOption('fast'),
        makeOption('balanced', { partial_bytes: 1_000_000_000 }),
      ],
      {
        state: { phase: 'downloading' },
        progress: PROGRESS,
        downloadingTier: 'fast',
      },
    );
    const resume = screen.getByRole('button', { name: 'Resume download' });
    expect(resume).toBeDisabled();
    fireEvent.mouseEnter(resume); // hover while disabled stays at rest
    fireEvent.click(resume);
    expect(onResume).not.toHaveBeenCalled();
    expect(screen.queryByText('Discard partial')).not.toBeInTheDocument();
  });

  it('shows the Ollama escape hatch only when detected and wired', () => {
    const onUseOllama = vi.fn();
    const { rerender } = renderMatrix(THREE_TIERS, {
      ollamaDetected: true,
      onUseOllama,
    });
    fireEvent.click(screen.getByRole('button', { name: 'Use it instead' }));
    expect(onUseOllama).toHaveBeenCalledTimes(1);

    const base = {
      options: THREE_TIERS,
      state: { phase: 'idle' } as DownloadUiState,
      progress: null,
      etaSeconds: null,
      downloadingTier: null,
      onDownload: vi.fn(),
      onResume: vi.fn(),
      onDiscard: vi.fn(),
      onCancel: vi.fn(),
      onRetry: vi.fn(),
    };
    rerender(
      <StarterMatrix
        {...base}
        ollamaDetected={false}
        onUseOllama={onUseOllama}
      />,
    );
    expect(screen.queryByText('Use it instead')).not.toBeInTheDocument();
    rerender(<StarterMatrix {...base} ollamaDetected={true} />);
    expect(screen.queryByText('Use it instead')).not.toBeInTheDocument();
  });
});
