import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
  cleanup,
} from '@testing-library/react';
import { describe, it, expect, beforeEach, beforeAll, vi } from 'vitest';
import { ModelCheckStep } from '../ModelCheckStep';
import {
  ConfigProviderForTest,
  DEFAULT_CONFIG,
} from '../../../contexts/ConfigContext';
import {
  invoke,
  enableChannelCaptureWithResponses,
} from '../../../testUtils/mocks/tauri';

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
      screen.getByText("Almost there. Let's pick a brain for Thuki."),
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
          model: { ...DEFAULT_CONFIG.model, ollamaUrl: 'http://10.0.0.5:9000' },
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
          model: { ...DEFAULT_CONFIG.model, ollamaUrl: 'not-a-url' },
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

  it('lights up the slug link on pointer hover', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    const link = screen.getByLabelText('Open phi4:14b on Ollama');
    const initialColor = link.style.color;
    fireEvent.mouseEnter(link);
    expect(link.style.color).not.toBe(initialColor);
    fireEvent.mouseLeave(link);
    expect(link.style.color).toBe(initialColor);
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

  it('lights up sub-line doc links on pointer hover', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    const link = screen.getByLabelText('Open Ollama documentation');
    const initialColor = link.style.color;
    fireEvent.mouseEnter(link);
    expect(link.style.color).not.toBe(initialColor);
    fireEvent.mouseLeave(link);
    expect(link.style.color).toBe(initialColor);
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
