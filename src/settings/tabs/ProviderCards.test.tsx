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

import { AddOpenAiProvider, OpenAiProviderCard } from './ProviderCards';
import type { RawAppConfig, RawProvider } from '../types';

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
  behavior: {
    auto_replace: false,
    auto_close: false,
    auto_search: true,
    search_notice_acknowledged: false,
  },
  debug: { trace_enabled: false },
};

/** Distinct snapshot so onSaved assertions cannot pass by referential luck. */
const NEW_CONFIG: RawAppConfig = {
  ...BASE_CONFIG,
  prompt: { system: 'updated' },
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

/** Marks a command response as a rejection in `mockCommands`. */
class Reject {
  constructor(public readonly value: unknown) {}
}

/**
 * Routes `invoke` by command name. Values: `Reject` throws its payload,
 * functions are called with the invoke args (for stateful sequences), and
 * anything else resolves as-is.
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

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

/**
 * A queue of externally-settled promises, used to control the resolution
 * order of overlapping async responses (e.g. two in-flight model-list calls).
 */
function deferredQueue<T>() {
  const items: Array<{
    resolve: (value: T) => void;
    reject: (reason: unknown) => void;
  }> = [];
  const next = () => {
    let resolve!: (value: T) => void;
    let reject!: (reason: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
      resolve = res;
      reject = rej;
    });
    items.push({ resolve, reject });
    return promise;
  };
  return { items, next };
}

beforeEach(() => {
  invokeMock.mockReset();
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

  it('ignores a stale model-list response that resolves after a newer one', async () => {
    const lists = deferredQueue<string[]>();
    mockCommands({
      list_openai_models: () => lists.next(),
      has_provider_api_key: false,
      update_provider_field: configWith({
        ...OPENAI_PROVIDER,
        base_url: 'http://127.0.0.1:9999',
      }),
    });
    render(<StatefulOpenAiCard />);
    await flush(); // mount fires the first refresh (lists.items[0]), still pending

    const url = screen.getByLabelText('OpenAI-compatible base URL');
    fireEvent.focus(url);
    fireEvent.change(url, { target: { value: 'http://127.0.0.1:9999' } });
    fireEvent.blur(url);
    // The committed base URL lifts a new config, re-running the effect and
    // firing a second refresh (lists.items[1]) while the first is in flight.
    await waitFor(() => expect(lists.items.length).toBe(2));

    // Newer refresh settles first and wins.
    await act(async () => {
      lists.items[1].resolve(['new-model']);
      await Promise.resolve();
    });
    expect(screen.getByText('new-model')).toBeInTheDocument();

    // Stale earlier refresh settles late and must not overwrite the newer one.
    await act(async () => {
      lists.items[0].resolve(['old-model']);
      await Promise.resolve();
    });
    expect(screen.queryByText('old-model')).not.toBeInTheDocument();
    expect(screen.getByText('new-model')).toBeInTheDocument();
  });

  it('ignores a stale model-list rejection that settles after a newer success', async () => {
    const lists = deferredQueue<string[]>();
    mockCommands({
      list_openai_models: () => lists.next(),
      has_provider_api_key: false,
      update_provider_field: configWith({
        ...OPENAI_PROVIDER,
        base_url: 'http://127.0.0.1:9999',
      }),
    });
    render(<StatefulOpenAiCard />);
    await flush();

    const url = screen.getByLabelText('OpenAI-compatible base URL');
    fireEvent.focus(url);
    fireEvent.change(url, { target: { value: 'http://127.0.0.1:9999' } });
    fireEvent.blur(url);
    await waitFor(() => expect(lists.items.length).toBe(2));

    await act(async () => {
      lists.items[1].resolve(['new-model']);
      await Promise.resolve();
    });
    expect(screen.getByText('new-model')).toBeInTheDocument();

    // A late rejection from the superseded refresh must not surface an error
    // or clear the newer model list.
    await act(async () => {
      lists.items[0].reject('late failure');
      await Promise.resolve();
    });
    expect(screen.queryByText('Couldn’t list models')).not.toBeInTheDocument();
    expect(screen.getByText('new-model')).toBeInTheDocument();
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
