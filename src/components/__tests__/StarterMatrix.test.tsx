import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { StarterMatrix } from '../StarterMatrix';
import { ALWAYS_REASONS_LABEL } from '../ModelPickerPanel';
import { invoke } from '../../testUtils/mocks/tauri';
import type { DownloadUiState } from '../../hooks/useDownloadModel';
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
      combinedBytes={null}
      speedBytesPerSec={null}
      downloadingTier={null}
      {...handlers}
      {...props}
    />,
  );
  return { ...utils, ...handlers };
}

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
    // Dashes: 1 Vision cell (smartest text-only) + 3 Reasoning cells (every
    // fixture has thinking:false, so all three read as "no reasoning").
    expect(screen.getAllByText('—')).toHaveLength(4);
    expect(screen.getByText('Comfortable')).toBeInTheDocument();
    expect(screen.getByText('Tight')).toBeInTheDocument();
    expect(screen.getByText('Heavy')).toBeInTheDocument();
  });

  it('renders the reasoning class per tier (always badge, on-demand, none)', () => {
    renderMatrix([
      makeOption('fast', undefined, {
        thinking: true,
        reasoning_always: false,
      }),
      makeOption('balanced', undefined, {
        thinking: false,
        reasoning_always: false,
      }),
      makeOption('smartest', undefined, {
        thinking: true,
        reasoning_always: true,
      }),
    ]);
    expect(screen.getByText('Reasoning')).toBeInTheDocument();
    // smartest: always-reasoning pill.
    expect(
      screen.getByTestId('starter-always-reasons-badge'),
    ).toHaveTextContent(ALWAYS_REASONS_LABEL);
    // fast: optional reasoning reads "On demand".
    expect(screen.getByText('On demand')).toBeInTheDocument();
    // balanced: no reasoning -> a dash (the none branch).
    expect(screen.getAllByText('—').length).toBeGreaterThanOrEqual(1);
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

  it('renders one combined bar with bytes and ETA (no speed), and cancels on click', () => {
    const { onCancel } = renderMatrix(THREE_TIERS, {
      state: { phase: 'downloading' },
      combinedBytes: 1_400_000_000,
      speedBytesPerSec: 8_000_000,
      downloadingTier: 'fast',
    });
    // 1.4 of the 3.3 GB card total; speed drives the ETA but is not shown:
    // (3.3e9 - 1.4e9) / 8e6 = 238s -> "3m".
    expect(screen.getByText('1.4 / 3.3 GB · 3m left')).toBeInTheDocument();
    expect(screen.queryByText(/MB\/s/)).not.toBeInTheDocument();
    const pause = screen.getByRole('button', { name: 'Pause download' });
    fireEvent.mouseEnter(pause); // cross-fade to grey/"Pause download"
    fireEvent.click(pause);
    expect(onCancel).toHaveBeenCalledTimes(1);
    fireEvent.mouseLeave(pause);
  });

  it('dims and disables the other columns while one is downloading', () => {
    const { container, onDownload } = renderMatrix(THREE_TIERS, {
      state: { phase: 'downloading' },
      combinedBytes: 1_400_000_000,
      speedBytesPerSec: null,
      downloadingTier: 'fast',
    });
    // No measurable rate yet -> just the byte counts, no speed or ETA.
    expect(screen.getByText('1.4 / 3.3 GB')).toBeInTheDocument();
    const balanced = container.querySelector('[data-tier="balanced"]');
    expect(balanced?.getAttribute('style')).toContain('opacity: 0.32');
    const downloads = screen.getAllByRole('button', { name: 'Download' });
    downloads.forEach((b) => expect(b).toBeDisabled());
    fireEvent.click(downloads[0]);
    expect(onDownload).not.toHaveBeenCalled();
  });

  it('formats an hour-scale ETA from the combined remaining bytes', () => {
    renderMatrix([makeOption('fast')], {
      state: { phase: 'downloading' },
      combinedBytes: 0,
      speedBytesPerSec: 200_000,
      downloadingTier: 'fast',
    });
    // 3.3e9 / 2e5 = 16500s -> 4h 35m (speed feeds the ETA, but is not shown).
    expect(screen.getByText('0.0 / 3.3 GB · 4h 35m left')).toBeInTheDocument();
  });

  it('shows "Starting…" before the first combined byte arrives', () => {
    renderMatrix([makeOption('fast')], {
      state: { phase: 'downloading' },
      combinedBytes: null,
      speedBytesPerSec: null,
      downloadingTier: 'fast',
    });
    expect(screen.getByText('Starting…')).toBeInTheDocument();
  });

  it('renders the mmproj phase as the same combined bar, with no second-file label', () => {
    renderMatrix([makeOption('fast')], {
      state: { phase: 'downloading_mmproj' },
      combinedBytes: 3_000_000_000,
      speedBytesPerSec: 8_000_000,
      downloadingTier: 'fast',
    });
    // One bar against the 3.3 GB total; (3.3e9 - 3.0e9) / 8e6 = 38s.
    expect(screen.getByText('3.0 / 3.3 GB · 38s left')).toBeInTheDocument();
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
        combinedBytes: 1_400_000_000,
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

  it('shows the Continue line while a download is in flight and fires onContinue', () => {
    const onContinue = vi.fn();
    renderMatrix([makeOption('fast')], {
      state: { phase: 'downloading' },
      downloadingTier: 'fast',
      onContinue,
    });
    expect(
      screen.getByText('Downloading in the background.'),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Continue setup →' }));
    expect(onContinue).toHaveBeenCalledTimes(1);
  });

  it('shows the Continue line through every in-flight phase', () => {
    const phases: DownloadUiState['phase'][] = [
      'downloading',
      'downloading_mmproj',
      'verifying',
      'installing',
      'warming_up',
    ];
    for (const phase of phases) {
      const { unmount } = renderMatrix([makeOption('fast')], {
        state: { phase } as DownloadUiState,
        downloadingTier: 'fast',
        onContinue: vi.fn(),
      });
      expect(
        screen.getByText('Downloading in the background.'),
      ).toBeInTheDocument();
      unmount();
    }
  });

  it('hides the Continue line outside the in-flight phases', () => {
    const states: DownloadUiState[] = [
      { phase: 'idle' },
      { phase: 'confirming', tier: 'fast' },
      { phase: 'resume_pending' },
      { phase: 'ready' },
      { phase: 'failed', kind: 'other', message: 'x' },
    ];
    for (const state of states) {
      const { unmount } = renderMatrix([makeOption('fast')], {
        state,
        downloadingTier: 'fast',
        onContinue: vi.fn(),
      });
      expect(
        screen.queryByText('Downloading in the background.'),
      ).not.toBeInTheDocument();
      unmount();
    }
  });

  it('hides the Continue line when onContinue is not wired', () => {
    renderMatrix([makeOption('fast')], {
      state: { phase: 'downloading' },
      downloadingTier: 'fast',
    });
    expect(
      screen.queryByText('Downloading in the background.'),
    ).not.toBeInTheDocument();
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
      combinedBytes: null,
      speedBytesPerSec: null,
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
