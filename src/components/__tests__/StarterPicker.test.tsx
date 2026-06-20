import {
  render,
  screen,
  fireEvent,
  renderHook,
  act,
} from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { StarterPicker, useStarterOptions } from '../StarterPicker';
import { invoke } from '../../testUtils/mocks/tauri';
import type { Starter, StarterOption, StarterTier } from '../../types/starter';

function makeStarter(tier: StarterTier, overrides?: Partial<Starter>): Starter {
  return {
    tier,
    display_name: `Model ${tier}`,
    repo: `org/${tier}-repo`,
    revision: 'a'.repeat(40),
    file_name: `${tier}.gguf`,
    sha256: 'b'.repeat(64),
    size_bytes: 7_300_000_000,
    quant: 'Q4_K_M',
    vision: false,
    thinking: false,
    mmproj_file: null,
    mmproj_sha256: null,
    mmproj_bytes: 0,
    est_runtime_gb: 10,
    license_note: 'MIT',
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
  makeOption('fast', { fit: 'fits' }),
  makeOption('balanced', { fit: 'tight' }),
  makeOption('smartest', { fit: 'too_big' }),
];

function renderPicker(
  options: StarterOption[],
  props?: Partial<Parameters<typeof StarterPicker>[0]>,
) {
  const handlers = {
    onSelect: vi.fn(),
    onDownload: vi.fn(),
    onResume: vi.fn(),
    onDiscard: vi.fn(),
  };
  const utils = render(
    <StarterPicker
      options={options}
      selected="balanced"
      {...handlers}
      {...props}
    />,
  );
  return { ...utils, ...handlers };
}

describe('StarterPicker', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('renders all three tiers with names and tier labels', () => {
    renderPicker(THREE_TIERS);
    expect(screen.getByText('Model fast')).toBeInTheDocument();
    expect(screen.getByText('Model balanced')).toBeInTheDocument();
    expect(screen.getByText('Model smartest')).toBeInTheDocument();
    expect(screen.getByText('Fast')).toBeInTheDocument();
    expect(screen.getByText('Balanced')).toBeInTheDocument();
    expect(screen.getByText('Smartest')).toBeInTheDocument();
  });

  it('renders the combined weights + mmproj size in GB with one decimal', () => {
    renderPicker([
      makeOption(
        'fast',
        {},
        { size_bytes: 2_489_757_856, mmproj_bytes: 851_251_104 },
      ),
    ]);
    // (2_489_757_856 + 851_251_104) / 1e9 = 3.341 -> "3.3 GB"
    expect(screen.getByText('3.3 GB')).toBeInTheDocument();
  });

  it('renders the exact RAM-fit badge copy for every fit', () => {
    renderPicker(THREE_TIERS);
    expect(
      screen.getByText('Runs comfortably on this Mac'),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Will run, but close to this Mac's memory limit"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Larger than this Mac's memory can comfortably hold. Expect heavy slowdown.",
      ),
    ).toBeInTheDocument();
  });

  it('opens the Hugging Face page via open_url from the license line', () => {
    const { onSelect } = renderPicker([makeOption('fast')]);
    expect(screen.getByText('MIT')).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole('button', { name: 'Open Model fast on Hugging Face' }),
    );
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://huggingface.co/org/fast-repo',
    });
    // stopPropagation keeps the link from also selecting the card.
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('renders the per-tier license notes: two Gemma Terms and one MIT', () => {
    // Mirrors the backend registry: both Gemma tiers carry the Gemma Terms
    // of Use; the Phi-4 tier is MIT. Each card links out via open_url.
    renderPicker([
      makeOption('fast', {}, { license_note: 'Gemma Terms of Use' }),
      makeOption('balanced', {}, { license_note: 'Gemma Terms of Use' }),
      makeOption('smartest', {}, { license_note: 'MIT' }),
    ]);
    expect(screen.getAllByText('Gemma Terms of Use')).toHaveLength(2);
    expect(screen.getByText('MIT')).toBeInTheDocument();
    for (const tier of ['fast', 'balanced', 'smartest']) {
      fireEvent.click(
        screen.getByRole('button', {
          name: `Open Model ${tier} on Hugging Face`,
        }),
      );
      expect(invoke).toHaveBeenCalledWith('open_url', {
        url: `https://huggingface.co/org/${tier}-repo`,
      });
    }
  });

  it('marks the selected tier card', () => {
    const { container } = renderPicker(THREE_TIERS);
    const cards = container.querySelectorAll('[data-starter-card]');
    expect(cards).toHaveLength(3);
    expect(
      container
        .querySelector('[data-tier="balanced"]')
        ?.getAttribute('data-selected'),
    ).toBe('true');
    expect(
      container
        .querySelector('[data-tier="fast"]')
        ?.getAttribute('data-selected'),
    ).toBe('false');
  });

  it('selects a tier when its card is clicked', () => {
    const { container, onSelect } = renderPicker(THREE_TIERS);
    fireEvent.click(container.querySelector('[data-tier="fast"]')!);
    expect(onSelect).toHaveBeenCalledWith('fast');
  });

  it('fires onDownload for a not-installed tier without a partial', () => {
    const { onDownload, onSelect } = renderPicker([makeOption('smartest')]);
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    expect(onDownload).toHaveBeenCalledWith('smartest');
    // stopPropagation: the action button must not also select the card.
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('shows the installed checkmark instead of a download button', () => {
    renderPicker([makeOption('fast', { installed: true })]);
    expect(screen.getByText('Installed')).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Download' }),
    ).not.toBeInTheDocument();
  });

  it('offers resume and discard when a partial exists', () => {
    const { onResume, onDiscard } = renderPicker([
      makeOption(
        'balanced',
        { partial_bytes: 1_200_000_000 },
        { size_bytes: 7_300_000_000, mmproj_bytes: 854_200_224 },
      ),
    ]);
    // 1.2 of (7_300_000_000 + 854_200_224)/1e9 = 8.154 -> 8.2 GB
    const resume = screen.getByRole('button', {
      name: 'Resume download (1.2 of 8.2 GB)',
    });
    fireEvent.click(resume);
    expect(onResume).toHaveBeenCalledWith('balanced');

    fireEvent.click(screen.getByRole('button', { name: 'Discard' }));
    expect(onDiscard).toHaveBeenCalledWith('b'.repeat(64));
  });

  it('shows the Ollama escape hatch only when detected and wired', () => {
    const onUseOllama = vi.fn();
    const { rerender } = renderPicker(THREE_TIERS, {
      ollamaDetected: true,
      onUseOllama,
    });
    fireEvent.click(
      screen.getByRole('button', { name: 'Use my existing Ollama instead' }),
    );
    expect(onUseOllama).toHaveBeenCalledTimes(1);

    rerender(
      <StarterPicker
        options={THREE_TIERS}
        selected="balanced"
        onSelect={vi.fn()}
        onDownload={vi.fn()}
        onResume={vi.fn()}
        onDiscard={vi.fn()}
        ollamaDetected={false}
        onUseOllama={onUseOllama}
      />,
    );
    expect(
      screen.queryByText('Use my existing Ollama instead'),
    ).not.toBeInTheDocument();

    rerender(
      <StarterPicker
        options={THREE_TIERS}
        selected="balanced"
        onSelect={vi.fn()}
        onDownload={vi.fn()}
        onResume={vi.fn()}
        onDiscard={vi.fn()}
        ollamaDetected={true}
      />,
    );
    expect(
      screen.queryByText('Use my existing Ollama instead'),
    ).not.toBeInTheDocument();
  });
});

describe('useStarterOptions', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('starts null and loads the options on mount', async () => {
    invoke.mockResolvedValueOnce(THREE_TIERS);
    const { result } = renderHook(() => useStarterOptions());
    expect(result.current.options).toBeNull();
    await act(async () => {});
    expect(result.current.options).toEqual(THREE_TIERS);
    expect(invoke).toHaveBeenCalledWith('get_starter_options');
  });

  it('degrades to an empty list when the fetch rejects', async () => {
    invoke.mockRejectedValueOnce('backend down');
    const { result } = renderHook(() => useStarterOptions());
    await act(async () => {});
    expect(result.current.options).toEqual([]);
  });

  it('coerces a malformed non-array payload to an empty list', async () => {
    invoke.mockResolvedValueOnce({ not: 'an array' });
    const { result } = renderHook(() => useStarterOptions());
    await act(async () => {});
    expect(result.current.options).toEqual([]);
  });

  it('re-fetches on refresh', async () => {
    invoke.mockResolvedValueOnce([]);
    const { result } = renderHook(() => useStarterOptions());
    await act(async () => {});
    expect(result.current.options).toEqual([]);

    invoke.mockResolvedValueOnce(THREE_TIERS);
    await act(() => result.current.refresh());
    expect(result.current.options).toEqual(THREE_TIERS);
  });
});
