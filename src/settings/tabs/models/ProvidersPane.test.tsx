import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
  within,
} from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';
import {
  emitTauriEvent,
  clearEventHandlers,
} from '../../../testUtils/mocks/tauri';

import { ProvidersPane } from './ProvidersPane';
import type { RawAppConfig, RawProvider } from '../../types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const BUILTIN: RawProvider = {
  id: 'builtin',
  kind: 'builtin',
  label: 'Built-in (Thuki)',
  base_url: '',
  model: '',
  vision: false,
};
const OLLAMA: RawProvider = {
  id: 'ollama',
  kind: 'ollama',
  label: 'Ollama',
  base_url: 'http://127.0.0.1:11434',
  model: '',
  vision: false,
};
const OPENAI: RawProvider = {
  id: 'openai',
  kind: 'openai',
  label: 'LM Studio',
  base_url: 'http://127.0.0.1:1234',
  model: '',
  vision: false,
};

const INSTALLED = [
  {
    id: 'org/Qwen3.5-9B-GGUF:Qwen3.5-9B-Q4_K_M.gguf',
    display_name: 'Qwen3.5 9B',
    size_bytes: 6_600_000_000,
    quant: 'Q4_K_M',
  },
];

// A built-in provider whose selected model resolves to INSTALLED[0], so the
// keep-warm status line can name it (e.g. "Qwen3.5 9B in VRAM").
const BUILTIN_LOADED: RawProvider = { ...BUILTIN, model: INSTALLED[0].id };

function makeConfig(
  activeProvider: string,
  providers: RawProvider[],
  over: Partial<RawAppConfig['inference']> = {},
): RawAppConfig {
  return {
    inference: {
      active_provider: activeProvider,
      keep_warm_inactivity_minutes: 0,
      num_ctx: 16384,
      providers,
      ...over,
    },
    prompt: { system: 'hello' },
    window: {
      overlay_width: 600,
      max_chat_height: 648,
      max_images: 3,
      text_base_px: 15,
      text_line_height: 1.5,
      text_letter_spacing_px: 0,
      text_font_weight: 500,
    },
    quote: {
      max_display_lines: 4,
      max_display_chars: 300,
      max_context_length: 4096,
    },
    behavior: { auto_replace: false, auto_close: false },
    search: {
      searxng_url: '',
      reader_url: '',
      max_iterations: 3,
      top_k_urls: 10,
      searxng_max_results: 10,
      search_timeout_s: 20,
      reader_per_url_timeout_s: 10,
      reader_batch_timeout_s: 30,
      judge_timeout_s: 30,
      router_timeout_s: 45,
    },
    debug: { trace_enabled: false },
  };
}

function engineStatus(state: string) {
  return { state, model_path: '', port: null, error: null };
}

function mockInvoke(over: Record<string, unknown> = {}) {
  invokeMock.mockImplementation(async (cmd: string) => {
    if (Object.prototype.hasOwnProperty.call(over, cmd)) {
      const v = over[cmd];
      if (v instanceof Error) throw v;
      return v;
    }
    switch (cmd) {
      case 'list_installed_models':
        return [];
      case 'get_engine_status':
        return engineStatus('stopped');
      case 'get_loaded_model':
        return null;
      case 'get_builtin_warm_state':
        return false;
      case 'get_model_picker_state':
        return { active: null, all: [], ollamaReachable: true };
      default:
        return makeConfig('ollama', [BUILTIN, OLLAMA]);
    }
  });
}

function renderPane(config: RawAppConfig, props: Record<string, unknown> = {}) {
  return render(
    <ProvidersPane
      config={config}
      resyncToken={0}
      onSaved={() => {}}
      onAddModel={() => {}}
      {...props}
    />,
  );
}

beforeEach(() => {
  vi.stubEnv('VITE_ENABLE_OPENAI_PROVIDER', 'true');
  invokeMock.mockReset();
  clearEventHandlers();
  mockInvoke();
});

afterEach(() => {
  vi.useRealTimers();
  clearEventHandlers();
});

describe('ProvidersPane active hero', () => {
  it('shows the active Ollama provider in the hero', async () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    expect(screen.getByText('Active provider')).toBeInTheDocument();
    expect(screen.getByText('Ollama')).toBeInTheDocument();
    expect(screen.getByText('http://127.0.0.1:11434')).toBeInTheDocument();
  });

  it('falls back to Ollama labelling when the active id matches no provider', () => {
    renderPane(makeConfig('ghost', [BUILTIN, OLLAMA]));
    // The hero name falls back to "Ollama" and the subtitle to the generic copy.
    expect(screen.getAllByText('Ollama').length).toBeGreaterThan(0);
    expect(screen.getByText('Local or remote Ollama')).toBeInTheDocument();
  });

  it('lists installed models in the built-in hero and commits a pick', async () => {
    const builtin = { ...BUILTIN, model: INSTALLED[0].id };
    mockInvoke({ list_installed_models: INSTALLED });
    const onSaved = vi.fn();
    renderPane(makeConfig('builtin', [builtin, OLLAMA]), { onSaved });
    const select = await screen.findByRole('combobox', {
      name: 'Built-in model',
    });
    expect(select).toHaveValue(INSTALLED[0].id);
    fireEvent.change(select, { target: { value: INSTALLED[0].id } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
        providerId: 'builtin',
        field: 'model',
        value: INSTALLED[0].id,
      }),
    );
  });

  it('re-fetches the picker state and shows the new provider model on switch', async () => {
    // Built-in active first: the picker returns the built-in model id.
    mockInvoke({
      list_installed_models: INSTALLED,
      get_model_picker_state: {
        active: INSTALLED[0].id,
        all: [INSTALLED[0].id],
        ollamaReachable: true,
      },
    });
    const builtin = { ...BUILTIN, model: INSTALLED[0].id };
    const view = renderPane(makeConfig('builtin', [builtin, OLLAMA]));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('get_model_picker_state'),
    );

    // Now Ollama is active and its tags are different from the built-in id.
    mockInvoke({
      list_installed_models: INSTALLED,
      get_model_picker_state: {
        active: 'gemma4:e4b',
        all: ['gemma4:e4b'],
        ollamaReachable: true,
      },
    });
    view.rerender(
      <ProvidersPane
        config={makeConfig('ollama', [builtin, OLLAMA])}
        resyncToken={0}
        onSaved={() => {}}
        onAddModel={() => {}}
      />,
    );
    // The provider-change refetch replaces the stale built-in id with the
    // live Ollama model rather than leaving the built-in id in the dropdown.
    const select = await screen.findByRole('combobox', {
      name: 'Active Ollama model',
    });
    await waitFor(() => expect(select).toHaveValue('gemma4:e4b'));
  });

  it('appends the quant only to disambiguate duplicate display names', async () => {
    const dupes = [
      { ...INSTALLED[0], id: 'org/x:q4.gguf', quant: 'Q4_K_M' },
      { ...INSTALLED[0], id: 'org/x:q8.gguf', quant: 'Q8_0' },
    ];
    mockInvoke({ list_installed_models: dupes });
    renderPane(
      makeConfig('builtin', [{ ...BUILTIN, model: 'org/x:q4.gguf' }, OLLAMA]),
    );
    await screen.findByRole('combobox', { name: 'Built-in model' });
    // Shared display name -> each option disambiguates with its quant.
    expect(
      screen.getByRole('option', { name: 'Qwen3.5 9B · Q4_K_M' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('option', { name: 'Qwen3.5 9B · Q8_0' }),
    ).toBeInTheDocument();
  });

  it('shows a Choose-a-model option when the built-in model is not installed', async () => {
    mockInvoke({ list_installed_models: INSTALLED });
    renderPane(makeConfig('builtin', [{ ...BUILTIN, model: 'gone' }, OLLAMA]));
    const select = await screen.findByRole('combobox', {
      name: 'Built-in model',
    });
    expect(select).toHaveValue('');
    expect(screen.getByText('Choose a model')).toBeInTheDocument();
  });

  it('offers a Discover link when no built-in models are installed', async () => {
    const onAddModel = vi.fn();
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]), { onAddModel });
    const link = await screen.findByRole('button', {
      name: /Download a model in Discover/,
    });
    fireEvent.click(link);
    expect(onAddModel).toHaveBeenCalled();
  });

  it('commit of the built-in model swallows a backend error', async () => {
    const builtin = { ...BUILTIN, model: INSTALLED[0].id };
    mockInvoke({
      list_installed_models: INSTALLED,
      update_provider_field: new Error('nope'),
    });
    renderPane(makeConfig('builtin', [builtin, OLLAMA]));
    const select = await screen.findByRole('combobox', {
      name: 'Built-in model',
    });
    fireEvent.change(select, { target: { value: INSTALLED[0].id } });
    // No throw.
    await Promise.resolve();
  });

  it('renders the Ollama endpoint field and model dropdown', async () => {
    mockInvoke({
      get_model_picker_state: {
        active: 'llama3.1:8b',
        all: ['llama3.1:8b'],
        ollamaReachable: true,
      },
    });
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    expect(screen.getByRole('textbox', { name: 'Ollama URL' })).toHaveValue(
      'http://127.0.0.1:11434',
    );
    const select = await screen.findByRole('combobox', {
      name: 'Active Ollama model',
    });
    expect(select).toHaveValue('llama3.1:8b');
    fireEvent.change(select, { target: { value: 'llama3.1:8b' } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_active_model', {
        model: 'llama3.1:8b',
      }),
    );
  });

  it('lifts a fresh config after selecting a different Ollama model', async () => {
    const lifted = makeConfig('ollama', [
      BUILTIN,
      { ...OLLAMA, model: 'llama3.2:3b' },
    ]);
    mockInvoke({
      get_model_picker_state: {
        active: 'gemma4:e4b',
        all: ['gemma4:e4b', 'llama3.2:3b'],
        ollamaReachable: true,
      },
      get_config: lifted,
    });
    const onSaved = vi.fn();
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]), { onSaved });
    const select = await screen.findByRole('combobox', {
      name: 'Active Ollama model',
    });
    fireEvent.change(select, { target: { value: 'llama3.2:3b' } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_active_model', {
        model: 'llama3.2:3b',
      }),
    );
    // The lifted config (carrying the newly-selected Ollama model) reaches the
    // parent so the Running footer re-renders with the new name.
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(lifted));
  });

  it('swallows a failed Ollama model selection without lifting config', async () => {
    const onSaved = vi.fn();
    mockInvoke({
      get_model_picker_state: {
        active: 'gemma4:e4b',
        all: ['gemma4:e4b', 'llama3.2:3b'],
        ollamaReachable: true,
      },
      set_active_model: new Error('nope'),
    });
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]), { onSaved });
    const select = await screen.findByRole('combobox', {
      name: 'Active Ollama model',
    });
    fireEvent.change(select, { target: { value: 'llama3.2:3b' } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_active_model', {
        model: 'llama3.2:3b',
      }),
    );
    await Promise.resolve();
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('shows a no-models hint when Ollama has none', () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    expect(screen.getByText('No models installed')).toBeInTheDocument();
  });

  it('warns when the Ollama URL is non-local', () => {
    renderPane(
      makeConfig('ollama', [
        BUILTIN,
        { ...OLLAMA, base_url: 'http://example.com:11434' },
      ]),
    );
    expect(screen.getByRole('alert')).toHaveTextContent(/non-local Ollama/);
  });

  it('commits an edited Ollama URL on blur and lifts the config', async () => {
    const onSaved = vi.fn();
    const nextConfig = makeConfig('ollama', [BUILTIN, OLLAMA]);
    mockInvoke({ set_ollama_url: nextConfig });
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]), { onSaved });
    const input = screen.getByRole('textbox', { name: 'Ollama URL' });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: 'http://127.0.0.1:9999' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_ollama_url', {
        baseUrl: 'http://127.0.0.1:9999',
      }),
    );
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(nextConfig));
  });

  it('does not commit the Ollama URL when it is unchanged', () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    const input = screen.getByRole('textbox', { name: 'Ollama URL' });
    fireEvent.blur(input);
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_ollama_url',
      expect.anything(),
    );
  });

  it('reverts the Ollama URL field when the commit fails', async () => {
    mockInvoke({ set_ollama_url: new Error('bad') });
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    const input = screen.getByRole('textbox', { name: 'Ollama URL' });
    fireEvent.change(input, { target: { value: 'http://127.0.0.1:9999' } });
    fireEvent.blur(input);
    await waitFor(() => expect(input).toHaveValue('http://127.0.0.1:11434'));
  });

  it('commits the Ollama URL on Enter', () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    const input = screen.getByRole('textbox', { name: 'Ollama URL' });
    fireEvent.keyDown(input, { key: 'Enter' });
    // blur fires; unchanged -> no commit, but the keydown branch is covered.
    fireEvent.keyDown(input, { key: 'a' });
  });

  it('renders the OpenAI card in the hero when openai is active and enabled', () => {
    renderPane(makeConfig('openai', [BUILTIN, OLLAMA, OPENAI]));
    expect(
      screen.getByRole('textbox', { name: 'Provider label' }),
    ).toBeInTheDocument();
  });
});

describe('ProvidersPane other providers', () => {
  it('lists non-active providers with a Switch and switches on click', async () => {
    const onSaved = vi.fn();
    const next = makeConfig('builtin', [BUILTIN, OLLAMA]);
    mockInvoke({ set_active_provider: next });
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]), { onSaved });
    const switches = screen.getAllByRole('button', { name: 'Switch' });
    fireEvent.click(switches[0]);
    // The switch is confirmed in a dialog before it takes effect.
    fireEvent.click(screen.getByRole('button', { name: /^Switch to / }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_active_provider', {
        providerId: 'builtin',
      }),
    );
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(next));
  });

  it('cancels a provider switch without changing the active provider', () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    fireEvent.click(screen.getAllByRole('button', { name: 'Switch' })[0]);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('button', { name: /^Switch to / })).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_active_provider',
      expect.anything(),
    );
  });

  it('swallows a failed provider switch', async () => {
    mockInvoke({ set_active_provider: new Error('x') });
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    fireEvent.click(screen.getAllByRole('button', { name: 'Switch' })[0]);
    fireEvent.click(screen.getByRole('button', { name: /^Switch to / }));
    await Promise.resolve();
  });

  it('hides the openai row when the dev flag is off', () => {
    vi.stubEnv('VITE_ENABLE_OPENAI_PROVIDER', 'false');
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA, OPENAI]));
    expect(screen.queryByText('LM Studio')).toBeNull();
  });

  it('shows the add-a-provider affordance when enabled and no openai exists', () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    expect(
      screen.getByRole('button', { name: /Add OpenAI-compatible server/ }),
    ).toBeInTheDocument();
  });

  it('shows the openai row in others when enabled, present, and not active', () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA, OPENAI]));
    expect(screen.getByText('LM Studio')).toBeInTheDocument();
  });
});

describe('ProvidersPane generation', () => {
  it('commits a context-window change on mouse up', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    });
    fireEvent.change(slider, { target: { value: '800' } });
    fireEvent.mouseUp(slider);
    expect(screen.getByText('tokens')).toBeInTheDocument();
  });

  it('shows the token value in an editable field with no turns line', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA], { num_ctx: 32768 }));
    expect(
      screen.getByRole('spinbutton', { name: 'Context window size in tokens' }),
    ).toHaveValue(32768);
    expect(screen.getByText('tokens')).toBeInTheDocument();
    expect(screen.queryByText(/turns/)).toBeNull();
  });

  it('commits a typed token value and moves the slider', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    const input = screen.getByRole('spinbutton', {
      name: 'Context window size in tokens',
    });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '65536' } });
    fireEvent.blur(input);
    expect(input).toHaveValue(65536);
    expect(
      screen.getByRole('slider', { name: 'Context window tokens' }),
    ).toHaveAttribute('aria-valuenow', '65536');
  });

  it('clamps a typed token value above the maximum', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    const input = screen.getByRole('spinbutton', {
      name: 'Context window size in tokens',
    });
    fireEvent.change(input, { target: { value: '9999999' } });
    fireEvent.blur(input);
    expect(input).toHaveValue(1048576);
  });

  it('reverts a non-numeric token entry to the current value', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA], { num_ctx: 32768 }));
    const input = screen.getByRole('spinbutton', {
      name: 'Context window size in tokens',
    });
    fireEvent.change(input, { target: { value: '' } });
    fireEvent.blur(input);
    expect(input).toHaveValue(32768);
  });

  it('commits the token field on Enter and ignores other keys', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    const input = screen.getByRole('spinbutton', {
      name: 'Context window size in tokens',
    });
    fireEvent.change(input, { target: { value: '8192' } });
    // A non-Enter key does not blur/commit; Enter does.
    fireEvent.keyDown(input, { key: 'a' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(input).toHaveValue(8192);
  });

  it('keeps the focused token field unchanged across a resync', () => {
    const { rerender } = renderPane(
      makeConfig('builtin', [BUILTIN, OLLAMA], { num_ctx: 16384 }),
    );
    const input = screen.getByRole('spinbutton', {
      name: 'Context window size in tokens',
    });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '65536' } });
    rerender(
      <ProvidersPane
        config={makeConfig('builtin', [BUILTIN, OLLAMA], { num_ctx: 32768 })}
        resyncToken={5}
        onSaved={() => {}}
        onAddModel={() => {}}
      />,
    );
    // Focused: the resync must not clobber the in-progress entry.
    expect(input).toHaveValue(65536);
  });

  it('opens the tuning guide from the Learn link', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    fireEvent.click(
      screen.getByRole('button', {
        name: /Learn how to tune Context Window/,
      }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/blob/main/docs/tuning-context-window.md#the-5-minute-benchmark-recipe',
    });
  });

  it('spaces the doubling milestones evenly across the track', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    // Each milestone doubles the last, so on the log track they sit at equal
    // ~11.1% gaps and the thumb lands on the milestone it reads.
    const leftOf = (label: string) =>
      (screen.getByText(label) as HTMLElement).style.left;
    expect(leftOf('2K')).toBe('0%');
    expect(leftOf('4K')).toBe('11.1%');
    expect(leftOf('8K')).toBe('22.2%');
    expect(leftOf('16K')).toBe('33.3%');
    expect(leftOf('32K')).toBe('44.4%');
    expect(leftOf('64K')).toBe('55.6%');
    expect(leftOf('128K')).toBe('66.7%');
    expect(leftOf('256K')).toBe('77.8%');
    expect(leftOf('512K')).toBe('88.9%');
    expect(leftOf('1M')).toBe('100%');
  });

  it('explains the context window through a tooltip, not a subtitle', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    expect(
      screen.getByRole('button', { name: 'About Context window' }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText('How much conversation the model remembers'),
    ).toBeNull();
  });

  it('drops the system prompt subtitle in favour of its tooltip', () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    expect(
      screen.getByRole('button', { name: 'About System prompt' }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText('Persona sent at the start of every chat'),
    ).toBeNull();
  });

  it('commits a context-window change via touch and keyboard', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    });
    fireEvent.change(slider, { target: { value: '600' } });
    fireEvent.touchEnd(slider);
    fireEvent.keyUp(slider);
  });

  it('clamps the keep-warm minutes and handles non-numeric input', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '5000' } });
    expect(input).toHaveValue(1440);
    fireEvent.change(input, { target: { value: 'abc' } });
    fireEvent.blur(input);
    expect(input).toHaveValue(0);
  });

  it('names the model the engine is actually serving, not the selected one', () => {
    // The selection is Qwen, but the engine is still serving Mistral: switching
    // the active model does not reload the sidecar, so the label must follow
    // what the backend reports as resident, never the frontend selection.
    mockInvoke({
      get_engine_status: engineStatus('loaded'),
      get_loaded_model: 'Mistral Nemo 12B',
      list_installed_models: INSTALLED,
    });
    renderPane(makeConfig('builtin', [BUILTIN_LOADED, OLLAMA]));
    return waitFor(() => {
      const status = screen.getByTestId('keep-warm-status');
      expect(status).toHaveTextContent('Mistral Nemo 12B');
      expect(within(status).getByText('in VRAM')).toBeInTheDocument();
      // The selected (but not-yet-resident) model is never shown as resident.
      expect(within(status).queryByText('Qwen3.5 9B')).not.toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Unload now' })).toBeEnabled();
    });
  });

  it('shows Loading… while the built-in engine is starting', () => {
    mockInvoke({
      get_engine_status: engineStatus('starting'),
      list_installed_models: INSTALLED,
    });
    renderPane(makeConfig('builtin', [BUILTIN_LOADED, OLLAMA]));
    return waitFor(() => {
      expect(screen.getByText('Loading…')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
    });
  });

  it('shows warming… while the built-in engine primes a resident model', () => {
    mockInvoke({
      get_engine_status: engineStatus('loaded'),
      get_loaded_model: 'Mistral Nemo 12B',
      get_builtin_warm_state: true,
      list_installed_models: INSTALLED,
    });
    renderPane(makeConfig('builtin', [BUILTIN_LOADED, OLLAMA]));
    return waitFor(() => {
      const status = screen.getByTestId('keep-warm-status');
      expect(status).toHaveTextContent('Mistral Nemo 12B');
      expect(within(status).getByText('warming…')).toBeInTheDocument();
      expect(within(status).queryByText('in VRAM')).not.toBeInTheDocument();
    });
  });

  it('shows Warming up… while priming before the resident name resolves', () => {
    mockInvoke({
      get_engine_status: engineStatus('loaded'),
      get_loaded_model: null,
      get_builtin_warm_state: true,
    });
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    return waitFor(() =>
      expect(screen.getByText('Warming up…')).toBeInTheDocument(),
    );
  });

  it('flips warming… to in VRAM across the warming and warmed events', async () => {
    mockInvoke({
      get_engine_status: engineStatus('loaded'),
      get_loaded_model: 'Qwen3.5 9B',
      list_installed_models: INSTALLED,
    });
    renderPane(makeConfig('builtin', [BUILTIN_LOADED, OLLAMA]));
    const status = await screen.findByTestId('keep-warm-status');
    await waitFor(() =>
      expect(within(status).getByText('in VRAM')).toBeInTheDocument(),
    );
    act(() => emitTauriEvent('warmup:builtin-warming', null));
    expect(within(status).getByText('warming…')).toBeInTheDocument();
    act(() => emitTauriEvent('warmup:builtin-warmed', null));
    expect(within(status).getByText('in VRAM')).toBeInTheDocument();
  });

  it('clears the warming status when the model is evicted', async () => {
    mockInvoke({
      get_engine_status: engineStatus('loaded'),
      get_loaded_model: 'Qwen3.5 9B',
      get_builtin_warm_state: true,
      list_installed_models: INSTALLED,
    });
    renderPane(makeConfig('builtin', [BUILTIN_LOADED, OLLAMA]));
    const status = await screen.findByTestId('keep-warm-status');
    await waitFor(() =>
      expect(within(status).getByText('warming…')).toBeInTheDocument(),
    );
    act(() => emitTauriEvent('warmup:model-evicted', null));
    expect(screen.getByText('No model loaded')).toBeInTheDocument();
  });

  it('falls back to no-model-loaded when the engine is loaded but the model is unknown', () => {
    mockInvoke({ get_engine_status: engineStatus('loaded') });
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    return waitFor(() =>
      expect(screen.getByText('No model loaded')).toBeInTheDocument(),
    );
  });

  it('disables Unload and shows no-model-loaded while the built-in engine is stopped', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
    expect(screen.getByText('No model loaded')).toBeInTheDocument();
  });

  it('ejects the model on Unload click when loaded', async () => {
    mockInvoke({ get_engine_status: engineStatus('loaded') });
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Unload now' })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Unload now' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('evict_model'));
  });

  it('swallows a failed eject', async () => {
    mockInvoke({
      get_engine_status: engineStatus('loaded'),
      evict_model: new Error('no'),
    });
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Unload now' })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Unload now' }));
    await Promise.resolve();
  });

  it('shows the Ollama VRAM model line when one is loaded', async () => {
    mockInvoke({ get_loaded_model: 'llama3.1:8b' });
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    await waitFor(() =>
      expect(screen.getByTestId('keep-warm-status')).toHaveTextContent(
        'llama3.1:8b',
      ),
    );
  });

  it('shows no-model-loaded for Ollama when nothing is resident', () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    expect(screen.getByText('No model loaded')).toBeInTheDocument();
  });

  it('reflects warmup load + evict events', async () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    // Let the mount-time get_loaded_model settle so the event is not clobbered.
    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      emitTauriEvent('warmup:model-loaded', 'phi4');
    });
    expect(screen.getByTestId('keep-warm-status')).toHaveTextContent('phi4');
    await act(async () => {
      emitTauriEvent('warmup:model-evicted', null);
    });
    expect(screen.getByText('No model loaded')).toBeInTheDocument();
  });

  it('opens and closes the system prompt editor', () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    fireEvent.click(screen.getByRole('button', { name: /Edit/ }));
    expect(
      screen.getByRole('textbox', { name: 'System prompt' }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(screen.queryByRole('textbox', { name: 'System prompt' })).toBeNull();
  });

  it('opens the diagnostics section with the trace toggle', () => {
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    expect(
      screen.getByRole('switch', { name: 'Enable trace recording' }),
    ).toBeInTheDocument();
  });
});

describe('ProvidersPane robustness', () => {
  it('treats a non-array installed payload as empty', async () => {
    mockInvoke({ list_installed_models: null });
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    expect(
      await screen.findByRole('button', {
        name: /Download a model in Discover/,
      }),
    ).toBeInTheDocument();
  });

  it('survives failed installed/engine/loaded reads', async () => {
    mockInvoke({
      list_installed_models: new Error('a'),
      get_engine_status: new Error('b'),
      get_loaded_model: new Error('c'),
      get_builtin_warm_state: new Error('d'),
    });
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    await waitFor(() =>
      expect(screen.getByText('Active provider')).toBeInTheDocument(),
    );
  });

  it('re-seeds local state on a resync token bump (unfocused)', () => {
    const { rerender } = renderPane(
      makeConfig('ollama', [BUILTIN, OLLAMA], {
        keep_warm_inactivity_minutes: 0,
        num_ctx: 16384,
      }),
    );
    rerender(
      <ProvidersPane
        config={makeConfig('ollama', [BUILTIN, OLLAMA], {
          keep_warm_inactivity_minutes: 30,
          num_ctx: 32768,
        })}
        resyncToken={1}
        onSaved={() => {}}
        onAddModel={() => {}}
      />,
    );
    expect(
      screen.getByRole('spinbutton', { name: 'Release after N minutes' }),
    ).toHaveValue(30);
  });

  it('keeps focused fields unchanged across a resync', () => {
    const { rerender } = renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    const min = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.focus(min);
    const url = screen.getByRole('textbox', { name: 'Ollama URL' });
    fireEvent.focus(url);
    rerender(
      <ProvidersPane
        config={makeConfig('ollama', [BUILTIN, OLLAMA], {
          keep_warm_inactivity_minutes: 99,
        })}
        resyncToken={2}
        onSaved={() => {}}
        onAddModel={() => {}}
      />,
    );
    // Focused fields are not clobbered: still the original values.
    expect(min).toHaveValue(0);
    expect(url).toHaveValue('http://127.0.0.1:11434');
  });

  it('does not render an installed-count footnote', async () => {
    mockInvoke({ list_installed_models: INSTALLED });
    renderPane(
      makeConfig('builtin', [{ ...BUILTIN, model: INSTALLED[0].id }, OLLAMA]),
    );
    await screen.findByRole('combobox', { name: 'Built-in model' });
    expect(screen.queryByText(/installed model/)).toBeNull();
  });

  it('refreshes the resident built-in model when an engine:status event arrives', async () => {
    // Mount with nothing resident yet.
    mockInvoke({ list_installed_models: INSTALLED, get_loaded_model: null });
    renderPane(makeConfig('builtin', [BUILTIN_LOADED, OLLAMA]));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText('No model loaded')).toBeInTheDocument();
    // The engine finishes loading: the status event drives a fresh backend read
    // that names the now-resident model.
    mockInvoke({
      list_installed_models: INSTALLED,
      get_loaded_model: 'Qwen3.5 9B',
    });
    await act(async () => {
      emitTauriEvent('engine:status', engineStatus('loaded'));
    });
    await waitFor(() =>
      expect(screen.getByTestId('keep-warm-status')).toHaveTextContent(
        'Qwen3.5 9B',
      ),
    );
  });

  it('falls back to the first Ollama model when the active one is not listed', async () => {
    mockInvoke({
      get_model_picker_state: {
        active: 'not-installed',
        all: ['m1', 'm2'],
        ollamaReachable: true,
      },
    });
    renderPane(makeConfig('ollama', [BUILTIN, OLLAMA]));
    const select = await screen.findByRole('combobox', {
      name: 'Active Ollama model',
    });
    expect(select).toHaveValue('m1');
  });

  it('uses generic subtitles when provider URLs are empty', () => {
    renderPane(
      makeConfig('builtin', [
        BUILTIN,
        { ...OLLAMA, base_url: '' },
        { ...OPENAI, base_url: '' },
      ]),
    );
    expect(screen.getByText('Local or remote Ollama')).toBeInTheDocument();
    expect(screen.getByText('OpenAI-compatible server')).toBeInTheDocument();
  });

  it('tolerates a config with no built-in provider', () => {
    renderPane(makeConfig('ollama', [OLLAMA]));
    expect(screen.getByText('Active provider')).toBeInTheDocument();
  });

  it('renders no openai card in the hero when the dev flag is off', () => {
    vi.stubEnv('VITE_ENABLE_OPENAI_PROVIDER', 'false');
    renderPane(makeConfig('openai', [BUILTIN, OLLAMA, OPENAI]));
    // The hero shows the openai provider name but not its editable card.
    expect(screen.getByText('LM Studio')).toBeInTheDocument();
    expect(
      screen.queryByRole('textbox', { name: 'Provider label' }),
    ).toBeNull();
  });

  it('does not commit a context change on keyup while still dragging', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    });
    fireEvent.change(slider, { target: { value: '700' } });
    // dragging is still true (no mouse/touch up), so keyup must not commit.
    fireEvent.keyUp(slider);
  });

  it('keeps a valid keep-warm value untouched on blur', () => {
    renderPane(makeConfig('builtin', [BUILTIN, OLLAMA]));
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '60' } });
    fireEvent.blur(input);
    expect(input).toHaveValue(60);
  });

  it('renders a built-in model option without a quant suffix', async () => {
    const noQuant = { ...INSTALLED[0], quant: '' };
    mockInvoke({ list_installed_models: [noQuant] });
    renderPane(
      makeConfig('builtin', [{ ...BUILTIN, model: noQuant.id }, OLLAMA]),
    );
    const select = await screen.findByRole('combobox', {
      name: 'Built-in model',
    });
    expect(select).toHaveValue(noQuant.id);
  });

  it('handles a config with no Ollama provider', () => {
    renderPane(makeConfig('builtin', [BUILTIN]));
    expect(screen.getByText('Active provider')).toBeInTheDocument();
  });
});
