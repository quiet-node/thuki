import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import App from '../App';
import type { DownloadContextValue } from '../contexts/DownloadContext';
import {
  invoke,
  emitTauriEvent,
  enableChannelCaptureWithResponses,
  DEFAULT_UPDATER_STATE,
} from '../testUtils/mocks/tauri';

// Same App-root mocks as src/__tests__/App.test.tsx: useDownloadCtx and
// useTips are app-root providers/hooks that `main.tsx` wires around `<App
// />` in production; these tests render `<App />` bare, so both need a
// controllable stand-in.

vi.mock('../hooks/useTips', () => ({
  useTips: vi.fn(() => ({ tip: '', tipKey: 0, isVisible: false })),
}));

const downloadHolder = vi.hoisted(() => ({ value: null as unknown }));
vi.mock('../contexts/DownloadContext', () => ({
  useDownloadCtx: () => downloadHolder.value,
}));

function makeDownloadCtx(
  overrides: Partial<DownloadContextValue> = {},
): DownloadContextValue {
  return {
    state: { phase: 'idle' },
    progress: null,
    etaSeconds: null,
    combinedBytes: null,
    speedBytesPerSec: null,
    beginConfirm: vi.fn(),
    cancelConfirm: vi.fn(),
    start: vi.fn(async () => {}),
    startRepo: vi.fn(async () => {}),
    startById: vi.fn(async () => {}),
    cancel: vi.fn(async () => {}),
    retry: vi.fn(async () => {}),
    resume: vi.fn(async () => {}),
    discard: vi.fn(async () => {}),
    enterResumePending: vi.fn(),
    reset: vi.fn(),
    downloadingTier: null,
    resumeSeedBytes: null,
    activeOption: null,
    grandTotalBytes: null,
    beginDownload: vi.fn(),
    resumeDownload: vi.fn(),
    isPaused: false,
    isPausing: false,
    pausedBytes: 0,
    pauseDownload: vi.fn(),
    resumeFromPause: vi.fn(),
    discardDownload: vi.fn(),
    ...overrides,
  };
}

/** Counts how many times `invoke` was called with the given command name. */
function callCount(cmd: string): number {
  return invoke.mock.calls.filter(([called]) => called === cmd).length;
}

const RECOVERY_HEADLINE = 'Recovered in Safe Mode';

describe('App - safe-mode recovery (issue #296)', () => {
  beforeEach(() => {
    invoke.mockClear();
    downloadHolder.value = makeDownloadCtx();
  });

  it('fires mark_startup_healthy exactly once on mount, unconditionally', async () => {
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: false, unclean_count: 0 },
    });

    render(<App />);
    await act(async () => {});

    expect(callCount('mark_startup_healthy')).toBe(1);
    expect(invoke).toHaveBeenCalledWith('mark_startup_healthy');
  });

  it('does not show the recovery screen when safe mode is false', async () => {
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: false, unclean_count: 0 },
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
    });

    render(<App />);
    await act(async () => {});

    expect(screen.queryByText(RECOVERY_HEADLINE)).not.toBeInTheDocument();
    expect(callCount('estimate_model_fit')).toBe(0);
  });

  it('lets onboarding win over the recovery screen when both would otherwise apply', async () => {
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: true, unclean_count: 4 },
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
      estimate_model_fit: {
        required_bytes: 8 * 1024 ** 3,
        available_bytes: 4 * 1024 ** 3,
        verdict: 'Tight',
      },
      check_accessibility_permission: false,
      check_screen_recording_permission: false,
    });

    render(<App />);
    await act(async () => {});

    await act(async () => {
      emitTauriEvent('thuki://onboarding', { stage: 'permissions' });
    });

    expect(screen.getByText("Let's get Thuki set up")).toBeInTheDocument();
    expect(screen.queryByText(RECOVERY_HEADLINE)).not.toBeInTheDocument();
    // Still fires unconditionally even though onboarding is gating render.
    expect(callCount('mark_startup_healthy')).toBe(1);
  });

  it('does not show the recovery screen when no active model resolves', async () => {
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: true, unclean_count: 4 },
      get_model_picker_state: { active: null, all: [], ollamaReachable: true },
    });

    render(<App />);
    await act(async () => {});

    expect(screen.queryByText(RECOVERY_HEADLINE)).not.toBeInTheDocument();
    expect(callCount('estimate_model_fit')).toBe(0);
  });

  it('does not show the recovery screen when the fit estimate cannot be resolved', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_model_picker_state') {
        return {
          active: 'gemma4:e2b',
          all: ['gemma4:e2b'],
          ollamaReachable: true,
        };
      }
      if (cmd === 'startup_safety')
        return { safe_mode: true, unclean_count: 4 };
      if (cmd === 'estimate_model_fit') {
        throw new Error('The selected model is not installed.');
      }
      if (cmd === 'get_updater_state') return DEFAULT_UPDATER_STATE;
      return undefined;
    });

    render(<App />);
    await act(async () => {});

    expect(screen.queryByText(RECOVERY_HEADLINE)).not.toBeInTheDocument();
    expect(callCount('estimate_model_fit')).toBe(1);
  });

  it('does not show the recovery screen when the fit estimate resolves with no result', async () => {
    // `estimate_model_fit` deliberately left unmocked: the shared test
    // double resolves any command it does not recognize to `undefined`
    // (matching a well-behaved but empty response), which must degrade to
    // "nothing to recover into" rather than throwing.
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: true, unclean_count: 4 },
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
    });

    render(<App />);
    await act(async () => {});

    expect(screen.queryByText(RECOVERY_HEADLINE)).not.toBeInTheDocument();
    expect(callCount('estimate_model_fit')).toBe(1);
  });

  it('shows the recovery screen with the interpolated model name and size when every gate passes', async () => {
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: true, unclean_count: 3 },
      get_model_picker_state: {
        active: 'llama-3.2-3b',
        all: ['llama-3.2-3b'],
        ollamaReachable: true,
        displayNames: { 'llama-3.2-3b': 'Llama 3.2 3B' },
      },
      estimate_model_fit: {
        required_bytes: 8 * 1024 ** 3,
        available_bytes: 4 * 1024 ** 3,
        verdict: 'Tight',
      },
    });

    render(<App />);
    await act(async () => {});

    expect(screen.getByText(RECOVERY_HEADLINE)).toBeInTheDocument();
    expect(
      screen.getByText(
        'Llama 3.2 3B (8.0 GB) was loading when the last session ended unexpectedly, possibly because it needed more memory than was available.',
      ),
    ).toBeInTheDocument();
    expect(callCount('estimate_model_fit')).toBe(1);
  });

  it('"Choose a different model" dismisses the screen and re-opens the model picker', async () => {
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: true, unclean_count: 3 },
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
      estimate_model_fit: {
        required_bytes: 8 * 1024 ** 3,
        available_bytes: 4 * 1024 ** 3,
        verdict: 'Tight',
      },
    });

    render(<App />);
    await act(async () => {});
    expect(screen.getByText(RECOVERY_HEADLINE)).toBeInTheDocument();

    const modelPickerCallsBefore = callCount('get_model_picker_state');
    const capabilitiesCallsBefore = callCount('get_model_capabilities');

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Choose a different model' }),
      );
    });

    expect(screen.queryByText(RECOVERY_HEADLINE)).not.toBeInTheDocument();
    // handleChooseDifferentModelFromSafeMode reuses the never-toggle "open"
    // mechanism, which refreshes both the model list and the capability map.
    expect(callCount('get_model_picker_state')).toBeGreaterThan(
      modelPickerCallsBefore,
    );
    expect(callCount('get_model_capabilities')).toBeGreaterThan(
      capabilitiesCallsBefore,
    );
  });

  // Regression coverage for issue #296 follow-up (bug 2): the model picker's
  // "Switch model" retry wiring shares `handleModelSelect` with this screen's
  // "Choose a different model" path, which has no associated chat turn at
  // all. Picking a model here must only switch it, never replay a turn.
  it('picking a model after "Choose a different model" never triggers a retry', async () => {
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: true, unclean_count: 3 },
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
      estimate_model_fit: {
        required_bytes: 8 * 1024 ** 3,
        available_bytes: 4 * 1024 ** 3,
        verdict: 'Tight',
      },
    });

    render(<App />);
    await act(async () => {});
    // The picker dropdown that "Choose a different model" opens lives in the
    // normal (post-recovery-screen) render tree, which is gated on the
    // overlay having been shown at least once - unlike the recovery screen
    // itself, which renders unconditionally as an early return.
    await act(async () => {
      emitTauriEvent('thuki://visibility', {
        state: 'show',
        selected_text: null,
        window_x: null,
        window_y: null,
        screen_bottom_y: null,
      });
    });
    expect(screen.getByText(RECOVERY_HEADLINE)).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Choose a different model' }),
      );
    });
    expect(
      screen.getByRole('option', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();

    invoke.mockClear();
    fireEvent.click(screen.getByRole('option', { name: 'qwen2.5:7b' }));
    await act(async () => {});

    expect(invoke).toHaveBeenCalledWith('set_active_model', {
      model: 'qwen2.5:7b',
    });
    expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
  });

  it('"Load last model anyway" just dismisses the screen with no extra invoke', async () => {
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: true, unclean_count: 3 },
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
      estimate_model_fit: {
        required_bytes: 8 * 1024 ** 3,
        available_bytes: 4 * 1024 ** 3,
        verdict: 'Tight',
      },
    });

    render(<App />);
    await act(async () => {});
    expect(screen.getByText(RECOVERY_HEADLINE)).toBeInTheDocument();

    const callsBefore = invoke.mock.calls.length;

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Load last model anyway' }),
      );
    });

    expect(screen.queryByText(RECOVERY_HEADLINE)).not.toBeInTheDocument();
    expect(invoke.mock.calls.length).toBe(callsBefore);
  });

  it('never re-shows the screen after dismissal, even if the active model later changes', async () => {
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: true, unclean_count: 3 },
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
      estimate_model_fit: {
        required_bytes: 8 * 1024 ** 3,
        available_bytes: 4 * 1024 ** 3,
        verdict: 'Tight',
      },
    });

    render(<App />);
    await act(async () => {});
    expect(screen.getByText(RECOVERY_HEADLINE)).toBeInTheDocument();
    expect(callCount('estimate_model_fit')).toBe(1);

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Load last model anyway' }),
      );
    });
    expect(screen.queryByText(RECOVERY_HEADLINE)).not.toBeInTheDocument();

    // Simulate the active model changing later in the same launch (e.g. a
    // pick made from the Settings webview), which is how `useModelSelection`
    // resyncs across windows. This changes `activeModel`, re-running the
    // resolution effect - but the ref guard must keep it a no-op.
    enableChannelCaptureWithResponses({
      startup_safety: { safe_mode: true, unclean_count: 3 },
      get_model_picker_state: {
        active: 'qwen2.5:7b',
        all: ['qwen2.5:7b'],
        ollamaReachable: true,
      },
      estimate_model_fit: {
        required_bytes: 8 * 1024 ** 3,
        available_bytes: 4 * 1024 ** 3,
        verdict: 'Tight',
      },
    });

    await act(async () => {
      emitTauriEvent('thuki://config-updated', {});
    });

    expect(screen.queryByText(RECOVERY_HEADLINE)).not.toBeInTheDocument();
    expect(callCount('estimate_model_fit')).toBe(1);
  });
});
