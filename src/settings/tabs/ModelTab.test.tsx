/**
 * Tests for the Models router's provider gating: Library and Discover manage
 * the built-in engine's models, so while a non-built-in provider is active
 * they are shown behind a switch-to-built-in gate. Providers is never gated.
 * The router's plain view switching is covered in tabs.test.
 */

import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';
import { clearEventHandlers } from '../../testUtils/mocks/tauri';

import { ModelTab } from './ModelTab';
import { DownloadsProvider } from '../../contexts/DownloadsContext';
import type { RawAppConfig } from '../types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

function buildConfig(
  activeProvider: string,
  providers: RawAppConfig['inference']['providers'],
): RawAppConfig {
  return {
    inference: {
      active_provider: activeProvider,
      keep_warm_inactivity_minutes: 0,
      num_ctx: 16384,
      providers,
    },
    prompt: { system: 'hello' },
    window: {
      overlay_width: 600,
      max_chat_height: 400,
      max_images: 4,
      text_base_px: 16,
      text_line_height: 1.5,
      text_letter_spacing_px: 0,
      text_font_weight: 400,
    },
    quote: {
      max_display_lines: 3,
      max_display_chars: 200,
      max_context_length: 4000,
    },
    behavior: { auto_replace: false, auto_close: false, auto_search: true },
    search: {
      searxng_url: '',
      reader_url: '',
      max_iterations: 3,
      top_k_urls: 5,
      searxng_max_results: 10,
      search_timeout_s: 30,
      reader_per_url_timeout_s: 10,
      reader_batch_timeout_s: 20,
      judge_timeout_s: 15,
      router_timeout_s: 15,
    },
    debug: { trace_enabled: false },
  };
}

const BUILTIN = {
  id: 'builtin',
  kind: 'builtin',
  label: 'Built-in',
  base_url: '',
  model: '',
  vision: false,
};
const OLLAMA = {
  id: 'ollama',
  kind: 'ollama',
  label: 'Ollama',
  base_url: 'http://127.0.0.1:11434',
  model: '',
  vision: false,
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'get_loaded_model') return Promise.resolve(null);
    if (cmd === 'get_engine_status') {
      return Promise.resolve({ state: 'stopped' });
    }
    if (cmd === 'get_model_picker_state') {
      return Promise.resolve({ active: null, all: [], ollamaReachable: false });
    }
    if (cmd === 'list_installed_models') return Promise.resolve([]);
    return Promise.resolve(buildConfig('ollama', [BUILTIN, OLLAMA]));
  });
});

afterEach(() => {
  clearEventHandlers();
});

async function renderTab(config: RawAppConfig, onSaved = () => {}) {
  const view = render(
    <ModelTab config={config} resyncToken={0} onSaved={onSaved} />,
    { wrapper: DownloadsProvider },
  );
  await act(async () => {
    await Promise.resolve();
  });
  return view;
}

async function open(name: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole('tab', { name }));
    await Promise.resolve();
  });
}

describe('ModelTab provider gating', () => {
  it('gates the Library view while a non-built-in provider is active', async () => {
    await renderTab(buildConfig('ollama', [BUILTIN, OLLAMA]));
    await open('Library');
    expect(
      screen.getByRole('button', { name: 'Switch to built-in' }),
    ).toBeInTheDocument();
    expect(screen.getByText(/You're using Ollama now/)).toBeInTheDocument();
  });

  it('gates the Discover view while a non-built-in provider is active', async () => {
    await renderTab(buildConfig('ollama', [BUILTIN, OLLAMA]));
    await open('Discover');
    expect(
      screen.getByRole('button', { name: 'Switch to built-in' }),
    ).toBeInTheDocument();
  });

  it('does not gate Library when the built-in engine is active', async () => {
    await renderTab(buildConfig('builtin', [BUILTIN, OLLAMA]));
    await open('Library');
    expect(
      screen.queryByRole('button', { name: 'Switch to built-in' }),
    ).toBeNull();
  });

  it('never gates the Providers view', async () => {
    await renderTab(buildConfig('ollama', [BUILTIN, OLLAMA]));
    expect(
      screen.queryByRole('button', { name: 'Switch to built-in' }),
    ).toBeNull();
    expect(screen.getByText('Active provider')).toBeInTheDocument();
  });

  it('switches to the built-in provider when the gate button is clicked', async () => {
    const next = buildConfig('builtin', [BUILTIN, OLLAMA]);
    const onSaved = vi.fn();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'set_active_provider') return Promise.resolve(next);
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_engine_status')
        return Promise.resolve({ state: 'stopped' });
      if (cmd === 'get_model_picker_state') {
        return Promise.resolve({
          active: null,
          all: [],
          ollamaReachable: false,
        });
      }
      if (cmd === 'list_installed_models') return Promise.resolve([]);
      return Promise.resolve(buildConfig('ollama', [BUILTIN, OLLAMA]));
    });
    await renderTab(buildConfig('ollama', [BUILTIN, OLLAMA]), onSaved);
    await open('Library');
    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Switch to built-in' }),
      );
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('set_active_provider', {
      providerId: 'builtin',
    });
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(next));
  });

  it('does nothing when there is no built-in provider to switch to', async () => {
    await renderTab(buildConfig('ollama', [OLLAMA]));
    await open('Library');
    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Switch to built-in' }),
      );
      await Promise.resolve();
    });
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_active_provider',
      expect.anything(),
    );
  });

  it('swallows a failed switch without throwing', async () => {
    const onSaved = vi.fn();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'set_active_provider')
        return Promise.reject(new Error('nope'));
      if (cmd === 'get_loaded_model') return Promise.resolve(null);
      if (cmd === 'get_engine_status')
        return Promise.resolve({ state: 'stopped' });
      if (cmd === 'get_model_picker_state') {
        return Promise.resolve({
          active: null,
          all: [],
          ollamaReachable: false,
        });
      }
      if (cmd === 'list_installed_models') return Promise.resolve([]);
      return Promise.resolve(buildConfig('ollama', [BUILTIN, OLLAMA]));
    });
    await renderTab(buildConfig('ollama', [BUILTIN, OLLAMA]), onSaved);
    await open('Library');
    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Switch to built-in' }),
      );
      await Promise.resolve();
    });
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('falls back to a generic label when the active provider is unresolved', async () => {
    await renderTab(buildConfig('mystery', [BUILTIN, OLLAMA]));
    await open('Library');
    expect(
      screen.getByText(/You're using another provider now/),
    ).toBeInTheDocument();
  });
});

describe('ModelTab pending deep-link', () => {
  it('opens straight on Discover for a pending view and clears it', async () => {
    const onConsumed = vi.fn();
    render(
      <ModelTab
        config={buildConfig('ollama', [BUILTIN, OLLAMA])}
        resyncToken={0}
        onSaved={() => {}}
        pendingView="discover"
        onPendingViewConsumed={onConsumed}
      />,
      { wrapper: DownloadsProvider },
    );
    await act(async () => {
      await Promise.resolve();
    });
    // Discover is the active sub-view (the ollama gate, not the default
    // Providers "Active provider" surface).
    expect(
      screen.getByRole('button', { name: 'Switch to built-in' }),
    ).toBeInTheDocument();
    expect(screen.queryByText('Active provider')).toBeNull();
    expect(onConsumed).toHaveBeenCalledTimes(1);
  });

  it('applies a pending view without a consume callback', async () => {
    render(
      <ModelTab
        config={buildConfig('ollama', [BUILTIN, OLLAMA])}
        resyncToken={0}
        onSaved={() => {}}
        pendingView="discover"
      />,
      { wrapper: DownloadsProvider },
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      screen.getByRole('button', { name: 'Switch to built-in' }),
    ).toBeInTheDocument();
  });
});
