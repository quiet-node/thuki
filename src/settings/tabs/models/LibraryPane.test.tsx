/**
 * Unit tests for the Models surface's Library pane.
 *
 * Covers the installed-model list (active + non-active cards, capability
 * badges), the Use action, the Delete confirm/cancel/success/error flow,
 * the empty state, the free-disk footer, and the defensive guards around
 * the manifest and disk probes.
 *
 * `invoke` comes from the global Tauri mock; capabilities are fetched
 * through the same `get_model_capabilities` command the hook reads.
 */

import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import { LibraryPane } from './LibraryPane';
import type { RawAppConfig, RawProvider } from '../../types';
import type { InstalledModel } from '../../../types/starter';
import type { ModelCapabilitiesMap } from '../../../types/model';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const BASE_CONFIG: RawAppConfig = {
  inference: {
    active_provider: 'builtin',
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

/** BASE_CONFIG with the builtin provider's model set to `id`. */
function makeConfig(builtinModel: string): RawAppConfig {
  const builtin: RawProvider = {
    ...BASE_CONFIG.inference.providers[0],
    model: builtinModel,
  };
  return {
    ...BASE_CONFIG,
    inference: {
      ...BASE_CONFIG.inference,
      providers: [builtin, BASE_CONFIG.inference.providers[1]],
    },
  };
}

const GEMMA: InstalledModel = {
  id: 'org/gemma:gemma.gguf',
  display_name: 'gemma',
  size_bytes: 2_489_757_856,
  quant: 'Q4_K_M',
};

const QWEN: InstalledModel = {
  id: 'org/qwen:qwen.gguf',
  display_name: 'qwen',
  size_bytes: 9_000_000_000,
  quant: '',
};

const INSTALLED: InstalledModel[] = [GEMMA, QWEN];

const CAPS: ModelCapabilitiesMap = {
  'org/gemma:gemma.gguf': { vision: true, thinking: false },
  'org/qwen:qwen.gguf': { vision: false, thinking: true },
};

/** Marks a command response as a rejection in `mockCommands`. */
class Reject {
  constructor(public readonly value: unknown) {}
}

/**
 * Routes `invoke` by command name. A `Reject` throws its payload, a function
 * is called with the invoke args (for stateful sequences), anything else
 * resolves as-is. Unmapped commands resolve to `undefined`.
 */
function mockCommands(responses: Record<string, unknown>) {
  invokeMock.mockImplementation(
    async (cmd: string, args?: Record<string, unknown>) => {
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

/** Default backend: two installed models, capability map, known free disk. */
function libraryResponses(overrides: Record<string, unknown> = {}) {
  return {
    list_installed_models: INSTALLED,
    get_model_capabilities: CAPS,
    get_models_dir_free_bytes: 30_400_000_000,
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
});

async function renderPane(
  config: RawAppConfig = makeConfig(''),
  onSaved: (next: RawAppConfig) => void = () => {},
  onAddModel: () => void = () => {},
) {
  const view = render(
    <LibraryPane config={config} onSaved={onSaved} onAddModel={onAddModel} />,
  );
  await flush();
  return view;
}

describe('LibraryPane', () => {
  it('lists each installed model with its org line, size, and quant', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    expect(screen.getByText('gemma')).toBeInTheDocument();
    expect(screen.getByText('org/gemma · Q4_K_M · 2.5 GB')).toBeInTheDocument();
    // Empty quant drops out of the org line.
    expect(screen.getByText('org/qwen · 9.0 GB')).toBeInTheDocument();
  });

  it('renders the uppercased first character as each avatar', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    expect(screen.getByText('G')).toBeInTheDocument();
    expect(screen.getByText('Q')).toBeInTheDocument();
  });

  it('marks the active model with an Active badge and no Use button', async () => {
    mockCommands(libraryResponses());
    await renderPane(makeConfig('org/gemma:gemma.gguf'));
    expect(screen.getByText('Active')).toBeInTheDocument();
    // The active model offers no Use button; the non-active one does.
    expect(
      screen.getByRole('button', { name: 'Use qwen' }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Use gemma' }),
    ).not.toBeInTheDocument();
  });

  it('shows a Vision badge only for vision-capable models', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    const vision = screen.getByText('Vision');
    expect(vision).toBeInTheDocument();
    // Only gemma is vision-capable, so exactly one Vision badge.
    expect(screen.getAllByText('Vision')).toHaveLength(1);
  });

  it('shows a Reasoning badge only for thinking-capable models', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    expect(screen.getByText('Reasoning')).toBeInTheDocument();
    expect(screen.getAllByText('Reasoning')).toHaveLength(1);
  });

  it('omits capability badges when no map entry exists for a model', async () => {
    mockCommands(libraryResponses({ get_model_capabilities: {} }));
    await renderPane();
    expect(screen.queryByText('Vision')).not.toBeInTheDocument();
    expect(screen.queryByText('Reasoning')).not.toBeInTheDocument();
  });

  it('Use commits the model, lifts the config, and refreshes', async () => {
    mockCommands(libraryResponses({ update_provider_field: undefined }));
    const onSaved = vi.fn();
    await renderPane(makeConfig('org/gemma:gemma.gguf'), onSaved);
    fireEvent.click(screen.getByRole('button', { name: 'Use qwen' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'builtin',
      field: 'model',
      value: 'org/qwen:qwen.gguf',
    });
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
  });

  it('leaves the lift to the focus resync when Use cannot read the config', async () => {
    mockCommands(
      libraryResponses({
        update_provider_field: undefined,
        get_config: new Reject(new Error('read failed')),
      }),
    );
    const onSaved = vi.fn();
    await renderPane(makeConfig('org/gemma:gemma.gguf'), onSaved);
    fireEvent.click(screen.getByRole('button', { name: 'Use qwen' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'builtin',
      field: 'model',
      value: 'org/qwen:qwen.gguf',
    });
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('swallows an update_provider_field failure on Use', async () => {
    mockCommands(
      libraryResponses({
        update_provider_field: new Reject(new Error('write failed')),
      }),
    );
    const onSaved = vi.fn();
    await renderPane(makeConfig('org/gemma:gemma.gguf'), onSaved);
    fireEvent.click(screen.getByRole('button', { name: 'Use qwen' }));
    await flush();
    expect(onSaved).not.toHaveBeenCalled();
    expect(screen.getByText('qwen')).toBeInTheDocument();
  });

  it('Delete asks for confirmation and Cancel backs out without deleting', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    fireEvent.click(screen.getByRole('button', { name: 'Manage gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete gemma' }));
    expect(
      screen.getByText('Delete gemma? Its files are removed from disk.'),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(
      screen.queryByText('Delete gemma? Its files are removed from disk.'),
    ).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'delete_installed_model',
      expect.anything(),
    );
  });

  it('confirmed Delete invokes delete_installed_model, refreshes, and lifts the config', async () => {
    let deleted = false;
    mockCommands(
      libraryResponses({
        list_installed_models: () => (deleted ? [QWEN] : INSTALLED),
        delete_installed_model: () => {
          deleted = true;
          return undefined;
        },
      }),
    );
    const onSaved = vi.fn();
    await renderPane(makeConfig(''), onSaved);
    fireEvent.click(screen.getByRole('button', { name: 'Manage gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm delete' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('delete_installed_model', {
      id: 'org/gemma:gemma.gguf',
    });
    expect(screen.queryByText('gemma')).not.toBeInTheDocument();
    expect(screen.getByText('qwen')).toBeInTheDocument();
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
  });

  it('leaves the lift to the focus resync when get_config fails post-delete', async () => {
    mockCommands(
      libraryResponses({
        delete_installed_model: undefined,
        get_config: new Reject(new Error('read failed')),
      }),
    );
    const onSaved = vi.fn();
    await renderPane(makeConfig(''), onSaved);
    fireEvent.click(screen.getByRole('button', { name: 'Manage qwen' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete qwen' }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm delete' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('delete_installed_model', {
      id: 'org/qwen:qwen.gguf',
    });
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('surfaces a delete failure as an alert and keeps the row', async () => {
    mockCommands(
      libraryResponses({ delete_installed_model: new Reject('file busy') }),
    );
    await renderPane();
    fireEvent.click(screen.getByRole('button', { name: 'Manage gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm delete' }));
    await flush();
    expect(screen.getByRole('alert')).toHaveTextContent('file busy');
    expect(screen.getByText('gemma')).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith('get_config');
  });

  it('renders the empty state and routes both add affordances to onAddModel', async () => {
    mockCommands(libraryResponses({ list_installed_models: [] }));
    const onAddModel = vi.fn();
    await renderPane(makeConfig(''), () => {}, onAddModel);
    expect(screen.getByText('No models downloaded yet.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Browse Discover' }));
    expect(onAddModel).toHaveBeenCalledTimes(1);
    // The top-right Add model button is present in the empty state too.
    fireEvent.click(screen.getByRole('button', { name: 'Add model' }));
    expect(onAddModel).toHaveBeenCalledTimes(2);
  });

  it('treats a non-array manifest payload as empty', async () => {
    mockCommands(libraryResponses({ list_installed_models: null }));
    await renderPane();
    expect(screen.getByText('No models downloaded yet.')).toBeInTheDocument();
  });

  it('falls back to the empty state when the manifest probe rejects', async () => {
    mockCommands(
      libraryResponses({
        list_installed_models: new Reject(new Error('manifest unreadable')),
      }),
    );
    await renderPane();
    expect(screen.getByText('No models downloaded yet.')).toBeInTheDocument();
  });

  it('shows the free-disk footer and the model count when both are known', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    expect(screen.getByText('30.4 GB free on disk')).toBeInTheDocument();
    expect(
      screen.getByText('2 models · capabilities detected automatically'),
    ).toBeInTheDocument();
  });

  it('hides the free-disk line when the probe returns a non-number', async () => {
    mockCommands(libraryResponses({ get_models_dir_free_bytes: null }));
    await renderPane();
    expect(screen.queryByText(/free on disk/)).not.toBeInTheDocument();
    expect(
      screen.getByText('2 models · capabilities detected automatically'),
    ).toBeInTheDocument();
  });

  it('hides the free-disk line when the disk probe rejects', async () => {
    mockCommands(
      libraryResponses({
        get_models_dir_free_bytes: new Reject(new Error('statfs failed')),
      }),
    );
    await renderPane();
    expect(screen.queryByText(/free on disk/)).not.toBeInTheDocument();
  });

  it('renders the top-right Add model button and routes it to onAddModel', async () => {
    mockCommands(libraryResponses());
    const onAddModel = vi.fn();
    await renderPane(makeConfig(''), () => {}, onAddModel);
    fireEvent.click(screen.getByRole('button', { name: 'Add model' }));
    expect(onAddModel).toHaveBeenCalledTimes(1);
  });

  it('treats every model as non-active when no builtin provider exists', async () => {
    mockCommands(libraryResponses());
    // A config whose only provider is Ollama: the builtin lookup misses and
    // the active model falls back to "", so no card is Active and both get Use.
    const ollamaOnly: RawAppConfig = {
      ...BASE_CONFIG,
      inference: {
        ...BASE_CONFIG.inference,
        providers: [BASE_CONFIG.inference.providers[1]],
      },
    };
    await renderPane(ollamaOnly);
    expect(screen.queryByText('Active')).not.toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Use gemma' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Use qwen' }),
    ).toBeInTheDocument();
  });

  it('toggles the Manage menu closed when its own button is clicked again', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    const manage = screen.getByRole('button', { name: 'Manage gemma' });
    fireEvent.click(manage);
    expect(
      screen.getByRole('button', { name: 'Delete gemma' }),
    ).toBeInTheDocument();
    // A second click on the same Manage button collapses the row.
    fireEvent.click(manage);
    expect(
      screen.queryByRole('button', { name: 'Delete gemma' }),
    ).not.toBeInTheDocument();
  });

  it('clears a stale delete error once a later delete succeeds', async () => {
    mockCommands(
      libraryResponses({ delete_installed_model: new Reject('file busy') }),
    );
    await renderPane();
    fireEvent.click(screen.getByRole('button', { name: 'Manage gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm delete' }));
    await flush();
    expect(screen.getByRole('alert')).toHaveTextContent('file busy');

    mockCommands(
      libraryResponses({
        list_installed_models: [QWEN],
        delete_installed_model: undefined,
      }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Manage gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete gemma' }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm delete' }));
    await waitFor(() =>
      expect(screen.queryByRole('alert')).not.toBeInTheDocument(),
    );
  });
});
