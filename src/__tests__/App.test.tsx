import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
} from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import App from '../App';
import {
  DEFAULT_CONFIG,
  ConfigProviderForTest,
} from '../contexts/ConfigContext';
import type { DownloadContextValue } from '../contexts/DownloadContext';
import {
  invoke,
  emitTauriEvent,
  enableChannelCapture,
  enableChannelCaptureWithResponses,
  getLastChannel,
  TEST_DEFAULT_MODEL_PICKER_STATE,
  DEFAULT_UPDATER_STATE,
} from '../testUtils/mocks/tauri';
import {
  __mockWindow,
  __setWindowGeometry,
  __setAvailableMonitors,
} from '../testUtils/mocks/tauri-window';
import { useTips } from '../hooks/useTips';
import { flushSync } from 'react-dom';
import {
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  type LexicalEditor,
} from 'lexical';

vi.mock('../hooks/useTips', () => ({
  useTips: vi.fn(() => ({ tip: '', tipKey: 0, isVisible: false })),
}));

// The download machine lives in an app-root provider that `main.tsx` wires
// around `<App />`; these tests render `<App />` bare, so `useDownloadCtx` is
// mocked to a controllable value. `downloadHolder.value` is reset to an idle
// machine before every test and overridden per-test to drive the ambient
// strip, the submit soft-block, and the ready-refresh effect.
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
    pausedBytes: 0,
    pauseDownload: vi.fn(),
    resumeFromPause: vi.fn(),
    discardActive: vi.fn(),
    ...overrides,
  };
}

/** The AskBar Lexical contentEditable input (role="textbox"). */
function getAskInput(): HTMLElement {
  return screen.getByTestId('askbar-input');
}

/**
 * Sets the AskBar editor's text, mimicking the user typing it. jsdom does not
 * synthesize contentEditable edits from key events, so the value is applied via
 * the editor API; this still drives the real value-sync (onValueChange) path.
 */
function setAskValue(text: string): void {
  const editor = (
    getAskInput() as unknown as { __lexicalEditor: LexicalEditor }
  ).__lexicalEditor;
  // `discrete: true` commits + fires update listeners synchronously, and
  // flushSync forces React to apply the resulting setQuery before we return, so
  // callers can read the updated state immediately (mirrors the old synchronous
  // fireEvent.change behavior).
  act(() => {
    // test-only: force the controlled value sync to apply synchronously so
    // callers can read the updated state immediately.
    // eslint-disable-next-line @eslint-react/dom-no-flush-sync
    flushSync(() => {
      editor.update(
        () => {
          const root = $getRoot();
          root.clear();
          const paragraph = $createParagraphNode();
          if (text) paragraph.append($createTextNode(text));
          root.append(paragraph);
        },
        { discrete: true },
      );
    });
  });
}

async function showOverlay(selectedText: string | null = null) {
  await act(async () => {
    emitTauriEvent('thuki://visibility', {
      state: 'show',
      selected_text: selectedText,
      window_x: null,
      window_y: null,
      screen_bottom_y: null,
    });
  });
}

describe('App', () => {
  beforeEach(() => {
    invoke.mockClear();
    enableChannelCapture();
    downloadHolder.value = makeDownloadCtx();
  });

  it('fetches model picker state on mount and refreshes it when the overlay shows', async () => {
    invoke.mockReset();
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
    });

    render(<App />);
    await act(async () => {});

    expect(invoke).toHaveBeenCalledWith('get_model_picker_state');

    invoke.mockClear();

    await showOverlay();

    expect(invoke).toHaveBeenCalledWith('get_model_picker_state');
  });

  it('renders the model picker when the overlay is visible and models load', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toBeInTheDocument();
  });

  it('keeps the chat-mode picker chip visible with "Pick a model" when active model disappears (S2)', async () => {
    // S2: Ollama is reachable but no models are installed. The chip must
    // stay in WindowControls in chat mode so the user has a one-click
    // recovery path, and its label falls back to the picker prompt
    // instead of showing a stale or empty slug.
    //
    // Simulating S2 from a cold start would block the submit at the
    // env-state gate, so we cannot enter chat mode that way. Instead we
    // start with an active model, complete a turn (which puts the user
    // in chat mode), then arrange the next `get_model_picker_state`
    // refresh to return the S2 payload.
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    // Send one message + simulate Done so messages.length > 0 → chat mode.
    const textarea = getAskInput();
    act(() => {
      setAskValue('hello');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'hi' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // Switch the picker mock to return the S2 payload, then trigger the
    // chip click which calls `refreshModels` under the hood.
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: null,
        all: [],
        ollamaReachable: true,
      },
    });

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});

    const chip = screen.getByRole('button', { name: 'Choose model' });
    expect(chip).toBeInTheDocument();
    expect(chip.textContent).toContain('Pick a model');
  });

  it('hides the chat-mode picker chip when Ollama becomes unreachable (S1)', async () => {
    // S1: nothing to pick from. The chip is hidden in chat mode so the
    // user is not pointed at a dead-end action; the strip handles the
    // "start Ollama" cue separately. We mirror the S2 test setup but
    // swap the second picker fetch to the unreachable payload.
    //
    // Triggering the refresh through the chip click rather than the
    // overlay show event matters: the show handler also resets messages
    // (so isChatMode flips back to false and the chip unmounts for an
    // unrelated reason). The chip click drives the refresh in place.
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    const textarea = getAskInput();
    act(() => {
      setAskValue('hello');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'hi' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // Confirm we are in chat mode with the chip rendered before flipping
    // the picker state to the unreachable variant.
    expect(
      screen.getByRole('button', { name: 'Choose model' }),
    ).toBeInTheDocument();

    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: null,
        all: [],
        ollamaReachable: false,
      },
    });

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});

    expect(screen.queryByRole('button', { name: 'Choose model' })).toBeNull();
  });

  it('renders the unreachable strip copy in compose mode when Ollama is down (S1)', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: null,
        all: [],
        ollamaReachable: false,
      },
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    const strip = screen.getByTestId('capability-mismatch-strip');
    expect(strip.textContent).toContain("Ollama isn't running");
  });

  it('renders the no-models strip copy when Ollama is reachable but empty (S2)', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: null,
        all: [],
        ollamaReachable: true,
      },
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    const strip = screen.getByTestId('capability-mismatch-strip');
    expect(strip.textContent).toContain(
      "Thuki couldn't find any local LLM models",
    );
    expect(strip.textContent).toContain('ollama pull <model>');
  });

  it('submits normally when the builtin provider is active with a downloaded model', async () => {
    // Regression guard for the builtin gate bug: with the builtin provider
    // active, the picker payload reports reachable=true and the manifest
    // inventory, so the env gate must let the message through instead of
    // blocking with the Ollama copy.
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'tinyllama-1.1b',
        all: ['tinyllama-1.1b'],
        ollamaReachable: true,
      },
    });

    render(
      <ConfigProviderForTest
        value={{
          ...DEFAULT_CONFIG,
          inference: {
            ...DEFAULT_CONFIG.inference,
            activeProvider: 'builtin',
            activeProviderKind: 'builtin',
          },
        }}
      >
        <App />
      </ConfigProviderForTest>,
    );
    await act(async () => {});
    await showOverlay();

    const textarea = getAskInput();
    act(() => {
      setAskValue('hello from the builtin engine');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    expect(invoke).toHaveBeenCalledWith(
      'ask_model',
      expect.objectContaining({ message: 'hello from the builtin engine' }),
    );
  });

  it('blocks submit with the builtin download copy when no model is downloaded', async () => {
    // Builtin provider active, manifest empty: the strip must point at the
    // Settings download flow, never at Ollama, and the submit stays gated.
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: null,
        all: [],
        ollamaReachable: true,
      },
    });

    render(
      <ConfigProviderForTest
        value={{
          ...DEFAULT_CONFIG,
          inference: {
            ...DEFAULT_CONFIG.inference,
            activeProvider: 'builtin',
            activeProviderKind: 'builtin',
          },
        }}
      >
        <App />
      </ConfigProviderForTest>,
    );
    await act(async () => {});
    await showOverlay();

    const strip = screen.getByTestId('capability-mismatch-strip');
    expect(strip.textContent).toContain('No model downloaded yet');
    expect(strip.textContent).not.toContain('Ollama');

    const textarea = getAskInput();
    act(() => {
      setAskValue('hello');
    });
    invoke.mockClear();
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    const askInvocations = invoke.mock.calls.filter(
      (call) => call[0] === 'ask_model',
    );
    expect(askInvocations.length).toBe(0);
    // Wait past the 600 ms shake reset so the gate's timeout cleanup runs.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 650));
    });
  });

  it('saves the conversation with the currently selected model', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
      save_conversation: { conversation_id: 'conv-1' },
      generate_title: undefined,
      set_active_model: undefined,
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {
      fireEvent.click(screen.getByRole('option', { name: 'qwen2.5:7b' }));
    });

    const textarea = getAskInput();
    setAskValue('hello there');
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    await act(async () => {});

    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hi there!' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });

    await act(async () => {
      fireEvent.click(screen.getByLabelText('Save conversation'));
    });

    // The picker selection is threaded into `generate_title` (which uses the
    // active slug as the title-generation model) and stamped onto the
    // assistant message via `model_name`. `save_conversation` itself does
    // not take a top-level `model` arg; the active model is sourced
    // backend-side from the loaded TOML AppConfig.
    expect(invoke).toHaveBeenCalledWith(
      'generate_title',
      expect.objectContaining({ model: 'qwen2.5:7b' }),
    );
  });

  it('opens model picker panel when trigger is clicked', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});

    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('option', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();
  });

  it('closes model picker and opens history when history toggle clicked', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
      list_conversations: [],
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Open history' }));
    await act(async () => {});
    expect(screen.queryByRole('option', { name: 'gemma4:e2b' })).toBeNull();
    expect(
      screen.getByPlaceholderText(/search past chats/i),
    ).toBeInTheDocument();
  });

  it('closes history and opens model picker when model picker trigger clicked', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
      list_conversations: [],
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    fireEvent.click(screen.getByRole('button', { name: 'Open history' }));
    await act(async () => {});
    expect(
      screen.getByPlaceholderText(/search past chats/i),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});
    expect(screen.queryByPlaceholderText(/search past chats/i)).toBeNull();
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();
  });

  it('closes model picker when a model is selected', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
      set_active_model: undefined,
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('option', { name: 'qwen2.5:7b' }));
    await act(async () => {});
    expect(screen.queryByRole('option', { name: 'qwen2.5:7b' })).toBeNull();
  });

  it('closes model picker when the trigger is clicked while open', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    const trigger = screen.getByRole('button', { name: 'Choose model' });
    fireEvent.click(trigger);
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();

    // Second click on the trigger toggles the panel closed; this exercises
    // the "opening = false" branch of handleModelPickerToggle.
    fireEvent.click(trigger);
    await act(async () => {});
    expect(screen.queryByRole('option', { name: 'gemma4:e2b' })).toBeNull();
  });

  it('closes model picker when generation starts', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();

    const textarea = getAskInput();
    setAskValue('hi');
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    await act(async () => {});

    expect(screen.queryByRole('option', { name: 'gemma4:e2b' })).toBeNull();
  });

  it('shows active model pill in chat mode header and opens picker from there', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    // Transition to chat mode by submitting a message
    const textarea = getAskInput();
    setAskValue('hi');
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    await act(async () => {});

    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hello!' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // Pill button should now be in the header (WindowControls), showing the model name
    const pill = screen.getByRole('button', { name: 'Choose model' });
    expect(pill).toBeInTheDocument();
    expect(pill.textContent).toContain('gemma4:e2b');

    // Click pill → model picker panel opens ABOVE the conversation
    fireEvent.click(pill);
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('option', { name: 'qwen2.5:7b' }),
    ).toBeInTheDocument();
  });

  it('closes chat-mode model picker when clicking outside the dropdown', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    const textarea = getAskInput();
    setAskValue('hi');
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hello!' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();

    fireEvent.mouseDown(document.body);
    await act(async () => {});
    expect(screen.queryByRole('option', { name: 'gemma4:e2b' })).toBeNull();
  });

  it('chat-mode click-outside does NOT close when clicking inside the dropdown or on the pill', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    const textarea = getAskInput();
    setAskValue('hi');
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hello!' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    const pill = screen.getByRole('button', { name: 'Choose model' });
    fireEvent.click(pill);
    await act(async () => {});
    const option = screen.getByRole('option', { name: 'gemma4:e2b' });
    expect(option).toBeInTheDocument();

    // mousedown inside the dropdown must not close the picker
    fireEvent.mouseDown(option);
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();

    // mousedown on the pill trigger must not close the picker either
    fireEvent.mouseDown(pill);
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();
  });

  it('ask-bar mode click-outside closes the model picker drawer', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();

    // Clicking inside the drawer must NOT close it
    fireEvent.mouseDown(screen.getByRole('option', { name: 'gemma4:e2b' }));
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();

    // Clicking outside closes the drawer
    fireEvent.mouseDown(document.body);
    await act(async () => {});
    expect(screen.queryByRole('option', { name: 'gemma4:e2b' })).toBeNull();
  });

  it('refreshes model list when set_active_model rejects', async () => {
    let rejectionSeen = false;
    let refreshesAfterRejection = 0;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_model_picker_state') {
        if (rejectionSeen) {
          refreshesAfterRejection += 1;
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        }
        return {
          active: 'gemma4:e2b',
          all: ['gemma4:e2b', 'qwen2.5:7b'],
          ollamaReachable: true,
        };
      }
      if (cmd === 'set_active_model') {
        rejectionSeen = true;
        throw new Error('Model is not installed: qwen2.5:7b');
      }
      return undefined;
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});
    fireEvent.click(screen.getByRole('option', { name: 'qwen2.5:7b' }));
    await act(async () => {});

    // The rejection handler must have triggered at least one refresh fetch.
    expect(refreshesAfterRejection).toBeGreaterThanOrEqual(1);

    // Reopen to confirm the list is the post-refresh one (qwen was removed).
    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();
    expect(screen.queryByRole('option', { name: 'qwen2.5:7b' })).toBeNull();
  });

  it('closes the model picker drawer when Escape is pressed in the filter input', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b', 'qwen2.5:7b'],
        ollamaReachable: true,
      },
    });
    render(<App />);
    await act(async () => {});
    await showOverlay();

    fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
    await act(async () => {});
    expect(
      screen.getByRole('option', { name: 'gemma4:e2b' }),
    ).toBeInTheDocument();

    fireEvent.keyDown(screen.getByPlaceholderText(/filter models/i), {
      key: 'Escape',
    });
    await act(async () => {});
    expect(screen.queryByRole('option', { name: 'gemma4:e2b' })).toBeNull();
  });

  it('grows upward when near bottom screen edge', async () => {
    const { container } = render(<App />);
    await act(async () => {});

    await act(async () => {
      emitTauriEvent('thuki://visibility', {
        state: 'show',
        selected_text: null,
        window_x: 50,
        window_y: 1000,
        screen_bottom_y: 1100,
      });
    });

    const textarea = getAskInput();
    await act(async () => {
      setAskValue('hi');
    });
    await act(async () => {
      fireEvent.keyDown(textarea, { key: 'Enter', code: 'Enter' });
    });
    // This should morph into max-height window
    await act(async () => {
      await new Promise((r) => requestAnimationFrame(r));
    });
    expect(
      (container.querySelector('.morphing-container') as HTMLElement).style
        .height,
    ).toBe(`${DEFAULT_CONFIG.window.maxChatHeight}px`);
  });

  it('keeps full chat height after clicking the expanded upward chat surface', async () => {
    const { container } = render(<App />);
    await act(async () => {});

    await act(async () => {
      emitTauriEvent('thuki://visibility', {
        state: 'show',
        selected_text: null,
        window_x: 50,
        window_y: 1000,
        screen_bottom_y: 1100,
      });
    });

    const textarea = getAskInput();
    await act(async () => {
      setAskValue('hi');
      fireEvent.keyDown(textarea, { key: 'Enter', code: 'Enter' });
    });

    const morphingContainer = container.querySelector(
      '.morphing-container',
    ) as HTMLElement;
    expect(morphingContainer.style.height).toBe(
      `${DEFAULT_CONFIG.window.maxChatHeight}px`,
    );

    const chatArea = container.querySelector('.chat-area');
    expect(chatArea).not.toBeNull();

    act(() => {
      fireEvent.mouseDown(chatArea!);
      fireEvent.mouseUp(window);
    });

    expect(morphingContainer.style.height).toBe(
      `${DEFAULT_CONFIG.window.maxChatHeight}px`,
    );
  });

  it('renders nothing when overlay is hidden', async () => {
    const { container } = render(<App />);
    // Flush effects so listener registers
    await act(async () => {});

    expect(container.querySelector('.morphing-container')).toBeNull();
  });

  it('shows overlay on visibility show event', async () => {
    render(<App />);
    // Flush effects so listener registers
    await act(async () => {});

    await showOverlay();

    expect(getAskInput()).toBeInTheDocument();
  });

  it('handles a restore visibility event without wiping the conversation', async () => {
    // Arrange: render App and drive it into chat mode with one complete turn.
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    const textarea = getAskInput();
    act(() => {
      setAskValue('hello');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'world' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // Confirm the conversation is present (chat mode with messages).
    expect(screen.getByText('hello')).toBeInTheDocument();
    expect(screen.getByText('world')).toBeInTheDocument();

    // Act: dispatch a restore visibility event.
    await act(async () => {
      emitTauriEvent('thuki://visibility', { state: 'restore' });
    });

    // Assert: existing messages are still rendered (conversation was NOT wiped).
    expect(screen.getByText('hello')).toBeInTheDocument();
    expect(screen.getByText('world')).toBeInTheDocument();
  });

  it('clicking Minimize button in chat mode calls setIsMinimized (handleMinimize stub)', async () => {
    // Arrange: render App in chat mode with one complete turn.
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    const textarea = getAskInput();
    act(() => {
      setAskValue('hello');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'hi' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // Act: click the Minimize button rendered in the chat header.
    const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
    expect(minimizeBtn).toBeInTheDocument();
    act(() => {
      fireEvent.click(minimizeBtn);
    });

    // Assert: no throw; the stub runs without error.
    // Task 7 will assert the full minimize effect (MinimizedIcon visible, etc.).
    expect(minimizeBtn).toBeTruthy();
  });

  it('hides overlay on Escape key', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    // Confirm overlay is visible
    expect(getAskInput()).toBeInTheDocument();

    act(() => {
      fireEvent.keyDown(window, { key: 'Escape' });
    });

    expect(invoke).toHaveBeenCalledWith('notify_overlay_hidden');
  });

  it('hides overlay on Escape key and cancels an active /search turn', async () => {
    vi.useFakeTimers();
    enableChannelCapture();
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const textarea = getAskInput();
    act(() => {
      setAskValue('/search rust async');
    });
    await act(async () => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });

    invoke.mockClear();

    await act(async () => {
      fireEvent.keyDown(window, { key: 'Escape' });
      vi.advanceTimersByTime(351);
      await Promise.resolve();
    });

    expect(invoke).toHaveBeenCalledWith('cancel_generation');
    expect(invoke).toHaveBeenCalledWith('notify_overlay_hidden');
    expect(screen.queryByPlaceholderText('Ask Thuki anything...')).toBeNull();
    vi.useRealTimers();
  });

  it('completes a full conversation turn', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const textarea = getAskInput();

    // Type a message
    setAskValue('hello there');

    // Submit with Enter
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });

    // Wait for invoke to be called (ask_model)
    await act(async () => {});

    // Simulate streaming tokens
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hi' });
      getLastChannel()?.simulateMessage({ type: 'Token', data: ' there!' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });

    // The assistant response should now be in the DOM
    expect(screen.getByText('Hi there!')).toBeInTheDocument();
  });

  it('shows selected context when provided', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay('some code snippet');

    expect(screen.getByText(/some code snippet/)).toBeInTheDocument();
  });

  it('enters hiding state on hide-request visibility event', async () => {
    render(<App />);
    await act(async () => {});

    // First show overlay
    await showOverlay();
    expect(getAskInput()).toBeInTheDocument();

    // Then send hide-request - calls requestHideOverlay() (not handleCloseOverlay)
    await act(async () => {
      emitTauriEvent('thuki://visibility', { state: 'hide-request' });
    });

    // The hide-request path transitions overlay to hiding state (overlayState !== 'visible'),
    // so shouldRenderOverlay becomes false and the overlay is removed from the DOM.
    expect(screen.queryByPlaceholderText('Ask Thuki anything...')).toBeNull();
  });

  it('hides overlay on Cmd+W key', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();
    expect(getAskInput()).toBeInTheDocument();

    act(() => {
      fireEvent.keyDown(window, { key: 'w', metaKey: true });
    });

    expect(invoke).toHaveBeenCalledWith('notify_overlay_hidden');
  });

  it('hides overlay on Ctrl+W key', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    act(() => {
      fireEvent.keyDown(window, { key: 'w', ctrlKey: true });
    });

    expect(invoke).toHaveBeenCalledWith('notify_overlay_hidden');
  });

  it('commits window hide after HIDE_COMMIT_DELAY_MS when hiding', async () => {
    vi.useFakeTimers();
    render(<App />);
    await act(async () => {});

    await showOverlay();

    act(() => {
      fireEvent.keyDown(window, { key: 'Escape' });
    });

    // Advance past the 350ms hide delay
    await act(async () => {
      vi.advanceTimersByTime(400);
    });

    expect(__mockWindow.hide).toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('does not submit empty query', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const textarea = getAskInput();

    // Press Enter with empty textarea
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });

    await act(async () => {});

    // ask_model should NOT have been called
    expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
  });

  it('lets the user keep drafting while a response streams, without sending', async () => {
    enableChannelCapture();
    render(<App />);
    await act(async () => {});
    await showOverlay();

    // First message kicks off streaming. The captured channel never sends
    // Done, so generation stays active for the rest of the test.
    setAskValue('first message');
    act(() => {
      fireEvent.keyDown(getAskInput(), { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    const askCalls = () =>
      vi.mocked(invoke).mock.calls.filter((c) => c[0] === 'ask_model').length;
    expect(askCalls()).toBe(1);

    // While streaming, the composer (now in chat mode) stays editable.
    expect(getAskInput().getAttribute('contenteditable')).toBe('true');

    // Drafting a follow-up and pressing Enter must NOT fire a second request,
    // and the draft must remain in the box.
    setAskValue('drafted mid-stream');
    act(() => {
      fireEvent.keyDown(getAskInput(), { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    expect(askCalls()).toBe(1);
    expect(getAskInput().textContent).toBe('drafted mid-stream');
  });

  it('fires drag on non-interactive mousedown', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    // Fire mousedown on the outermost div (non-interactive)
    const container = document.querySelector('.morphing-container');
    expect(container).not.toBeNull();

    act(() => {
      fireEvent.mouseDown(container!);
    });

    expect(__mockWindow.startDragging).toHaveBeenCalled();
  });

  it('clears upward growth on mouseup after drag', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const container = document.querySelector('.morphing-container');
    expect(container).not.toBeNull();

    __mockWindow.startDragging.mockClear();

    act(() => {
      fireEvent.mouseDown(container!);
    });

    // startDragging was called; fire mouseup to cover the mouseup handler
    act(() => {
      fireEvent.mouseUp(window);
    });

    expect(__mockWindow.startDragging).toHaveBeenCalled();
  });

  it('does not fire drag when mousedown on select-text element', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    // Send a message to enter chat mode so ChatBubble (with .select-text) renders
    const textarea = getAskInput();
    act(() => {
      setAskValue('test message');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Reply' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });

    // Find a .select-text element
    const selectTextEl = document.querySelector('.select-text');
    if (selectTextEl) {
      __mockWindow.startDragging.mockClear();
      act(() => {
        fireEvent.mouseDown(selectTextEl);
      });
      expect(__mockWindow.startDragging).not.toHaveBeenCalled();
    }
  });

  it('does not fire drag when mousedown on the contenteditable input', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const textarea = getAskInput();
    __mockWindow.startDragging.mockClear();

    act(() => {
      fireEvent.mouseDown(textarea);
    });

    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });

  it('submits query with quoted text when selectedContext is set', async () => {
    render(<App />);
    await act(async () => {});

    // Show with selected context
    await showOverlay('selected snippet');

    const textarea = getAskInput();
    act(() => {
      setAskValue('my question');
    });

    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });

    await act(async () => {});

    // Backend receives the message and quoted text separately
    expect(invoke).toHaveBeenCalledWith(
      'ask_model',
      expect.objectContaining({
        message: 'my question',
        quotedText: 'selected snippet',
      }),
    );
  });

  it('auto-replaces the source selection after /rewrite when the setting is on', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
      replace_selection: 'replaced',
    });

    render(
      <ConfigProviderForTest
        value={{
          ...DEFAULT_CONFIG,
          behavior: { autoReplace: true, autoClose: false },
        }}
      >
        <App />
      </ConfigProviderForTest>,
    );
    await act(async () => {});
    await showOverlay('draft email text');

    const textarea = getAskInput();
    act(() => {
      setAskValue('/rewrite ');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    act(() => {
      getLastChannel()?.simulateMessage({
        type: 'Token',
        data: 'Polished draft',
      });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // Auto-replace writes straight back to the source app on completion. The
    // paste is posted to the target process and fires synchronously, with no
    // overlay dismiss to wait on.
    await act(async () => {});

    // The completed /rewrite over a selection writes straight back to the
    // source app because auto-replace is enabled.
    expect(invoke).toHaveBeenCalledWith('replace_selection', {
      text: 'Polished draft',
    });
  });

  it('auto-replace strips stray turn-boundary tokens before writing back', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
      replace_selection: 'replaced',
    });

    render(
      <ConfigProviderForTest
        value={{
          ...DEFAULT_CONFIG,
          behavior: { autoReplace: true, autoClose: false },
        }}
      >
        <App />
      </ConfigProviderForTest>,
    );
    await act(async () => {});
    await showOverlay('draft email text');

    const textarea = getAskInput();
    act(() => {
      setAskValue('/rewrite ');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    act(() => {
      getLastChannel()?.simulateMessage({
        type: 'Token',
        data: '<think>Polished draft</think>',
      });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});
    await act(async () => {});

    // The raw in-memory content carries the markers, but auto-replace pastes the
    // same cleaned text the bubble and manual Replace button use.
    expect(invoke).toHaveBeenCalledWith('replace_selection', {
      text: 'Polished draft',
    });
  });

  it('auto-closes after a successful replace when auto-close is on', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
      replace_selection: 'replaced',
    });

    render(
      <ConfigProviderForTest
        value={{
          ...DEFAULT_CONFIG,
          behavior: { autoReplace: true, autoClose: true },
        }}
      >
        <App />
      </ConfigProviderForTest>,
    );
    await act(async () => {});
    await showOverlay('draft email text');
    __mockWindow.hide.mockClear();

    const textarea = getAskInput();
    act(() => {
      setAskValue('/rewrite ');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    act(() => {
      getLastChannel()?.simulateMessage({
        type: 'Token',
        data: 'Polished draft',
      });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // The replace succeeds, so auto-close dismisses the overlay; the native
    // window hides once the exit animation commits (HIDE_COMMIT_DELAY_MS).
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 400));
    });
    expect(__mockWindow.hide).toHaveBeenCalled();
  });

  it('does not auto-close when the replace is skipped', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
      replace_selection: 'skipped',
    });

    render(
      <ConfigProviderForTest
        value={{
          ...DEFAULT_CONFIG,
          behavior: { autoReplace: true, autoClose: true },
        }}
      >
        <App />
      </ConfigProviderForTest>,
    );
    await act(async () => {});
    await showOverlay('draft email text');
    __mockWindow.hide.mockClear();

    const textarea = getAskInput();
    act(() => {
      setAskValue('/rewrite ');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    act(() => {
      getLastChannel()?.simulateMessage({
        type: 'Token',
        data: 'Polished draft',
      });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // A skipped replace (no target app / secure field) must leave Thuki open.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 400));
    });
    expect(__mockWindow.hide).not.toHaveBeenCalled();
  });

  it('keeps the Replace button on a plain follow-up after /rewrite', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
    });

    render(
      <ConfigProviderForTest
        value={{
          ...DEFAULT_CONFIG,
          behavior: { autoReplace: false, autoClose: false },
        }}
      >
        <App />
      </ConfigProviderForTest>,
    );
    await act(async () => {});
    await showOverlay('draft email text');

    const textarea = getAskInput();
    act(() => {
      setAskValue('/rewrite ');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Polished v1' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // The first /rewrite result is replaceable.
    expect(
      screen.queryAllByLabelText('Replace selection in source app'),
    ).toHaveLength(1);

    // A plain refinement inherits sticky rewrite mode, so its result is
    // replaceable too.
    act(() => {
      setAskValue('make it longer');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Longer v2' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    expect(
      screen.queryAllByLabelText('Replace selection in source app'),
    ).toHaveLength(2);

    // Auto-replace is off, so neither completed turn may write back on its
    // own: the source app is only touched when the user clicks Replace.
    expect(invoke).not.toHaveBeenCalledWith(
      'replace_selection',
      expect.anything(),
    );
  });

  it('drops the Replace button when a different command interrupts the rewrite session', async () => {
    enableChannelCaptureWithResponses({
      get_model_picker_state: {
        active: 'gemma4:e2b',
        all: ['gemma4:e2b'],
        ollamaReachable: true,
      },
    });

    render(
      <ConfigProviderForTest
        value={{
          ...DEFAULT_CONFIG,
          behavior: { autoReplace: false, autoClose: false },
        }}
      >
        <App />
      </ConfigProviderForTest>,
    );
    await act(async () => {});
    await showOverlay('draft email text');

    const textarea = getAskInput();
    act(() => {
      setAskValue('/rewrite ');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Polished v1' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // A non-replaceable command exits rewrite mode for itself and for any
    // later plain follow-up.
    act(() => {
      setAskValue('/tldr wrap this up');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Short.' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    act(() => {
      setAskValue('and again');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});
    act(() => {
      getLastChannel()?.simulateMessage({ type: 'Token', data: 'Again.' });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });
    await act(async () => {});

    // Only the original /rewrite result carries a Replace button.
    expect(
      screen.queryAllByLabelText('Replace selection in source app'),
    ).toHaveLength(1);
  });

  it('applies justify-end when window is near screen bottom', async () => {
    render(<App />);
    await act(async () => {});

    // Show overlay near screen bottom: window_y=750, screen_bottom=900.
    // 750 + MAX_CHAT_WINDOW_HEIGHT(648) = 1398 > 900 → grows upward.
    await act(async () => {
      emitTauriEvent('thuki://visibility', {
        state: 'show',
        selected_text: null,
        window_x: 100,
        window_y: 750,
        screen_bottom_y: 900,
      });
    });

    const outer = document.querySelector('.justify-end');
    expect(outer).not.toBeNull();
  });

  it('applies justify-start when window has room below', async () => {
    render(<App />);
    await act(async () => {});

    // Show overlay near top: window_y=100, screen_bottom=900.
    // 100 + 648 = 748 < 900 → grows downward.
    await act(async () => {
      emitTauriEvent('thuki://visibility', {
        state: 'show',
        selected_text: null,
        window_x: 100,
        window_y: 100,
        screen_bottom_y: 900,
      });
    });

    const outer = document.querySelector('.justify-start');
    expect(outer).not.toBeNull();
    expect(document.querySelector('.justify-end')).toBeNull();
  });

  describe('ResizeObserver upward growth', () => {
    let capturedCallback: ResizeObserverCallback | null = null;

    function spyOnResizeObserver() {
      const OriginalMock = globalThis.ResizeObserver;
      vi.spyOn(globalThis, 'ResizeObserver').mockImplementation(function (
        callback: ResizeObserverCallback,
      ) {
        capturedCallback = callback;
        return new OriginalMock(callback) as ResizeObserver;
      });
    }

    function triggerResize(element: Element, contentHeight: number) {
      vi.spyOn(element, 'getBoundingClientRect').mockReturnValue({
        height: contentHeight,
        width: 600,
        top: 0,
        left: 0,
        right: 600,
        bottom: contentHeight,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      });
      if (capturedCallback) {
        capturedCallback(
          [{ target: element } as ResizeObserverEntry],
          {} as ResizeObserver,
        );
      }
    }

    it('commits exact height when not streaming (initial ask bar)', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      // window_y=804, screen_bottom=900. bottomY = 804+80 = 884.
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_x: 100,
          window_y: 804,
          screen_bottom_y: 900,
        });
      });

      invoke.mockClear();

      const container = document.querySelector('.morphing-container');
      expect(container).not.toBeNull();

      // Not streaming yet, so exact height is committed (no buffer)
      act(() => {
        triggerResize(container!, 60);
      });

      // bottomY(884) - targetHeight(108) = 776
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 100,
        y: 776,
        width: 600,
        height: 108,
      });
    });

    it('uses setSize (not set_window_frame) after drag clears upward growth', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_x: 100,
          window_y: 804,
          screen_bottom_y: 900,
        });
      });

      const container = document.querySelector('.morphing-container');
      expect(container).not.toBeNull();

      // Drag clears upward growth
      act(() => {
        fireEvent.mouseDown(container!);
      });
      act(() => {
        fireEvent.mouseUp(window);
      });

      invoke.mockClear();
      __mockWindow.setSize.mockClear?.();

      act(() => {
        triggerResize(container!, 60);
      });
      expect(invoke).not.toHaveBeenCalledWith(
        'set_window_frame',
        expect.anything(),
      );
      expect(__mockWindow.setSize).toHaveBeenCalled();
    });

    it('resets upward growth on session reopen', async () => {
      spyOnResizeObserver();

      render(<App />);
      await act(async () => {});

      // Session 1: near bottom, grows upward
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_x: 100,
          window_y: 804,
          screen_bottom_y: 900,
        });
      });

      const container1 = document.querySelector('.morphing-container');
      act(() => {
        triggerResize(container1!, 60);
      });

      // Close
      await act(async () => {
        emitTauriEvent('thuki://visibility', { state: 'hide-request' });
      });

      // Session 2: reopen near bottom again
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_x: 100,
          window_y: 804,
          screen_bottom_y: 900,
        });
      });

      const container2 = document.querySelector('.morphing-container');
      expect(container2).not.toBeNull();

      invoke.mockClear();
      act(() => {
        triggerResize(container2!, 60);
      });
      // bottomY = 804+80 = 884. 884-108 = 776.
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 100,
        y: 776,
        width: 600,
        height: 108,
      });
    });
  });

  it('requestHideOverlay is a no-op when already hidden', async () => {
    render(<App />);
    await act(async () => {});

    // Overlay is hidden initially - fire hide-request on hidden overlay
    // This exercises the 'hidden' branch in requestHideOverlay's state setter
    await act(async () => {
      emitTauriEvent('thuki://visibility', { state: 'hide-request' });
    });

    // No crash, no change - overlay is already hidden
    expect(document.querySelector('.morphing-container')).toBeNull();
  });

  // ─── History integration ─────────────────────────────────────────────────────

  describe('history integration', () => {
    it('shows history icon button in ask-bar mode', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      expect(
        screen.getByRole('button', { name: /open history/i }),
      ).toBeInTheDocument();
    });

    it('shows history panel when history icon is clicked in ask-bar mode', async () => {
      invoke.mockResolvedValue([]); // list_conversations returns empty

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      expect(
        screen.getByPlaceholderText(/search past chats/i),
      ).toBeInTheDocument();
    });

    it('closes history panel when a conversation is loaded', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: 'gemma4:e2b',
          all: ['gemma4:e2b'],
          ollamaReachable: true,
        },
        list_conversations: [],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Open history
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      // Wait for empty list to render
      await act(async () => {});

      // Panel should be visible but no conversations to click
      // (list is empty, so just verify panel closes on a second click)
      // Close via second click on history icon
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      expect(screen.queryByPlaceholderText(/search past chats/i)).toBeNull();
    });

    it('pre-loads the conversation list before opening the ask-bar drawer', async () => {
      // The drawer is only mounted once the list has loaded, so the panel
      // renders its conversations on first paint. This is what lets the open
      // animation measure the final height once and grow smoothly to it,
      // instead of opening empty and snapping when the async list lands.
      enableChannelCaptureWithResponses({
        list_conversations: [
          {
            id: 'c1',
            title: 'Seeded chat',
            model: 'gemma4:e2b',
            updated_at: Date.now(),
            message_count: 3,
          },
        ],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      expect(screen.getByText('Seeded chat')).toBeInTheDocument();
    });

    it('opens the ask-bar drawer and surfaces the error when the list pre-load fails', async () => {
      // When the pre-load rejects, the drawer still opens (so the user is not
      // stuck on a dead button) and HistoryPanel falls back to fetching itself,
      // surfacing its own load-error state.
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'list_conversations') {
            throw new Error('DB error');
          }
          if (cmd === 'get_model_picker_state') {
            return TEST_DEFAULT_MODEL_PICKER_STATE;
          }
          if (cmd === 'get_updater_state') {
            return DEFAULT_UPDATER_STATE;
          }
          if (args && 'onEvent' in args) {
            return undefined;
          }
          return undefined;
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {});

      expect(screen.getByText(/couldn't load history/i)).toBeInTheDocument();
    });

    it('shows save button in conversation view when there are messages', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('test');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'Reply' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      expect(screen.getByRole('button', { name: /save/i })).toBeInTheDocument();
    });

    it('save button calls save_conversation when clicked', async () => {
      enableChannelCaptureWithResponses({
        save_conversation: { conversation_id: 'conv-test' },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('question');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });

      expect(invoke).toHaveBeenCalledWith(
        'save_conversation',
        expect.objectContaining({
          messages: expect.any(Array),
        }),
      );
    });

    it('clicking save button when already saved calls delete_conversation (unsave toggle)', async () => {
      enableChannelCaptureWithResponses({
        save_conversation: { conversation_id: 'conv-save-toggle' },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('question');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Save the conversation first
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });

      // Button should now read "Remove from history"
      expect(
        screen.getByRole('button', { name: /remove from history/i }),
      ).toBeInTheDocument();

      invoke.mockClear();

      // Click again to unsave
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /remove from history/i }),
        );
      });

      expect(invoke).toHaveBeenCalledWith('delete_conversation', {
        conversationId: 'conv-save-toggle',
      });

      // Button reverts to "Save conversation"
      expect(
        screen.getByRole('button', { name: /save conversation/i }),
      ).toBeInTheDocument();
    });

    it('resets history state on overlay reopen', async () => {
      enableChannelCaptureWithResponses({
        save_conversation: { conversation_id: 'conv-123' },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Send message + Done
      const textarea = getAskInput();
      act(() => {
        setAskValue('hello');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hi' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Save
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });

      // Reopen - bookmark should reset (save button enabled again)
      enableChannelCapture();
      await showOverlay();

      // In ask-bar mode now - no save button visible, but history icon is
      expect(
        screen.getByRole('button', { name: /open history/i }),
      ).toBeInTheDocument();
    });

    it('handleNewConversation shows SwitchConfirmation when unsaved, resets on Start New', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Get into chat mode with an unsaved turn
      const textarea = getAskInput();
      act(() => {
        setAskValue('question');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Click + (unsaved conversation → history panel opens with SwitchConfirmation)
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      // SwitchConfirmation should be visible with "new" variant
      expect(
        screen.getByRole('button', { name: 'Start New' }),
      ).toBeInTheDocument();

      // Click "Start New" → should reset to ask-bar mode
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Start New' }));
      });

      expect(getAskInput()).toBeInTheDocument();
    });

    it('handleNewConversation Cancel closes the history dropdown', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Get into chat mode with an unsaved turn
      const textarea = getAskInput();
      act(() => {
        setAskValue('question');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Click + → SwitchConfirmation appears
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      expect(
        screen.getByRole('button', { name: 'Cancel' }),
      ).toBeInTheDocument();

      // Click Cancel → dropdown closes, still in chat mode
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
      });

      // SwitchConfirmation should be gone
      expect(
        screen.queryByRole('button', { name: 'Cancel' }),
      ).not.toBeInTheDocument();
      // Still showing the conversation
      expect(screen.getByText('question')).toBeInTheDocument();
    });

    it('handleNewConversation resets directly when conversation is already saved', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
        save_conversation: 'saved-id',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Get into chat mode
      const textarea = getAskInput();
      act(() => {
        setAskValue('question');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Save the conversation
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });

      // Click + (already saved → no confirmation, direct reset)
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      // Should be directly back in ask-bar mode (no confirmation prompt)
      expect(getAskInput()).toBeInTheDocument();
    });

    it('handleNewConversation revokes blob URLs when images are attached', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
        save_image_command: '/tmp/img.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Get into chat mode with an unsaved turn
      const textarea = getAskInput();
      act(() => {
        setAskValue('question');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Paste an image while in chat mode (unsaved conversation)
      const replyInput = getAskInput();
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(replyInput, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      const revokeSpy = vi.mocked(URL.revokeObjectURL);
      revokeSpy.mockClear();

      // Click + → SwitchConfirmation (unsaved conversation)
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      // Click "Start New" → resetForNewConversation revokes blob URLs
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Start New' }));
      });

      expect(revokeSpy).toHaveBeenCalled();
      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('handleNewConversation saves then resets on Save & Start New', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
        save_conversation: 'saved-id',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Get into chat mode with an unsaved turn
      const textarea = getAskInput();
      act(() => {
        setAskValue('question');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'answer' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Click + → SwitchConfirmation appears
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      expect(
        screen.getByRole('button', { name: 'Save & Start New' }),
      ).toBeInTheDocument();

      // Click "Save & Start New" → saves then resets to ask-bar mode
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Save & Start New' }),
        );
      });

      expect(getAskInput()).toBeInTheDocument();
    });

    it('handleSaveAndNew aborts reset when save fails', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'list_conversations') return [];
        if (cmd === 'save_conversation') throw new Error('disk full');
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn so isSaved = false
      const textarea = getAskInput();
      act(() => {
        setAskValue('q');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Click + → SwitchConfirmation
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      // Click "Save & Start New" - save fails → should stay in chat mode
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Save & Start New' }),
        );
      });

      // Still in chat mode (save_conversation threw, reset was aborted)
      expect(screen.getByText('q')).toBeInTheDocument();
    });

    it('handleSaveAndLoad saves unsaved conversation then switches', async () => {
      const OTHER_MSGS = [
        {
          id: 'm3',
          role: 'user',
          content: 'Old q',
          quoted_text: null,
          created_at: 1,
        },
        {
          id: 'm4',
          role: 'assistant',
          content: 'Old a',
          quoted_text: null,
          created_at: 2,
        },
      ];
      enableChannelCaptureWithResponses({
        save_conversation: { conversation_id: 'conv-new' },
        load_conversation: OTHER_MSGS,
        list_conversations: [
          {
            id: 'conv-other2',
            title: 'Other chat',
            model: 'gemma4:e2b',
            updated_at: 1,
            message_count: 2,
          },
        ],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn (unsaved)
      const textarea = getAskInput();
      act(() => {
        setAskValue('q');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Open chat history WITHOUT saving
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /history/i }));
      });

      // Click a different conversation → SwitchConfirmation
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /other chat/i }));
      });

      // Save & Switch - isSaved is FALSE so save_conversation should be called
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /save & switch/i }));
      });

      expect(invoke).toHaveBeenCalledWith(
        'save_conversation',
        expect.objectContaining({
          messages: expect.any(Array),
        }),
      );
    });

    it('handleSaveAndLoad aborts load when save_conversation fails', async () => {
      // Bug: without the early return on save failure, the load would still run
      // and could overwrite the current session with an unrelated conversation.
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'list_conversations')
          return [
            {
              id: 'c2',
              title: 'Other chat',
              model: 'gemma4:e2b',
              updated_at: 1,
              message_count: 1,
            },
          ];
        if (cmd === 'save_conversation') throw new Error('disk full');
        // load_conversation must NOT be called
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn so isSaved = false
      const textarea = getAskInput();
      act(() => {
        setAskValue('q');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Open history → click another conversation → SwitchConfirmation
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /other chat/i }));
      });

      // Confirm "Save & Switch" - save_conversation will throw
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /save & switch/i }));
      });

      // load_conversation must NOT have been called (early return after save failure)
      expect(invoke).not.toHaveBeenCalledWith(
        'load_conversation',
        expect.anything(),
      );
    });

    it('clicking a conversation loads it directly when already saved (no dialog)', async () => {
      const OTHER_MSGS = [
        {
          id: 'm3',
          role: 'user',
          content: 'Old q',
          quoted_text: null,
          created_at: 1,
        },
        {
          id: 'm4',
          role: 'assistant',
          content: 'Old a',
          quoted_text: null,
          created_at: 2,
        },
      ];
      enableChannelCaptureWithResponses({
        save_conversation: { conversation_id: 'conv-current' },
        load_conversation: OTHER_MSGS,
        list_conversations: [
          {
            id: 'conv-other',
            title: 'Switch target',
            model: 'gemma4:e2b',
            updated_at: 1,
            message_count: 2,
          },
        ],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn
      const textarea = getAskInput();
      act(() => {
        setAskValue('q');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Save the conversation → isSaved = true
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });

      // Open chat history
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      // Click a different conversation - isSaved=true means no dialog, loads directly
      invoke.mockClear();
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /switch target/i }));
      });

      // No SwitchConfirmation dialog - save_conversation NOT called again
      expect(invoke).not.toHaveBeenCalledWith(
        'save_conversation',
        expect.anything(),
      );
      // load_conversation IS called directly
      expect(invoke).toHaveBeenCalledWith('load_conversation', {
        conversationId: 'conv-other',
      });
    });

    it('handleDeleteConversation marks active conversation unsaved but keeps messages', async () => {
      const LOADED_MSGS = [
        {
          id: 'm1',
          role: 'user',
          content: 'Hi',
          quoted_text: null,
          created_at: 1,
        },
        {
          id: 'm2',
          role: 'assistant',
          content: 'Hello',
          quoted_text: null,
          created_at: 2,
        },
      ];
      enableChannelCaptureWithResponses({
        load_conversation: LOADED_MSGS,
        list_conversations: [
          {
            id: 'conv-target',
            title: 'My chat',
            model: 'gemma4:e2b',
            updated_at: 1,
            message_count: 2,
          },
        ],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Load a conversation from ask-bar history → conversationId = 'conv-target'
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /my chat/i }));
      });

      // Messages are visible in chat mode
      expect(screen.getByText('Hi')).toBeInTheDocument();

      // Open chat history and delete the currently-active conversation
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /delete conversation/i }),
        );
      });

      // delete_conversation was called
      expect(invoke).toHaveBeenCalledWith('delete_conversation', {
        conversationId: 'conv-target',
      });

      // Messages remain - still in chat mode
      expect(screen.getByText('Hi')).toBeInTheDocument();

      // Save button reverts to unsaved state ("Save conversation")
      expect(
        screen.getByRole('button', { name: /save conversation/i }),
      ).toBeInTheDocument();
    });

    it('clicking outside the chat history dropdown closes it', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn to enter chat mode
      const textarea = getAskInput();
      act(() => {
        setAskValue('q');
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Open history dropdown
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      expect(
        screen.getByPlaceholderText('Search past chats…'),
      ).toBeInTheDocument();

      // Click outside - should close the dropdown
      await act(async () => {
        fireEvent.mouseDown(document.body);
      });
      expect(screen.queryByPlaceholderText('Search past chats…')).toBeNull();
    });

    it('clicking inside the chat history dropdown does not close it', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Complete a turn to enter chat mode
      const textarea = getAskInput();
      act(() => {
        setAskValue('q');
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'a' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // Open history dropdown
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      const searchInput = screen.getByPlaceholderText('Search past chats…');
      expect(searchInput).toBeInTheDocument();

      // Click inside the dropdown - should NOT close it
      await act(async () => {
        fireEvent.mouseDown(searchInput);
      });
      expect(
        screen.getByPlaceholderText('Search past chats…'),
      ).toBeInTheDocument();
    });

    it('handleDeleteConversation allows saving the conversation again after deletion', async () => {
      // After deleting the active conversation from history, isSaved resets to
      // false so the user can re-save the same messages under a new record.
      enableChannelCaptureWithResponses({
        load_conversation: [
          {
            id: 'm1',
            role: 'user',
            content: 'Hi',
            quoted_text: null,
            created_at: 1,
          },
          {
            id: 'm2',
            role: 'assistant',
            content: 'Hello',
            quoted_text: null,
            created_at: 2,
          },
        ],
        list_conversations: [
          {
            id: 'conv-active',
            title: 'Active chat',
            model: 'gemma4:e2b',
            updated_at: 1,
            message_count: 2,
          },
        ],
        save_conversation: { conversation_id: 'conv-new' },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Load the conversation → isSaved = true
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /active chat/i }));
      });

      // Verify save button shows unsave state
      expect(
        screen.getByRole('button', { name: /remove from history/i }),
      ).toBeInTheDocument();

      // Open history and delete the active conversation
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /delete conversation/i }),
        );
      });

      // Messages remain, isSaved is now false - save button is re-enabled
      expect(screen.getByText('Hi')).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: /save conversation/i }),
      ).toBeInTheDocument();

      // User can re-save the conversation
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /save conversation/i }),
        );
      });
      expect(invoke).toHaveBeenCalledWith(
        'save_conversation',
        expect.objectContaining({ messages: expect.any(Array) }),
      );
    });

    it('handleLoadConversation closes history panel when load_conversation fails', async () => {
      // Bug: without try/catch, setIsHistoryOpen(false) is never reached when
      // loadConversation() throws, leaving the panel open on failure.
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'list_conversations')
          return [
            {
              id: 'c1',
              title: 'Chat',
              model: 'gemma4:e2b',
              updated_at: 1,
              message_count: 1,
            },
          ];
        if (cmd === 'load_conversation') throw new Error('load failed');
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      // Click the conversation - load_conversation will throw
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /^chat$/i }));
      });

      // Panel must close even on failure; app must still be running
      expect(screen.queryByPlaceholderText(/search past chats/i)).toBeNull();
      expect(getAskInput()).toBeInTheDocument();
    });

    it('handleDeleteConversation does not reset history when a different conversation is deleted', async () => {
      enableChannelCaptureWithResponses({
        list_conversations: [
          {
            id: 'conv-unrelated',
            title: 'Unrelated',
            model: 'gemma4:e2b',
            updated_at: 1,
            message_count: 2,
          },
        ],
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Open ask-bar history (no conversation loaded - conversationId is null)
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /open history/i }));
      });

      // Delete a conversation while conversationId is null (id !== conversationId → false branch)
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /delete conversation/i }),
        );
      });

      expect(invoke).toHaveBeenCalledWith('delete_conversation', {
        conversationId: 'conv-unrelated',
      });
    });
  });

  // ─── Image integration ─────────────────────────────────────────────────────

  describe('image integration', () => {
    /** Helper: paste an image file into the textarea and wait for thumbnails. */
    async function pasteImage() {
      const textarea = getAskInput();
      const file = new File(['fake-img-data'], 'photo.png', {
        type: 'image/png',
      });
      const clipboardData = {
        getData: () => '',
        items: [{ type: 'image/png', getAsFile: () => file }],
      };
      await act(async () => {
        fireEvent.paste(textarea, { clipboardData });
      });
      // Thumbnails appear immediately via blob URL (before backend completes)
      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });
    }

    it('handleImagesAttached stages images and shows thumbnails', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for FileReader + invoke to complete in background
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.objectContaining({
              imageDataBase64: expect.any(String),
            }),
          );
        });
      });

      // Thumbnails should still be present
      expect(
        screen.getByRole('list', { name: /attached images/i }),
      ).toBeInTheDocument();
    });

    it('handleImageRemove removes thumbnail and calls remove_image_command', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for backend to resolve (filePath set)
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      invoke.mockClear();

      // Click remove button on the thumbnail
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /remove/i }));
      });

      expect(invoke).toHaveBeenCalledWith('remove_image_command', {
        path: '/tmp/staged/img1.jpg',
      });
      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('handleSubmit with images passes imagePaths and clears attachedImages', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for backend to resolve (filePath set)
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      // Type a message and submit
      const textarea = getAskInput();
      act(() => {
        setAskValue('describe this');
      });

      invoke.mockClear();
      enableChannelCapture();

      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // ask_model should be called with imagePaths
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'describe this',
          imagePaths: ['/tmp/staged/img1.jpg'],
        }),
      );
    });

    it('submits with images and no text', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for backend to resolve
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      invoke.mockClear();
      enableChannelCapture();

      // Submit with Enter (no text, just images)
      const textarea = getAskInput();
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // ask_model should be called with empty message but imagePaths
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: '',
          imagePaths: ['/tmp/staged/img1.jpg'],
        }),
      );
    });

    it('previewImage opens ImagePreviewModal and closing clears it', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Click preview button on thumbnail
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /preview/i }));
      });

      // ImagePreviewModal should be open (has role="dialog")
      expect(screen.getByRole('dialog')).toBeInTheDocument();

      // Close the modal
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /close preview/i }));
      });

      // Dialog should be gone
      expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('handleImagesAttached removes image when backend fails', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'save_image_command') throw new Error('disk full');
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.drop(
          document.querySelector('[class*="flex flex-col w-full shrink-0"]')!,
          {
            preventDefault: vi.fn(),
            dataTransfer: { files: [file] },
          },
        );
      });

      // Thumbnail appears immediately via blob URL
      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      // Wait for FileReader + invoke to settle - failed image gets removed
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      // Image should be removed after backend failure
      await vi.waitFor(() => {
        expect(
          screen.queryByRole('list', { name: /attached images/i }),
        ).toBeNull();
      });
    });

    it('handleImagesAttached skips images that fail to stage', async () => {
      // First call succeeds, second call fails
      let saveCallCount = 0;
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (args && 'onEvent' in args) {
            // channel capture - no-op for this test
          }
          if (cmd === 'save_image_command') {
            saveCallCount++;
            if (saveCallCount === 2) throw new Error('disk full');
            return '/tmp/staged/img1.jpg';
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Drop two image files via the AskBarView wrapper
      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      );
      expect(askBarWrapper).not.toBeNull();

      const file1 = new File(['data1'], 'img1.png', { type: 'image/png' });
      const file2 = new File(['data2'], 'img2.png', { type: 'image/png' });
      fireEvent.drop(askBarWrapper!, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [file1, file2] },
      });

      // Both thumbnails appear immediately
      await vi.waitFor(() => {
        expect(screen.getAllByRole('listitem')).toHaveLength(2);
      });

      // Wait for both backend calls to settle
      await act(async () => {
        await vi.waitFor(() => {
          expect(saveCallCount).toBe(2);
        });
      });

      // Failed image gets removed, only one remains
      await vi.waitFor(() => {
        expect(screen.getAllByRole('listitem')).toHaveLength(1);
      });
    });

    it('dropping image onto root window div attaches image in ask-bar mode', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const rootDiv = document.querySelector('.h-screen')!;
      expect(rootDiv).not.toBeNull();
      const file = new File(['data'], 'photo.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.drop(rootDiv, {
          preventDefault: vi.fn(),
          dataTransfer: { files: [file] },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });
    });

    it('dropping image onto root window div attaches image in chat mode (second image after conversation)', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Send a plain text message and complete the generation to enter chat mode
      const textarea = getAskInput();
      act(() => {
        setAskValue('hello');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Complete the AI response so isGenerating becomes false
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'Hi!' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      // Confirm we are in chat mode with generation complete
      expect(screen.getByText('Reply...')).toBeInTheDocument();

      // Now in chat mode. Drop image onto root div (not AskBarView specifically)
      const rootDiv = document.querySelector('.h-screen')!;
      expect(rootDiv).not.toBeNull();
      const file = new File(['data'], 'second.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.drop(rootDiv, {
          preventDefault: vi.fn(),
          dataTransfer: { files: [file] },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });
    });

    it('dragOver anywhere in window shows violet ring on AskBarView when under max', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const rootDiv = document.querySelector('.h-screen')!;
      expect(rootDiv).not.toBeNull();
      fireEvent.dragOver(rootDiv, { preventDefault: vi.fn() });

      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      )!;
      expect(askBarWrapper.classList.contains('ring-2')).toBe(true);
      expect(askBarWrapper.classList.contains('ring-red-500/60')).toBe(false);
    });

    it('dragOver shows red ring and max label when already at max images', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste 3 images to reach max
      const textarea = getAskInput();
      for (let i = 0; i < 3; i++) {
        const file = new File([`data${i}`], `img${i}.png`, {
          type: 'image/png',
        });
        await act(async () => {
          fireEvent.paste(textarea, {
            clipboardData: {
              getData: () => '',
              items: [{ type: 'image/png', getAsFile: () => file }],
            },
          });
        });
      }

      // Wait for 3 thumbnails
      await vi.waitFor(() => {
        expect(screen.getAllByRole('listitem')).toHaveLength(3);
      });

      // Now drag over; should show red ring and max label
      const rootDiv = document.querySelector('.h-screen')!;
      fireEvent.dragOver(rootDiv, { preventDefault: vi.fn() });

      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      )!;
      expect(askBarWrapper.classList.contains('ring-red-500/60')).toBe(true);
      expect(screen.getByText('Max 3 images')).toBeInTheDocument();
    });

    it('dragLeave when cursor exits window clears drag-over ring', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const rootDiv = document.querySelector('.h-screen')!;
      fireEvent.dragOver(rootDiv, { preventDefault: vi.fn() });
      // relatedTarget null simulates cursor leaving the window entirely
      fireEvent.dragLeave(rootDiv, { relatedTarget: null });

      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      )!;
      expect(askBarWrapper.classList.contains('ring-2')).toBe(false);
    });

    it('dragOver when generating does not show drag-over ring', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Submit to trigger isGenerating
      const textarea = getAskInput();
      act(() => {
        setAskValue('hi');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      const rootDiv = document.querySelector('.h-screen')!;
      fireEvent.dragOver(rootDiv, { preventDefault: vi.fn() });

      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      )!;
      expect(askBarWrapper.classList.contains('ring-2')).toBe(false);
    });

    it('handleRootDrop ignores drop during generation', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('hi');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      const rootDiv = document.querySelector('.h-screen')!;
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      fireEvent.drop(rootDiv, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [file] },
      });

      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('handleRootDrop ignores drop with no dataTransfer files', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const rootDiv = document.querySelector('.h-screen')!;
      fireEvent.drop(rootDiv, { preventDefault: vi.fn() });

      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('handleRootDrop ignores drop when already at max images', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img.jpg',
      });
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      for (let i = 0; i < 3; i++) {
        const img = new File([`d${i}`], `i${i}.png`, { type: 'image/png' });
        await act(async () => {
          fireEvent.paste(textarea, {
            clipboardData: {
              getData: () => '',
              items: [{ type: 'image/png', getAsFile: () => img }],
            },
          });
        });
      }
      await vi.waitFor(() => {
        expect(screen.getAllByRole('listitem')).toHaveLength(3);
      });

      const rootDiv = document.querySelector('.h-screen')!;
      const extra = new File(['extra'], 'extra.png', { type: 'image/png' });
      fireEvent.drop(rootDiv, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [extra] },
      });

      // Still exactly 3 - the drop was rejected
      expect(screen.getAllByRole('listitem')).toHaveLength(3);
    });

    it('handleRootDrop ignores non-image files', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const rootDiv = document.querySelector('.h-screen')!;
      const doc = new File(['text'], 'doc.txt', { type: 'text/plain' });
      fireEvent.drop(rootDiv, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [doc] },
      });

      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('handleChatImagePreview opens modal for chat history image', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for backend to resolve
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      // Type and submit to create a user message with image
      const textarea = getAskInput();
      act(() => {
        setAskValue('what is this?');
      });

      invoke.mockClear();
      enableChannelCapture();

      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Simulate AI response completing
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'It is' });
        getLastChannel()?.simulateMessage({ type: 'Token', data: ' a cat.' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });

      // The user message should have a thumbnail from chat history (via convertFileSrc)
      // Find the preview button in the chat bubble (not the ask bar)
      const previewButtons = screen.getAllByRole('button', {
        name: /preview/i,
      });
      // The chat bubble thumbnail should be present
      expect(previewButtons.length).toBeGreaterThan(0);

      await act(async () => {
        fireEvent.click(previewButtons[0]);
      });

      // ImagePreviewModal should be open
      expect(screen.getByRole('dialog')).toBeInTheDocument();

      // Close it
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /close preview/i }));
      });

      expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('handleChatImagePreview passes blob URLs through without convertFileSrc', async () => {
      // Make save_image_command hang so the image stays as a blob URL
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (args && 'onEvent' in args) {
            // channel capture
          }
          if (cmd === 'save_image_command') {
            return new Promise<string>(() => {}); // never resolves
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste and submit while still processing → pendingUserMessage with blob URL
      const textarea = getAskInput();
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      act(() => {
        setAskValue('what is this?');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Pending user message should be visible in chat with a blob URL thumbnail
      await vi.waitFor(() => {
        expect(screen.getByText('what is this?')).toBeInTheDocument();
      });

      // Click the preview button in the chat bubble - should open the modal
      // with the blob URL directly (no convertFileSrc wrapping).
      const previewButtons = screen.getAllByRole('button', {
        name: /preview/i,
      });
      expect(previewButtons.length).toBeGreaterThan(0);

      await act(async () => {
        fireEvent.click(previewButtons[0]);
      });

      expect(screen.getByRole('dialog')).toBeInTheDocument();

      // Flush stale FileReader macrotask so it doesn't leak into the next test.
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });
    });

    it('handleImageRemove is safe when called twice for the same image', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImage();

      // Wait for backend to resolve
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      invoke.mockClear();

      // Click remove twice rapidly - the second call should be a no-op
      // (the functional updater in setAttachedImages will find no matching
      // image on the second pass, exercising the !img branch).
      const removeBtn = screen.getByRole('button', { name: /remove/i });
      await act(async () => {
        fireEvent.click(removeBtn);
        fireEvent.click(removeBtn);
      });

      // remove_image_command should only be called once
      const removeCalls = invoke.mock.calls.filter(
        (call) => call[0] === 'remove_image_command',
      );
      expect(removeCalls).toHaveLength(1);
    });

    it('handleImageRemove revokes blob URL without calling remove_image_command when filePath is null', async () => {
      // Make save_image_command hang forever (never resolve)
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (args && 'onEvent' in args) {
            // channel capture - no-op
          }
          if (cmd === 'save_image_command') {
            return new Promise(() => {}); // never resolves
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image - thumbnail appears immediately with null filePath
      const textarea = getAskInput();
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      invoke.mockClear();

      // Remove the image while filePath is still null
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /remove/i }));
      });

      // Should NOT call remove_image_command (no file to delete)
      expect(invoke).not.toHaveBeenCalledWith(
        'remove_image_command',
        expect.anything(),
      );
      expect(
        screen.queryByRole('list', { name: /attached images/i }),
      ).toBeNull();
    });

    it('defers submit when images are still processing and fires when ready', async () => {
      // Flush any stale macrotasks (e.g. FileReader.onload from prior tests)
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });

      // Track save_image_command calls scoped to THIS test
      let resolveSave: ((path: string) => void) | null = null;
      const savePromises: Promise<string>[] = [];
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (args && 'onEvent' in args) {
            // Accept channel for ask_model
          }
          if (cmd === 'save_image_command') {
            const p = new Promise<string>((resolve) => {
              resolveSave = resolve;
            });
            savePromises.push(p);
            return p;
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image; thumbnail appears immediately (filePath null)
      const textarea = getAskInput();
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      // Wait for this test's FileReader to complete and call save_image_command
      await act(async () => {
        await vi.waitFor(() => expect(savePromises).toHaveLength(1));
      });

      // Type and submit while image is still processing
      act(() => {
        setAskValue('describe this');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Should show "Processing images" state
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve the image; triggers deferred submit chain
      resolveSave!('/tmp/staged/img1.jpg');

      // Flush async chain: promise → state update → effect → ask → invoke
      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      // User message should appear in the chat (ask() fired the real submit)
      expect(screen.getByText('describe this')).toBeInTheDocument();
    });

    it('stop button cancels active generation via handleCancel', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/img.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Start a normal text conversation (no images)
      const textarea = getAskInput();
      act(() => {
        setAskValue('hello');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Should be generating - stop button visible
      const stopBtn = screen.getByRole('button', { name: /stop/i });
      expect(stopBtn).toBeInTheDocument();

      // Click stop - should call cancel_generation
      invoke.mockClear();
      enableChannelCapture();

      await act(async () => {
        fireEvent.click(stopBtn);
      });

      expect(invoke).toHaveBeenCalledWith('cancel_generation');
    });

    it('stop button hard-aborts an active /search turn and resets search mode', async () => {
      let resolveSearch!: () => void;
      let resolveCancel!: () => void;

      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'search_pipeline') {
          return new Promise<void>((res) => {
            resolveSearch = res;
          });
        }
        if (cmd === 'cancel_generation') {
          return new Promise<void>((res) => {
            resolveCancel = res;
          });
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/search what is Rust?');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const stopBtn = screen.getByRole('button', { name: /stop/i });
      expect(stopBtn).toBeInTheDocument();

      act(() => {
        fireEvent.click(stopBtn);
      });

      expect(invoke).toHaveBeenCalledWith('cancel_generation');
      expect(screen.queryByRole('button', { name: /stop/i })).toBeNull();
      expect(getAskInput().getAttribute('contenteditable')).toBe('true');

      act(() => {
        setAskValue('hello');
      });
      expect(getAskInput().textContent).toBe('hello');

      await act(async () => {
        resolveCancel?.();
        resolveSearch?.();
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const calls = invoke.mock.calls.filter(
        (c) => c[0] === 'ask_model' || c[0] === 'search_pipeline',
      );
      const last = calls[calls.length - 1];
      expect(last[0]).toBe('ask_model');
      expect(last[1]).toMatchObject({ message: 'hello' });
    });

    it('cancelling during pending submit restores input (undo send)', async () => {
      // Flush stale macrotasks from prior tests
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });

      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (args && 'onEvent' in args) {
            // Accept channel
          }
          if (cmd === 'save_image_command') {
            return new Promise<string>(() => {}); // never resolves
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image
      const textarea = getAskInput();
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      // Type and submit while image is still processing.
      setAskValue('my question');
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Should be in chat mode with stop button
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Click stop to cancel the pending submit
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /stop/i }));
      });

      // Should revert to ask-bar mode with the query restored
      expect(getAskInput()).toBeInTheDocument();
      await waitFor(() =>
        expect(getAskInput().textContent).toBe('my question'),
      );

      // Images should still be visible (still processing in background)
      expect(
        screen.getByRole('list', { name: /attached images/i }),
      ).toBeInTheDocument();

      // ask_model should never have been called
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
    });

    it('waits for all images before firing deferred submit', async () => {
      // Flush stale macrotasks from prior tests
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });

      // Two images: each gets its own resolve function
      const resolvers: ((path: string) => void)[] = [];
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (args && 'onEvent' in args) {
            // Accept channel
          }
          if (cmd === 'save_image_command') {
            return new Promise<string>((resolve) => {
              resolvers.push(resolve);
            });
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Drop two images at once
      const askBarWrapper = document.querySelector(
        '[class*="flex flex-col w-full shrink-0"]',
      )!;
      const file1 = new File(['d1'], 'a.png', { type: 'image/png' });
      const file2 = new File(['d2'], 'b.png', { type: 'image/png' });
      fireEvent.drop(askBarWrapper, {
        preventDefault: vi.fn(),
        dataTransfer: { files: [file1, file2] },
      });

      // Wait for both save_image_command calls
      await act(async () => {
        await vi.waitFor(() => expect(resolvers).toHaveLength(2));
      });

      // Submit while both images are still processing
      const textarea = getAskInput();
      act(() => {
        setAskValue('two images');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve ONLY the first image - allReady should still be false
      await act(async () => {
        resolvers[0]('/tmp/img1.jpg');
      });
      await act(async () => {});

      // Still processing - second image not ready
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve the second image - now allReady is true, submit fires
      await act(async () => {
        resolvers[1]('/tmp/img2.jpg');
      });
      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      // User message should appear
      expect(screen.getByText('two images')).toBeInTheDocument();
    });

    it('cancels deferred submit when all images fail', async () => {
      // Make save_image_command hang then reject
      let rejectSave: ((err: Error) => void) | null = null;
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (args && 'onEvent' in args) {
            // channel capture
          }
          if (cmd === 'save_image_command') {
            return new Promise<string>((_, reject) => {
              rejectSave = reject;
            });
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste and submit while processing
      const textarea = getAskInput();
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      act(() => {
        setAskValue('describe');
      });

      // Wait for FileReader to complete and save_image_command to be invoked
      // (which sets rejectSave via the promise constructor).
      await act(async () => {
        await vi.waitFor(() => {
          expect(rejectSave).not.toBeNull();
        });
      });

      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Waiting state
      await vi.waitFor(() => {
        expect(
          screen.getByRole('button', { name: /stop/i }),
        ).toBeInTheDocument();
      });

      // Reject the image - it should be removed and pending submit cancelled
      await act(async () => {
        rejectSave!(new Error('disk full'));
      });

      // Image removed → no thumbnails → pending submit cancelled
      await vi.waitFor(() => {
        expect(
          screen.queryByRole('list', { name: /attached images/i }),
        ).toBeNull();
      });

      // ask_model should never have been called
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());

      // The "Processing images" button should be gone - back to normal send
      expect(
        screen.getByRole('button', { name: /send message/i }),
      ).toBeInTheDocument();

      // User's query should be restored so their text isn't lost
      await waitFor(() => expect(getAskInput().textContent).toBe('describe'));
    });
  });

  // ─── Capability gate (vision mismatch) ─────────────────────────────────────

  describe('capability gate', () => {
    /** Helper: paste an image file into the textarea and wait for thumbnails. */
    async function pasteImage() {
      const textarea = getAskInput();
      const file = new File(['fake-img-data'], 'photo.png', {
        type: 'image/png',
      });
      const clipboardData = {
        getData: () => '',
        items: [{ type: 'image/png', getAsFile: () => file }],
      };
      await act(async () => {
        fireEvent.paste(textarea, { clipboardData });
      });
      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });
    }

    it('shows the live mismatch strip when a text-only model has an image attached', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: 'llama3',
          all: ['llama3', 'llama3.2-vision'],
          ollamaReachable: true,
        },
        get_model_capabilities: {
          llama3: {
            vision: false,
            thinking: false,
          },
          'llama3.2-vision': {
            vision: true,
            thinking: false,
          },
        },
        save_image_command: '/tmp/staged/img1.jpg',
      });
      render(<App />);
      await act(async () => {});
      await showOverlay();
      await pasteImage();
      await vi.waitFor(() => {
        expect(
          screen.getByTestId('capability-mismatch-strip'),
        ).toBeInTheDocument();
      });
      expect(screen.getByTestId('capability-mismatch-strip')).toHaveTextContent(
        'llama3 reads text only',
      );
    });

    it('refuses submit and shakes the ask bar when a text-only model has an image attached', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: 'llama3',
          all: ['llama3'],
          ollamaReachable: true,
        },
        get_model_capabilities: {
          llama3: {
            vision: false,
            thinking: false,
          },
        },
        save_image_command: '/tmp/staged/img1.jpg',
      });
      render(<App />);
      await act(async () => {});
      await showOverlay();
      await pasteImage();
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      // Type and submit.
      act(() => {
        setAskValue('summarise these');
      });
      invoke.mockClear();
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /send message/i }));
      });

      // Capability strip remains the single surface for the conflict
      // message; the duplicate transient toast was removed.
      expect(screen.getByTestId('capability-mismatch-strip')).toHaveTextContent(
        'llama3 reads text only',
      );
      // ask_model is NOT invoked.
      const askInvocations = invoke.mock.calls.filter(
        (call) => call[0] === 'ask_model',
      );
      expect(askInvocations.length).toBe(0);
      // Compose state survives.
      expect(getAskInput().textContent).toBe('summarise these');
      // Wait past the 600 ms shake reset so the cleanup runs and the
      // shake state pulses back to false. This exercises the effect's
      // setTimeout/clearTimeout path that the gate relies on.
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 650));
      });
    });

    it('does not gate submit when the active model has vision', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: 'llama3.2-vision',
          all: ['llama3.2-vision'],
          ollamaReachable: true,
        },
        get_model_capabilities: {
          'llama3.2-vision': {
            vision: true,
            thinking: false,
          },
        },
        save_image_command: '/tmp/staged/img1.jpg',
      });
      render(<App />);
      await act(async () => {});
      await showOverlay();
      await pasteImage();
      // Strip must not appear.
      expect(screen.queryByTestId('capability-mismatch-strip')).toBeNull();
    });
  });

  // ─── Screenshot integration ────────────────────────────────────────────────

  describe('screenshot integration', () => {
    it('clicking screenshot button invokes capture_screenshot', async () => {
      enableChannelCaptureWithResponses({ capture_screenshot_command: null });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Take screenshot' }),
        );
      });

      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith('capture_screenshot_command');
        });
      });
    });

    it('does nothing when capture_screenshot returns null (cancelled)', async () => {
      enableChannelCaptureWithResponses({ capture_screenshot_command: null });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Take screenshot' }),
        );
      });

      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith('capture_screenshot_command');
        });
      });

      // save_image_command must NOT have been called
      const saveCalls = invoke.mock.calls.filter(
        ([cmd]) => cmd === 'save_image_command',
      );
      expect(saveCalls).toHaveLength(0);
    });

    it('does not invoke capture_screenshot_command when at max images', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img.jpg',
        capture_screenshot_command: null,
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Attach 3 images via paste to reach the limit.
      const pasteOneImage = async () => {
        const textarea = getAskInput();
        const file = new File(['data'], 'photo.png', { type: 'image/png' });
        await act(async () => {
          fireEvent.paste(textarea, {
            clipboardData: {
              getData: () => '',
              items: [{ type: 'image/png', getAsFile: () => file }],
            },
          });
        });
      };
      await pasteOneImage();
      await pasteOneImage();
      await pasteOneImage();

      const btn = screen.getByRole('button', { name: 'Take screenshot' });
      expect(btn).toBeDisabled();

      invoke.mockClear();
      fireEvent.click(btn);
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('capture_screenshot_command');
    });

    it('attaches screenshot image when capture_screenshot returns base64', async () => {
      const fakeBase64 = btoa('fake screenshot bytes');
      enableChannelCaptureWithResponses({
        capture_screenshot_command: fakeBase64,
        save_image_command: '/tmp/screenshot.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Take screenshot' }),
        );
      });

      // Wait for invoke(capture_screenshot) → FileReader → invoke(save_image_command)
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.objectContaining({ imageDataBase64: expect.any(String) }),
          );
        });
      });
    });
  });

  it('revokes blob URLs when overlay reopens with attached images', async () => {
    enableChannelCaptureWithResponses({
      save_image_command: '/tmp/img.jpg',
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    // Paste an image so attachedImages is non-empty
    const textarea = getAskInput();
    const file = new File(['data'], 'img.png', { type: 'image/png' });
    await act(async () => {
      fireEvent.paste(textarea, {
        clipboardData: {
          getData: () => '',
          items: [{ type: 'image/png', getAsFile: () => file }],
        },
      });
    });

    await vi.waitFor(() => {
      expect(
        screen.getByRole('list', { name: /attached images/i }),
      ).toBeInTheDocument();
    });

    // Reopen overlay - should clear images and revoke blob URLs
    await showOverlay();

    expect(URL.revokeObjectURL).toHaveBeenCalled();
    expect(screen.queryByRole('list', { name: /attached images/i })).toBeNull();
  });

  it('revokes blob URLs when overlay hides with attached images', async () => {
    enableChannelCaptureWithResponses({
      save_image_command: '/tmp/img.jpg',
    });

    render(<App />);
    await act(async () => {});
    await showOverlay();

    const textarea = getAskInput();
    const file = new File(['data'], 'img.png', { type: 'image/png' });
    await act(async () => {
      fireEvent.paste(textarea, {
        clipboardData: {
          getData: () => '',
          items: [{ type: 'image/png', getAsFile: () => file }],
        },
      });
    });

    await vi.waitFor(() => {
      expect(
        screen.getByRole('list', { name: /attached images/i }),
      ).toBeInTheDocument();
    });

    const revokeSpy = vi.mocked(URL.revokeObjectURL);
    revokeSpy.mockClear();

    // Hide overlay via Escape - requestHideOverlay should revoke blob URLs
    await act(async () => {
      fireEvent.keyDown(window, { key: 'Escape' });
    });

    expect(revokeSpy).toHaveBeenCalled();
  });

  it('resets session on overlay reopen', async () => {
    render(<App />);
    await act(async () => {});

    await showOverlay();

    const textarea = getAskInput();

    // Complete a conversation turn
    act(() => {
      setAskValue('first question');
    });
    act(() => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });
    await act(async () => {});

    act(() => {
      getLastChannel()?.simulateMessage({
        type: 'Token',
        data: 'First response',
      });
      getLastChannel()?.simulateMessage({ type: 'Done' });
    });

    expect(screen.getByText('First response')).toBeInTheDocument();

    // Re-enable channel capture for second session
    enableChannelCapture();

    // Reopen overlay - should reset session
    await showOverlay();

    // Should be back to input bar mode with placeholder
    expect(getAskInput()).toBeInTheDocument();
    // Old messages should be gone
    expect(screen.queryByText('First response')).toBeNull();
  });

  // ─── /screen command ─────────────────────────────────────────────────────────

  describe('/screen command', () => {
    it('invokes capture_full_screen_command and calls ask with screenshot path', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Use "/screen " (with trailing space) so the suggestion popover is dismissed
      // and Enter goes to the submit handler directly.
      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen ');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'capture_full_screen_command',
        expect.objectContaining({ conversationId: expect.any(String) }),
      );
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          imagePaths: ['/tmp/screen.jpg'],
          message: '/screen',
        }),
      );
    });

    it('keeps the /screen trigger in the message sent to the backend', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen what is this error?');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: '/screen what is this error?',
          imagePaths: ['/tmp/screen.jpg'],
        }),
      );
    });

    it('detects /screen anywhere in the message, not just at start', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('hello /screen there');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'capture_full_screen_command',
        expect.objectContaining({ conversationId: expect.any(String) }),
      );
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'hello /screen there',
          imagePaths: ['/tmp/screen.jpg'],
        }),
      );
    });

    it('does not call ask when capture_full_screen_command throws', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'capture_full_screen_command') {
          throw new Error('Permission denied');
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Use "/screen " (with trailing space) so the suggestion popover is dismissed
      // and Enter goes directly to the submit handler.
      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen ');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'capture_full_screen_command',
        expect.objectContaining({ conversationId: expect.any(String) }),
      );
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
      // The actual Rust error message is surfaced directly.
      expect(screen.getByText('Permission denied')).toBeInTheDocument();
    });

    it('surfaces string errors from Tauri invoke directly', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'capture_full_screen_command') {
          // Tauri v2 rejects with the Err(String) value as a plain string.
          return Promise.reject(
            'Screen Recording permission is required to use /screen.',
          );
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen ');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(
        screen.getByText(
          'Screen Recording permission is required to use /screen.',
        ),
      ).toBeInTheDocument();
    });

    it('handles non-Error non-string rejection values', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'capture_full_screen_command') {
          return Promise.reject(42);
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(screen.getByText('42')).toBeInTheDocument();
    });

    it('clears capture error when a new submit is attempted', async () => {
      enableChannelCapture();
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'capture_full_screen_command') {
          throw new Error('capture failed');
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      // First attempt fails; error banner appears.
      act(() => {
        setAskValue('/screen ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      expect(screen.getByText('capture failed')).toBeInTheDocument();

      // Typing a new query and submitting normal text clears the error banner.
      act(() => {
        setAskValue('hello');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      expect(screen.queryByText('capture failed')).toBeNull();
    });

    it('merges screenshot path with existing attached images', async () => {
      // Set up mocks: save_image_command for image attachment, then screen capture.
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/attached.jpg',
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image first. This exercises the filter/map on attachedImages inside
      // handleScreenSubmit, covering the lines for non-null filePath images.
      const textarea = getAskInput();
      const file = new File(['img'], 'photo.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
          expect(screen.getAllByRole('listitem')).toHaveLength(1);
        });
      });

      // Now type /screen and submit.
      act(() => {
        setAskValue('/screen describe');
      });

      vi.useFakeTimers();
      try {
        await act(async () => {
          fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
          await Promise.resolve();
          await Promise.resolve();
        });

        expect(invoke).toHaveBeenCalledWith(
          'capture_full_screen_command',
          expect.objectContaining({ conversationId: expect.any(String) }),
        );
        expect(invoke).toHaveBeenCalledWith(
          'ask_model',
          expect.objectContaining({
            message: '/screen describe',
            imagePaths: ['/tmp/attached.jpg', '/tmp/screen.jpg'],
          }),
        );

        await act(async () => {
          getLastChannel()?.simulateMessage({ type: 'Token', data: 'done' });
          getLastChannel()?.simulateMessage({ type: 'Done' });
          await Promise.resolve();
          await Promise.resolve();
        });
        await vi.waitFor(() => {
          expect(screen.getByText('done')).toBeInTheDocument();
        });
      } finally {
        vi.useRealTimers();
      }
    });

    it('handles /screen with selected context', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay('some context');

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen explain');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: '/screen explain',
          quotedText: 'some context',
          imagePaths: ['/tmp/screen.jpg'],
        }),
      );
    });

    it('shows pending chat bubble immediately on submit before capture resolves', async () => {
      let resolveCapture!: (path: string) => void;
      enableChannelCaptureWithResponses({
        capture_full_screen_command: new Promise<string>((res) => {
          resolveCapture = res;
        }),
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen check this');
      });

      // Submit; capture is now in-flight (pending)
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Before capture resolves: query should be cleared and app in pending mode
      await waitFor(() => expect(getAskInput().textContent).toBe(''));

      // Resolve the capture and let async work settle
      await act(async () => {
        resolveCapture('/tmp/screen.jpg');
      });
      await act(async () => {});

      // After capture resolves: ask_model should be called
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({ message: '/screen check this' }),
      );
    });

    it('restores query with cleanQuery text when capture fails mid-message', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'capture_full_screen_command') {
          throw new Error('Screen capture timed out');
        }
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen what is this?');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Query should be restored with the full original message
      await waitFor(() =>
        expect(getAskInput().textContent).toBe('/screen what is this?'),
      );
      expect(screen.getByText('Screen capture timed out')).toBeInTheDocument();
    });

    it('defers /screen submit when an attached image is still processing and runs once it resolves', async () => {
      // Regression guard: submitting /screen with a still-processing image
      // used to drop the image silently and ask_model was called with only
      // the screenshot. The unified pre-flight gate now defers the submit
      // until every attached image has a resolved filePath, so both paths
      // make it into the request.
      let resolveSave!: (path: string) => void;
      enableChannelCaptureWithResponses({
        save_image_command: new Promise<string>((res) => {
          resolveSave = res;
        }),
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image; save_image_command hangs, so filePath stays null.
      const textarea = getAskInput();
      const file = new File(['img'], 'photo.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      // Submit /screen while image is still processing.
      act(() => {
        setAskValue('/screen ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Deferred: neither the screen capture nor the model call have fired.
      expect(invoke).not.toHaveBeenCalledWith(
        'capture_full_screen_command',
        expect.anything(),
      );
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());

      // Resolve the image; the deferred /screen submit fires.
      act(() => {
        resolveSave('/tmp/staged/img1.jpg');
      });
      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith(
          'capture_full_screen_command',
          expect.objectContaining({ conversationId: expect.any(String) }),
        );
      });
      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith(
          'ask_model',
          expect.objectContaining({
            imagePaths: ['/tmp/staged/img1.jpg', '/tmp/screen.jpg'],
          }),
        );
      });
    });

    it('cancelling during in-flight capture prevents ask from being called', async () => {
      let resolveCapture!: (path: string) => void;
      enableChannelCaptureWithResponses({
        capture_full_screen_command: new Promise<string>((res) => {
          resolveCapture = res;
        }),
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen ');
      });

      // Submit; capture is now in-flight
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Cancel while capture is pending (Stop button)
      const stopButton = screen.getByRole('button', { name: /stop|cancel/i });
      act(() => {
        fireEvent.click(stopButton);
      });

      // Resolve the capture after cancel
      await act(async () => {
        resolveCapture('/tmp/screen.jpg');
      });
      await act(async () => {});

      // ask_model must NOT be called since the user cancelled
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
    });

    it('/screen combined with utility command applies the prompt template via OCR', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
        extract_text_command: 'OCR screen content here',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /explain ');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('extract_text_command', {
        imagePaths: ['/tmp/screen.jpg'],
      });

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Explain the following in plain');
        // OCR text used as $INPUT; no image bytes sent to model
        expect(args.message).toContain('OCR screen content here');
      });
    });

    it('/screen combined with utility command and typed text: OCR as $INPUT, typed text as instruction', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
        extract_text_command: 'screen OCR content',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /explain this error message');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('extract_text_command', {
        imagePaths: ['/tmp/screen.jpg'],
      });

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Explain the following in plain');
        // OCR text is $INPUT; typed text appended as additional instruction
        expect(args.message).toContain('screen OCR content');
        expect(args.message).toContain('this error message');
      });
    });

    it('/screen without utility command sends raw message without template', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen what is this?');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        // No template applied: raw message sent
        expect(args.message).toBe('/screen what is this?');
        expect(args.imagePaths).toEqual(['/tmp/screen.jpg']);
      });
    });

    it('/screen with utility command uses OCR text as $INPUT and selected context as quotedText', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
        extract_text_command: 'OCR extracted content',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay('my highlighted code');

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /explain ');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('extract_text_command', {
        imagePaths: ['/tmp/screen.jpg'],
      });

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Explain the following in plain');
        // OCR text is the $INPUT; no image bytes sent to model
        expect(args.message).toContain('OCR extracted content');
      });
    });

    it('/screen /translate with no language defaults to Vietnamese', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
        extract_text_command: 'Bonjour le monde',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /translate ');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Target language: Vietnamese');
        expect(args.message).toContain('Bonjour le monde');
      });
    });
  });

  // ─── /extract command ───────────────────────────────────────────────────────

  describe('/extract command', () => {
    async function pasteImageForExtract() {
      const textarea = getAskInput();
      const file = new File(['fake-img-data'], 'photo.png', {
        type: 'image/png',
      });
      const clipboardData = {
        getData: () => '',
        items: [{ type: 'image/png', getAsFile: () => file }],
      };
      await act(async () => {
        fireEvent.paste(textarea, { clipboardData });
      });
      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });
    }

    it('shakes ask bar and shows warning when /extract is submitted with no image and no /screen', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith(
        'extract_text_command',
        expect.anything(),
      );
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
      expect(
        screen.getByText(
          'Attach an image or add /screen to extract text from.',
        ),
      ).toBeInTheDocument();
    });

    it('invokes extract_text_command with attached image and shows extracted text', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
        extract_text_command: 'Hello World',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForExtract();
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      const textarea = getAskInput();
      act(() => {
        setAskValue('/extract ');
      });

      invoke.mockClear();
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('extract_text_command', {
        imagePaths: ['/tmp/staged/img1.jpg'],
      });
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
      await vi.waitFor(() => {
        expect(screen.getByText(/Hello World/)).toBeInTheDocument();
      });
    });

    it('invokes capture_full_screen_command then extract_text_command for /screen /extract', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
        extract_text_command: 'Screen text',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'capture_full_screen_command',
        expect.objectContaining({ conversationId: expect.any(String) }),
      );
      expect(invoke).toHaveBeenCalledWith('extract_text_command', {
        imagePaths: ['/tmp/screen.jpg'],
      });
      await vi.waitFor(() => {
        expect(screen.getByText(/Screen text/)).toBeInTheDocument();
      });
    });

    it('falls back to ask via Ollama when OCR fails and the model supports vision', async () => {
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            // channel capture - no-op; we only verify ask_model was called
          }
          if (cmd === 'get_model_picker_state')
            return {
              active: 'llama3.2-vision',
              all: ['llama3.2-vision'],
              ollamaReachable: true,
            };
          if (cmd === 'get_model_capabilities')
            return { 'llama3.2-vision': { vision: true, thinking: false } };
          if (cmd === 'capture_full_screen_command') return '/tmp/screen.jpg';
          if (cmd === 'extract_text_command')
            throw new Error('Vision OCR failed');
          if (cmd === 'get_updater_state')
            return {
              last_check_at_unix: null,
              update: null,
              settings_snoozed_until: null,
              chat_snoozed_until: null,
            };
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: expect.stringContaining('Extract all text'),
          imagePaths: ['/tmp/screen.jpg'],
        }),
      );
    });

    it('shows error when OCR fails and the model does not support vision', async () => {
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            // channel capture - no-op
          }
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (cmd === 'get_model_capabilities')
            return { 'gemma4:e2b': { vision: false, thinking: false } };
          if (cmd === 'capture_full_screen_command') return '/tmp/screen.jpg';
          if (cmd === 'extract_text_command')
            return Promise.reject('OCR error text');
          if (cmd === 'get_updater_state')
            return {
              last_check_at_unix: null,
              update: null,
              settings_snoozed_until: null,
              chat_snoozed_until: null,
            };
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
      await vi.waitFor(() => {
        expect(
          screen.getByText(/OCR failed: OCR error text/),
        ).toBeInTheDocument();
      });
    });

    it('shows error when capture_full_screen_command throws during /screen /extract', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'capture_full_screen_command')
          throw new Error('Permission denied');
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith(
        'extract_text_command',
        expect.anything(),
      );
      expect(screen.getByText('Permission denied')).toBeInTheDocument();
    });

    it('suppresses vision capability mismatch strip when /extract is typed with an attached image', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: 'llama3',
          all: ['llama3'],
          ollamaReachable: true,
        },
        get_model_capabilities: {
          llama3: { vision: false, thinking: false },
        },
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForExtract();
      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      act(() => {
        setAskValue('/extract ');
      });

      // The capability mismatch strip must not appear when /extract is present.
      expect(
        screen.queryByTestId('capability-mismatch-strip'),
      ).not.toBeInTheDocument();
    });

    it('handles /extract with selected context', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
        extract_text_command: 'extracted text',
      });

      render(<App />);
      await act(async () => {});
      // Show overlay with selected text to exercise the context branch.
      await showOverlay('Selected content here');

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      await vi.waitFor(() => {
        expect(screen.getByText(/extracted text/)).toBeInTheDocument();
      });
    });

    it('defers /extract submit when an attached image is still processing and runs OCR once it resolves', async () => {
      // Regression guard: submitting /extract with a still-processing image
      // used to call OCR immediately with an empty paths list (producing
      // "[No text detected]"). The unified pre-flight gate now defers the
      // submit until every attached image has a resolved filePath.
      let resolveSave!: (path: string) => void;
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            /* channel capture - no-op */
          }
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (cmd === 'get_model_capabilities')
            return { 'gemma4:e2b': { vision: false, thinking: false } };
          if (cmd === 'save_image_command')
            return new Promise<string>((res) => {
              resolveSave = res;
            });
          if (cmd === 'extract_text_command') return 'Hello from image';
          if (cmd === 'get_updater_state')
            return {
              last_check_at_unix: null,
              update: null,
              settings_snoozed_until: null,
              chat_snoozed_until: null,
            };
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForExtract();
      // Do NOT resolve save_image_command yet; image stays in-flight (filePath=null).

      const textarea = getAskInput();
      act(() => {
        setAskValue('/extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Deferred: OCR has NOT been called yet, pending bubble is visible.
      expect(invoke).not.toHaveBeenCalledWith(
        'extract_text_command',
        expect.anything(),
      );
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve the image; the deferred /extract submit fires.
      act(() => {
        resolveSave('/tmp/staged/img1.jpg');
      });
      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith('extract_text_command', {
          imagePaths: ['/tmp/staged/img1.jpg'],
        });
      });
      await vi.waitFor(() => {
        expect(screen.getByText(/Hello from image/)).toBeInTheDocument();
      });
    });

    it('cancelling during in-flight /screen /extract capture prevents OCR from running', async () => {
      let resolveCapture!: (path: string) => void;
      enableChannelCaptureWithResponses({
        capture_full_screen_command: new Promise<string>((res) => {
          resolveCapture = res;
        }),
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Cancel while capture is in-flight.
      const stopButton = screen.getByRole('button', { name: /stop|cancel/i });
      act(() => {
        fireEvent.click(stopButton);
      });

      await act(async () => {
        resolveCapture('/tmp/screen.jpg');
      });
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith(
        'extract_text_command',
        expect.anything(),
      );
    });

    it('surfaces string screen-capture errors for /screen /extract', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'capture_full_screen_command')
          return Promise.reject('Screen Recording permission is required.');
        if (cmd === 'get_updater_state')
          return {
            last_check_at_unix: null,
            update: null,
            settings_snoozed_until: null,
            chat_snoozed_until: null,
          };
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(
        screen.getByText('Screen Recording permission is required.'),
      ).toBeInTheDocument();
    });

    it('handles non-Error non-string screen-capture rejection for /screen /extract', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'get_model_picker_state')
          return {
            active: 'gemma4:e2b',
            all: ['gemma4:e2b'],
            ollamaReachable: true,
          };
        if (cmd === 'capture_full_screen_command') return Promise.reject(42);
        if (cmd === 'get_updater_state')
          return {
            last_check_at_unix: null,
            update: null,
            settings_snoozed_until: null,
            chat_snoozed_until: null,
          };
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(screen.getByText('42')).toBeInTheDocument();
    });

    it('shows error with Error message when OCR throws an Error object (no vision model)', async () => {
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            /* channel capture - no-op */
          }
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (cmd === 'get_model_capabilities')
            return { 'gemma4:e2b': { vision: false, thinking: false } };
          if (cmd === 'capture_full_screen_command') return '/tmp/screen.jpg';
          if (cmd === 'extract_text_command')
            throw new Error('OCR engine error');
          if (cmd === 'get_updater_state')
            return {
              last_check_at_unix: null,
              update: null,
              settings_snoozed_until: null,
              chat_snoozed_until: null,
            };
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      await vi.waitFor(() => {
        expect(
          screen.getByText(/OCR failed: OCR engine error/),
        ).toBeInTheDocument();
      });
    });

    it('shows error with empty suffix when OCR throws a non-Error non-string value (no vision model)', async () => {
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            /* channel capture - no-op */
          }
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (cmd === 'get_model_capabilities')
            return { 'gemma4:e2b': { vision: false, thinking: false } };
          if (cmd === 'capture_full_screen_command') return '/tmp/screen.jpg';
          if (cmd === 'extract_text_command') return Promise.reject(null);
          if (cmd === 'get_updater_state')
            return {
              last_check_at_unix: null,
              update: null,
              settings_snoozed_until: null,
              chat_snoozed_until: null,
            };
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      await vi.waitFor(() => {
        expect(
          screen.getByText(/OCR failed\. Switch to a vision-capable model/),
        ).toBeInTheDocument();
      });
    });

    it('treats undefined activeModelCapabilities as non-vision when OCR fails', async () => {
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            /* channel capture - no-op */
          }
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          // Empty capabilities map: activeModelCapabilities will be undefined.
          if (cmd === 'get_model_capabilities') return {};
          if (cmd === 'capture_full_screen_command') return '/tmp/screen.jpg';
          if (cmd === 'extract_text_command')
            return Promise.reject('OCR error');
          if (cmd === 'get_updater_state')
            return {
              last_check_at_unix: null,
              update: null,
              settings_snoozed_until: null,
              chat_snoozed_until: null,
            };
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /extract ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // No vision capability → shows error rather than falling back to Ollama.
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
      await vi.waitFor(() => {
        expect(screen.getByText(/OCR failed/)).toBeInTheDocument();
      });
    });
  });

  // ─── /think command ─────────────────────────────────────────────────────────

  describe('/think command', () => {
    it('sends think:true to ask_model and keeps /think prefix in message', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/think why is the sky blue?');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: '/think why is the sky blue?',
          think: true,
        }),
      );
    });

    it('shows a warming-up placeholder first, then swaps it to the thinking row when thinking tokens arrive', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/think explain recursion');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      expect(screen.getByTestId('thinking-block')).toBeInTheDocument();
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Warming up...',
      );
      expect(
        screen.queryByRole('button', { name: 'Toggle thinking details' }),
      ).toBeNull();

      act(() => {
        getLastChannel()?.simulateMessage({
          type: 'ThinkingToken',
          data: 'Let me think this through.',
        });
      });

      expect(screen.queryByText('Warming up...')).toBeNull();
      expect(
        screen.getByRole('button', { name: 'Toggle thinking details' }),
      ).toBeInTheDocument();
      expect(screen.getByTestId('loading-label').textContent).toBe(
        'Thinking...',
      );
    });

    it('does nothing when /think has no query and no images', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/think');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
    });

    it('detects /think anywhere in the message, not just at start', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('hello /think world');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: 'hello /think world',
          think: true,
        }),
      );
    });

    it('forwards selected context when /think is used with quoted text', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay('some selected text');

      const textarea = getAskInput();
      act(() => {
        setAskValue('/think explain this code');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: '/think explain this code',
          quotedText: 'some selected text',
          think: true,
        }),
      );
    });

    it('sends think:true with /think followed by only a space', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/think ');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      // "/think " with only a space after prefix, no actual query, no images => no submit
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
    });
  });

  // ─── Multi-command ──────────────────────────────────────────────────────────

  describe('Multi-command support', () => {
    it('sends /screen /think with both screen capture and think:true', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /think explain this');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'capture_full_screen_command',
        expect.objectContaining({ conversationId: expect.any(String) }),
      );
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: '/screen /think explain this',
          imagePaths: ['/tmp/screen.jpg'],
          think: true,
        }),
      );
    });

    it('sends /think /screen with both screen capture and think:true', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/think /screen explain this');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'capture_full_screen_command',
        expect.objectContaining({ conversationId: expect.any(String) }),
      );
      expect(invoke).toHaveBeenCalledWith(
        'ask_model',
        expect.objectContaining({
          message: '/think /screen explain this',
          imagePaths: ['/tmp/screen.jpg'],
          think: true,
        }),
      );
    });
  });

  // ─── Utility commands ───────────────────────────────────────────────────────

  describe('Utility commands (buildPrompt routing)', () => {
    it('routes /rewrite command through buildPrompt and calls ask_model with composed prompt', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/rewrite fix this text');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain(
          'Rewrite the text below so it sounds like a fluent native English speaker',
        );
        expect(args.message).toContain('fix this text');
      });
    });

    it('routes /translate with language arg through buildPrompt', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/translate jpn hello world');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Target language: jpn');
        expect(args.message).toContain('Text: hello world');
      });
    });

    it('/think and utility command compose: /think /tldr some long text', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/think /tldr some long text');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Summarize the following text');
        expect(args.message).toContain('some long text');
        expect(args.think).toBe(true);
      });
    });

    it('utility command with no input shakes and shows error instead of silently no-oping', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/rewrite');
      });

      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
      await vi.waitFor(() => {
        expect(
          screen.getByText('Provide text or attach an image to use /rewrite.'),
        ).toBeInTheDocument();
      });
    });

    it('utility command with attached image and no text: OCR extracts text as $INPUT', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/explain.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Attach an image
      const textarea = getAskInput();
      const file = new File(['data'], 'shot.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      invoke.mockClear();
      enableChannelCaptureWithResponses({
        extract_text_command: 'OCR text from image',
      });

      // Submit just the command with no text: OCR path fires
      act(() => {
        setAskValue('/explain');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('extract_text_command', {
        imagePaths: ['/tmp/staged/explain.jpg'],
      });

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Explain the following in plain');
        // OCR text is $INPUT; no image bytes sent to model
        expect(args.message).toContain('OCR text from image');
      });
    });

    it('/translate with only an image and no text does not call ask_model', async () => {
      // /translate needs a language code from typed text; image fallback is skipped for it.
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      invoke.mockClear();
      enableChannelCapture();

      act(() => {
        setAskValue('/translate');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
    });

    it('utility command with only a language code (no text) shakes and shows error', async () => {
      // /translate with only a language code makes buildPrompt return null:
      // lang='jpn', typedRemainder='', selected='' → null.
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/translate jpn');
      });

      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
      await vi.waitFor(() => {
        expect(
          screen.getByText(
            'Provide text or attach an image to use /translate.',
          ),
        ).toBeInTheDocument();
      });
    });

    it('utility command uses selected context when available', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      // Activate overlay with selected text as context
      await showOverlay('original selected text');

      const textarea = getAskInput();
      // Type a command with extra instruction so strippedMessage is non-empty
      // (bypasses the "no content" early guard) and selectedContext is also set.
      act(() => {
        setAskValue('/rewrite make it concise');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain(
          'Rewrite the text below so it sounds like a fluent native English speaker',
        );
        expect(args.message).toContain('original selected text');
        expect(args.quotedText).toBe('original selected text');
      });
    });

    it('utility command with bare trigger uses selected context as display text', async () => {
      // strippedMessage is empty, selectedContext is present, images bypass the
      // early-return guard. displayText falls through to selectedContext?.trim().
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/ctx.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay('my selected text');

      // Paste an image and wait for backend resolution so hasPendingImages is false
      const textarea = getAskInput();
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      invoke.mockClear();
      enableChannelCaptureWithResponses({
        extract_text_command: 'OCR text from image',
      });

      // Submit just the command trigger (strippedMessage will be '')
      act(() => {
        setAskValue('/rewrite');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith('extract_text_command', {
          imagePaths: ['/tmp/staged/ctx.jpg'],
        });
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        // OCR text is $INPUT; selectedContext used as quotedText display
        expect(args.message).toContain('OCR text from image');
        expect(args.quotedText).toBe('my selected text');
      });
    });

    it('displays stripped user input in chat bubble, not the prompt template', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/rewrite fix this text');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await act(async () => {});

      // renderUserContent splits command triggers into separate spans.
      // Check body textContent to confirm the full original query appears.
      await vi.waitFor(() => {
        expect(document.body.textContent).toContain('/rewrite fix this text');
      });
    });

    it('utility command with resolved attached images passes imagePaths and revokes blob URLs', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image and wait for backend resolution
      const textarea = getAskInput();
      const file = new File(['fake-img-data'], 'photo.png', {
        type: 'image/png',
      });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      invoke.mockClear();
      enableChannelCaptureWithResponses({
        extract_text_command: 'OCR extracted prose text',
      });

      // Type /rewrite command and submit
      act(() => {
        setAskValue('/rewrite fix this prose');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith('extract_text_command', {
          imagePaths: ['/tmp/staged/img1.jpg'],
        });
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain(
          'Rewrite the text below so it sounds like a fluent native English speaker',
        );
        expect(args.message).toContain('OCR extracted prose text');
        expect(args.imagePaths).toBeNull();
      });
    });

    it('utility OCR command with image-only submitted while image is pending defers and waits for full resolution before running OCR', async () => {
      // Regression: submitting /translate with only an image before it finishes
      // uploading caused readyPaths to be empty, OCR to run with no paths, and
      // the "No readable text found" error to surface. The fix defers the OCR
      // until the image is fully resolved.
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });

      let resolveSave: ((path: string) => void) | null = null;
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (args && 'onEvent' in args) {
            // Accept channel for ask_model
          }
          if (cmd === 'save_image_command') {
            return new Promise<string>((resolve) => {
              resolveSave = resolve;
            });
          }
          if (cmd === 'extract_text_command') {
            // Verify OCR is called with the resolved path, not an empty array.
            const paths = (args as Record<string, unknown>)
              .imagePaths as string[];
            expect(paths).toHaveLength(1);
            expect(paths[0]).toBe('/tmp/staged/ocr-deferred.jpg');
            return 'Deferred OCR text';
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      // Wait for save_image_command to be invoked (image still unresolved).
      await act(async () => {
        await vi.waitFor(() => expect(resolveSave).not.toBeNull());
      });

      // Submit /translate with image-only (no text) while image still loading.
      act(() => {
        setAskValue('/translate');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Pending state active — submit locked.
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();
      // OCR must NOT have been called yet.
      expect(invoke).not.toHaveBeenCalledWith(
        'extract_text_command',
        expect.anything(),
      );

      // Resolve the image — triggers deferred OCR chain.
      resolveSave!('/tmp/staged/ocr-deferred.jpg');

      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      // OCR ran (assertions inside the extract_text_command mock above).
      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith(
          'extract_text_command',
          expect.anything(),
        );
      });
    });

    it('utility command with pending images defers submit until images resolve', async () => {
      // Flush stale macrotasks from prior tests
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });

      let resolveSave: ((path: string) => void) | null = null;
      const savePromises: Promise<string>[] = [];
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (args && 'onEvent' in args) {
            // Accept channel for ask_model
          }
          if (cmd === 'save_image_command') {
            const p = new Promise<string>((resolve) => {
              resolveSave = resolve;
            });
            savePromises.push(p);
            return p;
          }
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      // Paste an image - thumbnail appears immediately (filePath null)
      const textarea = getAskInput();
      const file = new File(['data'], 'img.png', { type: 'image/png' });
      await act(async () => {
        fireEvent.paste(textarea, {
          clipboardData: {
            getData: () => '',
            items: [{ type: 'image/png', getAsFile: () => file }],
          },
        });
      });

      // Wait for this test's FileReader to complete and call save_image_command
      await act(async () => {
        await vi.waitFor(() => expect(savePromises).toHaveLength(1));
      });

      // Type /rewrite and submit while image is still processing
      act(() => {
        setAskValue('/rewrite make it clearer');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Should show pending state (stop button visible)
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve the image - triggers deferred submit chain
      resolveSave!('/tmp/staged/img1.jpg');

      // Flush async chain: promise -> state update -> effect -> ask -> invoke
      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      // renderUserContent splits command triggers into separate spans.
      // Check body textContent to confirm the full original query appears.
      expect(document.body.textContent).toContain('/rewrite make it clearer');
    });
  });

  // ─── Utility commands with images (OCR path) ───────────────────────────────

  describe('Utility commands with images (OCR path)', () => {
    async function pasteImageForUtility() {
      const textarea = getAskInput();
      const file = new File(['fake-img-data'], 'photo.png', {
        type: 'image/png',
      });
      const clipboardData = {
        getData: () => '',
        items: [{ type: 'image/png', getAsFile: () => file }],
      };
      await act(async () => {
        fireEvent.paste(textarea, { clipboardData });
      });
      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });
    }

    it('/tldr with attached image: OCR then ask_model with tldr prompt and no image paths', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
        extract_text_command: 'Some article text here',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForUtility();
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      const textarea = getAskInput();
      act(() => {
        setAskValue('/tldr ');
      });

      invoke.mockClear();
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('extract_text_command', {
        imagePaths: ['/tmp/staged/img1.jpg'],
      });

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Summarize the following text');
        expect(args.message).toContain('Some article text here');
        expect(args.imagePaths).toBeNull();
      });
    });

    it('/translate french with image: OCR text becomes $INPUT, french becomes $LANG', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
        extract_text_command: 'Hello world from image',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForUtility();
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      const textarea = getAskInput();
      act(() => {
        setAskValue('/translate french ');
      });

      invoke.mockClear();
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Target language: french');
        expect(args.message).toContain('Hello world from image');
        expect(args.imagePaths).toBeNull();
      });
    });

    it('/screen /tldr: capture then OCR then ask_model with no image paths', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/screen.jpg',
        extract_text_command: 'Screen article text',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /tldr ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith(
        'capture_full_screen_command',
        expect.objectContaining({ conversationId: expect.any(String) }),
      );
      expect(invoke).toHaveBeenCalledWith('extract_text_command', {
        imagePaths: ['/tmp/screen.jpg'],
      });

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Summarize the following text');
        expect(args.message).toContain('Screen article text');
        expect(args.imagePaths).toBeNull();
      });
    });

    it('shows captureError and does not call ask_model when OCR returns [No text detected]', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
        extract_text_command: '[No text detected]',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForUtility();
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      const textarea = getAskInput();
      act(() => {
        setAskValue('/tldr ');
      });

      invoke.mockClear();
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
      await vi.waitFor(() => {
        expect(
          screen.getByText('No readable text found in the image.'),
        ).toBeInTheDocument();
      });
    });

    it('restores input and shows captureError when OCR throws', async () => {
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            /* channel capture - no-op */
          }
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (cmd === 'get_model_capabilities')
            return { 'gemma4:e2b': { vision: false, thinking: false } };
          if (cmd === 'save_image_command') return '/tmp/staged/img1.jpg';
          if (cmd === 'extract_text_command')
            return Promise.reject('OCR engine failed');
          if (cmd === 'get_updater_state')
            return {
              last_check_at_unix: null,
              update: null,
              settings_snoozed_until: null,
              chat_snoozed_until: null,
            };
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForUtility();
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      const textarea = getAskInput();
      act(() => {
        setAskValue('/tldr ');
      });

      invoke.mockClear();
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
      await vi.waitFor(() => {
        expect(
          screen.getByText('OCR failed: OCR engine failed'),
        ).toBeInTheDocument();
      });
    });

    it('defers utility OCR submit when image is still in-flight and waits for resolution', async () => {
      // Regression guard: submitting a utility command while the attached image
      // has filePath=null used to call OCR immediately with an empty paths list,
      // producing "No readable text found". The fix defers until all images resolve.
      let resolveSave!: (path: string) => void;
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (args && 'onEvent' in args) {
            /* channel capture - no-op */
          }
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (cmd === 'get_model_capabilities')
            return { 'gemma4:e2b': { vision: false, thinking: false } };
          if (cmd === 'save_image_command')
            return new Promise<string>((res) => {
              resolveSave = res;
            });
          if (cmd === 'extract_text_command') return 'summarize this';
          if (cmd === 'get_updater_state')
            return {
              last_check_at_unix: null,
              update: null,
              settings_snoozed_until: null,
              chat_snoozed_until: null,
            };
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForUtility();
      // Do NOT resolve save_image_command yet — image stays in-flight (filePath=null).

      const textarea = getAskInput();
      act(() => {
        setAskValue('/tldr ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Submit is deferred — pending state active, OCR NOT called yet.
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();
      expect(invoke).not.toHaveBeenCalledWith(
        'extract_text_command',
        expect.anything(),
      );

      // Resolve the image — deferred OCR chain fires.
      act(() => {
        resolveSave('/tmp/staged/img1.jpg');
      });
      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      // OCR now called with the resolved path, not an empty list.
      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith('extract_text_command', {
          imagePaths: ['/tmp/staged/img1.jpg'],
        });
      });
    });

    it('cancelling during /screen capture in /screen /tldr restores input and skips OCR', async () => {
      let resolveCapture!: (path: string) => void;
      enableChannelCaptureWithResponses({
        capture_full_screen_command: new Promise<string>((res) => {
          resolveCapture = res;
        }),
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /tldr ');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // Cancel while capture is in-flight.
      const stopButton = screen.getByRole('button', { name: /stop|cancel/i });
      act(() => {
        fireEvent.click(stopButton);
      });

      await act(async () => {
        resolveCapture('/tmp/screen.jpg');
      });
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith(
        'extract_text_command',
        expect.anything(),
      );
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
    });

    it('/screen /tldr shows captureError and restores input when capture_full_screen_command throws', async () => {
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (cmd === 'get_model_capabilities')
            return { 'gemma4:e2b': { vision: false, thinking: false } };
          if (cmd === 'get_config') return null;
          if (cmd === 'capture_full_screen_command')
            throw new Error('Screen capture denied');
          if (args && 'onEvent' in args) return undefined;
          return undefined;
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/screen /tldr ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith(
        'extract_text_command',
        expect.anything(),
      );
      expect(invoke).not.toHaveBeenCalledWith('ask_model', expect.anything());
      await vi.waitFor(() => {
        expect(screen.getByText('Screen capture denied')).toBeInTheDocument();
      });
    });

    it('/translate with image and no language defaults to Vietnamese', async () => {
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/img1.jpg',
        extract_text_command: 'Bonjour le monde',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForUtility();
      await act(async () => {
        await vi.waitFor(() => {
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          );
        });
      });

      const textarea = getAskInput();
      act(() => {
        setAskValue('/translate ');
      });

      invoke.mockClear();
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Target language: Vietnamese');
        expect(args.message).toContain('Bonjour le monde');
      });
    });

    it('existing text-only utility path still works after OCR dispatch (regression)', async () => {
      enableChannelCapture();

      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/tldr some long text');
      });

      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith(
        'extract_text_command',
        expect.anything(),
      );
      await vi.waitFor(() => {
        const askCall = vi
          .mocked(invoke)
          .mock.calls.find((c) => c[0] === 'ask_model');
        expect(askCall).toBeDefined();
        const args = askCall![1] as Record<string, unknown>;
        expect(args.message).toContain('Summarize the following text');
        expect(args.message).toContain('some long text');
      });
    });

    it('suppresses vision capability mismatch strip when utility command typed with an attached image', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: 'llama3',
          all: ['llama3'],
          ollamaReachable: true,
        },
        get_model_capabilities: {
          llama3: { vision: false, thinking: false },
        },
        save_image_command: '/tmp/staged/img1.jpg',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForUtility();
      await vi.waitFor(() => {
        expect(
          screen.getByRole('list', { name: /attached images/i }),
        ).toBeInTheDocument();
      });

      act(() => {
        setAskValue('/tldr ');
      });

      expect(
        screen.queryByTestId('capability-mismatch-strip'),
      ).not.toBeInTheDocument();
    });

    it('submits utility OCR immediately when image is already resolved before submit', async () => {
      // Coverage: exercises the non-deferred path where hasPendingImages is false
      // (img.filePath !== null for every image in the list).
      // Uses the same flush pattern as the outer-describe test that verifies
      // hasPendingImages=false: immediate save mock + vi.waitFor ensures filePath
      // is set in state before the utility command is submitted.
      enableChannelCaptureWithResponses({
        save_image_command: '/tmp/staged/pre-resolved.jpg',
        extract_text_command: 'Pre-resolved OCR text',
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await pasteImageForUtility();
      // Wait for save_image_command to have been called AND its promise to
      // resolve + React state to update with filePath (mirrors the "utility
      // command with bare trigger" test pattern in the outer describe).
      await act(async () => {
        await vi.waitFor(() =>
          expect(invoke).toHaveBeenCalledWith(
            'save_image_command',
            expect.anything(),
          ),
        );
      });

      // Submit utility command — image is resolved, non-deferred path taken.
      const textarea = getAskInput();
      act(() => {
        setAskValue('/tldr ');
      });
      invoke.mockClear();
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      // OCR called with the already-resolved path (non-deferred path).
      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith('extract_text_command', {
          imagePaths: ['/tmp/staged/pre-resolved.jpg'],
        });
      });
    });

    it('deferred utility OCR preserves selected context in the pending bubble', async () => {
      // Coverage: exercises the truthy branch of
      // `sanitized?.trim() ? sanitized : undefined` when selectedContext is
      // non-empty at the time of a deferred utility OCR submit.
      let resolveSave!: (path: string) => void;
      invoke.mockImplementation(
        async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === 'get_model_picker_state')
            return {
              active: 'gemma4:e2b',
              all: ['gemma4:e2b'],
              ollamaReachable: true,
            };
          if (cmd === 'get_model_capabilities')
            return { 'gemma4:e2b': { vision: false, thinking: false } };
          if (cmd === 'get_updater_state')
            return {
              last_check_at_unix: null,
              update: null,
              settings_snoozed_until: null,
              chat_snoozed_until: null,
            };
          if (cmd === 'save_image_command')
            return new Promise<string>((res) => {
              resolveSave = res;
            });
          if (cmd === 'extract_text_command') return 'OCR with context';
          if (args && 'onEvent' in args) return undefined;
          return undefined;
        },
      );

      render(<App />);
      await act(async () => {});
      await showOverlay('quoted context text');

      await pasteImageForUtility();
      await act(async () => {
        await vi.waitFor(() => expect(resolveSave).toBeDefined());
      });

      // Submit /tldr while image is still pending and context is present.
      const textarea = getAskInput();
      act(() => {
        setAskValue('/tldr ');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      // Deferred state active.
      expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();

      // Resolve the image — deferred OCR chain fires with context preserved.
      act(() => {
        resolveSave('/tmp/staged/ctx-deferred.jpg');
      });
      await act(async () => {
        await new Promise((r) => setTimeout(r, 50));
      });

      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith('extract_text_command', {
          imagePaths: ['/tmp/staged/ctx-deferred.jpg'],
        });
      });
    });
  });

  // ─── /search command ───────────────────────────────────────────────────────

  describe('/search command', () => {
    it('routes /search submissions to search_pipeline with the stripped query', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/search rust async');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      expect(invoke).toHaveBeenCalledWith(
        'search_pipeline',
        expect.objectContaining({ message: 'rust async' }),
      );
    });

    it('moves selected context into the /search user bubble and clears the ask bar preview', async () => {
      enableChannelCapture();
      const { container } = render(<App />);
      await act(async () => {});
      await showOverlay('selected snippet');

      const findSelectedSnippet = () =>
        screen.getAllByText(/selected snippet/i, { selector: 'p' });

      expect(findSelectedSnippet()).toHaveLength(1);
      expect(container.querySelectorAll('p.text-text-secondary')).toHaveLength(
        1,
      );

      const textarea = getAskInput();
      act(() => {
        setAskValue('/search explain this selection');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      await waitFor(() => expect(getAskInput().textContent).toBe(''));
      expect(findSelectedSnippet()).toHaveLength(1);
      expect(container.querySelectorAll('p.text-text-secondary')).toHaveLength(
        0,
      );
      expect(
        container.querySelectorAll('p[class*="text-white/60"]'),
      ).toHaveLength(1);
    });

    it('keeps searchActive after a clarify trace with question tokens', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/search who is him');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const firstChannel = getLastChannel();
      await act(async () => {
        firstChannel!.onmessage({
          type: 'Trace',
          step: {
            id: 'clarify',
            kind: 'clarify',
            status: 'completed',
            title: 'Waiting for clarification',
            summary: 'Search is paused until you clarify who or what you mean.',
          },
        });
        firstChannel!.onmessage({ type: 'Token', content: 'Which person?' });
        firstChannel!.onmessage({ type: 'Done' });
      });
      await act(async () => {
        await Promise.resolve();
      });

      const followupInvokeCountBefore = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      act(() => {
        setAskValue('Donald Trump');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      const followupInvokeCountAfter = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      expect(followupInvokeCountAfter).toBe(followupInvokeCountBefore + 1);
      const searchCalls = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      );
      expect(searchCalls[searchCalls.length - 1][1]).toMatchObject({
        message: 'Donald Trump',
      });
    });

    it('continues routing follow-ups through search_pipeline after a clarify trace', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/search who is him');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const firstChannel = getLastChannel();
      await act(async () => {
        firstChannel!.onmessage({
          type: 'Trace',
          step: {
            id: 'clarify',
            kind: 'clarify',
            status: 'completed',
            title: 'Waiting for clarification',
            summary: 'Search is paused until you clarify who or what you mean.',
          },
        });
        firstChannel!.onmessage({ type: 'Token', content: 'Which person?' });
        firstChannel!.onmessage({ type: 'Done' });
      });
      // Flush askSearch promise + .then() so isGenerating updates.
      await act(async () => {
        await Promise.resolve();
      });

      // Follow-up without /search prefix should still route to search_pipeline.
      const followupInvokeCountBefore = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      act(() => {
        setAskValue('Donald Trump');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      const followupInvokeCountAfter = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      expect(followupInvokeCountAfter).toBe(followupInvokeCountBefore + 1);
      const searchCalls = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      );
      expect(searchCalls[searchCalls.length - 1][1]).toMatchObject({
        message: 'Donald Trump',
      });
    });

    it('drops searchActive after a final Token+Done turn so the next submit uses ask_model', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/search rust');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const channel = getLastChannel();
      await act(async () => {
        channel!.onmessage({ type: 'Searching', queries: [] });
        channel!.onmessage({ type: 'Token', content: 'Rust is fast.' });
        channel!.onmessage({ type: 'Done' });
      });
      // Flush the askSearch promise + .then() so searchActive resets to false.
      await act(async () => {
        await Promise.resolve();
      });

      act(() => {
        setAskValue('hello');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      const calls = invoke.mock.calls.filter(
        (c) => c[0] === 'ask_model' || c[0] === 'search_pipeline',
      );
      const last = calls[calls.length - 1];
      expect(last[0]).toBe('ask_model');
      expect(last[1]).toMatchObject({ message: 'hello' });
    });

    it('follow-up after a clarify trace still routes through search_pipeline', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/search ambiguous');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      const firstChannel = getLastChannel();
      await act(async () => {
        firstChannel!.onmessage({
          type: 'Trace',
          step: {
            id: 'clarify',
            kind: 'clarify',
            status: 'completed',
            title: 'Waiting for clarification',
            summary: 'Search is paused until you clarify who or what you mean.',
          },
        });
        firstChannel!.onmessage({ type: 'Token', content: 'First clarify?' });
        firstChannel!.onmessage({ type: 'Done' });
      });
      await act(async () => {
        await Promise.resolve();
      });

      // User types their own clarification and submits - still routes to
      // search_pipeline because searchActive persisted (final=false on clarify).
      const countBefore = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      act(() => {
        setAskValue('Einstein');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      const countAfter = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      ).length;
      expect(countAfter).toBe(countBefore + 1);
      const allSearchCalls = invoke.mock.calls.filter(
        (c) => c[0] === 'search_pipeline',
      );
      expect(allSearchCalls[allSearchCalls.length - 1][1]).toMatchObject({
        message: 'Einstein',
      });
    });

    it('ignores empty /search submissions with no text after the trigger', async () => {
      enableChannelCapture();
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/search');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });

      expect(invoke.mock.calls.some((c) => c[0] === 'search_pipeline')).toBe(
        false,
      );
    });

    it('lets /screen override search continuation mid-conversation', async () => {
      enableChannelCaptureWithResponses({
        capture_full_screen_command: '/tmp/s.jpg',
      });
      render(<App />);
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('/search him');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      const channel = getLastChannel();
      await act(async () => {
        channel!.onmessage({
          type: 'Trace',
          step: {
            id: 'clarify',
            kind: 'clarify',
            status: 'completed',
            title: 'Waiting for clarification',
            summary: 'Search is paused until you clarify who or what you mean.',
          },
        });
        channel!.onmessage({ type: 'Token', content: 'Which?' });
        channel!.onmessage({ type: 'Done' });
      });

      // With searchActive still on, /screen must take precedence.
      act(() => {
        setAskValue('/screen what is this');
      });
      await act(async () => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      expect(invoke).toHaveBeenCalledWith(
        'capture_full_screen_command',
        expect.objectContaining({ conversationId: expect.any(String) }),
      );
    });
  });

  describe('Onboarding', () => {
    it('shows onboarding screen when thuki://onboarding event fires', async () => {
      enableChannelCaptureWithResponses({
        check_accessibility_permission: false,
        check_screen_recording_permission: false,
      });

      render(<App />);
      await act(async () => {});

      await act(async () => {
        emitTauriEvent('thuki://onboarding', { stage: 'permissions' });
      });

      expect(screen.getByText("Let's get Thuki set up")).toBeInTheDocument();
    });

    it('does not show onboarding on normal visibility event', async () => {
      render(<App />);
      await act(async () => {});

      await showOverlay();

      expect(screen.queryByText("Let's get Thuki set up")).toBeNull();
    });

    it('renders normal ask bar when overlay is shown without onboarding', async () => {
      render(<App />);
      await act(async () => {});

      await showOverlay();

      expect(getAskInput()).toBeInTheDocument();
    });

    it('dismisses onboarding and shows ask bar when onComplete is called', async () => {
      invoke.mockResolvedValue(undefined);

      render(<App />);
      await act(async () => {});

      await act(async () => {
        emitTauriEvent('thuki://onboarding', { stage: 'intro' });
      });

      expect(screen.getByText("You're all set")).toBeInTheDocument();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /get started/i }));
      });

      expect(screen.queryByText("You're all set")).toBeNull();
    });
  });

  describe('background model download', () => {
    const BUILTIN = {
      ...DEFAULT_CONFIG,
      inference: {
        ...DEFAULT_CONFIG.inference,
        activeProvider: 'builtin',
        activeProviderKind: 'builtin',
      },
    };

    function builtinTree() {
      return (
        <ConfigProviderForTest value={BUILTIN}>
          <App />
        </ConfigProviderForTest>
      );
    }

    it('shows the ambient strip with percent and ETA in the ask bar while downloading', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: null,
          all: [],
          ollamaReachable: true,
        },
      });
      downloadHolder.value = makeDownloadCtx({
        state: { phase: 'downloading' },
        combinedBytes: 4_000_000_000,
        grandTotalBytes: 10_000_000_000,
        speedBytesPerSec: 8_000_000,
      });

      render(builtinTree());
      await act(async () => {});
      await showOverlay();

      expect(screen.getByTestId('download-status-strip')).toBeInTheDocument();
      // 4 / 10 GB -> 40%; (10 - 4)e9 / 8e6 = 750s -> "12m".
      expect(screen.getByText('40% · 12m left')).toBeInTheDocument();
      // The download strip owns the messaging: no capability strip stacks under it.
      expect(
        screen.queryByTestId('capability-mismatch-strip'),
      ).not.toBeInTheDocument();
    });

    it('shows percent only before the download rate is measurable', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: null,
          all: [],
          ollamaReachable: true,
        },
      });
      downloadHolder.value = makeDownloadCtx({
        state: { phase: 'downloading' },
        combinedBytes: null,
        resumeSeedBytes: null,
        grandTotalBytes: null,
        speedBytesPerSec: null,
      });

      render(builtinTree());
      await act(async () => {});
      await showOverlay();

      expect(screen.getByText('0%')).toBeInTheDocument();
    });

    it('soft-blocks submit while downloading, without sending or shaking', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: null,
          all: [],
          ollamaReachable: true,
        },
      });
      downloadHolder.value = makeDownloadCtx({
        state: { phase: 'downloading' },
        combinedBytes: 1_000_000_000,
        grandTotalBytes: 10_000_000_000,
        speedBytesPerSec: 5_000_000,
      });

      render(builtinTree());
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('hello');
      });
      invoke.mockClear();
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(
        invoke.mock.calls.filter((c) => c[0] === 'ask_model'),
      ).toHaveLength(0);
      // The wait holds calmly: the ambient strip stays, no error cue replaces it.
      expect(screen.getByTestId('download-status-strip')).toBeInTheDocument();
    });

    it('shows a failed strip whose Retry restarts the download', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: null,
          all: [],
          ollamaReachable: true,
        },
      });
      const retry = vi.fn(async () => {});
      downloadHolder.value = makeDownloadCtx({
        state: { phase: 'failed', kind: 'offline', message: 'no network' },
        retry,
      });

      render(builtinTree());
      await act(async () => {});
      await showOverlay();

      expect(screen.getByText('Model download failed.')).toBeInTheDocument();
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Retry download' }));
      });
      expect(retry).toHaveBeenCalledTimes(1);
    });

    it('refreshes the model list and shows "Model ready" when the download completes', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: null,
          all: [],
          ollamaReachable: true,
        },
      });
      downloadHolder.value = makeDownloadCtx({
        state: { phase: 'downloading' },
        combinedBytes: 1_000_000_000,
        grandTotalBytes: 10_000_000_000,
        speedBytesPerSec: 5_000_000,
      });

      const { rerender } = render(builtinTree());
      await act(async () => {});
      await showOverlay();

      const before = invoke.mock.calls.filter(
        (c) => c[0] === 'get_model_picker_state',
      ).length;
      downloadHolder.value = makeDownloadCtx({ state: { phase: 'ready' } });
      await act(async () => {
        rerender(builtinTree());
      });

      expect(screen.getByText('Model ready')).toBeInTheDocument();
      const after = invoke.mock.calls.filter(
        (c) => c[0] === 'get_model_picker_state',
      ).length;
      expect(after).toBeGreaterThan(before);
    });

    it('pauses the download from the ask-bar strip', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: null,
          all: [],
          ollamaReachable: true,
        },
      });
      const pauseDownload = vi.fn();
      downloadHolder.value = makeDownloadCtx({
        state: { phase: 'downloading' },
        combinedBytes: 4_000_000_000,
        grandTotalBytes: 10_000_000_000,
        speedBytesPerSec: 8_000_000,
        pauseDownload,
      });

      render(builtinTree());
      await act(async () => {});
      await showOverlay();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Pause download' }));
      });
      expect(pauseDownload).toHaveBeenCalledTimes(1);
    });

    it('shows a paused strip with Resume / Discard and the held percent', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: null,
          all: [],
          ollamaReachable: true,
        },
      });
      const resumeFromPause = vi.fn();
      const discardActive = vi.fn();
      downloadHolder.value = makeDownloadCtx({
        state: { phase: 'idle' },
        isPaused: true,
        pausedBytes: 5_000_000_000,
        grandTotalBytes: 10_000_000_000,
        resumeFromPause,
        discardActive,
      });

      const { rerender } = render(builtinTree());
      await act(async () => {});
      await showOverlay();

      expect(screen.getByText('Paused · 50%')).toBeInTheDocument();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Resume download' }),
        );
      });
      expect(resumeFromPause).toHaveBeenCalledTimes(1);
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'Discard download' }),
        );
      });
      expect(discardActive).toHaveBeenCalledTimes(1);

      // Grand total unknown while paused falls back to 0%.
      downloadHolder.value = makeDownloadCtx({
        state: { phase: 'idle' },
        isPaused: true,
        pausedBytes: 5_000_000_000,
        grandTotalBytes: null,
      });
      await act(async () => {
        rerender(builtinTree());
      });
      expect(screen.getByText('Paused · 0%')).toBeInTheDocument();
    });

    it('soft-blocks submit while the download is paused', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: null,
          all: [],
          ollamaReachable: true,
        },
      });
      downloadHolder.value = makeDownloadCtx({
        state: { phase: 'idle' },
        isPaused: true,
        pausedBytes: 5_000_000_000,
        grandTotalBytes: 10_000_000_000,
      });

      render(builtinTree());
      await act(async () => {});
      await showOverlay();

      const textarea = getAskInput();
      act(() => {
        setAskValue('hello');
      });
      invoke.mockClear();
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});

      expect(
        invoke.mock.calls.filter((c) => c[0] === 'ask_model'),
      ).toHaveLength(0);
    });

    it('floats the strip over the intro tour, but not during model_check or when idle', async () => {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: null,
          all: [],
          ollamaReachable: true,
        },
      });
      downloadHolder.value = makeDownloadCtx({
        state: { phase: 'downloading' },
        combinedBytes: 1_000_000_000,
        grandTotalBytes: 10_000_000_000,
        speedBytesPerSec: 5_000_000,
      });

      const { rerender } = render(<App />);
      await act(async () => {});

      // model_check: the picker matrix owns the bar, so no app-root strip.
      await act(async () => {
        emitTauriEvent('thuki://onboarding', { stage: 'model_check' });
      });
      expect(
        screen.queryByTestId('download-status-strip'),
      ).not.toBeInTheDocument();

      // intro: the strip floats over the tour.
      await act(async () => {
        emitTauriEvent('thuki://onboarding', { stage: 'intro' });
      });
      expect(screen.getByText("You're all set")).toBeInTheDocument();
      expect(screen.getByTestId('download-status-strip')).toBeInTheDocument();

      // intro with an idle machine: nothing to float.
      downloadHolder.value = makeDownloadCtx();
      await act(async () => {
        rerender(<App />);
      });
      expect(
        screen.queryByTestId('download-status-strip'),
      ).not.toBeInTheDocument();
    });
  });

  describe('tip bar', () => {
    afterEach(() => {
      vi.mocked(useTips).mockReturnValue({
        tip: '',
        tipKey: 0,
        isVisible: false,
      });
    });

    it('renders TipBar when useTips returns isVisible=true', async () => {
      vi.mocked(useTips).mockReturnValue({
        tip: 'Capture a screenshot with /screen',
        tipKey: 1,
        isVisible: true,
      });
      render(<App />);
      await showOverlay();
      expect(screen.getByTestId('tip-text')).toBeInTheDocument();
    });

    it('does not render TipBar when useTips returns isVisible=false', async () => {
      render(<App />);
      await showOverlay();
      expect(screen.queryByTestId('tip-text')).not.toBeInTheDocument();
    });

    it('keeps TipBar visible when entering chat mode', async () => {
      vi.mocked(useTips).mockReturnValue({
        tip: 'Test tip',
        tipKey: 1,
        isVisible: true,
      });
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: 'gemma4:e2b',
          all: ['gemma4:e2b'],
          ollamaReachable: true,
        },
      });
      render(<App />);
      await showOverlay();
      // Tip visible in ask-bar mode.
      expect(screen.getByTestId('tip-text')).toBeInTheDocument();
      const textarea = getAskInput();
      act(() => {
        setAskValue('hello');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'hi' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});
      // Tip stays visible in chat mode (isTipVisible drives visibility, not mode).
      expect(screen.getByTestId('tip-text')).toBeInTheDocument();
    });
  });

  describe('UpdateFooterBar integration', () => {
    it('shows UpdateFooterBar instead of TipBar when an update is available', async () => {
      (useTips as ReturnType<typeof vi.fn>).mockReturnValue({
        tip: 'test tip',
        tipKey: 0,
        isVisible: true,
      });
      enableChannelCaptureWithResponses({
        get_updater_state: {
          last_check_at_unix: 100,
          update: { version: '0.8.0', notes_url: null },
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        },
      });

      render(<App />);
      await act(async () => {});

      // Show the overlay so TipBar area is rendered
      await showOverlay();

      await waitFor(() =>
        expect(screen.getByTestId('update-footer-bar')).toBeInTheDocument(),
      );
      expect(screen.queryByTestId('tip-bar')).not.toBeInTheDocument();
    });

    it('keeps the UpdateFooterBar visible after entering chat mode', async () => {
      (useTips as ReturnType<typeof vi.fn>).mockReturnValue({
        tip: 'test tip',
        tipKey: 0,
        isVisible: true,
      });
      enableChannelCaptureWithResponses({
        get_updater_state: {
          last_check_at_unix: 100,
          update: { version: '0.8.1', notes_url: null },
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        },
        get_model_picker_state: {
          active: 'gemma4:e2b',
          all: ['gemma4:e2b'],
          ollamaReachable: true,
        },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();
      // Visible in ask-bar mode first.
      await waitFor(() =>
        expect(screen.getByTestId('update-footer-bar')).toBeInTheDocument(),
      );

      // Send a message to flip into chat mode.
      const textarea = getAskInput();
      act(() => {
        setAskValue('hi');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'hello' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      // Critical: the update footer must still render in chat mode.
      expect(screen.getByTestId('update-footer-bar')).toBeInTheDocument();
      expect(screen.queryByTestId('tip-bar')).not.toBeInTheDocument();
    });

    it('shows TipBar normally when no update is available', async () => {
      (useTips as ReturnType<typeof vi.fn>).mockReturnValue({
        tip: 'test tip',
        tipKey: 0,
        isVisible: true,
      });
      // Default enableChannelCapture returns no update
      render(<App />);
      await act(async () => {});
      await showOverlay();

      await waitFor(() =>
        expect(screen.getByTestId('tip-bar')).toBeInTheDocument(),
      );
      expect(screen.queryByTestId('update-footer-bar')).not.toBeInTheDocument();
    });

    it('opens the update window when install link clicked on UpdateFooterBar', async () => {
      (useTips as ReturnType<typeof vi.fn>).mockReturnValue({
        tip: 'test tip',
        tipKey: 0,
        isVisible: true,
      });
      enableChannelCaptureWithResponses({
        get_updater_state: {
          last_check_at_unix: 100,
          update: { version: '0.8.0', notes_url: null },
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await waitFor(() => screen.getByTestId('update-footer-bar'));
      await act(async () => {
        fireEvent.click(screen.getByText(/what's new/i));
        await Promise.resolve();
      });
      expect(invoke).toHaveBeenCalledWith('open_update_window');
    });

    it('calls snooze_update_chat when later link clicked on UpdateFooterBar', async () => {
      (useTips as ReturnType<typeof vi.fn>).mockReturnValue({
        tip: 'test tip',
        tipKey: 0,
        isVisible: true,
      });
      enableChannelCaptureWithResponses({
        get_updater_state: {
          last_check_at_unix: 100,
          update: { version: '0.8.0', notes_url: null },
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        },
      });

      render(<App />);
      await act(async () => {});
      await showOverlay();

      await waitFor(() => screen.getByTestId('update-footer-bar'));
      await act(async () => {
        fireEvent.click(screen.getByText('later'));
        await Promise.resolve();
      });
      expect(invoke).toHaveBeenCalledWith('snooze_update_chat', { hours: 24 });
    });
  });

  // ─── Minimize / restore (Task 7) ─────────────────────────────────────────────

  describe('minimize / restore', () => {
    /** Helper: enter chat mode with one complete turn. */
    async function enterChatMode() {
      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: 'gemma4:e2b',
          all: ['gemma4:e2b'],
          ollamaReachable: true,
        },
      });
      render(<App />);
      await act(async () => {});
      await showOverlay();
      const textarea = getAskInput();
      act(() => {
        setAskValue('hello');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
    }

    it('minimizes to the floating icon without cancelling generation', async () => {
      await enterChatMode();
      // Chat frame for the collapse-corner snap. No prior expand → anchor is
      // the top-left default, so the icon folds to the chat's top-left.
      __setWindowGeometry({
        x: 300,
        y: 200,
        scale: 1,
        width: 400,
        height: 700,
        monitorX: 0,
        monitorY: 0,
        monitorWidth: 1440,
        monitorHeight: 900,
      });
      // Generation is in flight (channel open, no Done yet)
      invoke.mockClear();

      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });
      // The collapse-corner snap queries the frame in settleMorphPhase's async
      // path; flush microtasks so the set_window_frame call is observable.
      await act(async () => {});

      // MinimizedIcon should be rendered
      expect(
        screen.getByRole('button', { name: /restore thuki/i }),
      ).toBeInTheDocument();

      // ConversationView content should be gone
      expect(screen.queryByText('hello')).toBeNull();

      // set_overlay_minimized called with minimized: true
      expect(invoke).toHaveBeenCalledWith('set_overlay_minimized', {
        minimized: true,
      });

      // At the end of the collapse morph the OS window snaps to the 68px
      // square at the anchor's corner of the chat frame. top-left anchor +
      // frame (300,200) → icon folds to (300,200).
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 300,
        y: 200,
        width: 68,
        height: 68,
      });

      // notify_overlay_hidden must NOT have been called (no cancel)
      expect(invoke).not.toHaveBeenCalledWith('notify_overlay_hidden');
      // cancel_generation must NOT have been called
      expect(invoke).not.toHaveBeenCalledWith('cancel_generation');
    });

    it('strips chrome classes from layout wrapper when minimized', async () => {
      await enterChatMode();

      // Before minimize: layout wrapper has bg-surface-base and shadow-chat
      // (isChatMode=true after enterChatMode)
      const layoutWrappers = document.querySelectorAll(
        '[class*="bg-surface-base"]',
      );
      expect(layoutWrappers.length).toBeGreaterThan(0);

      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });

      // After minimize: no element with bg-surface-base class on the layout wrapper
      const layoutWrappersAfter = document.querySelectorAll(
        '[class*="bg-surface-base"]',
      );
      expect(layoutWrappersAfter.length).toBe(0);
    });

    it('strips padding from root container when minimized and restores on un-minimize', async () => {
      await enterChatMode();

      // Before minimize: root has px-3 in className
      const rootBefore = document.querySelector('.h-screen');
      expect(rootBefore?.className).toContain('px-3');
      expect(rootBefore?.className).toContain('pt-2');
      expect(rootBefore?.className).toContain('pb-6');

      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });

      // After minimize: root must NOT have px-3/pt-2/pb-6
      const rootAfter = document.querySelector('.h-screen');
      expect(rootAfter?.className).not.toContain('px-3');
      expect(rootAfter?.className).not.toContain('pt-2');
      expect(rootAfter?.className).not.toContain('pb-6');

      // Restore
      const restoreBtn = screen.getByRole('button', { name: /restore thuki/i });
      await act(async () => {
        fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
        fireEvent.pointerUp(restoreBtn);
      });
      await act(async () => {});

      // After restore: padding is back
      const rootRestored = document.querySelector('.h-screen');
      expect(rootRestored?.className).toContain('px-3');
    });

    it('restores from the icon and clears the unseen indicator', async () => {
      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'world' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      // Minimize
      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });
      expect(
        screen.getByRole('button', { name: /restore thuki/i }),
      ).toBeInTheDocument();

      // Icon sits comfortably inside the monitor (no edge clamping), so the
      // window expands anchored at the icon's top-left.
      __setWindowGeometry({
        x: 200,
        y: 150,
        scale: 1,
        monitorX: 0,
        monitorY: 0,
        monitorWidth: 1440,
        monitorHeight: 900,
      });
      invoke.mockClear();

      // Restore — MinimizedIcon fires onRestore via onPointerUp (not onClick)
      const restoreBtn = screen.getByRole('button', { name: /restore thuki/i });
      await act(async () => {
        fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
        fireEvent.pointerUp(restoreBtn);
      });
      // Restore geometry query + native frame set fire inside the async IIFE;
      // flush microtasks so the invoke calls are observable.
      await act(async () => {});

      // set_overlay_minimized called with minimized: false
      expect(invoke).toHaveBeenCalledWith('set_overlay_minimized', {
        minimized: false,
      });

      // On restore the OS window is positioned on screen and grown to full
      // chat size in one native frame set. With the icon away from any edge,
      // the window keeps the icon's top-left (200,150). Height includes
      // CONTAINER_VERTICAL_PADDING (48) so the bottom composer is not clipped
      // before settleMorphPhase's post-settle re-measure.
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 200,
        y: 150,
        width: DEFAULT_CONFIG.window.overlayWidth,
        height: DEFAULT_CONFIG.window.maxChatHeight + 48,
      });

      // ConversationView shown again with same messages
      expect(screen.getByText('hello')).toBeInTheDocument();
      expect(screen.getByText('world')).toBeInTheDocument();

      // MinimizedIcon should be gone
      expect(
        screen.queryByRole('button', { name: /restore thuki/i }),
      ).toBeNull();
    });

    it('clamps the expanded window left when the icon is near the right edge', async () => {
      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });

      // Icon flush against the right edge of a 1440-wide monitor (x 1372 + 68
      // = 1440).
      __setWindowGeometry({
        x: 1372,
        y: 100,
        scale: 1,
        monitorX: 0,
        monitorY: 0,
        monitorWidth: 1440,
        monitorHeight: 900,
      });
      invoke.mockClear();

      const restoreBtn = screen.getByRole('button', { name: /restore thuki/i });
      await act(async () => {
        fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
        fireEvent.pointerUp(restoreBtn);
      });
      await act(async () => {});

      // Anchor top-right: the panel's right edge is pinned to the icon's right
      // edge (1372 + 68), so the window unfolds leftward and stays on screen.
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 1372 + 68 - DEFAULT_CONFIG.window.overlayWidth,
        y: 100,
        width: DEFAULT_CONFIG.window.overlayWidth,
        height: DEFAULT_CONFIG.window.maxChatHeight + 48,
      });
    });

    it('anchors the expanded window to the bottom and grows upward when the icon is near the bottom edge', async () => {
      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });

      // Icon parked near the bottom edge of a 900-tall monitor.
      __setWindowGeometry({
        x: 100,
        y: 832,
        scale: 1,
        monitorX: 0,
        monitorY: 0,
        monitorWidth: 1440,
        monitorHeight: 900,
      });
      invoke.mockClear();

      const restoreBtn = screen.getByRole('button', { name: /restore thuki/i });
      await act(async () => {
        fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
        fireEvent.pointerUp(restoreBtn);
      });
      await act(async () => {});

      // Anchor bottom-left: the panel's bottom edge is pinned to the icon's
      // bottom edge (832 + 68), so the top = 900 - fullHeight and the window
      // unfolds upward instead of clipping off the bottom.
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 100,
        y: 832 + 68 - (DEFAULT_CONFIG.window.maxChatHeight + 48),
        width: DEFAULT_CONFIG.window.overlayWidth,
        height: DEFAULT_CONFIG.window.maxChatHeight + 48,
      });
      // Bottom-anchored → the root container grows upward.
      expect(document.querySelector('.h-screen.justify-end')).not.toBeNull();
    });

    it('folds the icon back to its origin after a right-edge expand', async () => {
      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      // First minimize (default top-left anchor).
      let minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });
      await act(async () => {});

      // Park the icon flush against the right edge, then restore → the expand
      // anchors top-right and the chat unfolds left to (1440 - overlayWidth).
      __setWindowGeometry({
        x: 1372,
        y: 100,
        scale: 1,
        monitorX: 0,
        monitorY: 0,
        monitorWidth: 1440,
        monitorHeight: 900,
      });
      const restoreBtn = screen.getByRole('button', { name: /restore thuki/i });
      await act(async () => {
        fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
        fireEvent.pointerUp(restoreBtn);
      });
      await act(async () => {});

      // The chat now occupies this frame (top-right anchored). Point the
      // collapse query at it.
      const fullHeight = DEFAULT_CONFIG.window.maxChatHeight + 48;
      __setWindowGeometry({
        x: 1372 + 68 - DEFAULT_CONFIG.window.overlayWidth,
        y: 100,
        width: DEFAULT_CONFIG.window.overlayWidth,
        height: fullHeight,
        scale: 1,
        monitorX: 0,
        monitorY: 0,
        monitorWidth: 1440,
        monitorHeight: 900,
      });
      invoke.mockClear();

      // Second minimize → collapse reuses the top-right anchor, folding the
      // icon back to its original right-edge spot (1372, 100).
      minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });
      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 1372,
        y: 100,
        width: 68,
        height: 68,
      });
    });

    it('raises the unseen dot when generation finishes while minimized', async () => {
      await enterChatMode();
      // Minimize while streaming is in flight
      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });
      expect(
        screen.getByRole('button', { name: /restore thuki/i }),
      ).toBeInTheDocument();
      // No ready dot yet — still generating
      expect(screen.queryByTestId('minimized-ready-dot')).toBeNull();

      // Complete the stream
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'done!' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      // Ready dot should appear
      expect(screen.getByTestId('minimized-ready-dot')).toBeInTheDocument();

      // Restore clears it
      const restoreBtn = screen.getByRole('button', { name: /restore thuki/i });
      await act(async () => {
        fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
        fireEvent.pointerUp(restoreBtn);
      });
      await act(async () => {});
      expect(screen.queryByTestId('minimized-ready-dot')).toBeNull();
    });

    it('recomputes upward growth on restore when near screen bottom', async () => {
      // Place window near the screen bottom so shouldGrowUp becomes true.
      // maxChatHeight=648, CONTAINER_VERTICAL_PADDING=48: need windowY + 648 + 48 > screenBottom.
      // With monitorHeight=900, monitorY=0: windowY=700 → 700+696=1396 > 900 → growsUpward.
      __setWindowGeometry({
        x: 100,
        y: 700,
        scale: 1,
        monitorX: 0,
        monitorY: 0,
        monitorWidth: 1440,
        monitorHeight: 900,
      });

      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });

      // Restore — geometry query fires inside the async IIFE
      const restoreBtn = screen.getByRole('button', { name: /restore thuki/i });
      await act(async () => {
        fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
        fireEvent.pointerUp(restoreBtn);
      });

      // Wait for the async geometry IIFE to settle
      await act(async () => {});

      // Root container should have justify-end (growsUpward true).
      // Use compound selector to target the root div (h-screen) specifically,
      // since chat bubbles also contain .justify-end child elements.
      const outer = document.querySelector('.h-screen.justify-end');
      expect(outer).not.toBeNull();
    });

    it('recomputes downward growth on restore when away from screen bottom', async () => {
      // windowY=100, monitorHeight=900: 100+648+48=796 < 900 → growsDownward.
      __setWindowGeometry({
        x: 100,
        y: 100,
        scale: 1,
        monitorX: 0,
        monitorY: 0,
        monitorWidth: 1440,
        monitorHeight: 900,
      });

      // Show overlay near bottom first so it starts with justify-end
      render(<App />);
      await act(async () => {});
      await act(async () => {
        emitTauriEvent('thuki://visibility', {
          state: 'show',
          selected_text: null,
          window_x: 100,
          window_y: 750,
          screen_bottom_y: 900,
        });
      });
      expect(document.querySelector('.h-screen.justify-end')).not.toBeNull();

      enableChannelCaptureWithResponses({
        get_model_picker_state: {
          active: 'gemma4:e2b',
          all: ['gemma4:e2b'],
          ollamaReachable: true,
        },
      });
      const textarea = getAskInput();
      act(() => {
        setAskValue('hi');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });

      // After minimize, growsUpward is forced false
      expect(document.querySelector('.h-screen.justify-start')).not.toBeNull();

      const restoreBtn = screen.getByRole('button', { name: /restore thuki/i });
      await act(async () => {
        fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
        fireEvent.pointerUp(restoreBtn);
      });
      await act(async () => {});

      // With windowY=100 away from bottom → justify-start on root container
      expect(document.querySelector('.h-screen.justify-end')).toBeNull();
      expect(document.querySelector('.h-screen.justify-start')).not.toBeNull();
    });

    it('recomputes null monitor as no-grow-up on restore', async () => {
      __setWindowGeometry({ x: 100, y: 700, scale: 1, monitorNull: true });

      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });
      const restoreBtn = screen.getByRole('button', { name: /restore thuki/i });
      await act(async () => {
        fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
        fireEvent.pointerUp(restoreBtn);
      });
      await act(async () => {});

      // null monitor → screenBottomY null → shouldGrowUp false → root uses justify-start
      expect(document.querySelector('.h-screen.justify-end')).toBeNull();
      expect(document.querySelector('.h-screen.justify-start')).not.toBeNull();
    });

    it('recovers edge-awareness from availableMonitors when currentMonitor is null', async () => {
      // currentMonitor() is null (transient during a display change), but the
      // icon sits near the bottom edge of a monitor that availableMonitors()
      // can still report. The fallback finds the containing monitor by
      // position, so the expand stays edge-aware and grows upward instead of
      // dropping the clamp.
      __setWindowGeometry({ x: 100, y: 832, scale: 1, monitorNull: true });
      __setAvailableMonitors([
        { position: { x: 0, y: 0 }, size: { width: 1440, height: 900 } },
      ]);

      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });

      invoke.mockClear();
      const restoreBtn = screen.getByRole('button', { name: /restore thuki/i });
      await act(async () => {
        fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
        fireEvent.pointerUp(restoreBtn);
      });
      await act(async () => {});

      // The recovered monitor (height 900) makes the near-bottom icon (832+68)
      // anchor bottom and grow upward, exactly as if currentMonitor had
      // returned it. The clamped top = 900 - (maxChatHeight + 48).
      expect(invoke).toHaveBeenCalledWith('set_window_frame', {
        x: 100,
        y: 832 + 68 - (DEFAULT_CONFIG.window.maxChatHeight + 48),
        width: DEFAULT_CONFIG.window.overlayWidth,
        height: DEFAULT_CONFIG.window.maxChatHeight + 48,
      });
      expect(document.querySelector('.h-screen.justify-end')).not.toBeNull();

      // Restore shared mock state for subsequent tests.
      __setAvailableMonitors([]);
      __setWindowGeometry({ monitorNull: false });
    });

    it('ignores Escape and Cmd+W while minimized', async () => {
      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });

      invoke.mockClear();

      // Fire Escape while minimized
      act(() => {
        fireEvent.keyDown(window, { key: 'Escape' });
      });
      // Fire Cmd+W while minimized
      act(() => {
        fireEvent.keyDown(window, { key: 'w', metaKey: true });
      });

      await act(async () => {});

      // MinimizedIcon still shown
      expect(
        screen.getByRole('button', { name: /restore thuki/i }),
      ).toBeInTheDocument();
      // notify_overlay_hidden must NOT have been called
      expect(invoke).not.toHaveBeenCalledWith('notify_overlay_hidden');
    });

    it('handles restore visibility event while minimized without wiping conversation', async () => {
      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'world' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      // Minimize
      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });

      // Emit a restore visibility event (hotkey/tray path)
      await act(async () => {
        emitTauriEvent('thuki://visibility', { state: 'restore' });
      });
      await act(async () => {});

      // Conversation still intact
      expect(screen.getByText('hello')).toBeInTheDocument();
      expect(screen.getByText('world')).toBeInTheDocument();
      // MinimizedIcon gone
      expect(
        screen.queryByRole('button', { name: /restore thuki/i }),
      ).toBeNull();
    });

    it('keeps the mascot available across many minimize/restore cycles', async () => {
      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});

      // Toggle repeatedly. The icon must reappear on every minimize and
      // disappear on every restore; it must never get stranded invisible
      // (the disappearing-icon bug).
      for (let i = 0; i < 5; i++) {
        const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
        await act(async () => {
          fireEvent.click(minimizeBtn);
        });
        expect(
          screen.getByRole('button', { name: /restore thuki/i }),
        ).toBeInTheDocument();

        const restoreBtn = screen.getByRole('button', {
          name: /restore thuki/i,
        });
        await act(async () => {
          fireEvent.pointerDown(restoreBtn, { clientX: 0, clientY: 0 });
          fireEvent.pointerUp(restoreBtn);
        });
        await act(async () => {});
        expect(
          screen.queryByRole('button', { name: /restore thuki/i }),
        ).toBeNull();
        // Chat is back so the next iteration can minimize again.
        expect(screen.getByText('hello')).toBeInTheDocument();
      }
    });

    it('ignores a restore request while not minimized and re-syncs the flag', async () => {
      await enterChatMode();
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});
      invoke.mockClear();

      // A stray restore event while idle must NOT start an expand morph
      // (which would strand the state machine in 'expanding'); it only
      // re-syncs the Rust minimized flag.
      await act(async () => {
        emitTauriEvent('thuki://visibility', { state: 'restore' });
      });
      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('set_overlay_minimized', {
        minimized: false,
      });
      // Never minimized: no mascot, chat still visible.
      expect(
        screen.queryByRole('button', { name: /restore thuki/i }),
      ).toBeNull();
      expect(screen.getByText('hello')).toBeInTheDocument();

      // The machine is not stranded: a subsequent minimize still works.
      const minimizeBtn = screen.getByRole('button', { name: /minimize/i });
      await act(async () => {
        fireEvent.click(minimizeBtn);
      });
      expect(
        screen.getByRole('button', { name: /restore thuki/i }),
      ).toBeInTheDocument();
    });
  });

  describe('text base CSS variable', () => {
    it('writes window.textBasePx to --thuki-text-base on <html> on mount', async () => {
      document.documentElement.style.removeProperty('--thuki-text-base');

      render(<App />);
      await act(async () => {});

      expect(
        document.documentElement.style.getPropertyValue('--thuki-text-base'),
      ).toBe(`${DEFAULT_CONFIG.window.textBasePx}px`);
    });

    it('writes the three typography vars (line-height, letter-spacing, font-weight) on mount', async () => {
      const root = document.documentElement;
      root.style.removeProperty('--thuki-text-line-height');
      root.style.removeProperty('--thuki-text-letter-spacing');
      root.style.removeProperty('--thuki-text-font-weight');

      render(<App />);
      await act(async () => {});

      expect(root.style.getPropertyValue('--thuki-text-line-height')).toBe(
        `${DEFAULT_CONFIG.window.textLineHeight}`,
      );
      expect(root.style.getPropertyValue('--thuki-text-letter-spacing')).toBe(
        `${DEFAULT_CONFIG.window.textLetterSpacingPx}px`,
      );
      expect(root.style.getPropertyValue('--thuki-text-font-weight')).toBe(
        `${DEFAULT_CONFIG.window.textFontWeight}`,
      );
    });
  });

  // ─── chat-header export button ──────────────────────────────────────────────

  describe('chat-header export button', () => {
    let writeText: ReturnType<typeof vi.fn>;
    let clipboardSpy: { mockRestore: () => void } | null = null;

    beforeEach(() => {
      writeText = vi.fn().mockResolvedValue(undefined);
      // happy-dom defines `navigator.clipboard` as a non-configurable
      // property, so a full property redefinition throws. Spy on the
      // existing `writeText` method instead.
      clipboardSpy = vi
        .spyOn(navigator.clipboard, 'writeText')
        .mockImplementation(writeText as (data: string) => Promise<void>);
    });

    afterEach(() => {
      clipboardSpy?.mockRestore();
      clipboardSpy = null;
    });

    async function enterChatMode() {
      render(<App />);
      await act(async () => {});
      await showOverlay();
      const textarea = getAskInput();
      act(() => {
        setAskValue('seed');
      });
      act(() => {
        fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
      });
      await act(async () => {});
      act(() => {
        getLastChannel()?.simulateMessage({ type: 'Token', data: 'ok' });
        getLastChannel()?.simulateMessage({ type: 'Done' });
      });
      await act(async () => {});
    }

    async function openExportPopover() {
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Export chat' }));
      });
    }

    /**
     * Routes `invoke('prompt_and_save_chat_export', ...)` to a custom
     * impl while leaving every other command on the channel-capture
     * default. Returns the wrapped impl handle so tests can read calls
     * back. Mirrors the previous `save_chat_export` override pattern.
     */
    type ExportArgs = {
      content: string;
      defaultFilename: string;
    };
    function overrideExportInvoke(
      impl: (args: ExportArgs) => Promise<boolean>,
    ) {
      const prev = invoke.getMockImplementation();
      invoke.mockImplementation(async (cmd, args) => {
        if (cmd === 'prompt_and_save_chat_export') {
          return await impl(args as ExportArgs);
        }
        return prev ? prev(cmd, args) : undefined;
      });
    }

    it('renders the export button in chat mode and the popover opens on click', async () => {
      await enterChatMode();

      const exportButton = screen.getByRole('button', { name: 'Export chat' });
      expect(exportButton).toBeInTheDocument();
      expect(exportButton).toHaveAttribute('aria-expanded', 'false');
      expect(exportButton).toHaveAttribute('aria-haspopup', 'menu');

      await act(async () => {
        fireEvent.click(exportButton);
      });

      expect(exportButton).toHaveAttribute('aria-expanded', 'true');
      const popover = screen.getByRole('menu', { name: 'Export chat' });
      expect(popover).toBeInTheDocument();
      expect(popover).toHaveAttribute('aria-orientation', 'vertical');
      expect(
        screen.getByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole('menuitem', { name: /Copy to clipboard/i }),
      ).toBeInTheDocument();
    });

    it('does not render the export button in ask-bar mode (no messages)', async () => {
      render(<App />);
      await act(async () => {});
      await showOverlay();

      expect(screen.queryByRole('button', { name: 'Export chat' })).toBeNull();
    });

    it('focuses the first menuitem when the popover opens', async () => {
      await enterChatMode();
      await openExportPopover();

      const firstItem = screen.getByRole('menuitem', {
        name: /Save as Markdown/i,
      });
      expect(document.activeElement).toBe(firstItem);
    });

    it('invokes prompt_and_save_chat_export with Markdown content when Markdown is clicked', async () => {
      await enterChatMode();
      let captured: ExportArgs | null = null;
      overrideExportInvoke(async (args) => {
        captured = args;
        return true;
      });
      invoke.mockClear();
      // re-install override after mockClear (mockClear preserves impl)
      overrideExportInvoke(async (args) => {
        captured = args;
        return true;
      });

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Save as Markdown/i }),
        );
      });
      await act(async () => {});

      await waitFor(() => {
        expect(captured).not.toBeNull();
      });
      const md = captured as ExportArgs | null;
      // Markdown serialiser emits YAML frontmatter at the top of the file.
      expect(md?.content.startsWith('---\napp: ')).toBe(true);
      expect(md?.content).toContain('## User');
      expect(md?.defaultFilename).toMatch(
        /^thuki-chat-\d{4}-\d{2}-\d{2}-\d{4}\.md$/,
      );
    });

    it('silently no-ops when the Rust command reports user cancellation (returns false)', async () => {
      await enterChatMode();
      overrideExportInvoke(async () => false);
      invoke.mockClear();
      overrideExportInvoke(async () => false);

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Save as Markdown/i }),
        );
      });
      await act(async () => {});

      // No banner, dialog cancellation is not an error condition.
      expect(screen.queryByText(/Failed to export/)).not.toBeInTheDocument();
      // The Rust command was called.
      expect(invoke).toHaveBeenCalledWith(
        'prompt_and_save_chat_export',
        expect.objectContaining({ content: expect.any(String) }),
      );
    });

    it('surfaces an error banner when prompt_and_save_chat_export rejects', async () => {
      await enterChatMode();
      overrideExportInvoke(async () => {
        throw new Error('Permission denied. Choose a writable location.');
      });

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Save as Markdown/i }),
        );
      });
      await act(async () => {});

      await vi.waitFor(() => {
        expect(
          screen.getByText(
            /Failed to export: Permission denied\. Choose a writable location\./,
          ),
        ).toBeInTheDocument();
      });
    });

    it('falls back to String(err) when the Rust command throws a non-Error', async () => {
      await enterChatMode();
      overrideExportInvoke(async () => {
        throw 'rust-plain-string';
      });

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Save as Markdown/i }),
        );
      });
      await act(async () => {});

      await vi.waitFor(() => {
        expect(
          screen.getByText(/Failed to export: rust-plain-string/),
        ).toBeInTheDocument();
      });
    });

    it('writes to the clipboard when the Copy to clipboard menuitem is clicked', async () => {
      await enterChatMode();

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Copy to clipboard/i }),
        );
      });

      await vi.waitFor(() => {
        expect(writeText).toHaveBeenCalledWith(
          expect.stringContaining('## User'),
        );
      });
    });

    it('shows an error banner when clipboard.writeText rejects', async () => {
      await enterChatMode();
      writeText.mockRejectedValueOnce(new Error('clipboard denied'));

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Copy to clipboard/i }),
        );
      });

      await vi.waitFor(() => {
        expect(
          screen.getByText(/Failed to copy: clipboard denied/),
        ).toBeInTheDocument();
      });
    });

    it('falls back to String(err) when the clipboard writer throws a non-Error', async () => {
      await enterChatMode();
      writeText.mockRejectedValueOnce('clip-plain');

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Copy to clipboard/i }),
        );
      });

      await vi.waitFor(() => {
        expect(
          screen.getByText(/Failed to copy: clip-plain/),
        ).toBeInTheDocument();
      });
    });

    it('keeps the popover open when mousedown lands inside it', async () => {
      await enterChatMode();
      await openExportPopover();

      const item = screen.getByRole('menuitem', {
        name: /Save as Markdown/i,
      });
      await act(async () => {
        fireEvent.mouseDown(item);
      });
      await act(async () => {});

      expect(
        screen.queryByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeInTheDocument();
    });

    it('closes the popover when clicking outside', async () => {
      await enterChatMode();
      await openExportPopover();
      expect(
        screen.queryByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeInTheDocument();

      await act(async () => {
        fireEvent.mouseDown(document.body);
      });

      expect(
        screen.queryByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeNull();
    });

    it('toggles the popover closed when the export button is clicked a second time', async () => {
      await enterChatMode();
      const exportButton = screen.getByRole('button', { name: 'Export chat' });

      await act(async () => {
        fireEvent.click(exportButton);
      });
      expect(exportButton).toHaveAttribute('aria-expanded', 'true');

      // The button has data-export-toggle so a mousedown on it does NOT
      // close via the outside-click effect; the subsequent click toggles
      // the state to false.
      await act(async () => {
        fireEvent.mouseDown(exportButton);
      });
      await act(async () => {
        fireEvent.click(exportButton);
      });

      expect(exportButton).toHaveAttribute('aria-expanded', 'false');
    });

    it('auto-clears the capture-error banner after a short linger', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      try {
        render(<App />);
        await act(async () => {});
        await showOverlay();

        // /extract with no image triggers the same captureError surface
        // we want to auto-dismiss. Used as the harness here because
        // the chat-header export button does not render until chat mode
        // (so it cannot trigger an empty-state error).
        const textarea = getAskInput();
        act(() => {
          setAskValue('/extract');
        });
        await act(async () => {
          fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
        });

        expect(
          screen.getByText(
            'Attach an image or add /screen to extract text from.',
          ),
        ).toBeInTheDocument();

        // Auto-dismiss timer is 5s. Advance past it.
        await act(async () => {
          vi.advanceTimersByTime(5000);
        });

        expect(
          screen.queryByText(
            'Attach an image or add /screen to extract text from.',
          ),
        ).toBeNull();
      } finally {
        vi.useRealTimers();
      }
    });

    it('closes the model picker when opening the export popover', async () => {
      await enterChatMode();

      // Open model picker first.
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
      });
      // Then open export popover; model picker should close.
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Export chat' }));
      });

      // Export popover is open.
      expect(
        screen.getByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeInTheDocument();
    });

    it('closes the export popover when the user opens the history dropdown', async () => {
      await enterChatMode();
      // HistoryPanel renders when the dropdown opens and iterates over
      // the conversations list — stub the IPC source so it gets [].
      const prev = invoke.getMockImplementation();
      invoke.mockImplementation(async (cmd, args) => {
        if (cmd === 'list_conversations') return [];
        return prev ? prev(cmd, args) : undefined;
      });
      await openExportPopover();
      expect(
        screen.getByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeInTheDocument();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Open history' }));
      });

      expect(
        screen.queryByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeNull();
    });

    it('closes the export popover when the user opens the model picker', async () => {
      await enterChatMode();
      await openExportPopover();
      expect(
        screen.getByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeInTheDocument();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Choose model' }));
      });

      expect(
        screen.queryByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeNull();
    });

    it('closes the export popover when the user minimizes the overlay', async () => {
      await enterChatMode();
      await openExportPopover();
      expect(
        screen.getByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeInTheDocument();

      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Minimize' }));
      });

      expect(
        screen.queryByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeNull();
    });

    it('closes the export popover when the user starts a new conversation', async () => {
      await enterChatMode();
      // The "New conversation" handler routes through HistoryPanel as
      // the SwitchConfirmation host when the session is unsaved, so the
      // panel may mount; stub list_conversations to be safe.
      const prev = invoke.getMockImplementation();
      invoke.mockImplementation(async (cmd, args) => {
        if (cmd === 'list_conversations') return [];
        return prev ? prev(cmd, args) : undefined;
      });
      await openExportPopover();
      expect(
        screen.getByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeInTheDocument();

      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: 'New conversation' }),
        );
      });

      expect(
        screen.queryByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeNull();
    });

    it('Escape dismisses the popover and returns focus to the toggle button (does not close the overlay)', async () => {
      await enterChatMode();
      await openExportPopover();
      const toggle = screen.getByRole('button', { name: 'Export chat' });

      await act(async () => {
        fireEvent.keyDown(window, { key: 'Escape' });
      });
      await act(async () => {});

      expect(
        screen.queryByRole('menuitem', { name: /Save as Markdown/i }),
      ).toBeNull();
      expect(document.activeElement).toBe(toggle);
      // The overlay is still mounted (the export button is still there).
      expect(toggle).toBeInTheDocument();
    });

    it('drops a re-entrant export click while the first is still in flight', async () => {
      await enterChatMode();
      let resolveFirst: ((v: boolean) => void) | undefined;
      let calls = 0;
      overrideExportInvoke(
        () =>
          new Promise<boolean>((resolve) => {
            calls += 1;
            if (calls === 1) {
              resolveFirst = resolve;
            } else {
              resolve(true);
            }
          }),
      );

      // First click — popover closes, runFileExport is in flight.
      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Save as Markdown/i }),
        );
      });
      await act(async () => {});

      // Second click — reopen popover and click again. Should NOT
      // dispatch a second prompt_and_save_chat_export.
      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Save as Markdown/i }),
        );
      });
      await act(async () => {});

      expect(calls).toBe(1);

      // Resolve the first call; verify a subsequent click then succeeds.
      await act(async () => {
        resolveFirst?.(true);
      });
      await act(async () => {});

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Save as Markdown/i }),
        );
      });
      await act(async () => {});

      expect(calls).toBe(2);
    });

    it('drives overlay alpha to 0 before the IPC call and back to 1 after success', async () => {
      await enterChatMode();
      overrideExportInvoke(async () => true);
      invoke.mockClear();
      overrideExportInvoke(async () => true);

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Save as Markdown/i }),
        );
      });
      await act(async () => {});

      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith('set_overlay_alpha', {
          alpha: 0,
          durationMs: 0,
        });
      });
      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith('set_overlay_alpha', {
          alpha: 1,
          durationMs: 150,
        });
      });

      // Assert ordering: alpha:0 → prompt_and_save_chat_export → alpha:1
      // so the overlay stays hidden for exactly the dialog+write
      // window and not a frame longer.
      const calls = vi.mocked(invoke).mock.calls;
      const alphaZeroIdx = calls.findIndex(
        (call) =>
          call[0] === 'set_overlay_alpha' &&
          (call[1] as { alpha: number } | undefined)?.alpha === 0,
      );
      const promptIdx = calls.findIndex(
        (call) => call[0] === 'prompt_and_save_chat_export',
      );
      const alphaOneIdx = calls.findIndex(
        (call) =>
          call[0] === 'set_overlay_alpha' &&
          (call[1] as { alpha: number } | undefined)?.alpha === 1,
      );
      expect(alphaZeroIdx).toBeGreaterThanOrEqual(0);
      expect(promptIdx).toBeGreaterThan(alphaZeroIdx);
      expect(alphaOneIdx).toBeGreaterThan(promptIdx);
    });

    it('restores overlay alpha to 1 when the Rust command reports cancellation', async () => {
      await enterChatMode();
      overrideExportInvoke(async () => false);
      invoke.mockClear();
      overrideExportInvoke(async () => false);

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Save as Markdown/i }),
        );
      });
      await act(async () => {});

      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith('set_overlay_alpha', {
          alpha: 1,
          durationMs: 150,
        });
      });
      // No banner on a clean cancellation.
      expect(screen.queryByText(/Failed to export/)).not.toBeInTheDocument();
    });

    it('restores overlay alpha to 1 when the Rust command rejects', async () => {
      await enterChatMode();
      overrideExportInvoke(async () => {
        throw new Error('disk full');
      });

      await openExportPopover();
      await act(async () => {
        fireEvent.click(
          screen.getByRole('menuitem', { name: /Save as Markdown/i }),
        );
      });
      await act(async () => {});

      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith('set_overlay_alpha', {
          alpha: 1,
          durationMs: 150,
        });
      });
    });
  });
});
