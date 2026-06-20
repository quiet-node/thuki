/**
 * Unit tests for the Discover host: the two-pathway tab shell that places the
 * curated Staff-picks accordion as the default front door and the raw Hugging
 * Face browser behind a "Browse all" advanced tab. The child panes have their
 * own suites; here we only test the tab control and which pane it shows.
 */

import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import { DiscoverPane } from './DiscoverPane';
import { clearHfSearchCache } from './useHfSearch';
import type { Starter, StarterOption } from '../../../types/starter';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const STARTER: StarterOption = {
  starter: {
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
    if (cmd === 'get_starter_options') return [STARTER];
    if (cmd === 'search_hf_models') return [];
    return undefined;
  });
});

function renderHost() {
  return render(<DiscoverPane onSaved={() => {}} />);
}

/** Staff picks is showing when its curated hint is on screen. */
function staffPicksVisible(): boolean {
  return screen.queryByText(/Hand-picked by Thuki/) !== null;
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
