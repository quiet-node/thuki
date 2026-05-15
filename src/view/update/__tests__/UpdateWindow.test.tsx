import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';

import { UpdateWindow } from '../UpdateWindow';
import { __mockWindow } from '../../../testUtils/mocks/tauri-window';
import type { UpdaterState } from '../../../hooks/useUpdater';

// The component is unit-tested against a mocked useUpdater so every render
// branch (incl. the no-notes_url path that the real hook's notes_url
// backfill makes unreachable) is deterministically exercisable.
const skip = vi.fn(async () => {});
const snoozeChat = vi.fn(async () => {});
const snoozeSettings = vi.fn(async () => {});
const install = vi.fn(async () => {});
const installAndQuit = vi.fn(async () => {});
const openWindow = vi.fn(async () => {});
const checkNow = vi.fn(async () => {});

let mockState: UpdaterState;

vi.mock('../../../hooks/useUpdater', () => ({
  useUpdater: () => ({
    state: mockState,
    checkNow,
    install,
    installAndQuit,
    openWindow,
    skip,
    snoozeChat,
    snoozeSettings,
  }),
}));

const BASE: UpdaterState = {
  last_check_at_unix: null,
  update: null,
  settings_snoozed_until: null,
  chat_snoozed_until: null,
  skipped_versions: [],
};

function withUpdate(
  over: Partial<NonNullable<UpdaterState['update']>>,
): UpdaterState {
  return {
    ...BASE,
    update: {
      version: '0.11.0',
      notes_url: null,
      body: null,
      date: null,
      ...over,
    },
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockState = BASE;
});

describe('UpdateWindow', () => {
  it('renders the empty state when no update is available', () => {
    mockState = BASE;
    render(<UpdateWindow />);
    expect(screen.getByTestId('update-empty')).toHaveTextContent(
      'Thuki is up to date.',
    );
  });

  it('renders version, release date, and markdown body', () => {
    mockState = withUpdate({
      body: '## Fixed\n\n- a crash',
      date: '2026-05-15T00:00:00Z',
    });
    render(<UpdateWindow />);
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(
      'Thuki 0.11.0',
    );
    expect(screen.getByText('Update Available')).toBeInTheDocument();
    expect(screen.getByText('Released 2026-05-15')).toBeInTheDocument();
    const notes = screen.getByTestId('update-notes');
    expect(notes).toHaveTextContent('Fixed');
    expect(notes).toHaveTextContent('a crash');
  });

  it('renders the GitHub-link fallback when body is empty but notes_url is set, and hides the date when absent', () => {
    mockState = withUpdate({
      body: null,
      notes_url: 'https://example.com/notes',
      date: null,
    });
    render(<UpdateWindow />);
    expect(screen.queryByText(/^Released/)).not.toBeInTheDocument();
    const link = screen.getByRole('link', { name: /view them on github/i });
    expect(link).toHaveAttribute('href', 'https://example.com/notes');
  });

  it('renders the no-notes message when body is whitespace and notes_url is null, and ignores an unparseable date', () => {
    mockState = withUpdate({
      body: '   ',
      notes_url: null,
      date: 'not-a-date',
    });
    render(<UpdateWindow />);
    expect(screen.queryByText(/^Released/)).not.toBeInTheDocument();
    expect(screen.getByTestId('update-notes')).toHaveTextContent(
      'No release notes are available for this version.',
    );
  });

  it('Skip This Version skips then hides the window', async () => {
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    fireEvent.click(screen.getByRole('button', { name: /skip this version/i }));
    expect(skip).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(__mockWindow.hide).toHaveBeenCalled());
  });

  it('logs a rejected action and still hides the window', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    skip.mockRejectedValueOnce(new Error('boom'));
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    fireEvent.click(screen.getByRole('button', { name: /skip this version/i }));
    await waitFor(() =>
      expect(errorSpy).toHaveBeenCalledWith(
        'update window action failed',
        expect.any(Error),
      ),
    );
    await waitFor(() => expect(__mockWindow.hide).toHaveBeenCalled());
    errorSpy.mockRestore();
  });

  it('Remind Me Later snoozes both surfaces for 24h then hides', async () => {
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    fireEvent.click(screen.getByRole('button', { name: /remind me later/i }));
    expect(snoozeChat).toHaveBeenCalledWith(24);
    expect(snoozeSettings).toHaveBeenCalledWith(24);
    await waitFor(() => expect(__mockWindow.hide).toHaveBeenCalled());
  });

  it('Install & Quit triggers installAndQuit', () => {
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    fireEvent.click(screen.getByRole('button', { name: /install & quit/i }));
    expect(installAndQuit).toHaveBeenCalledTimes(1);
  });

  it('Install & Restart triggers install', () => {
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    fireEvent.click(screen.getByRole('button', { name: /install & restart/i }));
    expect(install).toHaveBeenCalledTimes(1);
  });

  it('the WindowControls close button hides the window', async () => {
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    fireEvent.click(screen.getByRole('button', { name: /close window/i }));
    await waitFor(() => expect(__mockWindow.hide).toHaveBeenCalled());
  });

  it('drags the window from a non-interactive, non-text surface', () => {
    mockState = withUpdate({ body: 'x' });
    const { container } = render(<UpdateWindow />);
    fireEvent.mouseDown(container.firstChild as Element, { button: 0 });
    expect(__mockWindow.startDragging).toHaveBeenCalled();
  });

  it('does not drag on a non-primary mouse button', () => {
    mockState = withUpdate({ body: 'x' });
    const { container } = render(<UpdateWindow />);
    fireEvent.mouseDown(container.firstChild as Element, { button: 1 });
    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });

  it('does not drag when the press lands on an interactive element', () => {
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    fireEvent.mouseDown(
      screen.getByRole('button', { name: /install & restart/i }),
      { button: 0 },
    );
    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });

  it('does not drag when the press lands on a text-bearing leaf', () => {
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    fireEvent.mouseDown(screen.getByText('Update Available'), {
      button: 0,
    });
    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });
});
