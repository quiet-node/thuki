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
const openWindow = vi.fn(async () => {});
const checkNow = vi.fn(async () => {});

// `@tauri-apps/api/app` is not test-aliased (only core/event/window are),
// so the current-version lookup is mocked here directly.
const getVersion = vi.fn<() => Promise<string>>(async () => '0.10.0');
vi.mock('@tauri-apps/api/app', () => ({
  getVersion: () => getVersion(),
}));

let mockState: UpdaterState;

vi.mock('../../../hooks/useUpdater', () => ({
  useUpdater: () => ({
    state: mockState,
    checkNow,
    install,
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
  getVersion.mockResolvedValue('0.10.0');
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

  it('renders the title, the available + current version subline, and the markdown body', async () => {
    mockState = withUpdate({ body: '## Fixed\n\n- a crash' });
    render(<UpdateWindow />);
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(
      'A new version of Thuki is available!',
    );
    // Subline gains the "· you have X" clause once getVersion resolves.
    await screen.findByText('Version 0.11.0 · you have 0.10.0');
    const notes = screen.getByTestId('update-notes');
    expect(notes).toHaveTextContent('Fixed');
    expect(notes).toHaveTextContent('a crash');
  });

  it('omits the "you have" clause when the current version lookup fails', async () => {
    getVersion.mockRejectedValueOnce(new Error('no app version'));
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    await waitFor(() => expect(getVersion).toHaveBeenCalled());
    expect(screen.getByText('Version 0.11.0')).toBeInTheDocument();
    expect(screen.queryByText(/you have/i)).not.toBeInTheDocument();
  });

  it('renders the GitHub-link fallback when body is empty but notes_url is set', () => {
    mockState = withUpdate({
      body: null,
      notes_url: 'https://example.com/notes',
    });
    render(<UpdateWindow />);
    const link = screen.getByRole('link', { name: /view them on github/i });
    expect(link).toHaveAttribute('href', 'https://example.com/notes');
  });

  it('renders the no-notes message when body is whitespace and notes_url is null', () => {
    mockState = withUpdate({ body: '   ', notes_url: null });
    render(<UpdateWindow />);
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

  it('Install Update triggers install', () => {
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    fireEvent.click(screen.getByRole('button', { name: /install update/i }));
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
      screen.getByRole('button', { name: /install update/i }),
      { button: 0 },
    );
    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });

  it('does not drag when the press lands on a text-bearing leaf', () => {
    mockState = withUpdate({ body: 'x' });
    render(<UpdateWindow />);
    fireEvent.mouseDown(
      screen.getByText('A new version of Thuki is available!'),
      { button: 0 },
    );
    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });
});
