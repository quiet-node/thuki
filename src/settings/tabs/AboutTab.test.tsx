import {
  render,
  screen,
  fireEvent,
  act,
  waitFor,
} from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
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
  it('renders the Updates section with Current version and Last checked rows', async () => {
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() => screen.getByText('Current version'));
    expect(screen.getByText('Last checked')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /check now/i }),
    ).toBeInTheDocument();
  });

  it('shows Never for last checked when last_check_at_unix is null', async () => {
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() => expect(screen.getByText('Never')).toBeInTheDocument());
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
      expect(screen.getByText('2 minutes ago')).toBeInTheDocument(),
    );
  });

  it('calls check_for_update when Check now clicked', async () => {
    invokeMock.mockImplementation(async (cmd: string) => defaultInvoke(cmd));
    render(<AboutTab {...SAMPLE_PROPS} />);
    await waitFor(() => screen.getByRole('button', { name: /check now/i }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /check now/i }));
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('check_for_update');
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
