/**
 * Unit tests for the Models surface's Library pane.
 *
 * Covers the installed-model list (active + non-active rows, capability text
 * tags, RAM-fit hint), the model name's Hugging Face link, the popover menu
 * (Set as active / Reveal in Finder / Delete), the delete confirm/cancel/
 * success/error flow, menu dismissal (outside click + Escape), the empty
 * state, the footer, and the defensive guards around the manifest and disk
 * probes.
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
        label: 'Built-in',
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
  behavior: { auto_replace: false, auto_close: false, auto_search: true },
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
  size_bytes: 2_489_757_856,
  // A vision projector healed from the registry: folded into the shown total.
  mmproj_bytes: 500_000_000,
  display_name: 'gemma',
  quant: 'Q4_K_M',
  fit: 'fits',
  context_length: 262_144,
  origin: 'Google',
};

// No `fit`, `mmproj_bytes`, `origin`, or `context_length` here: a pasted repo
// that exercises the "RAM unknown" / weights-only / maker-fallback branches.
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

/** Opens the popover menu for the named model. */
function openMenu(name: string) {
  fireEvent.click(screen.getByRole('button', { name: `Manage ${name}` }));
}

describe('LibraryPane', () => {
  it('lists each installed model with the size · context · maker · quant line', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    expect(screen.getByText('gemma')).toBeInTheDocument();
    // Curated model: size is the weights + mmproj total (2.5 + 0.5 = 3.0 GB),
    // then the registry-healed context and maker, then the quant.
    expect(
      screen.getByText('3.0 GB · 256K · Google · Q4_K_M'),
    ).toBeInTheDocument();
    // Pasted model: no mmproj/context/quant, and the maker falls back to the
    // repo id, so only the size and repo remain.
    expect(screen.getByText('9.0 GB · org/qwen')).toBeInTheDocument();
  });

  it('shows the RAM-fit hint only when the backend provides one', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    // gemma carries fit: 'fits'; qwen has no fit, so only one hint renders.
    expect(screen.getByText('Comfortable')).toBeInTheDocument();
    expect(screen.getAllByText('Comfortable')).toHaveLength(1);
  });

  it('marks the active model and offers Set as active only on the rest', async () => {
    mockCommands(libraryResponses());
    await renderPane(makeConfig('org/gemma:gemma.gguf'));
    // The non-active model's menu offers Set as active.
    openMenu('qwen');
    expect(
      screen.getByRole('menuitem', { name: 'Set as active' }),
    ).toBeInTheDocument();
    // The active model's menu does not.
    openMenu('gemma');
    expect(
      screen.queryByRole('menuitem', { name: 'Set as active' }),
    ).not.toBeInTheDocument();
  });

  it('shows a Text pill on every model', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    // Text is the baseline capability, so it shows on both rows.
    expect(screen.getAllByText('Text')).toHaveLength(2);
  });

  it('shows a Vision tag only for vision-capable models', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    expect(screen.getByText('Vision')).toBeInTheDocument();
    expect(screen.getAllByText('Vision')).toHaveLength(1);
  });

  it('shows a Reasoning tag only for thinking-capable models', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    expect(screen.getByText('Reasoning')).toBeInTheDocument();
    expect(screen.getAllByText('Reasoning')).toHaveLength(1);
  });

  it('marks the active model with an edge, not an Active pill', async () => {
    mockCommands(libraryResponses());
    await renderPane(makeConfig('org/gemma:gemma.gguf'));
    // The accent edge is the only active signal; the textual pill is gone.
    expect(screen.queryByText('Active')).not.toBeInTheDocument();
  });

  it('omits Vision and Reasoning tags when no map entry exists, keeping Text', async () => {
    mockCommands(libraryResponses({ get_model_capabilities: {} }));
    await renderPane();
    expect(screen.queryByText('Vision')).not.toBeInTheDocument();
    expect(screen.queryByText('Reasoning')).not.toBeInTheDocument();
    // Text is unconditional, so it survives a missing capability map.
    expect(screen.getAllByText('Text')).toHaveLength(2);
  });

  it('Set as active commits the model, lifts the config, and refreshes', async () => {
    mockCommands(libraryResponses({ update_provider_field: undefined }));
    const onSaved = vi.fn();
    await renderPane(makeConfig('org/gemma:gemma.gguf'), onSaved);
    openMenu('qwen');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Set as active' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'builtin',
      field: 'model',
      value: 'org/qwen:qwen.gguf',
    });
    expect(onSaved).toHaveBeenCalledWith(NEW_CONFIG);
  });

  it('leaves the lift to the focus resync when Set as active cannot read the config', async () => {
    mockCommands(
      libraryResponses({
        update_provider_field: undefined,
        get_config: new Reject(new Error('read failed')),
      }),
    );
    const onSaved = vi.fn();
    await renderPane(makeConfig('org/gemma:gemma.gguf'), onSaved);
    openMenu('qwen');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Set as active' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('update_provider_field', {
      providerId: 'builtin',
      field: 'model',
      value: 'org/qwen:qwen.gguf',
    });
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('swallows an update_provider_field failure on Set as active', async () => {
    mockCommands(
      libraryResponses({
        update_provider_field: new Reject(new Error('write failed')),
      }),
    );
    const onSaved = vi.fn();
    await renderPane(makeConfig('org/gemma:gemma.gguf'), onSaved);
    openMenu('qwen');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Set as active' }));
    await flush();
    expect(onSaved).not.toHaveBeenCalled();
    expect(screen.getByText('qwen')).toBeInTheDocument();
  });

  it('opens the repo on Hugging Face from the model name link', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    fireEvent.click(screen.getByRole('button', { name: 'gemma' }));
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://huggingface.co/org/gemma',
    });
  });

  it('Delete asks for confirmation and Cancel backs out without deleting', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    openMenu('gemma');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete model' }));
    expect(screen.getByText('Delete gemma?')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByText('Delete gemma?')).not.toBeInTheDocument();
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
    openMenu('gemma');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete model' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
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
    openMenu('qwen');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete model' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
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
    openMenu('gemma');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete model' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
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

  it('reveals the model in Finder from the popover', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    openMenu('gemma');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Reveal in Finder' }));
    expect(invokeMock).toHaveBeenCalledWith('reveal_model_in_finder', {
      id: 'org/gemma:gemma.gguf',
    });
  });

  it('swallows a reveal-in-Finder failure', async () => {
    mockCommands(
      libraryResponses({ reveal_model_in_finder: new Reject('no blob') }),
    );
    await renderPane();
    openMenu('gemma');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Reveal in Finder' }));
    await flush();
    // The row is untouched; the failure is best-effort and silent.
    expect(screen.getByText('gemma')).toBeInTheDocument();
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
    // the active model falls back to "", so no row is Active.
    const ollamaOnly: RawAppConfig = {
      ...BASE_CONFIG,
      inference: {
        ...BASE_CONFIG.inference,
        providers: [BASE_CONFIG.inference.providers[1]],
      },
    };
    await renderPane(ollamaOnly);
    expect(screen.queryByText('Active')).not.toBeInTheDocument();
    openMenu('gemma');
    expect(
      screen.getByRole('menuitem', { name: 'Set as active' }),
    ).toBeInTheDocument();
  });

  it('drops the popover below the trigger when there is room beneath it', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    openMenu('gemma');
    expect(screen.getByRole('menu')).toHaveAttribute('data-side', 'bottom');
  });

  it('flips the popover above the trigger when there is more room above', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    const manage = screen.getByRole('button', { name: 'Manage qwen' });
    // Simulate the trigger sitting near the window's bottom edge: the space
    // above it (top) exceeds the space below, so the menu flips up rather than
    // spilling past the window's hidden overflow.
    manage.getBoundingClientRect = () =>
      ({
        top: window.innerHeight - 40,
        bottom: window.innerHeight - 8,
      }) as unknown as DOMRect;
    fireEvent.click(manage);
    expect(screen.getByRole('menu')).toHaveAttribute('data-side', 'top');
  });

  it('keeps a top row dropping down when the space above is tighter than below', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    const manage = screen.getByRole('button', { name: 'Manage gemma' });
    // A row high in the window: the space below it still beats the space above,
    // so the menu must drop down (the old fixed-height estimate wrongly flipped
    // such rows up into the window chrome).
    manage.getBoundingClientRect = () =>
      ({ top: 150, bottom: window.innerHeight - 200 }) as unknown as DOMRect;
    fireEvent.click(manage);
    expect(screen.getByRole('menu')).toHaveAttribute('data-side', 'bottom');
  });

  it('toggles the popover closed when its own button is clicked again', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    const manage = screen.getByRole('button', { name: 'Manage gemma' });
    fireEvent.click(manage);
    expect(
      screen.getByRole('menuitem', { name: 'Delete model' }),
    ).toBeInTheDocument();
    fireEvent.click(manage);
    expect(
      screen.queryByRole('menuitem', { name: 'Delete model' }),
    ).not.toBeInTheDocument();
  });

  it('closes the popover on an outside click', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    openMenu('gemma');
    expect(
      screen.getByRole('menuitem', { name: 'Delete model' }),
    ).toBeInTheDocument();
    fireEvent.mouseDown(document.body);
    expect(
      screen.queryByRole('menuitem', { name: 'Delete model' }),
    ).not.toBeInTheDocument();
  });

  it('closes the popover on Escape but ignores other keys', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    openMenu('gemma');
    fireEvent.keyDown(document.body, { key: 'a' });
    expect(
      screen.getByRole('menuitem', { name: 'Delete model' }),
    ).toBeInTheDocument();
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(
      screen.queryByRole('menuitem', { name: 'Delete model' }),
    ).not.toBeInTheDocument();
  });

  it('keeps the popover open when clicking inside it', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    openMenu('gemma');
    fireEvent.mouseDown(
      screen.getByRole('menuitem', { name: 'Reveal in Finder' }),
    );
    expect(
      screen.getByRole('menuitem', { name: 'Reveal in Finder' }),
    ).toBeInTheDocument();
  });

  it('clears a stale delete error once a later delete succeeds', async () => {
    mockCommands(
      libraryResponses({ delete_installed_model: new Reject('file busy') }),
    );
    await renderPane();
    openMenu('gemma');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete model' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await flush();
    expect(screen.getByRole('alert')).toHaveTextContent('file busy');

    mockCommands(
      libraryResponses({
        list_installed_models: [QWEN],
        delete_installed_model: undefined,
      }),
    );
    openMenu('gemma');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete model' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await waitFor(() =>
      expect(screen.queryByRole('alert')).not.toBeInTheDocument(),
    );
  });

  it('filters the installed list by the search query', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    fireEvent.change(screen.getByPlaceholderText(/filter models/i), {
      target: { value: 'gemma' },
    });
    expect(screen.getByText('gemma')).toBeInTheDocument();
    expect(screen.queryByText('qwen')).not.toBeInTheDocument();
  });

  it('shows a no-match message when the filter excludes everything', async () => {
    mockCommands(libraryResponses());
    await renderPane();
    fireEvent.change(screen.getByPlaceholderText(/filter models/i), {
      target: { value: 'zzz' },
    });
    expect(screen.getByText(/No models match/)).toBeInTheDocument();
    expect(screen.queryByText('gemma')).not.toBeInTheDocument();
  });

  it('hides the filter row when only one model is installed', async () => {
    mockCommands(libraryResponses({ list_installed_models: [GEMMA] }));
    await renderPane();
    expect(screen.getByText('gemma')).toBeInTheDocument();
    expect(
      screen.queryByPlaceholderText(/filter models/i),
    ).not.toBeInTheDocument();
  });

  it('keeps the lone survivor visible when a stale filter would hide it after a delete', async () => {
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
    await renderPane();
    // Narrow the filter to just gemma, then delete it: the count drops to one,
    // the filter input unmounts, and the now-stale "gemma" query must not hide
    // the surviving qwen behind a "No models match" dead-end.
    fireEvent.change(screen.getByPlaceholderText(/filter models/i), {
      target: { value: 'gemma' },
    });
    expect(screen.queryByText('qwen')).not.toBeInTheDocument();
    openMenu('gemma');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete model' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await flush();
    expect(screen.getByText('qwen')).toBeInTheDocument();
    expect(screen.queryByText(/No models match/)).not.toBeInTheDocument();
    expect(
      screen.queryByPlaceholderText(/filter models/i),
    ).not.toBeInTheDocument();
  });
});
