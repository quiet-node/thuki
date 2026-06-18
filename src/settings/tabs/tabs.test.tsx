/**
 * Smoke + interaction tests for the five Settings tabs.
 *
 * Each tab's body is mostly declarative `SaveField` markup whose behavior
 * is unit-tested in `components.test`, `SaveField.test`, and
 * `useDebouncedSave.test`. These tests exercise the tab-level wiring:
 * sections render, fields show up, helper tooltips have the right copy,
 * and the per-tab interactive affordances (About's icon-link buttons,
 * Reveal/Refresh/Reset) call the right Tauri commands.
 */

import {
  fireEvent,
  render,
  screen,
  waitFor,
  act,
} from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  clearEventHandlers,
  emitTauriEvent,
} from '../../testUtils/mocks/tauri';

import { ModelTab } from './ModelTab';
import { DisplayTab } from './DisplayTab';
import { SearchTab } from './SearchTab';
import { AboutTab } from './AboutTab';
import { BehaviorTab } from './BehaviorTab';
import type { RawAppConfig } from '../types';

// The OpenAI-compatible provider UI is gated behind a compile-time dev flag
// (off in shipped builds). Mock the flag module through a mutable holder so the
// suite can drive both branches: the existing openai-card tests run with it
// enabled, and a dedicated test asserts the affordance is absent when disabled.
const devFlags = { openaiEnabled: true };
vi.mock('../devFlags', () => ({
  get OPENAI_PROVIDER_ENABLED() {
    return devFlags.openaiEnabled;
  },
}));

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const CONFIG: RawAppConfig = {
  inference: {
    active_provider: 'ollama',
    keep_warm_inactivity_minutes: 0,
    num_ctx: 16384,
    providers: [
      {
        id: 'builtin',
        kind: 'builtin',
        label: 'Built-in (Thuki)',
        base_url: '',
        model: '',
        vision: false,
      },
      {
        id: 'ollama',
        kind: 'ollama',
        label: 'Ollama',
        base_url: 'http://127.0.0.1:11434',
        model: '',
        vision: false,
      },
    ],
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
  behavior: {
    auto_replace: false,
    auto_close: false,
  },
  search: {
    searxng_url: 'http://127.0.0.1:25017',
    reader_url: 'http://127.0.0.1:25018',
    max_iterations: 3,
    top_k_urls: 10,
    searxng_max_results: 10,
    search_timeout_s: 20,
    reader_per_url_timeout_s: 10,
    reader_batch_timeout_s: 30,
    judge_timeout_s: 30,
    router_timeout_s: 45,
  },
  debug: {
    trace_enabled: false,
  },
};

/** CONFIG with the built-in provider active (Keep Warm shows the engine-status row). */
const BUILTIN_ACTIVE_CONFIG: RawAppConfig = {
  ...CONFIG,
  inference: { ...CONFIG.inference, active_provider: 'builtin' },
};

/** CONFIG plus the single OpenAI-compatible provider record. */
const OPENAI_CONFIG: RawAppConfig = {
  ...CONFIG,
  inference: {
    ...CONFIG.inference,
    providers: [
      ...CONFIG.inference.providers,
      {
        id: 'openai',
        kind: 'openai',
        label: 'LM Studio',
        base_url: 'http://127.0.0.1:1234',
        model: '',
        vision: false,
      },
    ],
  },
};

/** Full engine lifecycle payload for `engine:status` emissions. */
function engineStatus(
  state: 'stopped' | 'starting' | 'loaded' | 'stopping' | 'failed',
) {
  return { state, model_path: '', port: null, error: null };
}

beforeEach(() => {
  // Default to the enabled branch so the openai-card tests render the gated
  // UI; the disabled-state test flips this within its own body.
  devFlags.openaiEnabled = true;
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'get_loaded_model') return Promise.resolve(null);
    if (cmd === 'get_engine_status') {
      return Promise.resolve(engineStatus('stopped'));
    }
    if (cmd === 'get_model_picker_state') {
      return Promise.resolve({ active: null, all: [], ollamaReachable: false });
    }
    if (cmd === 'get_updater_state') {
      return Promise.resolve({
        last_check_at_unix: null,
        update: null,
        settings_snoozed_until: null,
        chat_snoozed_until: null,
      });
    }
    return Promise.resolve(CONFIG);
  });
});

afterEach(() => {
  vi.useRealTimers();
  clearEventHandlers();
});

async function renderModelTab() {
  const view = render(
    <ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />,
  );
  await act(async () => {
    await Promise.resolve();
  });
  return view;
}

describe('ModelTab', () => {
  it('renders Providers and Prompt sections with the expected labels', async () => {
    await renderModelTab();
    expect(screen.getByText('Providers')).toBeInTheDocument();
    expect(screen.getByText('Built-in (Thuki)')).toBeInTheDocument();
    // Built-in is selectable (no more "upcoming version" badge); Ollama is
    // the active provider in this config.
    expect(
      screen.getByRole('radio', { name: 'Use Built-in (Thuki)' }),
    ).not.toBeChecked();
    expect(screen.getByRole('radio', { name: 'Use Ollama' })).toBeChecked();
    expect(screen.getByText('Prompt')).toBeInTheDocument();
    expect(screen.getByText('Ollama URL')).toBeInTheDocument();
    expect(screen.getByText('System prompt')).toBeInTheDocument();
  });

  it('renders the Ollama URL field seeded from the active provider base_url', async () => {
    await renderModelTab();
    const input = screen.getByRole('textbox', {
      name: 'Ollama URL',
    }) as HTMLInputElement;
    expect(input.value).toBe('http://127.0.0.1:11434');
  });

  it('committing a changed Ollama URL invokes set_ollama_url and lifts the config', async () => {
    let savedUrl: unknown;
    const onSaved = vi.fn();
    invokeMock.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_model_picker_state')
        return Promise.resolve({
          active: null,
          all: [],
          ollamaReachable: false,
        });
      if (cmd === 'set_ollama_url') {
        savedUrl = (args as { baseUrl: string }).baseUrl;
        return Promise.resolve(CONFIG);
      }
      return Promise.resolve(CONFIG);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={onSaved} />);
    await act(async () => {
      await Promise.resolve();
    });
    const input = screen.getByRole('textbox', { name: 'Ollama URL' });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: 'http://10.0.0.2:11434' } });
    fireEvent.blur(input);
    await act(async () => {
      await Promise.resolve();
    });
    expect(savedUrl).toBe('http://10.0.0.2:11434');
    expect(onSaved).toHaveBeenCalledWith(CONFIG);
  });

  it('committing an unchanged Ollama URL does not invoke set_ollama_url', async () => {
    await renderModelTab();
    const input = screen.getByRole('textbox', { name: 'Ollama URL' });
    fireEvent.focus(input);
    fireEvent.blur(input);
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_ollama_url',
      expect.anything(),
    );
  });

  it('Enter in the Ollama URL field commits via blur', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_model_picker_state')
        return Promise.resolve({
          active: null,
          all: [],
          ollamaReachable: false,
        });
      return Promise.resolve(CONFIG);
    });
    await renderModelTab();
    const input = screen.getByRole('textbox', { name: 'Ollama URL' });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: 'http://10.0.0.9:11434' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    // Programmatic blur() only fires when the element is focused.
    fireEvent.blur(input);
    await act(async () => {
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('set_ollama_url', {
      baseUrl: 'http://10.0.0.9:11434',
    });
  });

  it('a non-Enter keydown in the Ollama URL field does not commit', async () => {
    await renderModelTab();
    const input = screen.getByRole('textbox', { name: 'Ollama URL' });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: 'http://10.0.0.4:11434' } });
    fireEvent.keyDown(input, { key: 'Tab' });
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_ollama_url',
      expect.anything(),
    );
  });

  it('swallows a set_ollama_url failure without crashing', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_model_picker_state')
        return Promise.resolve({
          active: null,
          all: [],
          ollamaReachable: false,
        });
      if (cmd === 'set_ollama_url')
        return Promise.reject(new Error('write failed'));
      return Promise.resolve(CONFIG);
    });
    await renderModelTab();
    const input = screen.getByRole('textbox', { name: 'Ollama URL' });
    fireEvent.change(input, { target: { value: 'http://10.0.0.3:11434' } });
    fireEvent.blur(input);
    await act(async () => {
      await Promise.resolve();
    });
    // Field still rendered; no throw.
    expect(
      screen.getByRole('textbox', { name: 'Ollama URL' }),
    ).toBeInTheDocument();
  });

  it('shows the non-local warning for a remote URL and hides it for localhost', async () => {
    await renderModelTab();
    const input = screen.getByRole('textbox', { name: 'Ollama URL' });
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: 'http://example.com:11434' } });
    expect(screen.getByRole('alert')).toHaveTextContent(
      /responsible for securing it/,
    );
    fireEvent.change(input, { target: { value: 'http://127.0.0.1:11434' } });
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('renders the model dropdown with installed models and switches on change', async () => {
    let switched: unknown;
    invokeMock.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_model_picker_state') {
        return Promise.resolve({
          active: 'llama3.1:8b',
          all: ['llama3.1:8b', 'phi4:14b'],
          ollamaReachable: true,
        });
      }
      if (cmd === 'set_active_model') {
        switched = (args as { model: string }).model;
        return Promise.resolve(undefined);
      }
      return Promise.resolve(CONFIG);
    });
    await renderModelTab();
    const dropdown = screen.getByRole('combobox', {
      name: 'Active Ollama model',
    }) as HTMLSelectElement;
    expect(dropdown.value).toBe('llama3.1:8b');
    fireEvent.change(dropdown, { target: { value: 'phi4:14b' } });
    await act(async () => {
      await Promise.resolve();
    });
    expect(switched).toBe('phi4:14b');
  });

  it('falls back to the first installed model when none is active', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_model_picker_state') {
        return Promise.resolve({
          active: null,
          all: ['gemma3:12b', 'phi4:14b'],
          ollamaReachable: true,
        });
      }
      return Promise.resolve(CONFIG);
    });
    await renderModelTab();
    const dropdown = screen.getByRole('combobox', {
      name: 'Active Ollama model',
    }) as HTMLSelectElement;
    expect(dropdown.value).toBe('gemma3:12b');
  });

  it('shows a no-models hint when the provider reports no installed models', async () => {
    await renderModelTab();
    expect(screen.getByText('No models installed')).toBeInTheDocument();
    expect(
      screen.queryByRole('combobox', { name: 'Active Ollama model' }),
    ).not.toBeInTheDocument();
  });

  it('hides the Ollama model row entirely when the built-in provider is active', async () => {
    // get_model_picker_state is scoped to the ACTIVE provider, so with the
    // built-in active it returns builtin manifest ids. The Ollama card must
    // not render that inventory (or the no-models hint) as its own.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_model_picker_state') {
        return Promise.resolve({
          active: 'thuki-starter-4b',
          all: ['thuki-starter-4b'],
          ollamaReachable: true,
        });
      }
      return Promise.resolve(BUILTIN_ACTIVE_CONFIG);
    });
    render(
      <ModelTab
        config={BUILTIN_ACTIVE_CONFIG}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      screen.queryByRole('combobox', { name: 'Active Ollama model' }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText('No models installed')).not.toBeInTheDocument();
    // The rest of the Ollama card stays.
    expect(
      screen.getByRole('textbox', { name: 'Ollama URL' }),
    ).toBeInTheDocument();
  });

  it('shows an empty Ollama URL when no Ollama provider is configured', async () => {
    const builtinOnly: RawAppConfig = {
      ...CONFIG,
      inference: {
        ...CONFIG.inference,
        providers: [CONFIG.inference.providers[0]],
      },
    };
    render(
      <ModelTab config={builtinOnly} resyncToken={0} onSaved={() => {}} />,
    );
    await act(async () => {
      await Promise.resolve();
    });
    const input = screen.getByRole('textbox', {
      name: 'Ollama URL',
    }) as HTMLInputElement;
    expect(input.value).toBe('');
  });

  it('does not overwrite the Ollama URL on resync while the field is focused', async () => {
    const { rerender } = await renderModelTab();
    const input = screen.getByRole('textbox', {
      name: 'Ollama URL',
    }) as HTMLInputElement;
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: 'http://typing.in/progress' } });
    const updatedConfig: RawAppConfig = {
      ...CONFIG,
      inference: {
        ...CONFIG.inference,
        providers: [
          CONFIG.inference.providers[0],
          {
            ...CONFIG.inference.providers[1],
            base_url: 'http://10.0.0.8:11434',
          },
        ],
      },
    };
    rerender(
      <ModelTab config={updatedConfig} resyncToken={1} onSaved={() => {}} />,
    );
    expect(input.value).toBe('http://typing.in/progress');
  });

  it('resyncs the Ollama URL field when resyncToken changes', async () => {
    const { rerender } = await renderModelTab();
    const input = screen.getByRole('textbox', {
      name: 'Ollama URL',
    }) as HTMLInputElement;
    expect(input.value).toBe('http://127.0.0.1:11434');
    const updatedConfig: RawAppConfig = {
      ...CONFIG,
      inference: {
        ...CONFIG.inference,
        providers: [
          CONFIG.inference.providers[0],
          {
            ...CONFIG.inference.providers[1],
            base_url: 'http://10.0.0.7:11434',
          },
        ],
      },
    };
    rerender(
      <ModelTab config={updatedConfig} resyncToken={1} onSaved={() => {}} />,
    );
    expect(input.value).toBe('http://10.0.0.7:11434');
  });

  it('no longer renders the auto-replace toggle (moved to the Behavior tab)', async () => {
    await renderModelTab();
    expect(screen.queryByText('Text Replacement')).not.toBeInTheDocument();
    expect(screen.queryByText('Rewrite')).not.toBeInTheDocument();
    expect(
      screen.queryByRole('switch', {
        name: /Auto-replace selected text after \/rewrite or \/refine/,
      }),
    ).not.toBeInTheDocument();
  });

  it('renders the live char counter for the prompt textarea', async () => {
    await renderModelTab();
    expect(screen.getByText(/5 \/ 32000/)).toBeInTheDocument();
  });

  it('renders the prompt textarea with the configured persona text and a tall default size', async () => {
    await renderModelTab();
    const ta = screen.getByRole('textbox', {
      name: 'System prompt',
    }) as HTMLTextAreaElement;
    expect(ta.value).toBe('hello');
    // Default rows must be larger than the generic 4-row Textarea so the
    // seeded built-in prompt body is visible without manual resizing.
    expect(ta.rows).toBeGreaterThanOrEqual(8);
  });

  it('typing into the prompt textarea schedules a save with the typed text', async () => {
    vi.useFakeTimers();
    let savedValue: unknown = undefined;
    invokeMock.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'set_config_field') {
        savedValue = (args as { value: unknown }).value;
        return Promise.resolve(CONFIG);
      }
      return Promise.resolve(CONFIG);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    const ta = screen.getByRole('textbox', {
      name: 'System prompt',
    }) as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: 'new prompt body' } });
    await act(async () => {
      vi.advanceTimersByTime(300);
      await Promise.resolve();
    });
    expect(savedValue).toBe('new prompt body');
  });

  it('renders the Keep Warm section with Release after input and Unload now button', async () => {
    await renderModelTab();
    expect(screen.getByText('Keep Warm')).toBeInTheDocument();
    expect(screen.getByText('Keep active model in memory')).toBeInTheDocument();
    expect(screen.getByText('Release after')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Unload now' }),
    ).toBeInTheDocument();
  });

  it('Unload now button invokes evict_model', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(undefined);
    });
    await renderModelTab();
    fireEvent.click(screen.getByRole('button', { name: 'Unload now' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('evict_model'));
  });

  it('Unload now button is disabled while ejecting and stays disabled after model unloads', async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(undefined);
    });
    await renderModelTab();
    const btn = screen.getByRole('button', { name: 'Unload now' });
    expect(btn).not.toBeDisabled();
    fireEvent.click(btn);
    expect(btn).toBeDisabled(); // disabled from ejecting state
    // Flush microtasks so evict_model resolves, then backend emits model-evicted.
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      emitTauriEvent('warmup:model-evicted', null);
    });
    act(() => {
      vi.advanceTimersByTime(2500); // ejecting clears
    });
    // Button stays disabled because loadedModel is now null.
    expect(btn).toBeDisabled();
  });

  it('Unload now button resets immediately when evict_model rejects', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      if (cmd === 'evict_model')
        return Promise.reject(new Error('connection refused'));
      return Promise.resolve(undefined);
    });
    await renderModelTab();
    const btn = screen.getByRole('button', { name: 'Unload now' });
    expect(btn).not.toBeDisabled();
    fireEvent.click(btn);
    expect(btn).toBeDisabled();
    await act(async () => {
      await Promise.resolve();
    });
    // Ejecting cleared; loadedModel still set (eject failed), button re-enabled.
    expect(btn).not.toBeDisabled();
  });

  it('Unload now button is disabled when no model is loaded in VRAM', async () => {
    await renderModelTab();
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
  });

  it('Unload now button is enabled when a model is loaded in VRAM', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(CONFIG);
    });
    await renderModelTab();
    expect(
      screen.getByRole('button', { name: 'Unload now' }),
    ).not.toBeDisabled();
  });

  it('shows VRAM subtitle with model name and dot when a model is loaded', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(CONFIG);
    });
    await renderModelTab();
    expect(screen.getByText('llama3.2:3b')).toBeInTheDocument();
    expect(screen.getByTestId('vram-status-dot')).toBeInTheDocument();
  });

  it('hides VRAM subtitle when no model is loaded', async () => {
    await renderModelTab();
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
  });

  it('handles get_loaded_model failure gracefully and leaves button disabled', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model')
        return Promise.reject(new Error('network error'));
      return Promise.resolve(CONFIG);
    });
    await renderModelTab();
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
  });

  it('clears VRAM subtitle and keeps button disabled after successful eject', async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(undefined);
    });
    await renderModelTab();
    expect(screen.getByText('llama3.2:3b')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Unload now' }));
    // Flush microtasks so evict_model resolves, then backend emits model-evicted.
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      emitTauriEvent('warmup:model-evicted', null);
    });
    expect(screen.queryByText('llama3.2:3b')).not.toBeInTheDocument();
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
    // Button disabled: ejecting still true (timer not yet fired).
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
    act(() => {
      vi.advanceTimersByTime(2500);
    });
    // After timer: ejecting clears but loadedModel=null keeps button disabled.
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
  });

  it('changing the inactivity minutes input updates its value', async () => {
    await renderModelTab();
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.change(input, { target: { value: '60' } });
    expect((input as HTMLInputElement).value).toBe('60');
  });

  it('allows empty inactivity input mid-edit; blur defaults to 0', async () => {
    await renderModelTab();
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.change(input, { target: { value: '' } });
    expect((input as HTMLInputElement).value).toBe('');
    fireEvent.blur(input);
    expect((input as HTMLInputElement).value).toBe('0');
  });

  it('blur with a valid inactivity value does not reset the field', async () => {
    await renderModelTab();
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.change(input, { target: { value: '60' } });
    fireEvent.blur(input);
    expect((input as HTMLInputElement).value).toBe('60');
  });

  it('clamps below-range inactivity input to -1 immediately', async () => {
    await renderModelTab();
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.change(input, { target: { value: '-99' } });
    expect((input as HTMLInputElement).value).toBe('-1');
  });

  it('clamps above-range inactivity input to 1440 immediately', async () => {
    await renderModelTab();
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.change(input, { target: { value: '9999' } });
    expect((input as HTMLInputElement).value).toBe('1440');
  });

  it('updates VRAM subtitle when warmup:model-loaded event fires', async () => {
    await renderModelTab();
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
    act(() => {
      emitTauriEvent('warmup:model-loaded', 'phi3:mini');
    });
    expect(screen.getByText('phi3:mini')).toBeInTheDocument();
    expect(screen.getByTestId('vram-status-dot')).toBeInTheDocument();
  });

  it('clears VRAM subtitle when warmup:model-evicted event fires', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(CONFIG);
    });
    await renderModelTab();
    expect(screen.getByText('llama3.2:3b')).toBeInTheDocument();
    act(() => {
      emitTauriEvent('warmup:model-evicted', null);
    });
    expect(screen.queryByText('llama3.2:3b')).not.toBeInTheDocument();
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
  });

  it('re-queries get_loaded_model when visibilitychange fires and panel is visible', async () => {
    await renderModelTab();
    // Initially no model loaded.
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();

    // Switch mock: now a model is loaded in VRAM.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(CONFIG);
    });

    // Simulate settings panel becoming visible (document.hidden is false in happy-dom).
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await Promise.resolve();
    });

    expect(screen.getByTestId('vram-status-dot')).toBeInTheDocument();
    expect(screen.getByText('llama3.2:3b')).toBeInTheDocument();
  });

  it('handles get_loaded_model failure gracefully on visibilitychange', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.reject(new Error('fail'));
      return Promise.resolve(CONFIG);
    });
    await renderModelTab();
    // Fires visibilitychange with a rejecting get_loaded_model — covers the .catch path.
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
  });

  it('skips get_loaded_model when visibilitychange fires while document is hidden', async () => {
    await renderModelTab();

    invokeMock.mockClear();

    Object.defineProperty(document, 'hidden', {
      configurable: true,
      get: () => true,
    });

    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await Promise.resolve();
    });

    Object.defineProperty(document, 'hidden', {
      configurable: true,
      get: () => false,
    });

    expect(invokeMock).not.toHaveBeenCalledWith('get_loaded_model');
  });

  it('resyncs inactivity minutes when resyncToken changes', async () => {
    const { rerender } = await renderModelTab();
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    expect((input as HTMLInputElement).value).toBe('0');

    const updatedConfig: RawAppConfig = {
      ...CONFIG,
      inference: { ...CONFIG.inference, keep_warm_inactivity_minutes: 60 },
    };
    rerender(
      <ModelTab config={updatedConfig} resyncToken={1} onSaved={() => {}} />,
    );
    expect((input as HTMLInputElement).value).toBe('60');
  });

  it('resync does not overwrite rawMin while input is focused', async () => {
    const { rerender } = await renderModelTab();
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '' } });
    expect((input as HTMLInputElement).value).toBe('');

    const updatedConfig: RawAppConfig = {
      ...CONFIG,
      inference: { ...CONFIG.inference, keep_warm_inactivity_minutes: 60 },
    };
    rerender(
      <ModelTab config={updatedConfig} resyncToken={1} onSaved={() => {}} />,
    );
    expect((input as HTMLInputElement).value).toBe('');
  });

  it('renders Context Window section with label, slider, chip, tick marks, and VRAM note', async () => {
    await renderModelTab();
    expect(screen.getByText('Context Window')).toBeInTheDocument();
    expect(screen.getByText('Context window')).toBeInTheDocument();
    expect(
      screen.getByRole('slider', { name: 'Context window tokens' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('spinbutton', { name: 'Context window tokens' }),
    ).toBeInTheDocument();
    // Tick marks
    expect(screen.getByText('8K')).toBeInTheDocument();
    expect(screen.getByText('16K')).toBeInTheDocument();
    expect(screen.getByText('1M')).toBeInTheDocument();
    // VRAM note
    expect(
      screen.getByText(
        /doubling the context roughly doubles its memory footprint/,
      ),
    ).toBeInTheDocument();
    // Embedded button opens the tuning doc on GitHub via open_url so the
    // link works inside the Tauri webview (target="_blank" is a no-op here).
    const tuneButton = screen.getByRole('button', {
      name: /how to tune Context Window/i,
    });
    fireEvent.click(tuneButton);
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/blob/main/docs/tuning-context-window.md#the-5-minute-benchmark-recipe',
    });
  });

  it('typing a valid value in the chip and blurring commits it', async () => {
    await renderModelTab();
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: '32768' } });
    fireEvent.blur(chip);
    expect(chip.value).toBe('32768');
  });

  it('typing an invalid value in the chip and blurring reverts to committed value', async () => {
    await renderModelTab();
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: 'abc' } });
    fireEvent.blur(chip);
    expect(chip.value).toBe('16384');
  });

  it('typing a value below CTX_MIN and blurring reverts to committed value', async () => {
    await renderModelTab();
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: '512' } });
    fireEvent.blur(chip);
    expect(chip.value).toBe('16384');
  });

  it('typing a value above CTX_MAX and blurring clamps to CTX_MAX', async () => {
    await renderModelTab();
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: '99999999' } });
    fireEvent.blur(chip);
    expect(chip.value).toBe('1048576');
  });

  it('Enter key in chip commits by blurring', async () => {
    await renderModelTab();
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: '131072' } });
    fireEvent.keyDown(chip, { key: 'Enter' });
    expect(chip.value).toBe('131072');
  });

  it('non-Enter keyDown in chip does not commit', async () => {
    await renderModelTab();
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: '32768' } });
    fireEvent.keyDown(chip, { key: 'Tab' });
    // No blur triggered, so the chip still shows the in-progress text.
    expect(chip.value).toBe('32768');
  });

  it('slider onChange updates chip text via posToCtx', async () => {
    await renderModelTab();
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // pos=556 → 2048 * 512^(556/1000) ≈ 64K (65536) with CTX_MAX=1M
    fireEvent.change(slider, { target: { value: '556' } });
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    expect(chip.value).toBe('65536');
  });

  it('slider onMouseUp commits the current slider position', async () => {
    await renderModelTab();
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // pos=444 → 2048 * 512^(444/1000) ≈ 32K (32768) with CTX_MAX=1M
    fireEvent.change(slider, { target: { value: '444' } });
    fireEvent.mouseUp(slider);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    expect(chip.value).toBe('32768');
  });

  it('slider onTouchEnd commits the current slider position', async () => {
    await renderModelTab();
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // pos=667 → 2048 * 512^(667/1000) ≈ 128K (131072) with CTX_MAX=1M
    fireEvent.change(slider, { target: { value: '667' } });
    fireEvent.touchEnd(slider);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    expect(chip.value).toBe('131072');
  });

  it('slider onKeyUp commits when not in a drag sequence', async () => {
    await renderModelTab();
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // No preceding onChange, so ctxDraggingRef is false → onKeyUp commits.
    fireEvent.keyUp(slider);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // No position change yet; committed value stays 16384.
    expect(chip.value).toBe('16384');
  });

  it('slider onKeyUp does not commit when a drag is in progress', async () => {
    await renderModelTab();
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // onChange sets ctxDraggingRef to true; wrap in act so React flushes the
    // setCtxPos/setCtxChip state updates before the keyUp fires.
    act(() => {
      fireEvent.change(slider, { target: { value: '556' } });
    });
    // onKeyUp while dragging: skips commitCtx, chip still shows intermediate.
    fireEvent.keyUp(slider);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // pos=556 → 64K (65536); numCtx unchanged, chip shows the intermediate value.
    expect(chip.value).toBe('65536');
  });

  it('resyncs context window chip and slider when resyncToken changes', async () => {
    const { rerender } = await renderModelTab();
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    expect(chip.value).toBe('16384');

    const updatedConfig: RawAppConfig = {
      ...CONFIG,
      inference: { ...CONFIG.inference, num_ctx: 65536 },
    };
    rerender(
      <ModelTab config={updatedConfig} resyncToken={1} onSaved={() => {}} />,
    );
    expect(chip.value).toBe('65536');
  });

  it('renders the collapsed Diagnostics trigger and hides its content by default', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(
      screen.getByRole('button', { name: /Diagnostics/i }),
    ).toBeInTheDocument();
    expect(screen.queryByText('Trace recording')).not.toBeInTheDocument();
  });

  it('expands the Diagnostics section and reveals the trace toggle when clicked', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/i }));
    expect(screen.getByText('Trace recording')).toBeInTheDocument();
    const toggle = screen.getByRole('switch', {
      name: 'Enable trace recording',
    });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
  });

  it('reflects trace_enabled=true from config when the section is expanded', () => {
    const configOn: RawAppConfig = {
      ...CONFIG,
      debug: { trace_enabled: true },
    };
    render(<ModelTab config={configOn} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/i }));
    const toggle = screen.getByRole('switch', {
      name: 'Enable trace recording',
    });
    expect(toggle).toHaveAttribute('aria-checked', 'true');
  });

  // ─── Providers panel: radio selection ───────────────────────────────────

  it('selecting the Built-in radio invokes set_active_provider and lifts the config', async () => {
    const onSaved = vi.fn();
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={onSaved} />);
    await act(async () => {
      await Promise.resolve();
    });
    fireEvent.click(
      screen.getByRole('radio', { name: 'Use Built-in (Thuki)' }),
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('set_active_provider', {
      providerId: 'builtin',
    });
    expect(onSaved).toHaveBeenCalledWith(CONFIG);
  });

  it('falls back to the literal builtin id and label when no builtin provider is configured', async () => {
    const noBuiltin: RawAppConfig = {
      ...CONFIG,
      inference: {
        ...CONFIG.inference,
        providers: [CONFIG.inference.providers[1]],
      },
    };
    render(<ModelTab config={noBuiltin} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText('Built-in (Thuki)')).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole('radio', { name: 'Use Built-in (Thuki)' }),
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('set_active_provider', {
      providerId: 'builtin',
    });
  });

  it('selecting the Ollama radio invokes set_active_provider with the ollama id', async () => {
    const onSaved = vi.fn();
    render(
      <ModelTab
        config={BUILTIN_ACTIVE_CONFIG}
        resyncToken={0}
        onSaved={onSaved}
      />,
    );
    await act(async () => {
      await Promise.resolve();
    });
    fireEvent.click(screen.getByRole('radio', { name: 'Use Ollama' }));
    await act(async () => {
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('set_active_provider', {
      providerId: 'ollama',
    });
    expect(onSaved).toHaveBeenCalledWith(CONFIG);
  });

  it('swallows a set_active_provider failure without crashing', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_model_picker_state')
        return Promise.resolve({
          active: null,
          all: [],
          ollamaReachable: false,
        });
      if (cmd === 'set_active_provider')
        return Promise.reject(new Error('write failed'));
      return Promise.resolve(CONFIG);
    });
    const onSaved = vi.fn();
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={onSaved} />);
    await act(async () => {
      await Promise.resolve();
    });
    fireEvent.click(
      screen.getByRole('radio', { name: 'Use Built-in (Thuki)' }),
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(onSaved).not.toHaveBeenCalled();
    expect(
      screen.getByRole('radio', { name: 'Use Built-in (Thuki)' }),
    ).toBeInTheDocument();
  });

  it('renders the OpenAI-compatible card when configured and selects it via its radio', async () => {
    render(
      <ModelTab config={OPENAI_CONFIG} resyncToken={0} onSaved={() => {}} />,
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText('LM Studio')).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Add OpenAI-compatible server' }),
    ).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole('radio', { name: 'Use OpenAI-compatible server' }),
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('set_active_provider', {
      providerId: 'openai',
    });
  });

  it('hides every OpenAI-compatible affordance when the dev flag is disabled', async () => {
    devFlags.openaiEnabled = false;
    // No openai provider configured: the "add a server" affordance is gone.
    const { unmount } = render(
      <ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />,
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      screen.queryByRole('button', { name: 'Add OpenAI-compatible server' }),
    ).not.toBeInTheDocument();
    unmount();

    // An openai provider hand-edited into config: its management card and
    // radio stay hidden too (the backend still honors it).
    render(
      <ModelTab config={OPENAI_CONFIG} resyncToken={0} onSaved={() => {}} />,
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByText('LM Studio')).not.toBeInTheDocument();
    expect(
      screen.queryByRole('radio', { name: 'Use OpenAI-compatible server' }),
    ).not.toBeInTheDocument();
  });

  // ─── Keep Warm with the built-in provider active ────────────────────────

  async function renderBuiltinActive(
    onSaved: (next: RawAppConfig) => void = () => {},
  ) {
    const view = render(
      <ModelTab
        config={BUILTIN_ACTIVE_CONFIG}
        resyncToken={0}
        onSaved={onSaved}
      />,
    );
    await act(async () => {
      await Promise.resolve();
    });
    return view;
  }

  it('renders the unified Keep Warm control with the engine-status row when the built-in provider is active', async () => {
    await renderBuiltinActive();
    // Same single Keep Warm section as Ollama, but the built-in status row
    // reports the sidecar lifecycle instead of the VRAM slug.
    expect(screen.getByText('Keep Warm')).toBeInTheDocument();
    expect(screen.getByText('Keep active model in memory')).toBeInTheDocument();
    expect(screen.queryByText('Idle Unload')).not.toBeInTheDocument();
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
    expect(screen.getByText('Engine: stopped')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
  });

  it('clamps the Keep Warm input to the -1..1440 range while built-in is active', async () => {
    await renderBuiltinActive();
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    }) as HTMLInputElement;
    fireEvent.change(input, { target: { value: '45' } });
    expect(input.value).toBe('45');
    fireEvent.change(input, { target: { value: '-5' } });
    expect(input.value).toBe('-1');
    fireEvent.change(input, { target: { value: '99999' } });
    expect(input.value).toBe('1440');
  });

  it('resync does not overwrite the Keep Warm input while focused (built-in active)', async () => {
    const { rerender } = await renderBuiltinActive();
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    }) as HTMLInputElement;
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '25' } });
    const updatedConfig: RawAppConfig = {
      ...BUILTIN_ACTIVE_CONFIG,
      inference: {
        ...BUILTIN_ACTIVE_CONFIG.inference,
        keep_warm_inactivity_minutes: 90,
      },
    };
    rerender(
      <ModelTab config={updatedConfig} resyncToken={1} onSaved={() => {}} />,
    );
    expect(input.value).toBe('25');
  });

  it('engine:status loaded enables Unload now and clicking invokes evict_model', async () => {
    await renderBuiltinActive();
    act(() => {
      emitTauriEvent('engine:status', engineStatus('loaded'));
    });
    expect(screen.getByText('Engine: loaded')).toBeInTheDocument();
    const btn = screen.getByRole('button', { name: 'Unload now' });
    expect(btn).toBeEnabled();
    fireEvent.click(btn);
    await act(async () => {
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('evict_model');
  });

  it('swallows an evict_model failure from the engine Unload now button', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_model_picker_state')
        return Promise.resolve({
          active: null,
          all: [],
          ollamaReachable: false,
        });
      if (cmd === 'evict_model')
        return Promise.reject(new Error('no engine running'));
      return Promise.resolve(CONFIG);
    });
    await renderBuiltinActive();
    act(() => {
      emitTauriEvent('engine:status', engineStatus('loaded'));
    });
    fireEvent.click(screen.getByRole('button', { name: 'Unload now' }));
    await act(async () => {
      await Promise.resolve();
    });
    // The residency line is event-driven, so a failed eviction changes nothing.
    expect(screen.getByText('Engine: loaded')).toBeInTheDocument();
  });

  // ─── Context slider "Applying" hint ─────────────────────────────────────

  it('shows the Applying hint while the engine starts or stops and hides it otherwise', async () => {
    await renderBuiltinActive();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    act(() => {
      emitTauriEvent('engine:status', engineStatus('starting'));
    });
    expect(screen.getByRole('status')).toHaveTextContent(/Applying/);
    act(() => {
      emitTauriEvent('engine:status', engineStatus('stopping'));
    });
    expect(screen.getByRole('status')).toHaveTextContent(/Applying/);
    act(() => {
      emitTauriEvent('engine:status', engineStatus('loaded'));
    });
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  // ─── Engine status mount seeding + listener cleanup ─────────────────────

  it('seeds the residency line from get_engine_status on mount', async () => {
    // The backend emits engine:status only on transitions; an engine that
    // is already loaded must be reflected (and Unload now enabled) without
    // waiting for the next event.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_engine_status') {
        return Promise.resolve(engineStatus('loaded'));
      }
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_model_picker_state') {
        return Promise.resolve({
          active: null,
          all: [],
          ollamaReachable: false,
        });
      }
      return Promise.resolve(CONFIG);
    });
    await renderBuiltinActive();
    expect(screen.getByText('Engine: loaded')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeEnabled();
  });

  it('keeps the stopped default when the get_engine_status seed rejects', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_engine_status') {
        return Promise.reject(new Error('runner not managed'));
      }
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_model_picker_state') {
        return Promise.resolve({
          active: null,
          all: [],
          ollamaReachable: false,
        });
      }
      return Promise.resolve(CONFIG);
    });
    await renderBuiltinActive();
    expect(screen.getByText('Engine: stopped')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
  });

  it('detaches every listener even when unmount races the listen promise', async () => {
    // Regression for the leak where cleanup ran before listen() resolved
    // and the captured unlisten was still null, leaving the handler
    // registered forever. The promise-chained cleanup must detach all of
    // them once the registrations resolve.
    const listenMock = listen as unknown as ReturnType<typeof vi.fn>;
    const original = listenMock.getMockImplementation();
    let removed = 0;
    listenMock.mockImplementation(async () => () => {
      removed += 1;
    });
    try {
      const before = listenMock.mock.calls.length;
      const view = render(
        <ModelTab
          config={BUILTIN_ACTIVE_CONFIG}
          resyncToken={0}
          onSaved={() => {}}
        />,
      );
      const registered = listenMock.mock.calls.length - before;
      expect(registered).toBe(3); // engine:status + the warmup pair
      // Unmount before the listen promises are flushed.
      view.unmount();
      await act(async () => {
        await Promise.resolve();
      });
      expect(removed).toBe(registered);
    } finally {
      listenMock.mockImplementation(original!);
    }
  });

  // ─── Context Window helper copy per provider kind ────────────────────────

  it('shows the builtin ctx helper while the built-in provider is active', async () => {
    await renderBuiltinActive();
    expect(
      screen.getByText(/--ctx-size at start; changing it restarts the engine/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Ollama caps/)).not.toBeInTheDocument();
  });

  it('shows the server-controlled ctx helper for an openai provider', async () => {
    const cfg: RawAppConfig = {
      ...OPENAI_CONFIG,
      inference: { ...OPENAI_CONFIG.inference, active_provider: 'openai' },
    };
    render(<ModelTab config={cfg} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      screen.getByText(
        /Informational only; your server controls the actual context/,
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Ollama caps/)).not.toBeInTheDocument();
  });

  it('keeps the Ollama ctx helper for the ollama provider', async () => {
    await renderModelTab();
    expect(
      screen.getByText(/Ollama caps to your model's trained maximum\./),
    ).toBeInTheDocument();
  });
});

describe('DisplayTab', () => {
  it('renders Text, Window, and Input sections', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Text')).toBeInTheDocument();
    expect(screen.getByText('Window')).toBeInTheDocument();
    expect(screen.getByText('Input')).toBeInTheDocument();
    expect(screen.getByText('Text size')).toBeInTheDocument();
    expect(screen.getByText('Line height')).toBeInTheDocument();
    expect(screen.getByText('Letter spacing')).toBeInTheDocument();
    expect(screen.getByText('Font weight')).toBeInTheDocument();
    expect(screen.getByText('Overlay width')).toBeInTheDocument();
    expect(screen.getByText('Max display lines')).toBeInTheDocument();
  });

  it('exposes a text-size slider bound to the 11..22 px range', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', { name: 'Text size' });
    expect(slider).toHaveAttribute('min', '11');
    expect(slider).toHaveAttribute('max', '22');
    expect(slider).toHaveAttribute('step', '0.5');
    expect(slider).toHaveValue(String(CONFIG.window.text_base_px));
  });

  it('exposes a line-height slider bound to the 1..2.5 range', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', { name: 'Line height' });
    expect(slider).toHaveAttribute('min', '1');
    expect(slider).toHaveAttribute('max', '2.5');
    expect(slider).toHaveAttribute('step', '0.05');
  });

  it('exposes a letter-spacing slider bound to the -0.5..2 px range', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', { name: 'Letter spacing' });
    expect(slider).toHaveAttribute('min', '-0.5');
    expect(slider).toHaveAttribute('max', '2');
    expect(slider).toHaveAttribute('step', '0.05');
  });

  it('exposes a font-weight slider snapping to the four loaded Nunito weights', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', { name: 'Font weight' });
    expect(slider).toHaveAttribute('min', '400');
    expect(slider).toHaveAttribute('max', '700');
    expect(slider).toHaveAttribute('step', '100');
    expect(slider).toHaveValue(String(CONFIG.window.text_font_weight));
    // The chip + screen-reader text surface the descriptive weight label
    // (e.g. "Medium") rather than the raw numeric font-weight value.
    expect(slider).toHaveAttribute('aria-valuetext', 'Medium');
  });
});

describe('SearchTab', () => {
  it('renders Services, Pipeline, and Timeouts sections', () => {
    render(<SearchTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Services')).toBeInTheDocument();
    expect(screen.getByText('Pipeline')).toBeInTheDocument();
    expect(screen.getByText('Timeouts')).toBeInTheDocument();
    expect(screen.getByText('SearXNG URL')).toBeInTheDocument();
    expect(screen.getByText('Per-URL timeout')).toBeInTheDocument();
    expect(screen.getByText('Batch timeout')).toBeInTheDocument();
    expect(screen.getByText('Router timeout')).toBeInTheDocument();
  });

  it('does not render any Diagnostics affordance', () => {
    render(<SearchTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.queryByText(/Diagnostics/i)).not.toBeInTheDocument();
    expect(screen.queryByText('Trace recording')).not.toBeInTheDocument();
  });
});

describe('AboutTab', () => {
  async function renderAbout() {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return true;
      if (cmd === 'get_updater_state') {
        return {
          last_check_at_unix: null,
          update: null,
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        };
      }
      return CONFIG;
    });
    const view = render(
      <AboutTab onSaved={() => {}} onReload={async () => {}} />,
    );
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    return view;
  }

  it('renders the centered hero with title, version, and tagline', async () => {
    await renderAbout();
    expect(screen.getByText('Thuki')).toBeInTheDocument();
    expect(screen.getByText(/A floating, local-first AI/)).toBeInTheDocument();
    expect(
      screen.getByText(/No cloud\. No clutter\. Just answers\./),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getAllByText(/Granted/).length).toBeGreaterThan(0),
    );
  });

  it('version button links to the stable release tag when no SHA is set', async () => {
    await renderAbout();
    await waitFor(() => screen.getByText(/v\d/));
    fireEvent.click(
      screen.getByRole('button', { name: /release notes on GitHub/ }),
    );
    expect(invokeMock).toHaveBeenCalledWith(
      'open_url',
      expect.objectContaining({
        url: expect.stringContaining('/releases/tag/v'),
      }),
    );
  });

  it('version button links to the nightly release and shows build metadata when VITE_GIT_COMMIT_SHA is set', async () => {
    vi.stubEnv('VITE_GIT_COMMIT_SHA', 'abc1234def');
    await renderAbout();
    // The header version contains "nightly"
    await waitFor(() =>
      expect(screen.getAllByText(/nightly/).length).toBeGreaterThan(0),
    );
    fireEvent.click(
      screen.getByRole('button', { name: /release notes on GitHub/ }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/releases/tag/nightly',
    });
    vi.unstubAllEnvs();
  });

  it('GitHub icon button opens the repo', async () => {
    await renderAbout();
    fireEvent.click(
      screen.getByRole('button', { name: 'View Thuki on GitHub' }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki',
    });
  });

  it('X icon button opens @quiet_node', async () => {
    await renderAbout();
    fireEvent.click(
      screen.getByRole('button', { name: /Reach out to Logan on X/ }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://x.com/quiet_node',
    });
  });

  it('Feedback icon button opens GitHub Issues', async () => {
    await renderAbout();
    fireEvent.click(screen.getByRole('button', { name: /Open an issue/ }));
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/issues',
    });
  });

  it('Globe icon button opens thuki.app', async () => {
    await renderAbout();
    fireEvent.click(screen.getByRole('button', { name: /Visit thuki.app/ }));
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://www.thuki.app/',
    });
  });

  it('Reveal Thuki app data invokes reveal_config_in_finder', async () => {
    await renderAbout();
    await waitFor(() => screen.getByText(/Reveal Thuki app data/));
    fireEvent.click(
      screen.getByRole('button', { name: /Reveal Thuki app data/ }),
    );
    expect(invokeMock).toHaveBeenCalledWith('reveal_config_in_finder');
  });

  it('Refresh config.toml invokes the supplied onReload', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return true;
      return CONFIG;
    });
    const onReload = vi.fn(async () => {});
    render(<AboutTab onSaved={() => {}} onReload={onReload} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() => screen.getByText(/Refresh config\.toml/));
    fireEvent.click(
      screen.getByRole('button', { name: /Refresh config\.toml/ }),
    );
    expect(onReload).toHaveBeenCalled();
  });

  it('Reset all opens the confirm dialog and a Cancel keeps the file untouched', async () => {
    await renderAbout();
    fireEvent.click(screen.getByRole('button', { name: /Reset all/ }));
    expect(
      screen.getByText(/Reset all settings to defaults/),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByText(/Reset all settings to defaults\?/)).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'reset_config',
      expect.anything(),
    );
  });

  it('Reset all confirm invokes reset_config({ section: null }) and lifts the resolved config', async () => {
    const onSaved = vi.fn();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return true;
      if (cmd === 'reset_config') return CONFIG;
      return CONFIG;
    });
    render(<AboutTab onSaved={onSaved} onReload={async () => {}} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() => screen.getByRole('button', { name: /Reset all/ }));

    fireEvent.click(screen.getByRole('button', { name: /Reset all/ }));
    fireEvent.click(screen.getByRole('button', { name: 'Reset all' }));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('reset_config', { section: null });
    expect(onSaved).toHaveBeenCalledWith(CONFIG);
  });

  it('renders Required pills + System Settings shortcuts when permissions are missing', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return false;
      if (cmd === 'check_screen_recording_permission') return false;
      return CONFIG;
    });
    render(<AboutTab onSaved={() => {}} onReload={async () => {}} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    await waitFor(() =>
      expect(screen.getAllByText(/Required/).length).toBeGreaterThan(0),
    );

    const accBtn = screen.getAllByRole('button', {
      name: 'Open System Settings',
    })[0];
    fireEvent.click(accBtn);
    expect(invokeMock).toHaveBeenCalledWith('open_accessibility_settings');

    const screenBtn = screen.getAllByRole('button', {
      name: 'Open System Settings',
    })[1];
    fireEvent.click(screenBtn);
    expect(invokeMock).toHaveBeenCalledWith('open_screen_recording_settings');
  });

  it('window focus event triggers a permission re-probe', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return true;
      return CONFIG;
    });
    render(<AboutTab onSaved={() => {}} onReload={async () => {}} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('check_accessibility_permission'),
    );
    invokeMock.mockClear();
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('check_accessibility_permission');
  });

  it('drops the late permission probe result when the component unmounts first', async () => {
    let resolveAcc: ((v: boolean) => void) | undefined;
    let resolveScreen: ((v: boolean) => void) | undefined;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') {
        return new Promise<boolean>((r) => {
          resolveAcc = r;
        });
      }
      if (cmd === 'check_screen_recording_permission') {
        return new Promise<boolean>((r) => {
          resolveScreen = r;
        });
      }
      return CONFIG;
    });

    const { unmount } = render(
      <AboutTab onSaved={() => {}} onReload={async () => {}} />,
    );
    await act(async () => {
      await Promise.resolve();
    });

    // Tear down before the probe resolves — the post-await `if (mounted)`
    // guard must stop the setPerms call.
    unmount();
    await act(async () => {
      resolveAcc?.(true);
      resolveScreen?.(true);
      await Promise.resolve();
      await Promise.resolve();
    });
    // No assertion needed; the test passes if no React state-update warning
    // is logged.
  });

  it('permission probe failures leave the previous pill state in place', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (
        cmd === 'check_accessibility_permission' ||
        cmd === 'check_screen_recording_permission'
      ) {
        throw new Error('probe failed');
      }
      return CONFIG;
    });
    render(<AboutTab onSaved={() => {}} onReload={async () => {}} />);
    // Just confirm it doesn't crash; default state is "Required".
    await waitFor(() =>
      expect(screen.getAllByText(/Required/).length).toBeGreaterThan(0),
    );
  });
});

describe('BehaviorTab', () => {
  const TOGGLE_NAME = /Auto-replace selected text after \/rewrite or \/refine/;

  it('renders the Text Replacement section with the Auto-replace toggle', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Text Replacement')).toBeInTheDocument();
    // Label is the short one-line form; full copy lives in the "?" tooltip.
    expect(screen.getByText('Auto-replace')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: TOGGLE_NAME })).toHaveAttribute(
      'aria-checked',
      'false',
    );
  });

  it('reflects an enabled auto_replace value on the toggle', () => {
    render(
      <BehaviorTab
        config={{
          ...CONFIG,
          behavior: { auto_replace: true, auto_close: false },
        }}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    expect(screen.getByRole('switch', { name: TOGGLE_NAME })).toHaveAttribute(
      'aria-checked',
      'true',
    );
  });

  const CLOSE_NAME = /Close Thuki after replacing selected text/;

  it('renders the Auto-close toggle in the Text Replacement section', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Auto-close')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: CLOSE_NAME })).toHaveAttribute(
      'aria-checked',
      'false',
    );
  });

  it('reflects an enabled auto_close value on the toggle', () => {
    render(
      <BehaviorTab
        config={{
          ...CONFIG,
          behavior: { auto_replace: false, auto_close: true },
        }}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    expect(screen.getByRole('switch', { name: CLOSE_NAME })).toHaveAttribute(
      'aria-checked',
      'true',
    );
  });

  it('opens the help tooltip upward so it is not clipped at the short window edge', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const help = screen.getByRole('button', { name: 'About Auto-replace' });
    fireEvent.mouseEnter(help.parentElement!);
    // placement="top" positions the tooltip box with a `translate(..., -100%)`
    // transform (it sits above the trigger). The default "bottom" placement
    // uses `translateX(-50%)`, which here would overflow the bottom edge.
    expect(
      document.body.querySelector('[style*="translate(-50%, -100%)"]'),
    ).not.toBeNull();
  });

  it('shows a scope help tooltip on the Text Replacement section heading', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.mouseEnter(
      screen.getByRole('button', { name: 'About Text Replacement' })
        .parentElement!,
    );
    expect(
      screen.getByText(/Applies only to \/rewrite and \/refine/),
    ).toBeInTheDocument();
  });
});
