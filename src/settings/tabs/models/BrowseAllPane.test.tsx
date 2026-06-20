/**
 * Unit tests for the Browse-all pane: the in-app Hugging Face GGUF browser
 * (Discover's advanced pathway).
 *
 * Covers the search field wiring, family filter chips, the result rows (org
 * parsing, gated rows, RAM-fit hint, the Hugging Face link), pagination (Load
 * more), the per-row quant accordion (expand, per-quant fit, empty repo, list
 * error), and the download flow (start, progress, ready -> onSaved + collapse,
 * cancel, retry). The download channel is captured the same way
 * ProviderCards.test.tsx does it: `onEvent` is grabbed off the invoke args and
 * driven with `simulateMessage`.
 */

import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import { BrowseAllPane } from './BrowseAllPane';
import {
  HF_SEARCH_DEBOUNCE_MS,
  HF_PAGE_SIZE,
  clearHfSearchCache,
} from './useHfSearch';
import type { HfModelSummary } from '../../../types/hf';
import type { HfGgufFile } from '../../../types/starter';
import type { RawAppConfig } from '../../types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

type MockChannel = { simulateMessage: (msg: unknown) => void };
let lastChannel: MockChannel | null = null;

/** Marks a command response as a rejection in `mockCommands`. */
class Reject {
  constructor(public readonly value: unknown) {}
}

/**
 * Routes `invoke` by command name. `Reject` throws its payload, functions are
 * called with the invoke args, anything else resolves as-is. A channel passed
 * via `onEvent` is captured for download-event simulation.
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

const RESULTS: HfModelSummary[] = [
  {
    id: 'google/gemma-4-12b-it-GGUF',
    downloads: 1_200_000,
    gated: false,
    context_length: 262_144,
  },
  // No context window: covers the "skip the segment" path.
  { id: 'unsloth/gemma-4-27b-it-GGUF', downloads: 410_000, gated: false },
  { id: 'meta-llama/Llama-3-8B-GGUF', downloads: 9_000, gated: true },
];

const GGUFS: HfGgufFile[] = [
  { file: 'gemma-q4.gguf', size_bytes: 5_000_000_000, fit: 'tight' },
  { file: 'gemma-q8.gguf', size_bytes: 9_000_000_000 },
];

const CONFIG_AFTER_INSTALL = { marker: 'fresh' } as unknown as RawAppConfig;

/**
 * Default backend: the search returns RESULTS, a repo lookup returns GGUFS,
 * and get_config returns the post-install snapshot.
 */
function discoverResponses(overrides: Record<string, unknown> = {}) {
  return {
    search_hf_models: RESULTS,
    list_hf_repo_ggufs: GGUFS,
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
  clearHfSearchCache();
});

afterEach(() => {
  vi.useRealTimers();
});

/** Renders the pane and waits for the mount search to resolve. */
async function renderPane(
  onSaved: (next: RawAppConfig) => void = () => {},
  overrides: Record<string, unknown> = {},
) {
  mockCommands(discoverResponses(overrides));
  const view = render(<BrowseAllPane onSaved={onSaved} />);
  await waitFor(() =>
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', {
      query: '',
      limit: HF_PAGE_SIZE,
    }),
  );
  await flush();
  return view;
}

describe('BrowseAllPane', () => {
  it('renders a row per search result with the repo id and org line', async () => {
    await renderPane();
    expect(screen.getByText('google/gemma-4-12b-it-GGUF')).toBeInTheDocument();
    expect(
      screen.getByText('google · 1,200,000 downloads · 256K'),
    ).toBeInTheDocument();
    // The second result has no context window, so the segment is omitted.
    expect(screen.getByText('unsloth · 410,000 downloads')).toBeInTheDocument();
  });

  it('shows the result count in the sub-bar', async () => {
    await renderPane();
    expect(screen.getByText(/chat models/)).toHaveTextContent('3 chat models');
  });

  it('does not show a RAM-fit hint on the collapsed model row', async () => {
    await renderPane();
    // The row-level fit was an unreliable repo-id estimate and is gone; fit
    // shows only on the per-quant rows once a row is expanded.
    expect(screen.queryByText('Comfortable')).not.toBeInTheDocument();
    expect(screen.queryByText('Tight')).not.toBeInTheDocument();
    expect(screen.queryByText('Heavy')).not.toBeInTheDocument();
  });

  it('parses the org line from the full id when it has no org segment', async () => {
    await renderPane(() => {}, {
      search_hf_models: [
        { id: 'standalone-repo', downloads: 12, gated: false },
      ],
    });
    expect(screen.getByText('standalone-repo')).toBeInTheDocument();
    expect(
      screen.getByText('standalone-repo · 12 downloads'),
    ).toBeInTheDocument();
  });

  it('typing in the search drives a debounced fetch and re-renders results', async () => {
    vi.useFakeTimers();
    mockCommands(discoverResponses());
    render(<BrowseAllPane onSaved={() => {}} />);
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    invokeMock.mockClear();
    mockCommands(
      discoverResponses({
        search_hf_models: [
          { id: 'qwen/Qwen3-GGUF', downloads: 50, gated: false },
        ],
      }),
    );
    fireEvent.change(screen.getByRole('searchbox'), {
      target: { value: 'qwen' },
    });
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', {
      query: 'qwen',
      limit: HF_PAGE_SIZE,
    });
  });

  it('clicking a family chip sets the query to that family', async () => {
    vi.useFakeTimers();
    mockCommands(discoverResponses());
    render(<BrowseAllPane onSaved={() => {}} />);
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    invokeMock.mockClear();
    fireEvent.click(screen.getByRole('button', { name: 'Llama' }));
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', {
      query: 'Llama',
      limit: HF_PAGE_SIZE,
    });
    expect(screen.getByRole('button', { name: 'Llama' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  it('the All chip clears the query and is active by default', async () => {
    vi.useFakeTimers();
    mockCommands(discoverResponses());
    render(<BrowseAllPane onSaved={() => {}} />);
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'All' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    fireEvent.click(screen.getByRole('button', { name: 'Gemma' }));
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'All' })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
    invokeMock.mockClear();
    fireEvent.click(screen.getByRole('button', { name: 'All' }));
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    // The empty query was cached by the mount fetch, so returning to All is
    // served from cache without another Hub call.
    expect(invokeMock).not.toHaveBeenCalledWith(
      'search_hf_models',
      expect.anything(),
    );
    expect(screen.getByRole('button', { name: 'All' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  it('renders every family chip', async () => {
    await renderPane();
    for (const family of [
      'All',
      'Qwen',
      'Llama',
      'Gemma',
      'gpt-oss',
      'DeepSeek',
      'Phi',
    ]) {
      expect(screen.getByRole('button', { name: family })).toBeInTheDocument();
    }
  });

  it('opens the repo on Hugging Face when the title is clicked', async () => {
    await renderPane();
    fireEvent.click(
      screen.getByRole('button', { name: 'google/gemma-4-12b-it-GGUF' }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://huggingface.co/google/gemma-4-12b-it-GGUF',
    });
  });

  it('disables the download button and shows a Gated badge for a gated repo', async () => {
    await renderPane();
    const gatedRow = screen
      .getByText('meta-llama/Llama-3-8B-GGUF')
      .closest('[data-row]') as HTMLElement;
    expect(
      within(gatedRow).getByRole('button', { name: 'Show files' }),
    ).toBeDisabled();
    expect(within(gatedRow).getByText('Gated')).toBeInTheDocument();
  });

  it('expanding a row lists each GGUF file with its size and per-quant fit', async () => {
    await renderPane();
    const row = screen
      .getByText('google/gemma-4-12b-it-GGUF')
      .closest('[data-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Show files' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('list_hf_repo_ggufs', {
      repo: 'google/gemma-4-12b-it-GGUF',
    });
    expect(screen.getByText('gemma-q4.gguf')).toBeInTheDocument();
    expect(screen.getByText('5.0 GB')).toBeInTheDocument();
    // The first quant carries an accurate per-quant fit; the second does not.
    expect(within(row).getByText('Tight')).toBeInTheDocument();
    expect(screen.getByText('gemma-q8.gguf')).toBeInTheDocument();
    expect(screen.getByText('9.0 GB')).toBeInTheDocument();
  });

  it('shows the per-repo context window on the collapsed row, after downloads', async () => {
    await renderPane();
    // No need to expand: the search call already carried the context window.
    expect(
      screen.getByText('google · 1,200,000 downloads · 256K'),
    ).toBeInTheDocument();
    // A repo with no context window keeps the plain org · downloads line.
    expect(screen.getByText('unsloth · 410,000 downloads')).toBeInTheDocument();
  });

  it('collapses an expanded row when the download button is clicked again', async () => {
    await renderPane();
    const row = screen
      .getByText('google/gemma-4-12b-it-GGUF')
      .closest('[data-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Show files' }));
    await flush();
    expect(screen.getByText('gemma-q4.gguf')).toBeInTheDocument();
    fireEvent.click(within(row).getByRole('button', { name: 'Show files' }));
    expect(screen.queryByText('gemma-q4.gguf')).not.toBeInTheDocument();
  });

  it('shows an empty-repo note when the lookup finds no GGUF files', async () => {
    await renderPane(() => {}, { list_hf_repo_ggufs: [] });
    const row = screen
      .getByText('google/gemma-4-12b-it-GGUF')
      .closest('[data-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Show files' }));
    await flush();
    expect(screen.getByText('No GGUF files in this repo.')).toBeInTheDocument();
  });

  it('treats a non-array gguf payload as an empty file list', async () => {
    await renderPane(() => {}, { list_hf_repo_ggufs: 'nope' });
    const row = screen
      .getByText('google/gemma-4-12b-it-GGUF')
      .closest('[data-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Show files' }));
    await flush();
    expect(screen.getByText('No GGUF files in this repo.')).toBeInTheDocument();
  });

  it('surfaces a lookup failure as an inline error', async () => {
    await renderPane(() => {}, {
      list_hf_repo_ggufs: new Reject('repo unavailable'),
    });
    const row = screen
      .getByText('google/gemma-4-12b-it-GGUF')
      .closest('[data-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Show files' }));
    await flush();
    expect(screen.getByText(/repo unavailable/)).toBeInTheDocument();
  });

  it('downloads a chosen quant, progresses, and on ready lifts config and collapses', async () => {
    const onSaved = vi.fn();
    await renderPane(onSaved);
    const row = screen
      .getByText('google/gemma-4-12b-it-GGUF')
      .closest('[data-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Show files' }));
    await flush();
    const downloadButtons = screen.getAllByRole('button', {
      name: 'Download',
    });
    fireEvent.click(downloadButtons[1]);
    await flush();
    expect(invokeMock).toHaveBeenCalledWith(
      'download_repo_model',
      expect.objectContaining({
        repo: 'google/gemma-4-12b-it-GGUF',
        file: 'gemma-q8.gguf',
      }),
    );
    act(() => {
      lastChannel?.simulateMessage({
        type: 'Started',
        data: {
          file: 'gemma-q8.gguf',
          total_bytes: 9_000_000_000,
          resumed_from: 0,
        },
      });
    });
    expect(screen.getByTestId('download-figures')).toBeInTheDocument();
    act(() => {
      lastChannel?.simulateMessage({ type: 'AllDone' });
    });
    await flush();
    expect(onSaved).toHaveBeenCalledWith(CONFIG_AFTER_INSTALL);
    await waitFor(() =>
      expect(screen.queryByText('gemma-q4.gguf')).not.toBeInTheDocument(),
    );
  });

  it('leaves the lift to a later resync when get_config fails post-download', async () => {
    const onSaved = vi.fn();
    await renderPane(onSaved, {
      get_config: new Reject(new Error('config read failed')),
    });
    const row = screen
      .getByText('google/gemma-4-12b-it-GGUF')
      .closest('[data-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Show files' }));
    await flush();
    fireEvent.click(screen.getAllByRole('button', { name: 'Download' })[0]);
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
      .getByText('google/gemma-4-12b-it-GGUF')
      .closest('[data-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Show files' }));
    await flush();
    fireEvent.click(screen.getAllByRole('button', { name: 'Download' })[0]);
    await flush();
    expect(screen.getByTestId('download-figures')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await flush();
    expect(invokeMock).toHaveBeenCalledWith('cancel_model_download');
  });

  it('retries after a failure and offers a path back to the quant list', async () => {
    await renderPane();
    const row = screen
      .getByText('google/gemma-4-12b-it-GGUF')
      .closest('[data-row]') as HTMLElement;
    fireEvent.click(within(row).getByRole('button', { name: 'Show files' }));
    await flush();
    fireEvent.click(screen.getAllByRole('button', { name: 'Download' })[0]);
    await flush();
    act(() => {
      lastChannel?.simulateMessage({
        type: 'Failed',
        data: { kind: 'other', message: 'connection dropped' },
      });
    });
    expect(screen.getByText('connection dropped')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await flush();
    const repoDownloads = invokeMock.mock.calls.filter(
      (c: unknown[]) => c[0] === 'download_repo_model',
    );
    expect(repoDownloads).toHaveLength(2);
    act(() => {
      lastChannel?.simulateMessage({
        type: 'Failed',
        data: { kind: 'other', message: 'again' },
      });
    });
    fireEvent.click(
      screen.getByRole('button', { name: 'Choose a different model' }),
    );
    expect(screen.getByText('gemma-q4.gguf')).toBeInTheDocument();
  });

  it('shows a loading hint while the search is in flight', async () => {
    let resolveSearch!: (value: HfModelSummary[]) => void;
    const pending = new Promise<HfModelSummary[]>((res) => {
      resolveSearch = res;
    });
    mockCommands(discoverResponses({ search_hf_models: pending }));
    render(<BrowseAllPane onSaved={() => {}} />);
    await flush();
    expect(screen.getByText('Searching…')).toBeInTheDocument();
    await act(async () => {
      resolveSearch(RESULTS);
      await Promise.resolve();
    });
    await waitFor(() =>
      expect(screen.queryByText('Searching…')).not.toBeInTheDocument(),
    );
  });

  it('shows a no-results message when the search returns nothing', async () => {
    await renderPane(() => {}, { search_hf_models: [] });
    expect(screen.getByText('No models found.')).toBeInTheDocument();
    expect(screen.getByText(/chat models/)).toHaveTextContent('0 chat models');
  });

  it('offers Load more on a full page and pages to the next batch', async () => {
    vi.useFakeTimers();
    const full = (n: number): HfModelSummary[] =>
      Array.from({ length: n }, (_, i) => ({
        id: `org/repo-${i}-GGUF`,
        downloads: n - i,
        gated: false,
      }));
    mockCommands(discoverResponses({ search_hf_models: full(HF_PAGE_SIZE) }));
    render(<BrowseAllPane onSaved={() => {}} />);
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    const loadMore = screen.getByRole('button', { name: 'Load more' });
    invokeMock.mockClear();
    mockCommands(
      discoverResponses({ search_hf_models: full(HF_PAGE_SIZE + 5) }),
    );
    fireEvent.click(loadMore);
    await act(async () => {
      vi.advanceTimersByTime(HF_SEARCH_DEBOUNCE_MS);
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('search_hf_models', {
      query: '',
      limit: HF_PAGE_SIZE * 2,
    });
  });

  it('hides Load more when the page is not full', async () => {
    await renderPane();
    expect(
      screen.queryByRole('button', { name: 'Load more' }),
    ).not.toBeInTheDocument();
  });
});
