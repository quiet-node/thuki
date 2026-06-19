import { render, screen, waitFor, act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';
import {
  emitTauriEvent,
  clearEventHandlers,
} from '../../testUtils/mocks/tauri';

import { RunningModelFooter } from './RunningModelFooter';
import type { RawAppConfig, RawProvider } from '../types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const BUILTIN: RawProvider = {
  id: 'builtin',
  kind: 'builtin',
  label: 'Built-in',
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

function makeConfig(
  activeProvider: string,
  providers: RawProvider[],
): RawAppConfig {
  return {
    inference: {
      active_provider: activeProvider,
      keep_warm_inactivity_minutes: 0,
      num_ctx: 16384,
      providers,
    },
    prompt: { system: '' },
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

const QWEN_ROW = {
  id: 'org/Qwen3.5-9B-GGUF:Qwen3.5-9B-Q4_K_M.gguf',
  display_name: 'Qwen3.5 9B',
  size_bytes: 6_600_000_000,
  quant: 'Q4_K_M',
};

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
        return { state: 'stopped', model_path: '', port: null, error: null };
      default:
        return undefined;
    }
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  clearEventHandlers();
  mockInvoke();
});

afterEach(() => {
  clearEventHandlers();
});

describe('RunningModelFooter', () => {
  it('shows the built-in model name, size, and a live dot when the engine is loaded', async () => {
    const builtin = { ...BUILTIN, model: QWEN_ROW.id };
    mockInvoke({
      list_installed_models: [QWEN_ROW],
      get_engine_status: {
        state: 'loaded',
        model_path: '/x',
        port: 1,
        error: null,
      },
    });

    render(
      <RunningModelFooter config={makeConfig('builtin', [builtin, OLLAMA])} />,
    );

    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    await waitFor(() => expect(footer).toHaveTextContent('Qwen3.5 9B'));
    expect(footer).toHaveTextContent('Built-in · 6.6 GB');
    expect(footer.querySelector('[class*="runningModelDot"]')).not.toBeNull();
    // Live dot, not the idle variant.
    expect(footer.querySelector('[class*="DotIdle"]')).toBeNull();
  });

  it('shows a placeholder when the active built-in model is not installed', async () => {
    const builtin = { ...BUILTIN, model: 'org/missing:m.gguf' };
    mockInvoke({ list_installed_models: [QWEN_ROW] });

    render(
      <RunningModelFooter config={makeConfig('builtin', [builtin, OLLAMA])} />,
    );

    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    await waitFor(() => expect(footer).toHaveTextContent(/No model/i));
  });

  it('shows the Ollama model name and label with an idle dot', async () => {
    const ollama = { ...OLLAMA, model: 'llama3.1:8b' };
    render(
      <RunningModelFooter config={makeConfig('ollama', [BUILTIN, ollama])} />,
    );

    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    expect(footer).toHaveTextContent('llama3.1:8b');
    expect(footer).toHaveTextContent('Ollama');
    expect(footer.querySelector('[class*="DotIdle"]')).not.toBeNull();
  });

  it('shows a placeholder when the active Ollama provider has no model', async () => {
    render(
      <RunningModelFooter config={makeConfig('ollama', [BUILTIN, OLLAMA])} />,
    );
    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    expect(footer).toHaveTextContent(/No model/i);
  });

  it('shows the OpenAI provider model and label', async () => {
    const openai = { ...OPENAI, model: 'qwen2.5-coder' };
    render(
      <RunningModelFooter config={makeConfig('openai', [BUILTIN, openai])} />,
    );
    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    expect(footer).toHaveTextContent('qwen2.5-coder');
    expect(footer).toHaveTextContent('LM Studio');
  });

  it('falls back to a placeholder when the active provider id matches nothing', async () => {
    render(
      <RunningModelFooter config={makeConfig('ghost', [BUILTIN, OLLAMA])} />,
    );
    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    expect(footer).toHaveTextContent(/No model/i);
  });

  it('tolerates a config with no built-in provider', async () => {
    const ollama = { ...OLLAMA, model: 'llama3.1:8b' };
    render(<RunningModelFooter config={makeConfig('ollama', [ollama])} />);
    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    expect(footer).toHaveTextContent('llama3.1:8b');
  });

  it('treats a non-array installed payload as empty', async () => {
    const builtin = { ...BUILTIN, model: QWEN_ROW.id };
    mockInvoke({ list_installed_models: null });
    render(
      <RunningModelFooter config={makeConfig('builtin', [builtin, OLLAMA])} />,
    );
    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    await waitFor(() => expect(footer).toHaveTextContent(/No model/i));
  });

  it('survives a failed installed-models read', async () => {
    const builtin = { ...BUILTIN, model: QWEN_ROW.id };
    mockInvoke({ list_installed_models: new Error('io') });
    render(
      <RunningModelFooter config={makeConfig('builtin', [builtin, OLLAMA])} />,
    );
    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    await waitFor(() => expect(footer).toHaveTextContent(/No model/i));
  });

  it('survives a failed engine-status read', async () => {
    const builtin = { ...BUILTIN, model: QWEN_ROW.id };
    mockInvoke({
      list_installed_models: [QWEN_ROW],
      get_engine_status: new Error('engine down'),
    });
    render(
      <RunningModelFooter config={makeConfig('builtin', [builtin, OLLAMA])} />,
    );
    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    await waitFor(() => expect(footer).toHaveTextContent('Qwen3.5 9B'));
    // Engine status unknown -> idle dot.
    expect(footer.querySelector('[class*="DotIdle"]')).not.toBeNull();
  });

  it('reflects a live engine via the engine:status event stream', async () => {
    const builtin = { ...BUILTIN, model: QWEN_ROW.id };
    mockInvoke({ list_installed_models: [QWEN_ROW] });
    render(
      <RunningModelFooter config={makeConfig('builtin', [builtin, OLLAMA])} />,
    );
    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    await waitFor(() => expect(footer).toHaveTextContent('Qwen3.5 9B'));
    expect(footer.querySelector('[class*="DotIdle"]')).not.toBeNull();

    await act(async () => {
      emitTauriEvent('engine:status', {
        state: 'loaded',
        model_path: '/x',
        port: 1,
        error: null,
      });
    });
    expect(footer.querySelector('[class*="DotIdle"]')).toBeNull();
  });

  it('omits the meta line when the active provider has a model but no label', async () => {
    const ollama = { ...OLLAMA, model: 'llama3.1:8b', label: '' };
    render(
      <RunningModelFooter config={makeConfig('ollama', [BUILTIN, ollama])} />,
    );
    const footer = await screen.findByRole('status', {
      name: /running model/i,
    });
    expect(footer).toHaveTextContent('llama3.1:8b');
    expect(footer.querySelector('[class*="runningModelMeta"]')).toBeNull();
  });
});
