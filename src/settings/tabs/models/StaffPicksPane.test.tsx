/**
 * Unit tests for the Staff-picks pane: Discover's curated front door.
 *
 * Models are grouped into use-case sections (Everyday chat / Compact & fast /
 * Deep reasoning), known sections first in a fixed order, then any extra
 * category alphabetically; within a section models are alphabetical. Each
 * compact row shows the model name, capability pills (Text always, plus Vision
 * / Thinking), a `size · maker` sub-line, a RAM-fit hint, and a single icon
 * download that runs the VERIFIED starter path (`download_staff_pick`, pinned
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
    id: 'gemma-4-12b',
    tier: 'balanced',
    family: 'Gemma',
    category: 'Everyday chat',
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

/** Two everyday models + one reasoning model (deliberately NOT alpha order). */
const QWEN = option({
  id: 'qwen3.5-9b',
  tier: 'fast',
  context_length: 262_144,
  family: 'Qwen',
  category: 'Everyday chat',
  display_name: 'Qwen3.5 9B',
  repo: 'unsloth/Qwen3.5-9B-GGUF',
  file_name: 'Qwen3.5-9B-Q4_K_M.gguf',
  quant: 'Q4_K_M',
  vision: true,
  thinking: true,
  origin: 'Alibaba',
});
const GEMMA = option({ context_length: 131_072 });
const GPT_OSS = option({
  id: 'gpt-oss-20b',
  tier: 'smartest',
  context_length: 131_072,
  family: 'gpt-oss',
  category: 'Deep reasoning',
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
    get_staff_picks: STARTERS,
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
    expect(invokeMock).toHaveBeenCalledWith('get_staff_picks'),
  );
  await flush();
  return view;
}

/** The row element wrapping a model name. */
function rowFor(name: string): HTMLElement {
  return screen.getByText(name).closest('[data-model-row]') as HTMLElement;
}

describe('StaffPicksPane', () => {
  it('renders a section only for categories that have models', async () => {
    await renderPane();
    expect(screen.getByText('Everyday chat')).toBeInTheDocument();
    expect(screen.getByText('Deep reasoning')).toBeInTheDocument();
    // No model carries "Compact & fast", so that section never renders.
    expect(screen.queryByText('Compact & fast')).not.toBeInTheDocument();
  });

  it('orders sections by the known order and models alphabetically within', async () => {
    await renderPane();
    const sections = screen
      .getAllByTestId('staff-section-label')
      .map((el) => el.textContent);
    expect(sections).toEqual(['Everyday chat', 'Deep reasoning']);
    const names = screen
      .getAllByTestId('staff-model-name')
      .map((el) => el.textContent);
    // Everyday: Gemma before Qwen (alpha); then the reasoning section.
    expect(names).toEqual(['Gemma 4 12B', 'Qwen3.5 9B', 'gpt-oss 20B']);
  });

  it('shows no Recommended badge on any row', async () => {
    await renderPane();
    expect(screen.queryByText(/Recommended/)).not.toBeInTheDocument();
  });

  it('shows the name, pills, the size · context · maker sub-line, and fit', async () => {
    await renderPane();
    const row = rowFor('Gemma 4 12B');
    expect(within(row).getByText('Text')).toBeInTheDocument();
    expect(within(row).getByText('Vision')).toBeInTheDocument();
    expect(within(row).queryByText('Thinking')).not.toBeInTheDocument();
    // Context window sits in the metadata sub-line, between size and maker.
    expect(within(row).getByText('7.2 GB · 128K · Google')).toBeInTheDocument();
    expect(within(row).getByText('Comfortable')).toBeInTheDocument();
  });

  it('places the context window between size and maker for each model', async () => {
    await renderPane();
    expect(
      within(rowFor('Qwen3.5 9B')).getByText('7.2 GB · 256K · Alibaba'),
    ).toBeInTheDocument();
  });

  it('falls back to size · maker when a model has no context window', async () => {
    await renderPane(() => {}, {
      get_staff_picks: [
        option({ context_length: undefined, display_name: 'Mystery 7B' }),
      ],
    });
    const row = rowFor('Mystery 7B');
    expect(within(row).getByText('7.2 GB · Google')).toBeInTheDocument();
  });

  it('shows a Thinking pill on a thinking model and omits Vision on a text-only one', async () => {
    await renderPane();
    const qwen = rowFor('Qwen3.5 9B');
    expect(within(qwen).getByText('Thinking')).toBeInTheDocument();
    expect(within(qwen).getByText('Vision')).toBeInTheDocument();
    const oss = rowFor('gpt-oss 20B');
    expect(within(oss).getByText('Thinking')).toBeInTheDocument();
    expect(within(oss).queryByText('Vision')).not.toBeInTheDocument();
  });

  it('appends an unrecognized category after the known sections', async () => {
    await renderPane(() => {}, {
      get_staff_picks: [
        GEMMA,
        option({
          id: 'qwen3-coder-7b',
          tier: 'fast',
          category: 'Coding',
          display_name: 'Qwen3 Coder 7B',
        }),
      ],
    });
    const sections = screen
      .getAllByTestId('staff-section-label')
      .map((el) => el.textContent);
    expect(sections).toEqual(['Everyday chat', 'Coding']);
  });

  it('buckets a model with no category under Other', async () => {
    await renderPane(() => {}, {
      get_staff_picks: [
        option({ category: undefined, display_name: 'Mystery 7B' }),
      ],
    });
    expect(screen.getByText('Other')).toBeInTheDocument();
    expect(screen.getByText('Mystery 7B')).toBeInTheDocument();
  });

  it('downloads a model through the verified starter path', async () => {
    await renderPane();
    const row = rowFor('Gemma 4 12B');
    fireEvent.click(within(row).getByRole('button', { name: 'Download' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith(
      'download_staff_pick',
      expect.objectContaining({ id: 'gemma-4-12b' }),
    );
  });

  it('lifts a fresh config and refreshes when a download completes', async () => {
    const onSaved = vi.fn();
    await renderPane(onSaved);
    const row = rowFor('Gemma 4 12B');
    fireEvent.click(within(row).getByRole('button', { name: 'Download' }));
    await flush();
    expect(screen.getByTestId('download-figures')).toBeInTheDocument();
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
    const row = rowFor('Gemma 4 12B');
    fireEvent.click(within(row).getByRole('button', { name: 'Download' }));
    await flush();
    act(() => {
      lastChannel?.simulateMessage({ type: 'AllDone' });
    });
    await flush();
    expect(onSaved).not.toHaveBeenCalled();
  });

  it('cancels an in-flight download', async () => {
    await renderPane();
    const row = rowFor('Gemma 4 12B');
    fireEvent.click(within(row).getByRole('button', { name: 'Download' }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('cancel_model_download');
  });

  it('retries after a failed download', async () => {
    await renderPane();
    const row = rowFor('Gemma 4 12B');
    fireEvent.click(within(row).getByRole('button', { name: 'Download' }));
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
      (c: unknown[]) => c[0] === 'download_staff_pick',
    );
    expect(starts).toHaveLength(2);
  });

  it('returns to the row from a terminal failure via Choose a different model', async () => {
    await renderPane();
    const row = rowFor('Gemma 4 12B');
    fireEvent.click(within(row).getByRole('button', { name: 'Download' }));
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
    expect(
      within(rowFor('Gemma 4 12B')).getByRole('button', { name: 'Download' }),
    ).toBeInTheDocument();
  });

  it('shows no download button and no label for an installed model', async () => {
    await renderPane(() => {}, {
      get_staff_picks: [{ ...GEMMA, installed: true }, QWEN, GPT_OSS],
    });
    const row = rowFor('Gemma 4 12B');
    // Already installed: no download affordance and no "Installed" badge; the
    // row still shows the model and its fit.
    expect(
      within(row).queryByRole('button', { name: 'Download' }),
    ).not.toBeInTheDocument();
    expect(within(row).queryByText('Installed')).not.toBeInTheDocument();
    expect(within(row).getByText('Comfortable')).toBeInTheDocument();
  });

  it('offers Resume and Discard for an interrupted partial', async () => {
    await renderPane(() => {}, {
      get_staff_picks: [
        { ...GEMMA, partial_bytes: 2_000_000_000 },
        QWEN,
        GPT_OSS,
      ],
    });
    const row = rowFor('Gemma 4 12B');
    fireEvent.click(within(row).getByRole('button', { name: /Resume/ }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith(
      'download_staff_pick',
      expect.objectContaining({ id: 'gemma-4-12b' }),
    );
  });

  it('discards an interrupted partial and refreshes', async () => {
    await renderPane(() => {}, {
      get_staff_picks: [
        { ...GEMMA, partial_bytes: 2_000_000_000 },
        QWEN,
        GPT_OSS,
      ],
    });
    const row = rowFor('Gemma 4 12B');
    fireEvent.click(within(row).getByRole('button', { name: 'Discard' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'b'.repeat(64),
    });
  });

  it('shows an empty state when no starters are available', async () => {
    await renderPane(() => {}, { get_staff_picks: [] });
    expect(screen.getByText(/No curated models/)).toBeInTheDocument();
  });

  it('degrades to the empty state when the probe rejects', async () => {
    await renderPane(() => {}, {
      get_staff_picks: new Reject(new Error('probe failed')),
    });
    expect(screen.getByText(/No curated models/)).toBeInTheDocument();
  });
});
