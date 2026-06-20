/**
 * Unit tests for the Staff-picks pane: Discover's curated front door.
 *
 * Covers the family accordion (grouping, default-expanded recommended family,
 * expand/collapse), the model rows (name, recommended star, capability pills,
 * quant/size/license meta, RAM-fit hint), and the verified starter download
 * flow (download -> progress -> ready lifts config + refreshes; installed;
 * resume/discard of a partial; failure). The download channel is captured the
 * same way BrowseAllPane.test.tsx does it: `onEvent` is grabbed off the invoke
 * args and driven with `simulateMessage`.
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

/** Three single-model families, mirroring the shipped registry. */
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

/** The accordion header button for a family. */
function familyHeader(name: string): HTMLElement {
  return screen.getByRole('button', { name: new RegExp(`^${name}`) });
}

describe('StaffPicksPane', () => {
  it('renders a section per family with its name', async () => {
    await renderPane();
    expect(familyHeader('Qwen')).toBeInTheDocument();
    expect(familyHeader('Gemma')).toBeInTheDocument();
    expect(familyHeader('gpt-oss')).toBeInTheDocument();
  });

  it('expands the recommended family by default and collapses the rest', async () => {
    await renderPane();
    // Gemma holds the balanced (recommended) tier, so its model row is shown.
    expect(screen.getByText('Gemma 4 12B')).toBeInTheDocument();
    // The other families start collapsed.
    expect(screen.queryByText('Qwen3.5 9B')).not.toBeInTheDocument();
    expect(screen.queryByText('gpt-oss 20B')).not.toBeInTheDocument();
  });

  it('expands a collapsed family on click and collapses it again', async () => {
    await renderPane();
    fireEvent.click(familyHeader('Qwen'));
    expect(screen.getByText('Qwen3.5 9B')).toBeInTheDocument();
    fireEvent.click(familyHeader('Qwen'));
    expect(screen.queryByText('Qwen3.5 9B')).not.toBeInTheDocument();
  });

  it('marks the recommended model and shows its meta and pills', async () => {
    await renderPane();
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
    expect(within(row).getByText('Recommended')).toBeInTheDocument();
    expect(within(row).getByText('Text')).toBeInTheDocument();
    expect(within(row).getByText('Vision')).toBeInTheDocument();
    expect(within(row).queryByText('Thinking')).not.toBeInTheDocument();
    expect(within(row).getByText(/Q4_0/)).toBeInTheDocument();
    expect(within(row).getByText(/7\.2 GB/)).toBeInTheDocument();
    expect(within(row).getByText('Comfortable')).toBeInTheDocument();
  });

  it('shows a Thinking pill on a thinking-capable model', async () => {
    await renderPane();
    fireEvent.click(familyHeader('Qwen'));
    const row = screen
      .getByText('Qwen3.5 9B')
      .closest('[data-model-row]') as HTMLElement;
    expect(within(row).getByText('Thinking')).toBeInTheDocument();
    expect(within(row).getByText('Vision')).toBeInTheDocument();
  });

  it('omits the Vision pill on a text-only model', async () => {
    await renderPane();
    fireEvent.click(familyHeader('gpt-oss'));
    const row = screen
      .getByText('gpt-oss 20B')
      .closest('[data-model-row]') as HTMLElement;
    expect(within(row).getByText('Text')).toBeInTheDocument();
    expect(within(row).getByText('Thinking')).toBeInTheDocument();
    expect(within(row).queryByText('Vision')).not.toBeInTheDocument();
  });

  it('downloads a model through the verified starter path', async () => {
    await renderPane();
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Download' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith(
      'download_starter',
      expect.objectContaining({ tier: 'balanced' }),
    );
  });

  it('lifts a fresh config and refreshes when a download completes', async () => {
    const onSaved = vi.fn();
    await renderPane(onSaved);
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Download' }));
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
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
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
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Download' }));
    await flush();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('cancel_model_download');
  });

  it('retries after a failed download', async () => {
    await renderPane();
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
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
      (c: unknown[]) => c[0] === 'download_starter',
    );
    expect(starts).toHaveLength(2);
  });

  it('returns to the row from a terminal failure via Choose a different model', async () => {
    await renderPane();
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
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
      screen.getByRole('button', { name: 'Download' }),
    ).toBeInTheDocument();
  });

  it('shows Installed instead of a download button', async () => {
    await renderPane(() => {}, {
      get_starter_options: [{ ...GEMMA, installed: true }, QWEN, GPT_OSS],
    });
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
    expect(within(row).getByText('Installed')).toBeInTheDocument();
    expect(
      within(row).queryByRole('button', { name: 'Download' }),
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
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: /Resume/ }));
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
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Discard' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('discard_partial_download', {
      sha256: 'b'.repeat(64),
    });
  });

  it('opens the model on Hugging Face from its provenance link', async () => {
    await renderPane();
    const row = screen
      .getByText('Gemma 4 12B')
      .closest('[data-model-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: /Hugging Face/ }));
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

  it('groups several sizes of one family under a single section', async () => {
    const gemma4b = option({
      tier: 'fast',
      family: 'Gemma',
      display_name: 'Gemma 4 4B',
      file_name: 'gemma-4-4b.gguf',
    });
    const gemma12b = option({}); // balanced Gemma 4 12B
    await renderPane(() => {}, {
      get_starter_options: [gemma4b, gemma12b],
    });
    // One Gemma section (it holds the recommended tier, so it is open) lists
    // both sizes, and the header counts them.
    const header = familyHeader('Gemma');
    expect(header).toHaveTextContent('2 models');
    expect(screen.getByText('Gemma 4 4B')).toBeInTheDocument();
    expect(screen.getByText('Gemma 4 12B')).toBeInTheDocument();
  });

  it('falls back to the maker blurb and display name for an unlabelled family', async () => {
    const orphan = option({
      tier: 'balanced',
      family: undefined,
      display_name: 'Mystery 7B',
      origin: 'Acme',
    });
    await renderPane(() => {}, { get_starter_options: [orphan] });
    // No family label: the section is keyed by the model name and its blurb
    // falls back to the maker.
    const header = familyHeader('Mystery 7B');
    expect(header).toHaveTextContent('Acme');
  });

  it('falls back to expanding the first family when none is recommended', async () => {
    // A catalog with no balanced tier: the first family expands so the pane is
    // never fully collapsed.
    const fastOnly = option({
      tier: 'fast',
      family: 'Qwen',
      display_name: 'Qwen3.5 9B',
    });
    await renderPane(() => {}, { get_starter_options: [fastOnly] });
    expect(screen.getByText('Qwen3.5 9B')).toBeInTheDocument();
  });
});
