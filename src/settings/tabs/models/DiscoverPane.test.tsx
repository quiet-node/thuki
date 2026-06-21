/**
 * Unit tests for the Discover host: the two-pathway tab shell that places the
 * curated Staff-picks accordion as the default front door and the raw Hugging
 * Face browser behind a "Browse all" advanced tab. The child panes have their
 * own suites; here we only test the tab control and which pane it shows.
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

import { DiscoverPane } from './DiscoverPane';
import { clearHfSearchCache } from './useHfSearch';
import { DownloadsProvider } from '../../../contexts/DownloadsContext';
import type { Starter, StarterOption } from '../../../types/starter';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const STARTER: StarterOption = {
  starter: {
    id: 'gemma-4-12b',
    tier: 'balanced',
    family: 'Gemma',
    display_name: 'Gemma 4 12B',
    repo: 'google/gemma',
    revision: 'a'.repeat(40),
    file_name: 'gemma.gguf',
    sha256: 'b'.repeat(64),
    size_bytes: 7_000_000_000,
    quant: 'Q4_0',
    vision: true,
    thinking: false,
    reasoning_always: false,
    mmproj_file: null,
    mmproj_sha256: null,
    mmproj_bytes: 0,
    est_runtime_gb: 9.5,
    license_note: 'Apache 2.0',
    origin: 'Google',
    origin_repo: 'google/gemma',
  } as Starter,
  fit: 'fits',
  installed: false,
  partial_bytes: null,
};

beforeEach(() => {
  invokeMock.mockReset();
  clearHfSearchCache();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === 'get_staff_picks') return [STARTER];
    if (cmd === 'search_hf_models') return [];
    return undefined;
  });
});

function renderHost() {
  return render(<DiscoverPane onSaved={() => {}} />, {
    wrapper: DownloadsProvider,
  });
}

/** Staff picks is showing when its curated use-case sections are on screen. */
function staffPicksVisible(): boolean {
  return screen.queryByTestId('staff-section-label') !== null;
}

/** Browse all is showing when its Hugging Face search box is on screen. */
function browseAllVisible(): boolean {
  return screen.queryByRole('searchbox') !== null;
}

describe('DiscoverPane host', () => {
  it('shows two pathway tabs', () => {
    renderHost();
    expect(
      screen.getByRole('tab', { name: 'Staff picks' }),
    ).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Browse all' })).toBeInTheDocument();
  });

  it('defaults to the curated Staff-picks pathway', async () => {
    renderHost();
    await waitFor(() => expect(staffPicksVisible()).toBe(true));
    expect(browseAllVisible()).toBe(false);
    expect(screen.getByRole('tab', { name: 'Staff picks' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });

  it('switches to the Browse-all pathway on click and back again', async () => {
    renderHost();
    fireEvent.click(screen.getByRole('tab', { name: 'Browse all' }));
    await waitFor(() => expect(browseAllVisible()).toBe(true));
    expect(staffPicksVisible()).toBe(false);
    expect(screen.getByRole('tab', { name: 'Browse all' })).toHaveAttribute(
      'aria-selected',
      'true',
    );

    fireEvent.click(screen.getByRole('tab', { name: 'Staff picks' }));
    await waitFor(() => expect(staffPicksVisible()).toBe(true));
    expect(browseAllVisible()).toBe(false);
  });

  it('moves between tabs with the arrow keys', async () => {
    renderHost();
    const staff = screen.getByRole('tab', { name: 'Staff picks' });
    fireEvent.keyDown(staff, { key: 'ArrowRight' });
    await waitFor(() => expect(browseAllVisible()).toBe(true));
    const browse = screen.getByRole('tab', { name: 'Browse all' });
    fireEvent.keyDown(browse, { key: 'ArrowLeft' });
    await waitFor(() => expect(staffPicksVisible()).toBe(true));
  });

  it('ignores non-arrow keys', () => {
    renderHost();
    const staff = screen.getByRole('tab', { name: 'Staff picks' });
    fireEvent.keyDown(staff, { key: 'Enter' });
    expect(screen.getByRole('tab', { name: 'Staff picks' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });
});

describe('DiscoverPane download persistence', () => {
  type MockChannel = { simulateMessage: (msg: unknown) => void };
  let channel: MockChannel | null = null;

  async function flush() {
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  beforeEach(() => {
    invokeMock.mockReset();
    clearHfSearchCache();
    channel = null;
    invokeMock.mockImplementation(
      async (cmd: string, args?: Record<string, unknown>) => {
        if (args && 'onEvent' in args) {
          channel = args.onEvent as unknown as MockChannel;
        }
        if (cmd === 'get_staff_picks') return [STARTER];
        if (cmd === 'search_hf_models') {
          return [
            {
              id: 'google/gemma-4-12b-it-GGUF',
              downloads: 1_200_000,
              gated: false,
            },
          ];
        }
        if (cmd === 'list_hf_repo_ggufs') {
          return [
            {
              file: 'gemma-q4.gguf',
              size_bytes: 5_000_000_000,
              fit: 'tight',
              sha256: 'a'.repeat(64),
              partial_bytes: null,
              installed: false,
            },
          ];
        }
        return undefined;
      },
    );
  });

  // The bug: starting a download in Staff picks, switching to Browse all, then
  // back drops the live progress (the pane owned a component-local download
  // machine that died on unmount while the single-slot backend download kept
  // running). The shared app-root machine must keep the progress alive.
  it('keeps a live Staff-picks download visible across a Browse-all round trip', async () => {
    render(<DiscoverPane onSaved={() => {}} />, { wrapper: DownloadsProvider });
    await waitFor(() => expect(staffPicksVisible()).toBe(true));

    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await flush();
    act(() =>
      channel?.simulateMessage({
        type: 'Started',
        data: {
          file: 'gemma.gguf',
          total_bytes: 7_000_000_000,
          resumed_from: 0,
        },
      }),
    );
    act(() =>
      channel?.simulateMessage({
        type: 'Progress',
        data: {
          file: 'gemma.gguf',
          bytes: 2_520_000_000,
          total_bytes: 7_000_000_000,
        },
      }),
    );
    expect(screen.getByTestId('download-figures')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('tab', { name: 'Browse all' }));
    await waitFor(() => expect(browseAllVisible()).toBe(true));
    fireEvent.click(screen.getByRole('tab', { name: 'Staff picks' }));
    await waitFor(() => expect(staffPicksVisible()).toBe(true));

    // Live progress is still on screen: no Paused row, no "already in progress".
    expect(screen.getByTestId('download-figures')).toBeInTheDocument();
    expect(screen.queryByText(/^Paused · /)).not.toBeInTheDocument();
    expect(
      screen.queryByText('a download is already in progress'),
    ).not.toBeInTheDocument();
  });

  // The symmetric case for the advanced pathway: a Browse-all repo download must
  // also survive a Staff-picks round trip, re-binding to the owning row (which
  // re-expands) instead of resetting to a collapsed, idle row.
  it('keeps a live Browse-all download visible across a Staff-picks round trip', async () => {
    render(<DiscoverPane onSaved={() => {}} />, { wrapper: DownloadsProvider });
    await waitFor(() => expect(staffPicksVisible()).toBe(true));

    fireEvent.click(screen.getByRole('tab', { name: 'Browse all' }));
    await waitFor(() => expect(browseAllVisible()).toBe(true));
    await waitFor(() =>
      expect(
        screen.getByText('google/gemma-4-12b-it-GGUF'),
      ).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Show files' }));
    await waitFor(() =>
      expect(screen.getByText('gemma-q4.gguf')).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await flush();
    act(() =>
      channel?.simulateMessage({
        type: 'Started',
        data: {
          file: 'gemma-q4.gguf',
          total_bytes: 5_000_000_000,
          resumed_from: 0,
        },
      }),
    );
    act(() =>
      channel?.simulateMessage({
        type: 'Progress',
        data: {
          file: 'gemma-q4.gguf',
          bytes: 1_500_000_000,
          total_bytes: 5_000_000_000,
        },
      }),
    );
    expect(screen.getByTestId('download-figures')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('tab', { name: 'Staff picks' }));
    await waitFor(() => expect(staffPicksVisible()).toBe(true));
    fireEvent.click(screen.getByRole('tab', { name: 'Browse all' }));
    await waitFor(() => expect(browseAllVisible()).toBe(true));

    await waitFor(() =>
      expect(screen.getByTestId('download-figures')).toBeInTheDocument(),
    );
  });
});
