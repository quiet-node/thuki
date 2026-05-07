import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
} from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { AboutTab } from './AboutTab';

const invokeMock = invoke as unknown as ReturnType<
  typeof import('vitest').vi.fn
>;

const SAMPLE_PROPS = {
  onSaved: () => {},
  onReload: async () => {},
};

function defaultInvoke(cmd: string): unknown {
  switch (cmd) {
    case 'check_accessibility_permission':
      return true;
    case 'check_screen_recording_permission':
      return true;
    case 'get_updater_state':
      return {
        last_check_at_unix: null,
        update: null,
        settings_snoozed_until: null,
        chat_snoozed_until: null,
      };
    default:
      return undefined;
  }
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => defaultInvoke(cmd));
});

describe('AboutTab', () => {
  it('renders the Updates hero showing up-to-date status and a check button', async () => {
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() =>
      expect(screen.getByText('Thuki is up to date')).toBeInTheDocument(),
    );
    expect(
      screen.getByRole('button', { name: /check for updates/i }),
    ).toBeInTheDocument();
  });

  it('shows "Never checked for updates" when last_check_at_unix is null', async () => {
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() =>
      expect(screen.getByText('Never checked for updates')).toBeInTheDocument(),
    );
  });

  it('shows relative time when last_check_at_unix is set', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_updater_state') {
        return {
          last_check_at_unix: Math.floor(Date.now() / 1000) - 120,
          update: null,
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        };
      }
      return defaultInvoke(cmd);
    });
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() =>
      expect(
        screen.getByText('Last checked 2 minutes ago'),
      ).toBeInTheDocument(),
    );
  });

  it('renders the available state when an update is pending', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_updater_state') {
        return {
          last_check_at_unix: Math.floor(Date.now() / 1000),
          update: { version: '0.9.0', notes_url: null },
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        };
      }
      return defaultInvoke(cmd);
    });
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() =>
      expect(screen.getByText('Thuki 0.9.0 is ready')).toBeInTheDocument(),
    );
  });

  it('calls check_for_update when Check for updates is clicked', async () => {
    invokeMock.mockImplementation(async (cmd: string) => defaultInvoke(cmd));
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() =>
      screen.getByRole('button', { name: /check for updates/i }),
    );
    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /check for updates/i }),
      );
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('check_for_update');
  });

  it('disables the button while checking and re-enables after the animation hold', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === 'check_for_update') {
          return {
            last_check_at_unix: Math.floor(Date.now() / 1000),
            update: null,
            settings_snoozed_until: null,
            chat_snoozed_until: null,
          };
        }
        return defaultInvoke(cmd);
      });
      render(<AboutTab {...SAMPLE_PROPS} />);
      await waitFor(() =>
        screen.getByRole('button', { name: /check for updates/i }),
      );
      const btn = screen.getByRole('button', { name: /check for updates/i });
      await act(async () => {
        fireEvent.click(btn);
        await Promise.resolve();
      });
      expect(btn).toHaveAttribute('data-checking', 'true');
      expect(btn).toBeDisabled();

      // A second click while checking is a no-op.
      const callsBefore = invokeMock.mock.calls.filter(
        (c: unknown[]) => c[0] === 'check_for_update',
      ).length;
      await act(async () => {
        fireEvent.click(btn);
        await Promise.resolve();
      });
      const callsAfter = invokeMock.mock.calls.filter(
        (c: unknown[]) => c[0] === 'check_for_update',
      ).length;
      expect(callsAfter).toBe(callsBefore);

      // Advance past the animation hold so the timer callback resets state.
      await act(async () => {
        vi.advanceTimersByTime(1200);
        await Promise.resolve();
      });
      expect(btn).toHaveAttribute('data-checking', 'false');
      expect(btn).not.toBeDisabled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('clears the pending animation timer on unmount', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === 'check_for_update') {
          return {
            last_check_at_unix: Math.floor(Date.now() / 1000),
            update: null,
            settings_snoozed_until: null,
            chat_snoozed_until: null,
          };
        }
        return defaultInvoke(cmd);
      });
      const { unmount } = render(<AboutTab {...SAMPLE_PROPS} />);
      await waitFor(() =>
        screen.getByRole('button', { name: /check for updates/i }),
      );
      await act(async () => {
        fireEvent.click(
          screen.getByRole('button', { name: /check for updates/i }),
        );
        await Promise.resolve();
      });
      // Unmount while the post-check timer is still pending. The cleanup
      // effect must clear it; otherwise vitest fake timers would still hold
      // a queued callback on unmount.
      unmount();
      await act(async () => {
        vi.advanceTimersByTime(2000);
        await Promise.resolve();
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('renders the Permissions section', async () => {
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() => screen.getByText('Accessibility'));
    expect(screen.getByText('Screen Recording')).toBeInTheDocument();
  });

  it('renders the File section with Reveal and Refresh buttons', async () => {
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() =>
      screen.getByRole('button', { name: /reveal thuki app data/i }),
    );
    expect(
      screen.getByRole('button', { name: /refresh config\.toml/i }),
    ).toBeInTheDocument();
  });

  it('shows Reset all confirm dialog when Reset button clicked', async () => {
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() =>
      screen.getByRole('button', { name: /reset all to defaults/i }),
    );
    fireEvent.click(
      screen.getByRole('button', { name: /reset all to defaults/i }),
    );
    expect(
      screen.getByText(/reset all settings to defaults/i),
    ).toBeInTheDocument();
  });

  it('cancels reset when Cancel is clicked in dialog', async () => {
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() =>
      screen.getByRole('button', { name: /reset all to defaults/i }),
    );
    fireEvent.click(
      screen.getByRole('button', { name: /reset all to defaults/i }),
    );
    fireEvent.click(screen.getByRole('button', { name: /cancel/i }));
    expect(
      screen.queryByText(/your entire config\.toml/i),
    ).not.toBeInTheDocument();
  });
});
