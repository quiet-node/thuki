import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
} from '@testing-library/react';
import { describe, it, expect, beforeEach, beforeAll, vi } from 'vitest';
import { ModelCheckStep } from '../ModelCheckStep';
import {
  invoke,
  enableChannelCaptureWithResponses,
} from '../../../testUtils/mocks/tauri';

const READY_RESPONSE = {
  state: 'ready',
  active_slug: 'gemma4:e2b',
  installed: ['gemma4:e2b'],
};

// happy-dom does not provide navigator.clipboard, and Object.defineProperty
// on Navigator can collide with property descriptors set elsewhere in the
// test suite. Stash a single writeText spy on a permissive shape so each
// test can assert on it without redefining the host property.
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
    expect(screen.getByText('Install & start Ollama')).toBeInTheDocument();
    expect(screen.getByText('STEP 1 · ACTION NEEDED')).toBeInTheDocument();
    expect(screen.getByText('STEP 2 · WAITING')).toBeInTheDocument();
    expect(screen.getByText('brew install ollama')).toBeInTheDocument();
    expect(screen.getByText('open -a Ollama')).toBeInTheDocument();
  });

  it('shows Step 1 done and Step 2 active on no_models_installed', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    expect(screen.getByText('Ollama is running')).toBeInTheDocument();
    expect(screen.getByText('Connected')).toBeInTheDocument();
    expect(screen.getByText('STEP 1 · DONE')).toBeInTheDocument();
    expect(screen.getByText('STEP 2 · ACTION NEEDED')).toBeInTheDocument();
    expect(screen.getByText('gemma4:e2b')).toBeInTheDocument();
    expect(screen.getByText('llama3:8b')).toBeInTheDocument();
    expect(screen.getByText('qwen2.5:7b')).toBeInTheDocument();
    expect(screen.getByText('RECOMMENDED')).toBeInTheDocument();
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
    expect(screen.getByText('STEP 1 · ACTION NEEDED')).toBeInTheDocument();
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
      fireEvent.click(screen.getByLabelText('Re-check setup'));
    });

    expect(screen.getByText('Ollama is running')).toBeInTheDocument();
    expect(screen.getByText('Connected')).toBeInTheDocument();
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
      fireEvent.click(screen.getByLabelText('Re-check setup'));
    });
    expect(probeCalls).toBe(2);

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Re-check setup'));
    });
    expect(probeCalls).toBe(2);

    await act(async () => {
      resolveSecond({ state: 'no_models_installed' });
    });
  });

  it('copies brew install command when Install Ollama copy button is clicked', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Copy install ollama command'));
    });
    expect(writeText).toHaveBeenCalledWith('brew install ollama');
  });

  it('copies open command when Already installed copy button is clicked', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'ollama_unreachable' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Copy already installed? command'));
    });
    expect(writeText).toHaveBeenCalledWith('open -a Ollama');
  });

  it('copies the pull command for a recommended model', async () => {
    enableChannelCaptureWithResponses({
      check_model_setup: { state: 'no_models_installed' },
    });

    render(<ModelCheckStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByLabelText('Copy install command for llama3:8b'),
      );
    });
    expect(writeText).toHaveBeenCalledWith('ollama pull llama3:8b');
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
});
