/**
 * Smoke + interaction tests for the four Settings tabs.
 *
 * Each tab's body is mostly declarative `SaveField` markup whose behavior
 * is unit-tested in `components.test`, `SaveField.test`, and
 * `useDebouncedSave.test`. These tests exercise the tab-level wiring:
 * sections render, fields show up, helper tooltips have the right copy,
 * and the per-tab interactive affordances (About's icon-link buttons,
 * Reveal/Refresh/Reset) call the right Tauri commands.
 */

import {
  fireEvent,
  render,
  screen,
  waitFor,
  act,
} from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import { ModelTab } from './ModelTab';
import { DisplayTab } from './DisplayTab';
import { SearchTab } from './SearchTab';
import { AboutTab } from './AboutTab';
import type { RawAppConfig } from '../types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const CONFIG: RawAppConfig = {
  inference: { ollama_url: 'http://127.0.0.1:11434' },
  prompt: { system: 'hello' },
  window: {
    overlay_width: 600,
    max_chat_height: 648,
    max_images: 3,
  },
  quote: {
    max_display_lines: 4,
    max_display_chars: 300,
    max_context_length: 4096,
  },
  search: {
    searxng_url: 'http://127.0.0.1:25017',
    reader_url: 'http://127.0.0.1:25018',
    max_iterations: 3,
    top_k_urls: 10,
    searxng_max_results: 10,
    search_timeout_s: 20,
    reader_per_url_timeout_s: 10,
    reader_batch_timeout_s: 30,
    judge_timeout_s: 30,
    router_timeout_s: 45,
  },
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(CONFIG);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('ModelTab', () => {
  it('renders Ollama and Prompt sections with the expected labels', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Ollama')).toBeInTheDocument();
    expect(screen.getByText('Prompt')).toBeInTheDocument();
    expect(screen.getByText('Ollama URL')).toBeInTheDocument();
    expect(screen.getByText('System prompt')).toBeInTheDocument();
  });

  it('renders the live char counter for the prompt textarea', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText(/5 \/ 8000/)).toBeInTheDocument();
  });
});

describe('DisplayTab', () => {
  it('renders Window and Input sections', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Window')).toBeInTheDocument();
    expect(screen.getByText('Input')).toBeInTheDocument();
    expect(screen.getByText('Overlay width')).toBeInTheDocument();
    expect(screen.getByText('Max display lines')).toBeInTheDocument();
  });
});

describe('SearchTab', () => {
  it('renders Services, Pipeline, and Timeouts sections', () => {
    render(<SearchTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Services')).toBeInTheDocument();
    expect(screen.getByText('Pipeline')).toBeInTheDocument();
    expect(screen.getByText('Timeouts')).toBeInTheDocument();
    expect(screen.getByText('SearXNG URL')).toBeInTheDocument();
    expect(screen.getByText('Router timeout')).toBeInTheDocument();
  });
});

describe('AboutTab', () => {
  function renderAbout() {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return true;
      return CONFIG;
    });
    return render(<AboutTab onSaved={() => {}} onReload={async () => {}} />);
  }

  it('renders the centered hero with title, version, and tagline', async () => {
    renderAbout();
    expect(screen.getByText('Thuki')).toBeInTheDocument();
    expect(screen.getByText(/A floating, local-first AI/)).toBeInTheDocument();
    expect(
      screen.getByText(/No cloud\. No clutter\. Just answers\./),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getAllByText(/Granted/).length).toBeGreaterThan(0),
    );
  });

  it('version button links to the stable release tag when no SHA is set', async () => {
    renderAbout();
    await waitFor(() => screen.getByText(/v\d/));
    fireEvent.click(
      screen.getByRole('button', { name: /release notes on GitHub/ }),
    );
    expect(invokeMock).toHaveBeenCalledWith(
      'open_url',
      expect.objectContaining({
        url: expect.stringContaining('/releases/tag/v'),
      }),
    );
  });

  it('version button links to the nightly release and shows build metadata when VITE_GIT_COMMIT_SHA is set', async () => {
    vi.stubEnv('VITE_GIT_COMMIT_SHA', 'abc1234def');
    renderAbout();
    await waitFor(() => screen.getByText(/nightly/));
    fireEvent.click(
      screen.getByRole('button', { name: /release notes on GitHub/ }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/releases/tag/nightly',
    });
    vi.unstubAllEnvs();
  });

  it('GitHub icon button opens the repo', async () => {
    renderAbout();
    fireEvent.click(
      screen.getByRole('button', { name: 'View Thuki on GitHub' }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki',
    });
  });

  it('X icon button opens @quiet_node', async () => {
    renderAbout();
    fireEvent.click(
      screen.getByRole('button', { name: /Reach out to Logan on X/ }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://x.com/quiet_node',
    });
  });

  it('Feedback icon button opens GitHub Issues', async () => {
    renderAbout();
    fireEvent.click(screen.getByRole('button', { name: /Open an issue/ }));
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/issues',
    });
  });

  it('Reveal Thuki app data invokes reveal_config_in_finder', async () => {
    renderAbout();
    await waitFor(() => screen.getByText(/Reveal Thuki app data/));
    fireEvent.click(
      screen.getByRole('button', { name: /Reveal Thuki app data/ }),
    );
    expect(invokeMock).toHaveBeenCalledWith('reveal_config_in_finder');
  });

  it('Refresh config.toml invokes the supplied onReload', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return true;
      return CONFIG;
    });
    const onReload = vi.fn(async () => {});
    render(<AboutTab onSaved={() => {}} onReload={onReload} />);
    await waitFor(() => screen.getByText(/Refresh config\.toml/));
    fireEvent.click(
      screen.getByRole('button', { name: /Refresh config\.toml/ }),
    );
    expect(onReload).toHaveBeenCalled();
  });

  it('Reset all opens the confirm dialog and a Cancel keeps the file untouched', async () => {
    renderAbout();
    fireEvent.click(screen.getByRole('button', { name: /Reset all/ }));
    expect(
      screen.getByText(/Reset all settings to defaults/),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByText(/Reset all settings to defaults\?/)).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'reset_config',
      expect.anything(),
    );
  });

  it('Reset all confirm invokes reset_config({ section: null }) and lifts the resolved config', async () => {
    const onSaved = vi.fn();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return true;
      if (cmd === 'reset_config') return CONFIG;
      return CONFIG;
    });
    render(<AboutTab onSaved={onSaved} onReload={async () => {}} />);
    await waitFor(() => screen.getByRole('button', { name: /Reset all/ }));

    fireEvent.click(screen.getByRole('button', { name: /Reset all/ }));
    fireEvent.click(screen.getByRole('button', { name: 'Reset all' }));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('reset_config', { section: null });
    expect(onSaved).toHaveBeenCalledWith(CONFIG);
  });

  it('renders Required pills + System Settings shortcuts when permissions are missing', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return false;
      if (cmd === 'check_screen_recording_permission') return false;
      return CONFIG;
    });
    render(<AboutTab onSaved={() => {}} onReload={async () => {}} />);

    await waitFor(() =>
      expect(screen.getAllByText(/Required/).length).toBeGreaterThan(0),
    );

    const accBtn = screen.getAllByRole('button', {
      name: 'Open System Settings',
    })[0];
    fireEvent.click(accBtn);
    expect(invokeMock).toHaveBeenCalledWith('open_accessibility_settings');

    const screenBtn = screen.getAllByRole('button', {
      name: 'Open System Settings',
    })[1];
    fireEvent.click(screenBtn);
    expect(invokeMock).toHaveBeenCalledWith('open_screen_recording_settings');
  });

  it('window focus event triggers a permission re-probe', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return true;
      return CONFIG;
    });
    render(<AboutTab onSaved={() => {}} onReload={async () => {}} />);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('check_accessibility_permission'),
    );
    invokeMock.mockClear();
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('check_accessibility_permission');
  });

  it('drops the late permission probe result when the component unmounts first', async () => {
    let resolveAcc: ((v: boolean) => void) | undefined;
    let resolveScreen: ((v: boolean) => void) | undefined;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') {
        return new Promise<boolean>((r) => {
          resolveAcc = r;
        });
      }
      if (cmd === 'check_screen_recording_permission') {
        return new Promise<boolean>((r) => {
          resolveScreen = r;
        });
      }
      return CONFIG;
    });

    const { unmount } = render(
      <AboutTab onSaved={() => {}} onReload={async () => {}} />,
    );

    // Tear down before the probe resolves — the post-await `if (mounted)`
    // guard must stop the setPerms call.
    unmount();
    await act(async () => {
      resolveAcc?.(true);
      resolveScreen?.(true);
      await Promise.resolve();
      await Promise.resolve();
    });
    // No assertion needed; the test passes if no React state-update warning
    // is logged.
  });

  it('permission probe failures leave the previous pill state in place', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (
        cmd === 'check_accessibility_permission' ||
        cmd === 'check_screen_recording_permission'
      ) {
        throw new Error('probe failed');
      }
      return CONFIG;
    });
    render(<AboutTab onSaved={() => {}} onReload={async () => {}} />);
    // Just confirm it doesn't crash; default state is "Required".
    await waitFor(() =>
      expect(screen.getAllByText(/Required/).length).toBeGreaterThan(0),
    );
  });
});
