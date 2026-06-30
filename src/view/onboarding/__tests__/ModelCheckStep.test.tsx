import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
  within,
  cleanup,
} from '@testing-library/react';
import { describe, it, expect, beforeEach, beforeAll, vi } from 'vitest';
import { ModelCheckStep, buildConfirmInfo } from '../ModelCheckStep';
import {
  ConfigProviderForTest,
  DEFAULT_CONFIG,
  type AppConfig,
} from '../../../contexts/ConfigContext';
import { DownloadProvider } from '../../../contexts/DownloadContext';
import {
  invoke,
  enableChannelCaptureWithResponses,
  getLastChannel,
  resetChannelCapture,
} from '../../../testUtils/mocks/tauri';
import type {
  Starter,
  StarterOption,
  StarterTier,
} from '../../../types/starter';

const READY_RESPONSE = {
  state: 'ready',
  active_slug: 'gemma4:e4b',
  installed: ['gemma4:e4b'],
};

const writeText = vi.fn().mockResolvedValue(undefined);

beforeAll(() => {
  if (!('clipboard' in navigator)) {
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      writable: true,
      value: { writeText },
    });
  } else {
    Object.assign(navigator.clipboard, { writeText });
  }
});

describe('ModelCheckStep', () => {
  beforeEach(() => {
    invoke.mockClear();
    writeText.mockReset();
    writeText.mockResolvedValue(undefined);
  });

  it('shows Step 1 active and Step 2 waiting on Ollama unreachable', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(screen.getByText('Set up your local AI')).toBeInTheDocument();
    expect(
      screen.getByText('Runs Ollama locally. Your chats stay on this machine.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Install & start Ollama')).toBeInTheDocument();
    expect(
      screen.queryByText('STEP 1 · ACTION NEEDED'),
    ).not.toBeInTheDocument();
    expect(screen.queryByText('STEP 2 · WAITING')).not.toBeInTheDocument();
    expect(screen.getByText('Pull a starter model')).toBeInTheDocument();
    expect(
      screen.getByText('curl -fsSL https://ollama.com/install.sh | sh'),
    ).toBeInTheDocument();
  });

  it('reveals the panel on mount (the Ollama gate has no fit hook)', async () => {
    // The announcement -> model_check transition covers the panel (alpha 0);
    // this gate fades it back in on mount. Run the two animation frames
    // synchronously to assert the reveal fires with the expected arguments.
    const raf = vi
      .spyOn(globalThis, 'requestAnimationFrame')
      .mockImplementation((cb: FrameRequestCallback) => {
        cb(0);
        return 0;
      });
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(invoke).toHaveBeenCalledWith('set_overlay_alpha', {
      alpha: 1,
      durationMs: 150,
    });
    raf.mockRestore();
  });

  it('shows Step 1 done and Step 2 active on no_models_installed', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(screen.getByText('Ollama is running')).toBeInTheDocument();
    expect(
      screen.getByText('Listening on 127.0.0.1:11434'),
    ).toBeInTheDocument();
    expect(screen.getByText('live')).toBeInTheDocument();
    expect(screen.queryByText('Connected')).not.toBeInTheDocument();
    expect(screen.queryByText('STEP 1 · DONE')).not.toBeInTheDocument();
    expect(
      screen.queryByText('STEP 2 · ACTION NEEDED'),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText("Almost there. Let's pick a model for Thuki."),
    ).toBeInTheDocument();
    expect(
      screen.getByText('You can swap or add more later.'),
    ).toBeInTheDocument();
    expect(screen.getByText('gemma4:e4b')).toBeInTheDocument();
    expect(screen.getByText('llama3.2-vision:11b')).toBeInTheDocument();
    expect(screen.getByText('phi4:14b')).toBeInTheDocument();
    expect(screen.queryByText('RECOMMENDED')).not.toBeInTheDocument();
  });

  it('renders the configured Ollama URL host:port in the listening line', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
    });

    render(
      <ConfigProviderForTest
        value={{
          ...DEFAULT_CONFIG,
          inference: {
            ...DEFAULT_CONFIG.inference,
            ollamaUrl: 'http://10.0.0.5:9000',
          },
        }}
      >
        <ModelCheckStep />
      </ConfigProviderForTest>,
    );
    await act(async () => {});

    expect(screen.getByText('Listening on 10.0.0.5:9000')).toBeInTheDocument();
  });

  it('falls back to the raw Ollama URL string when it is not parseable', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
    });

    render(
      <ConfigProviderForTest
        value={{
          ...DEFAULT_CONFIG,
          inference: { ...DEFAULT_CONFIG.inference, ollamaUrl: 'not-a-url' },
        }}
      >
        <ModelCheckStep />
      </ConfigProviderForTest>,
    );
    await act(async () => {});

    expect(screen.getByText('Listening on not-a-url')).toBeInTheDocument();
  });

  it('fires advance_past_model_check when Ready', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: READY_RESPONSE,
      advance_past_model_check: undefined,
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('advance_past_model_check');
    });
  });

  it('treats IPC failure as Ollama unreachable so the user sees a recovery path', async () => {
    invoke.mockRejectedValueOnce(new Error('ipc broken'));

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(screen.getByText('Install & start Ollama')).toBeInTheDocument();
  });

  it('Re-check button re-runs the probe and updates state', async () => {
    let calls = 0;
    invoke.mockImplementation(async (name: string) => {
      if (name === 'check_model_setup') {
        calls += 1;
        return calls === 1
          ? { state: 'ollama_unreachable' }
          : { state: 'no_models_installed' };
      }
      return undefined;
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(screen.getByText('Install & start Ollama')).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Verify setup'));
    });

    expect(screen.getByText('Ollama is running')).toBeInTheDocument();
    expect(screen.getByText('live')).toBeInTheDocument();
  });

  it('Re-check button is no-op while a probe is in flight', async () => {
    let probeCalls = 0;
    let resolveSecond: (value: unknown) => void = () => {};
    invoke.mockImplementation(async (name: string) => {
      if (name === 'check_model_setup') {
        probeCalls += 1;
        if (probeCalls === 1) return { state: 'ollama_unreachable' };
        return new Promise((resolve) => {
          resolveSecond = resolve;
        });
      }
      return undefined;
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Verify setup'));
    });
    expect(probeCalls).toBe(2);

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Verify setup'));
    });
    expect(probeCalls).toBe(2);

    await act(async () => {
      resolveSecond({ state: 'no_models_installed' });
    });
  });

  it('copies the selected install command (Install Ollama default)', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Copy install ollama command'));
    });
    expect(writeText).toHaveBeenCalledWith(
      'curl -fsSL https://ollama.com/install.sh | sh',
    );
  });

  it('switching tabs swaps the displayed install command', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(
      screen.getByText('curl -fsSL https://ollama.com/install.sh | sh'),
    ).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Already Installed?' }),
      );
    });
    expect(screen.getByText('open -a Ollama')).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install Ollama' }));
    });
    expect(
      screen.getByText('curl -fsSL https://ollama.com/install.sh | sh'),
    ).toBeInTheDocument();
  });

  it('copies the open command after switching to the Already Installed? tab', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Already Installed?' }),
      );
    });
    await act(async () => {
      fireEvent.click(screen.getByLabelText('Copy already installed? command'));
    });
    expect(writeText).toHaveBeenCalledWith('open -a Ollama');
  });

  it('lights up the active tab with the brand orange', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    const installTab = screen.getByRole('button', { name: 'Install Ollama' });
    expect(installTab.style.color).toContain('255, 141, 92');

    const alreadyTab = screen.getByRole('button', {
      name: 'Already Installed?',
    });
    expect(alreadyTab.style.color).not.toContain('255, 141, 92');
  });

  it('hovering an inactive tab brightens the label', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    const alreadyTab = screen.getByRole('button', {
      name: 'Already Installed?',
    });
    const before = alreadyTab.style.color;
    fireEvent.mouseEnter(alreadyTab);
    expect(alreadyTab.style.color).not.toBe(before);
    fireEvent.mouseLeave(alreadyTab);
    expect(alreadyTab.style.color).toBe(before);
  });

  it('copies the pull command for a starter model', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByLabelText('Copy install command for phi4:14b'),
      );
    });
    expect(writeText).toHaveBeenCalledWith('ollama pull phi4:14b');
  });

  it('renders each starter model with its description and size', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(screen.getByText('Google · vision · 9.6 GB')).toBeInTheDocument();
    expect(screen.getByText('Meta · vision · 7.8 GB')).toBeInTheDocument();
    expect(screen.getByText('Microsoft · text · 9.1 GB')).toBeInTheDocument();
  });

  it('clicking a model slug opens its Ollama library page', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
      open_url: undefined,
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Open gemma4:e4b on Ollama'));
    });

    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://ollama.com/library/gemma4',
    });
  });

  it('renders the slug as an underlined link with the URL in a hover title', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    const link = screen.getByLabelText('Open phi4:14b on Ollama');
    expect(link).toHaveAttribute('title', 'https://ollama.com/library/phi4');
    expect(link.getAttribute('style')).toContain('underline');
  });

  it('swallows clipboard write errors silently', async () => {
    writeText.mockReset();
    writeText.mockRejectedValue(new Error('denied'));
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await expect(
      act(async () => {
        fireEvent.click(screen.getByLabelText('Copy install ollama command'));
      }),
    ).resolves.not.toThrow();
  });

  it('renders the privacy footer', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(
      screen.getByText(
        'Private by default · All inference runs on your machine',
      ),
    ).toBeInTheDocument();
  });

  it('renders the Step 1 sub-line below the code box with the Ollama docs link', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(
      screen.getByText('Paste this in Terminal or visit'),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText('Open Ollama documentation'),
    ).toBeInTheDocument();
  });

  it('opens the Ollama docs URL when its sub-line link is clicked', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
      open_url: undefined,
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Open Ollama documentation'));
    });

    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://ollama.com/download',
    });
  });

  it('opens the Ollama library URL when the Browse link is clicked', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
      open_url: undefined,
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Browse all models on Ollama'));
    });

    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://ollama.com/search',
    });
  });

  it('renders the Step 2 helper block under the model list', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(
      screen.getByText('Paste the command in Terminal'),
    ).toBeInTheDocument();
    expect(screen.getByText('or')).toBeInTheDocument();
    expect(
      screen.getByText('Browse all models on ollama.com ↗'),
    ).toBeInTheDocument();
  });

  it('renders the sub-line doc link as an underlined link with the URL in a hover title', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    const link = screen.getByLabelText('Open Ollama documentation');
    expect(link).toHaveAttribute('title', 'https://ollama.com/download');
    expect(link.getAttribute('style')).toContain('underline');
  });

  it('icon-only install copy button shows only the green check on success (no Copied text)', async () => {
    vi.useFakeTimers();
    try {
      enableChannelCaptureWithResponses({
        check_model_setup: { state: 'ollama_unreachable' },
      });

      render(<ModelCheckStep />);
      await act(async () => {});

      await act(async () => {
        fireEvent.click(screen.getByLabelText('Copy install ollama command'));
      });

      expect(screen.queryByText('Copied')).not.toBeInTheDocument();
      const button = screen.getByLabelText('Copy install ollama command');
      expect(button.style.borderColor).toContain('34, 197, 94');

      await act(async () => {
        vi.advanceTimersByTime(1500);
      });

      expect(button.style.borderColor).not.toContain('34, 197, 94');
    } finally {
      vi.useRealTimers();
    }
  });

  it('model-row copy button swaps into a Copied confirmation after a successful copy', async () => {
    vi.useFakeTimers();
    try {
      enableChannelCaptureWithResponses({
        check_model_setup: { state: 'no_models_installed' },
      });

      render(<ModelCheckStep />);
      await act(async () => {});

      await act(async () => {
        fireEvent.click(
          screen.getByLabelText('Copy install command for gemma4:e4b'),
        );
      });

      expect(screen.getByText('Copied')).toBeInTheDocument();

      await act(async () => {
        vi.advanceTimersByTime(1500);
      });

      expect(screen.queryByText('Copied')).not.toBeInTheDocument();
      expect(screen.getAllByText('Copy').length).toBeGreaterThan(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('clears the previous Copied timer when the model-row copy button is clicked twice quickly', async () => {
    vi.useFakeTimers();
    try {
      enableChannelCaptureWithResponses({
        check_model_setup: { state: 'no_models_installed' },
      });

      render(<ModelCheckStep />);
      await act(async () => {});

      const button = screen.getByLabelText('Copy install command for phi4:14b');

      await act(async () => {
        fireEvent.click(button);
      });
      expect(screen.getByText('Copied')).toBeInTheDocument();

      await act(async () => {
        vi.advanceTimersByTime(800);
      });
      await act(async () => {
        fireEvent.click(button);
      });
      expect(screen.getByText('Copied')).toBeInTheDocument();

      await act(async () => {
        vi.advanceTimersByTime(800);
      });
      expect(screen.getByText('Copied')).toBeInTheDocument();

      await act(async () => {
        vi.advanceTimersByTime(800);
      });
      expect(screen.queryByText('Copied')).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it('lights up the copy button border on pointer hover', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    const button = screen.getByLabelText('Copy install ollama command');
    fireEvent.mouseEnter(button);
    expect(button.style.borderColor).toContain('255, 141, 92');
    fireEvent.mouseLeave(button);
    expect(button.style.borderColor).toContain('255, 255, 255');
  });

  it('drops the probe success when the component unmounts mid-flight', async () => {
    let resolveProbe: (value: unknown) => void = () => {};
    invoke.mockImplementation(async (name: string) => {
      if (name === 'check_model_setup') {
        return new Promise((resolve) => {
          resolveProbe = resolve;
        });
      }
      return undefined;
    });

    const { unmount } = render(<ModelCheckStep />);
    unmount();

    await act(async () => {
      resolveProbe({ state: 'no_models_installed' });
    });

    expect(invoke).not.toHaveBeenCalledWith('advance_past_model_check');
  });

  it('drops the probe failure when the component unmounts mid-flight', async () => {
    let rejectProbe: (reason: unknown) => void = () => {};
    invoke.mockImplementation(async (name: string) => {
      if (name === 'check_model_setup') {
        return new Promise((_resolve, reject) => {
          rejectProbe = reject;
        });
      }
      return undefined;
    });

    const { unmount } = render(<ModelCheckStep />);
    unmount();

    await act(async () => {
      rejectProbe(new Error('late failure'));
    });
  });

  it('skips re-render when the recheck probe finishes after unmount', async () => {
    let calls = 0;
    let resolveSecond: (value: unknown) => void = () => {};
    invoke.mockImplementation(async (name: string) => {
      if (name === 'check_model_setup') {
        calls += 1;
        if (calls === 1) return { state: 'ollama_unreachable' };
        return new Promise((resolve) => {
          resolveSecond = resolve;
        });
      }
      return undefined;
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Verify setup'));
    });

    cleanup();

    await act(async () => {
      resolveSecond({ state: 'no_models_installed' });
    });
  });

  it('does not show the Copied confirmation when the clipboard write fails', async () => {
    writeText.mockReset();
    writeText.mockRejectedValue(new Error('denied'));
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Copy install ollama command'));
    });

    expect(screen.queryByText('Copied')).not.toBeInTheDocument();
  });
});

// ─── Built-in engine flow ────────────────────────────────────────────────────

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
): StarterOption {
  return {
    starter: makeStarter(tier),
    fit: 'fits',
    installed: false,
    partial_bytes: null,
    ...overrides,
  };
}

const BUILTIN_OPTIONS: StarterOption[] = [
  makeOption('fast', { fit: 'fits' }),
  makeOption('balanced', { fit: 'tight' }),
  makeOption('smartest', { fit: 'too_big' }),
];

const BUILTIN_CONFIG: AppConfig = {
  ...DEFAULT_CONFIG,
  inference: {
    ...DEFAULT_CONFIG.inference,
    activeProvider: 'builtin',
    activeProviderKind: 'builtin',
  },
};

function builtinResponses(overrides: Record<string, unknown> = {}) {
  enableChannelCaptureWithResponses({
    // This flow IS the model_check picker, which owns the resume decision, so
    // the DownloadProvider's launch auto-resume gates itself out here.
    onboarding_stage: 'model_check',
    check_model_setup: { state: 'needs_download' },
    get_starter_options: BUILTIN_OPTIONS,
    detect_ollama: true,
    // Brand-new user by default: the announcement has not been latched, so the
    // "use my existing Ollama instead" escape hatch is offered.
    is_builtin_announced: false,
    get_models_dir_free_bytes: 50_000_000_000,
    ...overrides,
  });
}

function renderBuiltin() {
  return render(
    <ConfigProviderForTest value={BUILTIN_CONFIG}>
      <DownloadProvider>
        <ModelCheckStep />
      </DownloadProvider>
    </ConfigProviderForTest>,
  );
}

/** One tap on a column's Download starts the download directly (no confirm). */
async function startDownload(container: HTMLElement, tier: StarterTier) {
  const card = container.querySelector(`[data-tier="${tier}"]`)!;
  await act(async () => {
    fireEvent.click(
      within(card as HTMLElement).getByRole('button', { name: 'Download' }),
    );
  });
}

describe('ModelCheckStep (builtin flow)', () => {
  beforeEach(() => {
    invoke.mockClear();
    resetChannelCapture();
  });

  it('renders the matrix with equal tiers (no recommended column), the more-options stub, and the escape hatch', async () => {
    builtinResponses();

    const { container } = renderBuiltin();
    await act(async () => {});

    // Every tier reads as an equal peer: the recommended highlight
    // (data-recommended attr + the ★ marker) is gone.
    expect(container.querySelector('[data-recommended]')).toBeNull();
    expect(screen.queryByText(/★/)).toBeNull();
    expect(screen.getByText('Use it instead')).toBeInTheDocument();
    expect(
      screen.getByText(
        'Private by default · All inference runs on your machine',
      ),
    ).toBeInTheDocument();
  });

  it('hides the escape hatch when Ollama is not detected', async () => {
    builtinResponses({ detect_ollama: false });

    renderBuiltin();
    await act(async () => {});

    expect(screen.queryByText('Use it instead')).not.toBeInTheDocument();
  });

  it('hides the escape hatch for an upgrader even when Ollama is detected', async () => {
    // Upgrader: the announcement has been latched, so the picker does not
    // re-offer Ollama despite it running on the machine.
    builtinResponses({ detect_ollama: true, is_builtin_announced: true });

    renderBuiltin();
    await act(async () => {});

    expect(screen.queryByText('Use it instead')).not.toBeInTheDocument();
  });

  it('one-tap download starts immediately (no confirm), walks to ready, refreshes, and advances', async () => {
    builtinResponses({ advance_past_model_check: undefined });

    const { container } = renderBuiltin();
    await act(async () => {});

    await startDownload(container as HTMLElement, 'balanced');
    // No confirm step: the download command fires straight away.
    expect(invoke).toHaveBeenCalledWith(
      'download_starter',
      expect.objectContaining({ tier: 'balanced' }),
    );

    const channel = getLastChannel()!;
    await act(async () => {
      channel.simulateMessage({
        type: 'Started',
        data: { file: 'balanced.gguf', total_bytes: 100, resumed_from: 0 },
      });
    });
    // The active column fills in place; the matrix itself stays mounted.
    expect(container.querySelector('[data-starter-matrix]')).not.toBeNull();
    expect(
      screen.getByRole('button', { name: 'Pause download' }),
    ).toBeInTheDocument();

    await act(async () => {
      channel.simulateMessage({ type: 'AllDone' });
    });
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('advance_past_model_check');
    });
    // The picker re-reads the options after the download lands (so the row
    // flips to Installed): more get_starter_options calls than the mount probes.
    expect(
      invoke.mock.calls.filter((c) => c[0] === 'get_starter_options').length,
    ).toBeGreaterThanOrEqual(3);
  });

  it('Continue line advances onboarding while the download keeps running', async () => {
    builtinResponses({ advance_past_model_check: undefined });

    const { container } = renderBuiltin();
    await act(async () => {});
    await startDownload(container as HTMLElement, 'balanced');

    const channel = getLastChannel()!;
    await act(async () => {
      channel.simulateMessage({
        type: 'Started',
        data: { file: 'balanced.gguf', total_bytes: 100, resumed_from: 0 },
      });
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Continue setup →' }));
    });
    expect(invoke).toHaveBeenCalledWith('advance_past_model_check');
  });

  it('advances immediately when check_model_setup already reports ready', async () => {
    builtinResponses({
      check_model_setup: READY_RESPONSE,
      advance_past_model_check: undefined,
    });

    renderBuiltin();
    await act(async () => {});

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('advance_past_model_check');
    });
  });

  it('stays on the picker when the setup probe rejects', async () => {
    builtinResponses();
    const base = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (cmd, args) => {
      if (cmd === 'check_model_setup') throw new Error('ipc broken');
      return base(cmd, args);
    });

    renderBuiltin();
    await act(async () => {});

    expect(screen.getByText('Model balanced')).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith('advance_past_model_check');
  });

  it('hides the escape hatch when the detect probe rejects', async () => {
    builtinResponses();
    const base = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (cmd, args) => {
      if (cmd === 'detect_ollama') throw new Error('down');
      return base(cmd, args);
    });

    renderBuiltin();
    await act(async () => {});

    expect(screen.queryByText('Use it instead')).not.toBeInTheDocument();
    expect(screen.getByText('Model balanced')).toBeInTheDocument();
  });

  it('shows the escape hatch when the announced query rejects (treated as a new user)', async () => {
    builtinResponses();
    const base = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (cmd, args) => {
      if (cmd === 'is_builtin_announced') throw new Error('db down');
      return base(cmd, args);
    });

    renderBuiltin();
    await act(async () => {});

    expect(screen.getByText('Use it instead')).toBeInTheDocument();
  });

  it('pausing a download cancels it and returns the matrix to its download buttons', async () => {
    builtinResponses({ cancel_model_download: undefined });

    const { container } = renderBuiltin();
    await act(async () => {});
    await startDownload(container as HTMLElement, 'balanced');

    const channel = getLastChannel()!;
    await act(async () => {
      channel.simulateMessage({
        type: 'Started',
        data: { file: 'balanced.gguf', total_bytes: 100, resumed_from: 0 },
      });
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Pause download' }));
    });
    expect(invoke).toHaveBeenCalledWith('cancel_model_download', {
      key: 'tier:balanced',
    });

    await act(async () => {
      channel.simulateMessage({ type: 'Cancelled' });
    });
    // Back to the matrix's plain Download buttons.
    expect(
      screen.getAllByRole('button', { name: 'Download' }).length,
    ).toBeGreaterThan(0);
  });

  it('resumes from a partial, showing the bytes and re-invoking the download', async () => {
    const withPartial = [
      makeOption('fast'),
      makeOption('balanced', { fit: 'tight', partial_bytes: 1_200_000_000 }),
      makeOption('smartest'),
    ];
    builtinResponses({ get_starter_options: withPartial });

    renderBuiltin();
    await act(async () => {});

    // 1.2 of the 7.3 GB weights file, mirroring the download view.
    expect(screen.getByText('1.2 / 7.3 GB')).toBeInTheDocument();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Resume download' }));
    });
    expect(invoke).toHaveBeenCalledWith(
      'download_starter',
      expect.objectContaining({ tier: 'balanced' }),
    );
  });

  it('discard invokes discard_partial_download and refreshes the options', async () => {
    const withPartial = [
      makeOption('fast'),
      makeOption('balanced', { partial_bytes: 1_200_000_000 }),
      makeOption('smartest'),
    ];
    builtinResponses({
      get_starter_options: withPartial,
      discard_partial_download: undefined,
    });

    renderBuiltin();
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByText('Discard partial'));
    });
    expect(invoke).toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'b'.repeat(64),
    });
    await waitFor(() => {
      // The discard re-reads the options (also cross-window via models-changed)
      // so the row drops its partial: more calls than the mount probes.
      expect(
        invoke.mock.calls.filter((c) => c[0] === 'get_starter_options').length,
      ).toBeGreaterThanOrEqual(3);
    });
  });

  it('escape hatch from the picker switches the provider and lands in the legacy flow', async () => {
    builtinResponses({ set_active_provider: undefined });

    renderBuiltin();
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByText('Use it instead'));
    });

    expect(invoke).toHaveBeenCalledWith('set_active_provider', {
      providerId: 'ollama',
    });
    // No download in flight from the picker: nothing to cancel.
    expect(invoke).not.toHaveBeenCalledWith('cancel_model_download');
    // The legacy machine renders (its Verify button does not exist in the
    // builtin flow).
    expect(screen.getByLabelText('Verify setup')).toBeInTheDocument();
    expect(screen.getByText('Install & start Ollama')).toBeInTheDocument();
  });

  it('escape hatch during a download cancels it before switching', async () => {
    builtinResponses({
      set_active_provider: undefined,
      cancel_model_download: undefined,
    });

    const { container } = renderBuiltin();
    await act(async () => {});
    await startDownload(container as HTMLElement, 'balanced');

    const channel = getLastChannel()!;
    await act(async () => {
      channel.simulateMessage({
        type: 'Started',
        data: { file: 'balanced.gguf', total_bytes: 100, resumed_from: 0 },
      });
    });

    await act(async () => {
      fireEvent.click(screen.getByText('Use it instead'));
    });

    expect(invoke).toHaveBeenCalledWith('cancel_model_download', {
      key: 'tier:balanced',
    });
    expect(invoke).toHaveBeenCalledWith('set_active_provider', {
      providerId: 'ollama',
    });
    expect(screen.getByLabelText('Verify setup')).toBeInTheDocument();
  });

  it('escape hatch is hidden during a download when Ollama is not detected', async () => {
    builtinResponses({ detect_ollama: false });

    const { container } = renderBuiltin();
    await act(async () => {});
    await startDownload(container as HTMLElement, 'balanced');

    const channel = getLastChannel()!;
    await act(async () => {
      channel.simulateMessage({
        type: 'Started',
        data: { file: 'balanced.gguf', total_bytes: 100, resumed_from: 0 },
      });
    });

    expect(screen.queryByText('Use it instead')).not.toBeInTheDocument();
  });

  it('stays on the builtin flow when switching the provider fails', async () => {
    builtinResponses();
    const base = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (cmd, args) => {
      if (cmd === 'set_active_provider') throw new Error('disk error');
      return base(cmd, args);
    });

    renderBuiltin();
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByText('Use it instead'));
    });

    expect(screen.queryByLabelText('Verify setup')).not.toBeInTheDocument();
    expect(screen.getByText('Model balanced')).toBeInTheDocument();
  });

  it('failure shows the failed card with the escape hatch; retry restarts the download', async () => {
    builtinResponses();

    const { container } = renderBuiltin();
    await act(async () => {});
    await startDownload(container as HTMLElement, 'balanced');

    const channel = getLastChannel()!;
    await act(async () => {
      channel.simulateMessage({
        type: 'Failed',
        data: { kind: 'offline', message: 'no network' },
      });
    });

    expect(screen.getByText("You're offline")).toBeInTheDocument();
    expect(screen.getByText('Use it instead')).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    });
    expect(
      invoke.mock.calls.filter((c) => c[0] === 'download_starter'),
    ).toHaveLength(2);
  });

  it('leaves the other tiers usable after a failure (no lock, no "choose another")', async () => {
    builtinResponses();

    const { container } = renderBuiltin();
    await act(async () => {});
    await startDownload(container as HTMLElement, 'balanced');

    const channel = getLastChannel()!;
    await act(async () => {
      channel.simulateMessage({
        type: 'Failed',
        data: { kind: 'disk_full', message: 'no space left' },
      });
    });
    expect(screen.getByText('Not enough disk')).toBeInTheDocument();

    // The Fast column stays in the matrix and is immediately downloadable;
    // there is no separate "choose another" affordance.
    const fast = container.querySelector('[data-tier="fast"]')!;
    const fastDownload = within(fast as HTMLElement).getByRole('button', {
      name: 'Download',
    });
    expect(fastDownload).not.toBeDisabled();
    await act(async () => {
      fireEvent.click(fastDownload);
    });
    expect(invoke).toHaveBeenCalledWith(
      'download_starter',
      expect.objectContaining({ tier: 'fast' }),
    );
  });

  it('drops probe results that resolve after unmount', async () => {
    let resolveSetup: (v: unknown) => void = () => {};
    let resolveDetect: (v: unknown) => void = () => {};
    let resolveAnnounced: (v: unknown) => void = () => {};
    let resolveFree: (v: unknown) => void = () => {};
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_model_setup') {
        return new Promise((r) => {
          resolveSetup = r;
        });
      }
      if (cmd === 'detect_ollama') {
        return new Promise((r) => {
          resolveDetect = r;
        });
      }
      if (cmd === 'is_builtin_announced') {
        return new Promise((r) => {
          resolveAnnounced = r;
        });
      }
      if (cmd === 'get_models_dir_free_bytes') {
        return new Promise((r) => {
          resolveFree = r;
        });
      }
      if (cmd === 'get_starter_options') return BUILTIN_OPTIONS;
      return undefined;
    });

    const { unmount } = renderBuiltin();
    await act(async () => {});
    unmount();

    await act(async () => {
      resolveSetup(READY_RESPONSE);
      resolveDetect(true);
      resolveAnnounced(false);
      resolveFree(1);
    });

    expect(invoke).not.toHaveBeenCalledWith('advance_past_model_check');
  });
});

describe('buildConfirmInfo', () => {
  it('returns undefined outside the confirming phase', () => {
    expect(buildConfirmInfo({ phase: 'idle' }, BUILTIN_OPTIONS, null)).toBe(
      undefined,
    );
  });

  it('returns undefined when the confirming tier has no option row', () => {
    expect(
      buildConfirmInfo({ phase: 'confirming', tier: 'balanced' }, [], null),
    ).toBe(undefined);
  });

  it('maps size, free disk, and the RAM caution for a non-fits tier', () => {
    expect(
      buildConfirmInfo(
        { phase: 'confirming', tier: 'smartest' },
        BUILTIN_OPTIONS,
        20_000_000_000,
      ),
    ).toEqual({
      sizeGb: 7.3,
      freeDiskGb: 20,
      ramWarning:
        "Larger than this Mac's memory can comfortably hold. Expect heavy slowdown.",
    });
  });

  it('hides the disk line and the warning for a comfortable fit', () => {
    expect(
      buildConfirmInfo(
        { phase: 'confirming', tier: 'fast' },
        BUILTIN_OPTIONS,
        null,
      ),
    ).toEqual({ sizeGb: 7.3, freeDiskGb: null, ramWarning: null });
  });
});
