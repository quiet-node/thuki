/**
 * Unit tests for the Staff-picks pane: Discover's curated front door.
 *
 * A flat, alphabetically-ordered list of rich model cards (no family grouping,
 * no recommended highlight). Each card shows the model name, its maker and a
 * one-line blurb, capability pills (Text always, plus Vision / Thinking), the
 * one quant Thuki chose with its size and license, a RAM-fit hint, and a single
 * icon download that runs the VERIFIED starter path (`download_starter`, pinned
 * revision + sha256). The download channel is captured the same way
 * BrowseAllPane.test.tsx does it.
 */

import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import { StaffPicksPane } from './StaffPicksPane';
import type { RawAppConfig } from '../../types';
import type { Starter, StarterOption } from '../../../types/starter';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

type MockChannel = { simulateMessage: (msg: unknown) => void };
let lastChannel: MockChannel | null = null;

/** Marks a command response as a rejection in `mockCommands`. */
class Reject {
  constructor(public readonly value: unknown) {}
}

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

function starter(over: Partial<Starter>): Starter {
  return {
    tier: 'balanced',
    family: 'Gemma',
    display_name: 'Gemma 4 12B',
    repo: 'google/gemma-4-12B-it-qat-q4_0-gguf',
    revision: 'a'.repeat(40),
    file_name: 'gemma-4-12b-it-qat-q4_0.gguf',
    sha256: 'b'.repeat(64),
    size_bytes: 6_975_877_728,
    quant: 'Q4_0',
    vision: true,
    thinking: false,
    reasoning_always: false,
    mmproj_file: 'mmproj.gguf',
    mmproj_sha256: 'c'.repeat(64),
    mmproj_bytes: 175_115_264,
    est_runtime_gb: 9.5,
    license_note: 'Apache 2.0',
    origin: 'Google',
    origin_repo: 'google/gemma-4-12B-it',
    ...over,
  };
}

function option(
  over: Partial<Starter>,
  opts: Partial<StarterOption> = {},
): StarterOption {
  return {
    starter: starter(over),
    fit: 'fits',
    installed: false,
    partial_bytes: null,
    ...opts,
  };
}

/** Three models, mirroring the shipped registry (deliberately NOT alpha order). */
const QWEN = option({
  tier: 'fast',
  family: 'Qwen',
  display_name: 'Qwen3.5 9B',
  repo: 'unsloth/Qwen3.5-9B-GGUF',
  file_name: 'Qwen3.5-9B-Q4_K_M.gguf',
  quant: 'Q4_K_M',
  vision: true,
  thinking: true,
  origin: 'Alibaba',
});
const GEMMA = option({});
const GPT_OSS = option({
  tier: 'smartest',
  family: 'gpt-oss',
  display_name: 'gpt-oss 20B',
  repo: 'ggml-org/gpt-oss-20b-GGUF',
  file_name: 'gpt-oss-20b-mxfp4.gguf',
  quant: 'MXFP4',
  vision: false,
  thinking: true,
  reasoning_always: true,
  mmproj_file: null,
  mmproj_sha256: null,
  mmproj_bytes: 0,
  origin: 'OpenAI',
});

const STARTERS: StarterOption[] = [QWEN, GEMMA, GPT_OSS];

const CONFIG_AFTER_INSTALL = { marker: 'fresh' } as unknown as RawAppConfig;

function picksResponses(overrides: Record<string, unknown> = {}) {
  return {
    get_starter_options: STARTERS,
    get_config: CONFIG_AFTER_INSTALL,
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

async function renderPane(
  onSaved: (next: RawAppConfig) => void = () => {},
  overrides: Record<string, unknown> = {},
) {
  mockCommands(picksResponses(overrides));
  const view = render(<StaffPicksPane onSaved={onSaved} />);
  await waitFor(() =>
    expect(invokeMock).toHaveBeenCalledWith('get_starter_options'),
  );
  await flush();
  return view;
}

/** The card element wrapping a model name. */
function cardFor(name: string): HTMLElement {
  return screen.getByText(name).closest('[data-model-card]') as HTMLElement;
}

describe('StaffPicksPane', () => {
  it('renders every model as a flat card, all visible at once', async () => {
    await renderPane();
    expect(screen.getByText('Gemma 4 12B')).toBeInTheDocument();
    expect(screen.getByText('Qwen3.5 9B')).toBeInTheDocument();
    expect(screen.getByText('gpt-oss 20B')).toBeInTheDocument();
  });

  it('orders the cards alphabetically by model name', async () => {
    await renderPane();
    const names = screen
      .getAllByTestId('staff-model-name')
      .map((el) => el.textContent);
    expect(names).toEqual(['Gemma 4 12B', 'gpt-oss 20B', 'Qwen3.5 9B']);
  });

  it('shows no Recommended badge on any card', async () => {
    await renderPane();
    expect(screen.queryByText(/Recommended/)).not.toBeInTheDocument();
  });

  it('shows the maker, blurb, pills, quant, size, license and fit on a card', async () => {
    await renderPane();
    const card = cardFor('Gemma 4 12B');
    expect(
      within(card).getByText(/Google · Well-rounded, reads images/),
    ).toBeInTheDocument();
    expect(within(card).getByText('Text')).toBeInTheDocument();
    expect(within(card).getByText('Vision')).toBeInTheDocument();
    expect(within(card).queryByText('Thinking')).not.toBeInTheDocument();
    expect(within(card).getByText(/Q4_0/)).toBeInTheDocument();
    expect(within(card).getByText(/7\.2 GB/)).toBeInTheDocument();
    expect(within(card).getByText(/Apache 2\.0/)).toBeInTheDocument();
    expect(within(card).getByText('Comfortable')).toBeInTheDocument();
  });

  it('shows a Thinking pill on a thinking model and omits Vision on a text-only one', async () => {
    await renderPane();
    const qwen = cardFor('Qwen3.5 9B');
    expect(within(qwen).getByText('Thinking')).toBeInTheDocument();
    expect(within(qwen).getByText('Vision')).toBeInTheDocument();
    const oss = cardFor('gpt-oss 20B');
    expect(within(oss).getByText('Thinking')).toBeInTheDocument();
    expect(within(oss).queryByText('Vision')).not.toBeInTheDocument();
  });

  it('falls back to the maker alone when a model has no blurb', async () => {
    await renderPane(() => {}, {
      get_starter_options: [
        option({
          family: 'Llama',
          display_name: 'Llama 3.3 8B',
          origin: 'Meta',
        }),
      ],
    });
    const card = cardFor('Llama 3.3 8B');
    // No blurb for the Llama family: the maker line is just the maker.
    expect(within(card).getByText('Meta')).toBeInTheDocument();
  });

  it('shows just the maker when a model carries no family at all', async () => {
    await renderPane(() => {}, {
      get_starter_options: [
        option({
          family: undefined,
          display_name: 'Mystery 7B',
          origin: 'Acme',
        }),
      ],
    });
    const card = cardFor('Mystery 7B');
    expect(within(card).getByText('Acme')).toBeInTheDocument();
  });

  it('downloads a model through the verified starter path', async () => {
    await renderPane();
    const card = cardFor('Gemma 4 12B');
    fireEvent.click(within(card).getByRole('button', { name: 'Download' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith(
      'download_starter',
      expect.objectContaining({ tier: 'balanced' }),
    );
  });

  it('lifts a fresh config and refreshes when a download completes', async () => {
    const onSaved = vi.fn();
    await renderPane(onSaved);
    const card = cardFor('Gemma 4 12B');
    fireEvent.click(within(card).getByRole('button', { name: 'Download' }));
    await flush();
    expect(screen.getByText('Downloading model')).toBeInTheDocument();
    act(() => {
      lastChannel?.simulateMessage({ type: 'AllDone' });
    });
    await flush();
    expect(onSaved).toHaveBeenCalledWith(CONFIG_AFTER_INSTALL);
  });

  it('leaves the lift to a later resync when get_config fails post-download', async () => {
    const onSaved = vi.fn();
    await renderPane(onSaved, {
      get_config: new Reject(new Error('read failed')),
    });
    const card = cardFor('Gemma 4 12B');
    fireEvent.click(within(card).getByRole('button', { name: 'Download' }));
    await flush();
    act(() => {
      lastChannel?.simulateMessage({ type: 'AllDone' });
    });
    await flush();
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('cancels an in-flight download', async () => {
    await renderPane();
    const card = cardFor('Gemma 4 12B');
    fireEvent.click(within(card).getByRole('button', { name: 'Download' }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('cancel_model_download');
  });

  it('retries after a failed download', async () => {
    await renderPane();
    const card = cardFor('Gemma 4 12B');
    fireEvent.click(within(card).getByRole('button', { name: 'Download' }));
    await flush();
    act(() => {
      lastChannel?.simulateMessage({
        type: 'Failed',
        data: { kind: 'other', message: 'boom' },
      });
    });
    expect(screen.getByText('boom')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await flush();
    const starts = invokeMock.mock.calls.filter(
      (c: unknown[]) => c[0] === 'download_starter',
    );
    expect(starts).toHaveLength(2);
  });

  it('returns to the card from a terminal failure via Choose a different model', async () => {
    await renderPane();
    const card = cardFor('Gemma 4 12B');
    fireEvent.click(within(card).getByRole('button', { name: 'Download' }));
    await flush();
    act(() => {
      lastChannel?.simulateMessage({
        type: 'Failed',
        data: { kind: 'disk_full', message: 'no space' },
      });
    });
    fireEvent.click(
      screen.getByRole('button', { name: 'Choose a different model' }),
    );
    // The Gemma card is back to its download button, not stuck on the failure.
    expect(
      within(cardFor('Gemma 4 12B')).getByRole('button', { name: 'Download' }),
    ).toBeInTheDocument();
  });

  it('shows Installed instead of a download button', async () => {
    await renderPane(() => {}, {
      get_starter_options: [{ ...GEMMA, installed: true }, QWEN, GPT_OSS],
    });
    const card = cardFor('Gemma 4 12B');
    expect(within(card).getByText('Installed')).toBeInTheDocument();
    expect(
      within(card).queryByRole('button', { name: 'Download' }),
    ).not.toBeInTheDocument();
  });

  it('offers Resume and Discard for an interrupted partial', async () => {
    await renderPane(() => {}, {
      get_starter_options: [
        { ...GEMMA, partial_bytes: 2_000_000_000 },
        QWEN,
        GPT_OSS,
      ],
    });
    const card = cardFor('Gemma 4 12B');
    fireEvent.click(within(card).getByRole('button', { name: /Resume/ }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith(
      'download_starter',
      expect.objectContaining({ tier: 'balanced' }),
    );
  });

  it('discards an interrupted partial and refreshes', async () => {
    await renderPane(() => {}, {
      get_starter_options: [
        { ...GEMMA, partial_bytes: 2_000_000_000 },
        QWEN,
        GPT_OSS,
      ],
    });
    const card = cardFor('Gemma 4 12B');
    fireEvent.click(within(card).getByRole('button', { name: 'Discard' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'b'.repeat(64),
    });
  });

  it('opens the model on Hugging Face from its provenance link', async () => {
    await renderPane();
    const card = cardFor('Gemma 4 12B');
    fireEvent.click(within(card).getByRole('button', { name: /Hugging Face/ }));
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://huggingface.co/google/gemma-4-12B-it-qat-q4_0-gguf',
    });
  });

  it('shows an empty state when no starters are available', async () => {
    await renderPane(() => {}, { get_starter_options: [] });
    expect(screen.getByText(/No curated models/)).toBeInTheDocument();
  });

  it('degrades to the empty state when the probe rejects', async () => {
    await renderPane(() => {}, {
      get_starter_options: new Reject(new Error('probe failed')),
    });
    expect(screen.getByText(/No curated models/)).toBeInTheDocument();
  });
});
