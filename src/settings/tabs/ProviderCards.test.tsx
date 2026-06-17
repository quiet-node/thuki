/**
 * Unit tests for the Providers panel card bodies.
 *
 * - `BuiltinProviderCard`: installed-model picker, the shared download kit
 *   (starter picker, confirm card, paste-a-repo lookup), and the post-download
 *   config lift.
 * - `OpenAiProviderCard`: editable label/base URL/model, write-only API key,
 *   vision toggle, and removal with confirm.
 * - `AddOpenAiProvider`: the inline add-a-server affordance.
 *
 * `invoke` and `Channel` come from the global Tauri mocks; download events
 * are driven by simulating messages on the captured channel.
 */

import { useState } from 'react';
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import {
  AddOpenAiProvider,
  BuiltinProviderCard,
  OpenAiProviderCard,
} from './ProviderCards';
import type { RawAppConfig, RawProvider } from '../types';
import type { InstalledModel, StarterOption } from '../../types/starter';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const BASE_CONFIG: RawAppConfig = {
  inference: {
    active_provider: 'builtin',
    keep_warm_inactivity_minutes: 0,
    idle_unload_minutes: 0,
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
  behavior: { auto_replace: false, auto_close: false },
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
  debug: { trace_enabled: false },
};

/** Distinct snapshot so onSaved assertions cannot pass by referential luck. */
const NEW_CONFIG: RawAppConfig = {
  ...BASE_CONFIG,
  prompt: { system: 'updated' },
};

function makeConfig(builtinModel: string): RawAppConfig {
  return {
    ...BASE_CONFIG,
    inference: {
      ...BASE_CONFIG.inference,
      providers: [
        { ...BASE_CONFIG.inference.providers[0], model: builtinModel },
        BASE_CONFIG.inference.providers[1],
      ],
    },
  };
}

const INSTALLED: InstalledModel[] = [
  {
    id: 'org/gemma:gemma.gguf',
    display_name: 'gemma',
    size_bytes: 2_489_757_856,
    quant: 'Q4_K_M',
  },
  {
    id: 'org/qwen:qwen.gguf',
    display_name: 'qwen',
    size_bytes: 9_000_000_000,
    quant: '',
  },
];

const STARTER_OPTION: StarterOption = {
  starter: {
    tier: 'balanced',
    display_name: 'Gemma 4',
    repo: 'org/gemma',
    revision: 'abc123',
    file_name: 'gemma.gguf',
    sha256: 'sha-balanced',
    size_bytes: 5_000_000_000,
    quant: 'Q4_K_M',
    vision: false,
    thinking: false,
    mmproj_file: null,
    mmproj_sha256: null,
    mmproj_bytes: 0,
    est_runtime_gb: 6,
    license_note: '',
    origin: 'Google',
    origin_repo: 'google/gemma-4-12B-it',
  },
  fit: 'fits',
  installed: false,
  partial_bytes: null,
};

const OPENAI_PROVIDER: RawProvider = {
  id: 'openai',
  kind: 'openai',
  label: 'LM Studio',
  base_url: 'http://127.0.0.1:1234',
  model: '',
  vision: false,
};

/** BASE_CONFIG with the given OpenAI-compatible provider row appended. */
function configWith(provider: RawProvider): RawAppConfig {
  return {
    ...BASE_CONFIG,
    inference: {
      ...BASE_CONFIG.inference,
      providers: [...BASE_CONFIG.inference.providers, provider],
    },
  };
}

/**
 * Wraps the card the way ModelTab does: `onSaved` lifts the returned config
 * and the card re-renders with the updated provider row.
 */
function StatefulOpenAiCard() {
  const [provider, setProvider] = useState<RawProvider>(OPENAI_PROVIDER);
  return (
    <OpenAiProviderCard
      provider={provider}
      resyncToken={0}
      onSaved={(cfg) => {
        const next = cfg.inference.providers.find((p) => p.id === 'openai');
        if (next) setProvider(next);
      }}
    />
  );
}

/**
 * Wraps the builtin card the way ModelTab does: `onSaved` lifts the returned
 * config snapshot so a backend-side model clear reaches the dropdown.
 */
function StatefulBuiltinCard({ initialModel }: { initialModel: string }) {
  const [config, setConfig] = useState<RawAppConfig>(() =>
    makeConfig(initialModel),
  );
  return <BuiltinProviderCard config={config} onSaved={setConfig} />;
}

type MockChannel = { simulateMessage: (msg: unknown) => void };

/** Marks a command response as a rejection in `mockCommands`. */
class Reject {
  constructor(public readonly value: unknown) {}
}

let lastChannel: MockChannel | null = null;

/**
 * Routes `invoke` by command name. Values: `Reject` throws its payload,
 * functions are called with the invoke args (for stateful sequences), and
 * anything else resolves as-is. Channels passed via `onEvent` are captured.
 */
function mockCommands(responses: Record<string, unknown>) {
  invokeMock.mockImplementation(
    async (cmd: string, args?: Record<string, unknown>) => {
      if (args && 'onEvent' in args) {
        lastChannel = args.onEvent as unknown as MockChannel;
      }
      if (Object.prototype.hasOwnProperty.call(responses, cmd)) {
        const v = responses[cmd];
        if (v instanceof Reject) throw v.value;
        if (typeof v === 'function') {
          return (v as (a?: Record<string, unknown>) => unknown)(args);
        }
        return v;
      }
      return undefined;
    },
  );
}

/** Default backend for the builtin card: two installed models, one starter. */
function builtinResponses(overrides: Record<string, unknown> = {}) {
  return {
    list_installed_models: INSTALLED,
    get_starter_options: [STARTER_OPTION],
    get_models_dir_free_bytes: 50_000_000_000,
    get_config: NEW_CONFIG,
    ...overrides,
  };
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  lastChannel = null;
});

// ─── BuiltinProviderCard ─────────────────────────────────────────────────────

describe('BuiltinProviderCard', () => {
  async function renderCard(
    builtinModel = '',
    onSaved: (next: RawAppConfig) => void = () => {},
  ) {
    const view = render(
      <BuiltinProviderCard
        config={makeConfig(builtinModel)}
        onSaved={onSaved}
      />,
    );
    await flush();
    return view;
  }

  it('renders installed models with a Choose placeholder when none is selected', async () => {
    mockCommands(builtinResponses());
    await renderCard('');
    const select = screen.getByRole('combobox', {
      name: 'Built-in model',
    }) as HTMLSelectElement;
    expect(select.value).toBe('');
    expect(screen.getByText('Choose a model')).toBeInTheDocument();
    expect(screen.getByText('gemma · Q4_K_M')).toBeInTheDocument();
    expect(screen.getByText('qwen')).toBeInTheDocument();
  });

  it('selects the persisted builtin model and omits the placeholder', async () => {
    mockCommands(builtinResponses());
    await renderCard('org/gemma:gemma.gguf');
    const select = screen.getByRole('combobox', {
      name: 'Built-in model',
    }) as HTMLSelectElement;
    expect(select.value).toBe('org/gemma:gemma.gguf');
    expect(screen.queryByText('Choose a model')).not.toBeInTheDocument();
  });

  it('committing a model invokes update_provider_field and lifts the config', async () => {
    mockCommands(builtinResponses({ update_provider_field: NEW_CONFIG }));
    const onSaved = vi.fn();
    await renderCard('', onSaved);
    fireEvent.change(screen.getByRole('combobox', { name: 'Built-in model' }), {
      target: { value: 'org/qwen:qwen.gguf' },
    });
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'builtin',
      field: 'model',
      value: 'org/qwen:qwen.gguf',
    });
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
  });

  it('swallows an update_provider_field failure on model commit', async () => {
    mockCommands(
      builtinResponses({
        update_provider_field: new Reject(new Error('write failed')),
      }),
    );
    const onSaved = vi.fn();
    await renderCard('', onSaved);
    fireEvent.change(screen.getByRole('combobox', { name: 'Built-in model' }), {
      target: { value: 'org/qwen:qwen.gguf' },
    });
    await flush();
    expect(onSaved).not.toHaveBeenCalled();
    expect(
      screen.getByRole('combobox', { name: 'Built-in model' }),
    ).toBeInTheDocument();
  });

  it('shows the no-models hint when the manifest is empty', async () => {
    mockCommands(builtinResponses({ list_installed_models: [] }));
    await renderCard();
    expect(screen.getByText('No models downloaded yet')).toBeInTheDocument();
  });

  it('treats a non-array list_installed_models payload as empty', async () => {
    mockCommands(builtinResponses({ list_installed_models: null }));
    await renderCard();
    expect(screen.getByText('No models downloaded yet')).toBeInTheDocument();
  });

  it('falls back to empty state when the manifest and disk probes reject', async () => {
    mockCommands(
      builtinResponses({
        list_installed_models: new Reject(new Error('manifest unreadable')),
        get_models_dir_free_bytes: new Reject(new Error('statfs failed')),
      }),
    );
    await renderCard();
    expect(screen.getByText('No models downloaded yet')).toBeInTheDocument();
  });

  it('keeps the download kit hidden until starter options resolve', async () => {
    mockCommands(
      builtinResponses({ get_starter_options: new Promise(() => {}) }),
    );
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    expect(
      screen.queryByRole('button', { name: 'Look up' }),
    ).not.toBeInTheDocument();
  });

  it('toggles the download kit open and closed', async () => {
    mockCommands(builtinResponses());
    await renderCard();
    const trigger = screen.getByRole('button', { name: 'Download a model' });
    fireEvent.click(trigger);
    expect(screen.getByText('Gemma 4')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Look up' })).toBeInTheDocument();
    fireEvent.click(trigger);
    expect(screen.queryByText('Gemma 4')).not.toBeInTheDocument();
  });

  it('walks the confirm flow and lifts the config when the download finishes', async () => {
    mockCommands(builtinResponses());
    const onSaved = vi.fn();
    await renderCard('', onSaved);
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    // Row-level Download opens the confirm card.
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    expect(screen.getByText('5.0 GB download.')).toBeInTheDocument();
    expect(screen.getByText('50.0 GB free on this disk.')).toBeInTheDocument();
    // Two Download buttons now: the picker row's and the confirm card's.
    const confirmBtn = screen.getAllByRole('button', { name: 'Download' })[1];
    fireEvent.click(confirmBtn);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith(
      'download_starter',
      expect.objectContaining({ tier: 'balanced' }),
    );
    act(() => {
      lastChannel?.simulateMessage({ type: 'AllDone' });
    });
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG));
  });

  it('returns to the picker once the Ready card dwell elapses', async () => {
    vi.useFakeTimers();
    try {
      mockCommands(builtinResponses());
      await renderCard();
      fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
      fireEvent.click(screen.getByRole('button', { name: 'Download' }));
      fireEvent.click(screen.getAllByRole('button', { name: 'Download' })[1]);
      await flush();
      act(() => {
        lastChannel?.simulateMessage({ type: 'AllDone' });
      });
      await flush();
      // Success card up, starter rows hidden.
      expect(screen.getByText('Ready')).toBeInTheDocument();
      expect(
        screen.queryByRole('button', { name: 'Download' }),
      ).not.toBeInTheDocument();

      await act(async () => {
        vi.advanceTimersByTime(2500);
      });
      expect(screen.queryByText('Ready')).not.toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: 'Download' }),
      ).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it('Choose a different model on the failed card returns to the picker', async () => {
    mockCommands(builtinResponses());
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    fireEvent.click(screen.getAllByRole('button', { name: 'Download' })[1]);
    await flush();
    act(() => {
      lastChannel?.simulateMessage({
        type: 'Failed',
        data: { kind: 'disk_full', message: 'no space left' },
      });
    });
    expect(
      screen.getByText('Not enough disk space. Free up space and retry.'),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Download' }),
    ).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole('button', { name: 'Choose a different model' }),
    );
    expect(
      screen.getByRole('button', { name: 'Download' }),
    ).toBeInTheDocument();
  });

  it('leaves the lift to the focus resync when get_config fails post-download', async () => {
    mockCommands(
      builtinResponses({ get_config: new Reject(new Error('read failed')) }),
    );
    const onSaved = vi.fn();
    await renderCard('', onSaved);
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    fireEvent.click(screen.getAllByRole('button', { name: 'Download' })[1]);
    await flush();
    act(() => {
      lastChannel?.simulateMessage({ type: 'AllDone' });
    });
    await flush();
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('hides the free-disk line when the free-bytes probe returns a non-number', async () => {
    mockCommands(builtinResponses({ get_models_dir_free_bytes: null }));
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    expect(screen.getByText('5.0 GB download.')).toBeInTheDocument();
    expect(screen.queryByText(/free on this disk/)).not.toBeInTheDocument();
    // Cancel returns to the plain picker.
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByText('5.0 GB download.')).not.toBeInTheDocument();
  });

  it('cancels an in-flight download and retries after a failure', async () => {
    mockCommands(builtinResponses());
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    fireEvent.click(screen.getAllByRole('button', { name: 'Download' })[1]);
    await flush();
    expect(screen.getByText('Downloading model')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('cancel_model_download');
    act(() => {
      lastChannel?.simulateMessage({
        type: 'Failed',
        data: { kind: 'other', message: 'socket closed' },
      });
    });
    expect(screen.getByText('socket closed')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await flush();
    const starts = invokeMock.mock.calls.filter(
      (c: unknown[]) => c[0] === 'download_starter',
    );
    expect(starts).toHaveLength(2);
  });

  it('enters resume_pending for an interrupted partial and resumes from it', async () => {
    mockCommands(
      builtinResponses({
        get_starter_options: [
          { ...STARTER_OPTION, partial_bytes: 1_000_000_000 },
        ],
      }),
    );
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: /Resume download/ }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith(
      'download_starter',
      expect.objectContaining({ tier: 'balanced' }),
    );
  });

  it('discards an interrupted partial and refreshes the starter options', async () => {
    mockCommands(
      builtinResponses({
        get_starter_options: [
          { ...STARTER_OPTION, partial_bytes: 1_000_000_000 },
        ],
      }),
    );
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: 'Discard' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'sha-balanced',
    });
  });

  it('looks up a pasted repo and downloads the chosen GGUF file', async () => {
    mockCommands(
      builtinResponses({
        list_hf_repo_ggufs: [
          { file: 'a.gguf', size_bytes: 2_000_000_000 },
          { file: 'b.gguf', size_bytes: 3_000_000_000 },
        ],
      }),
    );
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    const lookupBtn = screen.getByRole('button', { name: 'Look up' });
    expect(lookupBtn).toBeDisabled();
    fireEvent.change(screen.getByLabelText('Hugging Face repo id'), {
      target: { value: '  owner/repo  ' },
    });
    expect(lookupBtn).toBeEnabled();
    fireEvent.click(lookupBtn);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('list_hf_repo_ggufs', {
      repo: 'owner/repo',
    });
    const fileSelect = screen.getByRole('combobox', {
      name: 'GGUF file',
    }) as HTMLSelectElement;
    expect(fileSelect.value).toBe('a.gguf');
    expect(screen.getByText('a.gguf · 2.0 GB')).toBeInTheDocument();
    fireEvent.change(fileSelect, { target: { value: 'b.gguf' } });
    // The repo Download sits after the picker row's Download button.
    const downloads = screen.getAllByRole('button', { name: 'Download' });
    fireEvent.click(downloads[downloads.length - 1]);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith(
      'download_repo_model',
      expect.objectContaining({ repo: 'owner/repo', file: 'b.gguf' }),
    );
  });

  it('shows the empty-repo hint when the lookup finds no GGUF files', async () => {
    mockCommands(builtinResponses({ list_hf_repo_ggufs: [] }));
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    fireEvent.change(screen.getByLabelText('Hugging Face repo id'), {
      target: { value: 'owner/empty' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Look up' }));
    await flush();
    expect(
      screen.getByText('No GGUF files found in this repo.'),
    ).toBeInTheDocument();
  });

  it('treats a non-array lookup payload as an empty file list', async () => {
    mockCommands(builtinResponses({ list_hf_repo_ggufs: 'nope' }));
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    fireEvent.change(screen.getByLabelText('Hugging Face repo id'), {
      target: { value: 'owner/odd' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Look up' }));
    await flush();
    expect(
      screen.getByText('No GGUF files found in this repo.'),
    ).toBeInTheDocument();
  });

  it('surfaces a lookup failure as an inline error', async () => {
    mockCommands(
      builtinResponses({
        list_hf_repo_ggufs: new Reject('repo not found'),
      }),
    );
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Download a model' }));
    fireEvent.change(screen.getByLabelText('Hugging Face repo id'), {
      target: { value: 'owner/missing' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Look up' }));
    await flush();
    expect(screen.getByRole('alert')).toHaveTextContent('repo not found');
  });

  it('lists each installed model with size, quant, and a delete affordance', async () => {
    mockCommands(builtinResponses());
    await renderCard();
    expect(screen.getByText('gemma · 2.5 GB · Q4_K_M')).toBeInTheDocument();
    // Empty quant omits the trailing separator.
    expect(screen.getByText('qwen · 9.0 GB')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Delete gemma' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Delete qwen' }),
    ).toBeInTheDocument();
  });

  it('delete asks for confirmation and Cancel backs out without deleting', async () => {
    mockCommands(builtinResponses());
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Delete gemma' }));
    expect(
      screen.getByText('Delete gemma? Its files are removed from disk.'),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(
      screen.queryByText('Delete gemma? Its files are removed from disk.'),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Delete gemma' }),
    ).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'delete_installed_model',
      expect.anything(),
    );
  });

  it('confirmed delete invokes delete_installed_model and refreshes the rows', async () => {
    let deleted = false;
    mockCommands(
      builtinResponses({
        list_installed_models: () => (deleted ? [INSTALLED[1]] : INSTALLED),
        delete_installed_model: () => {
          deleted = true;
          return undefined;
        },
      }),
    );
    const onSaved = vi.fn();
    await renderCard('', onSaved);
    fireEvent.click(screen.getByRole('button', { name: 'Delete gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('delete_installed_model', {
      id: 'org/gemma:gemma.gguf',
    });
    expect(
      screen.queryByText('gemma · 2.5 GB · Q4_K_M'),
    ).not.toBeInTheDocument();
    expect(screen.getByText('qwen · 9.0 GB')).toBeInTheDocument();
    // The deletion also re-fetches the starter rows (an installed starter
    // flips back to downloadable) and lifts the fresh config snapshot.
    expect(invokeMock).toHaveBeenCalledWith('get_starter_options');
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
  });

  it('deleting the active model clears the selection and shows the picker affordance', async () => {
    let deleted = false;
    mockCommands(
      builtinResponses({
        list_installed_models: () => (deleted ? [INSTALLED[1]] : INSTALLED),
        delete_installed_model: () => {
          deleted = true;
          return undefined;
        },
        // The backend cleared the builtin provider's model field itself.
        get_config: () => makeConfig(''),
      }),
    );
    render(<StatefulBuiltinCard initialModel="org/gemma:gemma.gguf" />);
    await flush();
    const select = screen.getByRole('combobox', {
      name: 'Built-in model',
    }) as HTMLSelectElement;
    expect(select.value).toBe('org/gemma:gemma.gguf');
    fireEvent.click(screen.getByRole('button', { name: 'Delete gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await flush();
    expect(select.value).toBe('');
    expect(screen.getByText('Choose a model')).toBeInTheDocument();
  });

  it('surfaces a delete failure and keeps the row', async () => {
    mockCommands(
      builtinResponses({
        delete_installed_model: new Reject('file busy'),
      }),
    );
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Delete gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await flush();
    expect(screen.getByRole('alert')).toHaveTextContent('file busy');
    expect(screen.getByText('gemma · 2.5 GB · Q4_K_M')).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith('get_config');
    // A later successful delete clears the stale error.
    mockCommands(
      builtinResponses({
        list_installed_models: [INSTALLED[1]],
        delete_installed_model: undefined,
      }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Delete gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await flush();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('leaves the lift to the focus resync when get_config fails post-delete', async () => {
    mockCommands(
      builtinResponses({
        delete_installed_model: undefined,
        get_config: new Reject(new Error('read failed')),
      }),
    );
    const onSaved = vi.fn();
    await renderCard('', onSaved);
    fireEvent.click(screen.getByRole('button', { name: 'Delete qwen' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('delete_installed_model', {
      id: 'org/qwen:qwen.gguf',
    });
    expect(onSaved).not.toHaveBeenCalled();
  });
});

// ─── OpenAiProviderCard ──────────────────────────────────────────────────────

describe('OpenAiProviderCard', () => {
  async function renderCard(
    overrides: Partial<RawProvider> = {},
    onSaved: (next: RawAppConfig) => void = () => {},
    resyncToken = 0,
  ) {
    const view = render(
      <OpenAiProviderCard
        provider={{ ...OPENAI_PROVIDER, ...overrides }}
        resyncToken={resyncToken}
        onSaved={onSaved}
      />,
    );
    await flush();
    return view;
  }

  it('lists models from list_openai_models and commits a selection', async () => {
    mockCommands({
      list_openai_models: ['model-a', 'model-b'],
      has_provider_api_key: false,
      update_provider_field: NEW_CONFIG,
    });
    const onSaved = vi.fn();
    await renderCard({}, onSaved);
    const select = screen.getByRole('combobox', {
      name: 'OpenAI-compatible model',
    }) as HTMLSelectElement;
    expect(select.value).toBe('');
    expect(screen.getByText('Choose a model')).toBeInTheDocument();
    fireEvent.change(select, { target: { value: 'model-b' } });
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'openai',
      field: 'model',
      value: 'model-b',
    });
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
  });

  it('shows the loading hint while the model probe is in flight', async () => {
    mockCommands({
      list_openai_models: new Promise(() => {}),
      has_provider_api_key: false,
    });
    await renderCard();
    expect(screen.getByText('Loading models…')).toBeInTheDocument();
  });

  it('shows the error state with Retry when listing fails, then recovers', async () => {
    let calls = 0;
    mockCommands({
      list_openai_models: () => {
        calls += 1;
        if (calls === 1) throw new Error('connection refused');
        return ['model-x'];
      },
      has_provider_api_key: false,
    });
    await renderCard();
    expect(screen.getByText('Couldn’t list models')).toBeInTheDocument();
    expect(screen.getByRole('alert')).toHaveTextContent('connection refused');
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await flush();
    expect(
      screen.getByRole('combobox', { name: 'OpenAI-compatible model' }),
    ).toBeInTheDocument();
    expect(screen.getByText('model-x')).toBeInTheDocument();
  });

  it('shows the empty-inventory hint when the server lists no models', async () => {
    mockCommands({ list_openai_models: [], has_provider_api_key: false });
    await renderCard();
    expect(
      screen.getByText('No models reported by the server'),
    ).toBeInTheDocument();
  });

  it('treats a non-array model payload as empty', async () => {
    mockCommands({ list_openai_models: 'huh', has_provider_api_key: false });
    await renderCard();
    expect(
      screen.getByText('No models reported by the server'),
    ).toBeInTheDocument();
  });

  it('keeps the persisted model selectable when the server no longer lists it', async () => {
    mockCommands({
      list_openai_models: ['model-a'],
      has_provider_api_key: false,
    });
    await renderCard({ model: 'retired-model' });
    const select = screen.getByRole('combobox', {
      name: 'OpenAI-compatible model',
    }) as HTMLSelectElement;
    expect(select.value).toBe('retired-model');
    expect(screen.getByText('retired-model')).toBeInTheDocument();
    expect(screen.queryByText('Choose a model')).not.toBeInTheDocument();
  });

  it('surfaces a model-commit failure inline', async () => {
    mockCommands({
      list_openai_models: ['model-a'],
      has_provider_api_key: false,
      update_provider_field: new Reject({
        kind: 'type_mismatch',
        message: 'Model write failed.',
      }),
    });
    await renderCard();
    fireEvent.change(
      screen.getByRole('combobox', { name: 'OpenAI-compatible model' }),
      { target: { value: 'model-a' } },
    );
    await flush();
    expect(screen.getByText('Model write failed.')).toBeInTheDocument();
  });

  it('commits a changed label on blur and ignores non-Enter keys', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      update_provider_field: NEW_CONFIG,
    });
    const onSaved = vi.fn();
    await renderCard({}, onSaved);
    const label = screen.getByLabelText('Provider label');
    fireEvent.focus(label);
    fireEvent.change(label, { target: { value: '  My server  ' } });
    fireEvent.keyDown(label, { key: 'a' });
    expect(invokeMock).not.toHaveBeenCalledWith(
      'update_provider_field',
      expect.anything(),
    );
    fireEvent.blur(label);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'openai',
      field: 'label',
      value: 'My server',
    });
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
    // The returned config carries no openai row, so the input falls back to
    // the committed (trimmed) value.
    expect((label as HTMLInputElement).value).toBe('My server');
  });

  it('heals an empty label commit to the persisted default label', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      update_provider_field: configWith({
        ...OPENAI_PROVIDER,
        label: 'OpenAI-compatible',
      }),
    });
    render(<StatefulOpenAiCard />);
    await flush();
    const label = screen.getByLabelText('Provider label') as HTMLInputElement;
    fireEvent.focus(label);
    fireEvent.change(label, { target: { value: '   ' } });
    fireEvent.blur(label);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'openai',
      field: 'label',
      value: '',
    });
    expect(label.value).toBe('OpenAI-compatible');
  });

  it('leaves a refocused label input alone when the commit resolves', async () => {
    let resolveUpdate: (cfg: RawAppConfig) => void = () => {};
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      update_provider_field: () =>
        new Promise<RawAppConfig>((resolve) => {
          resolveUpdate = resolve;
        }),
    });
    await renderCard();
    const label = screen.getByLabelText('Provider label') as HTMLInputElement;
    fireEvent.focus(label);
    fireEvent.change(label, { target: { value: 'Renamed' } });
    fireEvent.blur(label);
    // The user starts typing again while the commit is still in flight.
    fireEvent.focus(label);
    fireEvent.change(label, { target: { value: 'Typing again' } });
    await act(async () => {
      resolveUpdate(configWith({ ...OPENAI_PROVIDER, label: 'Renamed' }));
      await Promise.resolve();
    });
    expect(label.value).toBe('Typing again');
  });

  it('Enter commits the label via blur; an unchanged label does not commit', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      update_provider_field: NEW_CONFIG,
    });
    await renderCard();
    const label = screen.getByLabelText('Provider label');
    fireEvent.focus(label);
    fireEvent.keyDown(label, { key: 'Enter' });
    fireEvent.blur(label);
    await flush();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'update_provider_field',
      expect.anything(),
    );
    fireEvent.focus(label);
    fireEvent.change(label, { target: { value: 'Renamed' } });
    fireEvent.keyDown(label, { key: 'Enter' });
    fireEvent.blur(label);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'openai',
      field: 'label',
      value: 'Renamed',
    });
  });

  it('reverts the label and shows the error when the commit fails', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      update_provider_field: new Reject({
        kind: 'type_mismatch',
        message: 'Label rejected.',
      }),
    });
    await renderCard();
    const label = screen.getByLabelText('Provider label') as HTMLInputElement;
    fireEvent.change(label, { target: { value: 'Bad' } });
    fireEvent.blur(label);
    await flush();
    expect(screen.getByText('Label rejected.')).toBeInTheDocument();
    expect(label.value).toBe('LM Studio');
  });

  it('commits a changed base URL on blur and warns about non-local URLs', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      update_provider_field: NEW_CONFIG,
    });
    const onSaved = vi.fn();
    await renderCard({}, onSaved);
    const url = screen.getByLabelText('OpenAI-compatible base URL');
    fireEvent.focus(url);
    fireEvent.change(url, { target: { value: 'http://example.com:1234' } });
    expect(screen.getByRole('alert')).toHaveTextContent(
      /responsible for securing it/,
    );
    fireEvent.keyDown(url, { key: 'a' });
    fireEvent.keyDown(url, { key: 'Enter' });
    fireEvent.blur(url);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'openai',
      field: 'base_url',
      value: 'http://example.com:1234',
    });
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
  });

  it('re-lists models after a successful base URL commit', async () => {
    let listCalls = 0;
    mockCommands({
      list_openai_models: () => {
        listCalls += 1;
        return listCalls === 1 ? ['old-model'] : ['new-model'];
      },
      has_provider_api_key: false,
      update_provider_field: configWith({
        ...OPENAI_PROVIDER,
        base_url: 'http://127.0.0.1:9999',
      }),
    });
    render(<StatefulOpenAiCard />);
    await flush();
    expect(screen.getByText('old-model')).toBeInTheDocument();
    const url = screen.getByLabelText('OpenAI-compatible base URL');
    fireEvent.focus(url);
    fireEvent.change(url, { target: { value: 'http://127.0.0.1:9999' } });
    fireEvent.blur(url);
    await waitFor(() => expect(listCalls).toBe(2));
    expect(screen.getByText('new-model')).toBeInTheDocument();
    expect(screen.queryByText('old-model')).not.toBeInTheDocument();
  });

  it('reverts the base URL when the commit fails; unchanged URL never commits', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      update_provider_field: new Reject({
        kind: 'type_mismatch',
        message: 'Base URL must start with http:// or https://.',
      }),
    });
    await renderCard();
    const url = screen.getByLabelText(
      'OpenAI-compatible base URL',
    ) as HTMLInputElement;
    fireEvent.focus(url);
    fireEvent.blur(url);
    await flush();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'update_provider_field',
      expect.anything(),
    );
    fireEvent.change(url, { target: { value: 'ftp://nope' } });
    fireEvent.blur(url);
    await flush();
    expect(
      screen.getByText('Base URL must start with http:// or https://.'),
    ).toBeInTheDocument();
    expect(url.value).toBe('http://127.0.0.1:1234');
    // A failed commit reverts the value and must not refetch the model list.
    const listCalls = invokeMock.mock.calls.filter(
      (c: unknown[]) => c[0] === 'list_openai_models',
    ).length;
    expect(listCalls).toBe(1);
  });

  it('resyncs label and base URL from the provider when not focused', async () => {
    mockCommands({ list_openai_models: [], has_provider_api_key: false });
    const { rerender } = await renderCard();
    rerender(
      <OpenAiProviderCard
        provider={{
          ...OPENAI_PROVIDER,
          label: 'Jan',
          base_url: 'http://127.0.0.1:1337',
        }}
        resyncToken={1}
        onSaved={() => {}}
      />,
    );
    expect(
      (screen.getByLabelText('Provider label') as HTMLInputElement).value,
    ).toBe('Jan');
    expect(
      (screen.getByLabelText('OpenAI-compatible base URL') as HTMLInputElement)
        .value,
    ).toBe('http://127.0.0.1:1337');
  });

  it('does not overwrite focused fields on resync', async () => {
    mockCommands({ list_openai_models: [], has_provider_api_key: false });
    const { rerender } = await renderCard();
    const label = screen.getByLabelText('Provider label') as HTMLInputElement;
    const url = screen.getByLabelText(
      'OpenAI-compatible base URL',
    ) as HTMLInputElement;
    fireEvent.focus(label);
    fireEvent.change(label, { target: { value: 'typing label' } });
    fireEvent.focus(url);
    fireEvent.change(url, { target: { value: 'http://typing' } });
    rerender(
      <OpenAiProviderCard
        provider={{ ...OPENAI_PROVIDER, label: 'Jan', base_url: 'http://x' }}
        resyncToken={1}
        onSaved={() => {}}
      />,
    );
    expect(label.value).toBe('typing label');
    expect(url.value).toBe('http://typing');
  });

  it('saves the API key write-only and refreshes the model list', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      set_provider_api_key: undefined,
    });
    await renderCard();
    const keyInput = screen.getByPlaceholderText('sk-…') as HTMLInputElement;
    const saveBtn = screen.getByRole('button', { name: 'Save key' });
    expect(saveBtn).toBeDisabled();
    fireEvent.change(keyInput, { target: { value: 'sk-test' } });
    expect(saveBtn).toBeEnabled();
    const listCallsBefore = invokeMock.mock.calls.filter(
      (c: unknown[]) => c[0] === 'list_openai_models',
    ).length;
    fireEvent.click(saveBtn);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('set_provider_api_key', {
      providerId: 'openai',
      key: 'sk-test',
    });
    expect(keyInput.value).toBe('');
    expect(screen.getByText('Key saved')).toBeInTheDocument();
    const listCallsAfter = invokeMock.mock.calls.filter(
      (c: unknown[]) => c[0] === 'list_openai_models',
    ).length;
    expect(listCallsAfter).toBe(listCallsBefore + 1);
  });

  it('surfaces a set_provider_api_key failure', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      set_provider_api_key: new Reject('keychain locked'),
    });
    await renderCard();
    fireEvent.change(screen.getByPlaceholderText('sk-…'), {
      target: { value: 'sk-test' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save key' }));
    await flush();
    expect(screen.getByRole('alert')).toHaveTextContent('keychain locked');
  });

  it('shows Key saved from has_provider_api_key and clears the key', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: true,
      clear_provider_api_key: undefined,
    });
    await renderCard();
    expect(screen.getByText('Key saved')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Clear key' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('clear_provider_api_key', {
      providerId: 'openai',
    });
    expect(screen.queryByText('Key saved')).not.toBeInTheDocument();
  });

  it('surfaces a clear_provider_api_key failure and keeps the chip', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: true,
      clear_provider_api_key: new Reject('keychain locked'),
    });
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Clear key' }));
    await flush();
    expect(screen.getByRole('alert')).toHaveTextContent('keychain locked');
    expect(screen.getByText('Key saved')).toBeInTheDocument();
  });

  it('hides the chip when the key probe fails', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: new Reject(new Error('keychain unavailable')),
    });
    await renderCard();
    expect(screen.queryByText('Key saved')).not.toBeInTheDocument();
  });

  it('writes the vision flag through update_provider_field', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      update_provider_field: NEW_CONFIG,
    });
    const onSaved = vi.fn();
    await renderCard({}, onSaved);
    const toggle = screen.getByRole('switch', {
      name: 'Model accepts image inputs',
    });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(toggle);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'openai',
      field: 'vision',
      value: 'true',
    });
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
  });

  it('turns the vision flag off and surfaces a write failure', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      update_provider_field: new Reject({
        kind: 'type_mismatch',
        message: 'Vision write failed.',
      }),
    });
    await renderCard({ vision: true });
    const toggle = screen.getByRole('switch', {
      name: 'Model accepts image inputs',
    });
    expect(toggle).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(toggle);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'openai',
      field: 'vision',
      value: 'false',
    });
    expect(screen.getByText('Vision write failed.')).toBeInTheDocument();
  });

  it('removes the provider after an explicit confirm', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      remove_openai_provider: NEW_CONFIG,
    });
    const onSaved = vi.fn();
    await renderCard({}, onSaved);
    fireEvent.click(screen.getByRole('button', { name: 'Remove provider' }));
    expect(
      screen.getByText(
        'Remove this provider? Its saved API key is deleted too.',
      ),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('remove_openai_provider');
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
  });

  it('cancel keeps the provider; a failed removal closes the confirm row', async () => {
    mockCommands({
      list_openai_models: [],
      has_provider_api_key: false,
      remove_openai_provider: new Reject(new Error('write failed')),
    });
    await renderCard();
    fireEvent.click(screen.getByRole('button', { name: 'Remove provider' }));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(
      screen.getByRole('button', { name: 'Remove provider' }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Remove provider' }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
    await flush();
    expect(
      screen.getByRole('button', { name: 'Remove provider' }),
    ).toBeInTheDocument();
  });
});

// ─── AddOpenAiProvider ───────────────────────────────────────────────────────

describe('AddOpenAiProvider', () => {
  it('expands from the add button and gates Add on a non-empty base URL', () => {
    mockCommands({});
    render(<AddOpenAiProvider onSaved={() => {}} />);
    fireEvent.click(
      screen.getByRole('button', { name: 'Add OpenAI-compatible server' }),
    );
    const addBtn = screen.getByRole('button', { name: 'Add' });
    expect(addBtn).toBeDisabled();
    fireEvent.change(screen.getByLabelText('OpenAI-compatible base URL'), {
      target: { value: '   ' },
    });
    expect(addBtn).toBeDisabled();
    fireEvent.change(screen.getByLabelText('OpenAI-compatible base URL'), {
      target: { value: 'http://example.com:1234' },
    });
    expect(addBtn).toBeEnabled();
    expect(screen.getByRole('alert')).toHaveTextContent(
      /responsible for securing it/,
    );
  });

  it('adds the provider and resets the form on success', async () => {
    mockCommands({ add_openai_provider: NEW_CONFIG });
    const onSaved = vi.fn();
    render(<AddOpenAiProvider onSaved={onSaved} />);
    fireEvent.click(
      screen.getByRole('button', { name: 'Add OpenAI-compatible server' }),
    );
    fireEvent.change(screen.getByLabelText('Provider label'), {
      target: { value: 'LM Studio' },
    });
    fireEvent.change(screen.getByLabelText('OpenAI-compatible base URL'), {
      target: { value: ' http://127.0.0.1:1234 ' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('add_openai_provider', {
      label: 'LM Studio',
      baseUrl: 'http://127.0.0.1:1234',
    });
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
    // Collapsed back to the affordance with cleared fields.
    fireEvent.click(
      screen.getByRole('button', { name: 'Add OpenAI-compatible server' }),
    );
    expect(
      (screen.getByLabelText('Provider label') as HTMLInputElement).value,
    ).toBe('');
    expect(
      (screen.getByLabelText('OpenAI-compatible base URL') as HTMLInputElement)
        .value,
    ).toBe('');
  });

  it('shows the backend error when adding fails and Cancel clears it', async () => {
    mockCommands({
      add_openai_provider: new Reject({
        kind: 'type_mismatch',
        message: 'An OpenAI-compatible provider already exists.',
      }),
    });
    render(<AddOpenAiProvider onSaved={() => {}} />);
    fireEvent.click(
      screen.getByRole('button', { name: 'Add OpenAI-compatible server' }),
    );
    fireEvent.change(screen.getByLabelText('OpenAI-compatible base URL'), {
      target: { value: 'http://127.0.0.1:1234' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await flush();
    expect(
      screen.getByText('An OpenAI-compatible provider already exists.'),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    fireEvent.click(
      screen.getByRole('button', { name: 'Add OpenAI-compatible server' }),
    );
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});
