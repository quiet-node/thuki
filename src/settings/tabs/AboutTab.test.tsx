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

  describe('Help shape Thuki card', () => {
    it('renders the email ask with a Share email button', async () => {
      render(<AboutTab {...SAMPLE_PROPS} />);
      await waitFor(() => screen.getByText('Help shape Thuki'));
      expect(screen.getByLabelText('Email address')).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: /share email/i }),
      ).toBeInTheDocument();
    });

    it("opens Logan's X profile when the inline link is clicked", async () => {
      render(<AboutTab {...SAMPLE_PROPS} />);
      await waitFor(() => screen.getByText('Help shape Thuki'));

      fireEvent.click(
        screen.getByRole('button', { name: /open logan's profile on x/i }),
      );
      expect(invokeMock).toHaveBeenCalledWith('open_url', {
        url: 'https://x.com/quiet_node',
      });
    });

    it('rejects an invalid email without calling subscribe_email', async () => {
      render(<AboutTab {...SAMPLE_PROPS} />);
      await waitFor(() => screen.getByText('Help shape Thuki'));

      fireEvent.change(screen.getByLabelText('Email address'), {
        target: { value: 'not-an-email' },
      });
      fireEvent.click(screen.getByRole('button', { name: /share email/i }));

      expect(
        screen.getByText(/enter a valid email address/i),
      ).toBeInTheDocument();
      expect(invokeMock).not.toHaveBeenCalledWith(
        'subscribe_email',
        expect.anything(),
      );
    });

    it('clears the error once the user edits the email', async () => {
      render(<AboutTab {...SAMPLE_PROPS} />);
      await waitFor(() => screen.getByText('Help shape Thuki'));

      fireEvent.click(screen.getByRole('button', { name: /share email/i }));
      expect(
        screen.getByText(/enter a valid email address/i),
      ).toBeInTheDocument();

      fireEvent.change(screen.getByLabelText('Email address'), {
        target: { value: 'a' },
      });
      expect(
        screen.queryByText(/enter a valid email address/i),
      ).not.toBeInTheDocument();
    });

    it('subscribes the trimmed email and shows a thank-you on success', async () => {
      render(<AboutTab {...SAMPLE_PROPS} />);
      await waitFor(() => screen.getByText('Help shape Thuki'));

      fireEvent.change(screen.getByLabelText('Email address'), {
        target: { value: '  founder@thuki.app  ' },
      });
      fireEvent.click(screen.getByRole('button', { name: /share email/i }));

      expect(invokeMock).toHaveBeenCalledWith('subscribe_email', {
        email: 'founder@thuki.app',
      });
      expect(await screen.findByText(/i'll be in touch/i)).toBeInTheDocument();
      expect(screen.getByText('– Logan')).toBeInTheDocument();
    });

    it('shows a sending state while the request is in flight', async () => {
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === 'subscribe_email') {
          return new Promise(() => {});
        }
        return defaultInvoke(cmd);
      });
      render(<AboutTab {...SAMPLE_PROPS} />);
      await waitFor(() => screen.getByText('Help shape Thuki'));

      fireEvent.change(screen.getByLabelText('Email address'), {
        target: { value: 'founder@thuki.app' },
      });
      fireEvent.click(screen.getByRole('button', { name: /share email/i }));

      const button = screen.getByRole('button', { name: /sending/i });
      expect(button).toBeDisabled();
    });

    it('surfaces a retryable error when the send fails', async () => {
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === 'subscribe_email') {
          throw new Error('network');
        }
        return defaultInvoke(cmd);
      });
      render(<AboutTab {...SAMPLE_PROPS} />);
      await waitFor(() => screen.getByText('Help shape Thuki'));

      fireEvent.change(screen.getByLabelText('Email address'), {
        target: { value: 'founder@thuki.app' },
      });
      fireEvent.click(screen.getByRole('button', { name: /share email/i }));

      expect(
        await screen.findByText(/couldn't send right now/i),
      ).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: /share email/i }),
      ).not.toBeDisabled();
    });
  });
});
