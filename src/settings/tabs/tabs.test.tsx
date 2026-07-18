/**
 * Smoke + interaction tests for the Settings tabs.
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
import { clearEventHandlers, emit, listen } from '../../testUtils/mocks/tauri';

import { ModelTab } from './ModelTab';
import { DownloadsProvider } from '../../contexts/DownloadsContext';
import { DisplayTab } from './DisplayTab';

import { AboutTab } from './AboutTab';
import {
  BehaviorTab,
  FREE_SUCCESS_HOLD_MS,
  HISTORY_CLEARED_EVENT,
  HISTORY_RETENTION_ZERO_ERROR,
} from './BehaviorTab';
import type { RawAppConfig } from '../types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const CONFIG: RawAppConfig = {
  inference: {
    active_provider: 'ollama',
    keep_warm_inactivity_minutes: 0,
    num_ctx: 16384,
    providers: [
      {
        id: 'builtin',
        kind: 'builtin',
        label: 'Built-in',
        base_url: '',
        model: '',
        vision: false,
      },
      {
        id: 'ollama',
        kind: 'ollama',
        label: 'Ollama',
        base_url: 'http://127.0.0.1:11434',
        model: '',
        vision: false,
      },
    ],
  },
  prompt: { system: 'hello' },
  window: {
    overlay_width: 600,
    max_chat_height: 648,
    max_images: 3,
    text_base_px: 15,
    text_line_height: 1.5,
    text_letter_spacing_px: 0,
    text_font_weight: 500,
  },
  quote: {
    max_display_lines: 4,
    max_display_chars: 300,
    max_context_length: 4096,
  },
  behavior: {
    auto_replace: false,
    auto_close: false,
    auto_search: true,
    search_notice_acknowledged: false,
    auto_save_conversations: true,
    history_retention_days: -1,
    auto_save_notice_acknowledged: false,
    dismissed_memory_fit_models: [],
  },
  debug: {
    trace_enabled: false,
    trace_retention_days: 7,
  },
};

/** Full engine lifecycle payload for `engine:status` emissions. */
function engineStatus(
  state: 'stopped' | 'starting' | 'loaded' | 'stopping' | 'failed',
) {
  return { state, model_path: '', port: null, error: null };
}

beforeEach(() => {
  // Default to the enabled branch so the openai-card tests render the gated
  // UI; the disabled-state test flips this within its own body. ModelTab reads
  // the flag from `import.meta.env` at render, so stubbing it here is enough.
  vi.stubEnv('VITE_ENABLE_OPENAI_PROVIDER', 'true');
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'get_loaded_model') return Promise.resolve(null);
    if (cmd === 'get_engine_status') {
      return Promise.resolve(engineStatus('stopped'));
    }
    if (cmd === 'get_model_picker_state') {
      return Promise.resolve({ active: null, all: [], ollamaReachable: false });
    }
    if (cmd === 'get_updater_state') {
      return Promise.resolve({
        last_check_at_unix: null,
        update: null,
        settings_snoozed_until: null,
        chat_snoozed_until: null,
      });
    }
    // Footprint probes: shape `{ count, bytes }`, not the full config blob.
    // Default non-zero so Free chats / Free traces stay enabled unless a test
    // stubs empty or failing stats.
    if (cmd === 'history_stats') {
      return Promise.resolve({ count: 1, bytes: 100 });
    }
    if (cmd === 'traces_stats') {
      return Promise.resolve({ count: 1, bytes: 100 });
    }
    // Shortening retention probes prune impact; default >0 so confirm-dialog
    // tests still open unless they stub zero or a throw.
    if (cmd === 'history_retention_prune_count') {
      return Promise.resolve(1);
    }
    return Promise.resolve(CONFIG);
  });
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  clearEventHandlers();
});

async function renderModelTab() {
  const view = render(
    <ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />,
    { wrapper: DownloadsProvider },
  );
  await act(async () => {
    await Promise.resolve();
  });
  return view;
}

describe('ModelTab (router)', () => {
  it('defaults to the Providers view and renders the active provider hero', async () => {
    await renderModelTab();
    expect(screen.getByRole('tab', { name: 'Providers' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.getByText('Active provider')).toBeInTheDocument();
  });

  it('switches to the Discover view', async () => {
    await renderModelTab();
    await act(async () => {
      fireEvent.click(screen.getByRole('tab', { name: 'Discover' }));
      await Promise.resolve();
    });
    expect(screen.getByRole('tab', { name: 'Discover' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    // The Providers hero is unmounted while Discover is showing.
    expect(screen.queryByText('Active provider')).toBeNull();
  });

  it('switches to the Library view', async () => {
    await renderModelTab();
    await act(async () => {
      fireEvent.click(screen.getByRole('tab', { name: 'Library' }));
      await Promise.resolve();
    });
    expect(screen.getByRole('tab', { name: 'Library' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.queryByText('Active provider')).toBeNull();
  });

  it('navigates to Discover from the built-in no-model hint', async () => {
    const builtinActive: RawAppConfig = {
      ...CONFIG,
      inference: { ...CONFIG.inference, active_provider: 'builtin' },
    };
    render(
      <ModelTab config={builtinActive} resyncToken={0} onSaved={() => {}} />,
      { wrapper: DownloadsProvider },
    );
    await act(async () => {
      await Promise.resolve();
    });
    fireEvent.click(
      await screen.findByRole('button', {
        name: /Download a model in Discover/,
      }),
    );
    // The onAddModel callback flips the view: the Providers hero unmounts.
    await waitFor(() =>
      expect(screen.queryByText('Active provider')).toBeNull(),
    );
  });
});

describe('DisplayTab', () => {
  it('renders Text, Window, and Input sections', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Text')).toBeInTheDocument();
    expect(screen.getByText('Window')).toBeInTheDocument();
    expect(screen.getByText('Input')).toBeInTheDocument();
    expect(screen.getByText('Text size')).toBeInTheDocument();
    expect(screen.getByText('Line height')).toBeInTheDocument();
    expect(screen.getByText('Letter spacing')).toBeInTheDocument();
    expect(screen.getByText('Font weight')).toBeInTheDocument();
    expect(screen.getByText('Overlay width')).toBeInTheDocument();
    expect(screen.getByText('Max display lines')).toBeInTheDocument();
  });

  it('exposes a text-size slider bound to the 11..22 px range', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', { name: 'Text size' });
    expect(slider).toHaveAttribute('min', '11');
    expect(slider).toHaveAttribute('max', '22');
    expect(slider).toHaveAttribute('step', '0.5');
    expect(slider).toHaveValue(String(CONFIG.window.text_base_px));
  });

  it('exposes a line-height slider bound to the 1..2.5 range', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', { name: 'Line height' });
    expect(slider).toHaveAttribute('min', '1');
    expect(slider).toHaveAttribute('max', '2.5');
    expect(slider).toHaveAttribute('step', '0.05');
  });

  it('exposes a letter-spacing slider bound to the -0.5..2 px range', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', { name: 'Letter spacing' });
    expect(slider).toHaveAttribute('min', '-0.5');
    expect(slider).toHaveAttribute('max', '2');
    expect(slider).toHaveAttribute('step', '0.05');
  });

  it('exposes a font-weight slider snapping to the four loaded Nunito weights', () => {
    render(<DisplayTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', { name: 'Font weight' });
    expect(slider).toHaveAttribute('min', '400');
    expect(slider).toHaveAttribute('max', '700');
    expect(slider).toHaveAttribute('step', '100');
    expect(slider).toHaveValue(String(CONFIG.window.text_font_weight));
    // The chip + screen-reader text surface the descriptive weight label
    // (e.g. "Medium") rather than the raw numeric font-weight value.
    expect(slider).toHaveAttribute('aria-valuetext', 'Medium');
  });
});

describe('AboutTab', () => {
  async function renderAbout() {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return true;
      if (cmd === 'get_updater_state') {
        return {
          last_check_at_unix: null,
          update: null,
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        };
      }
      return CONFIG;
    });
    const view = render(
      <AboutTab onSaved={() => {}} onReload={async () => {}} />,
    );
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    return view;
  }

  it('renders the centered hero with title, version, and tagline', async () => {
    await renderAbout();
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
    await renderAbout();
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
    await renderAbout();
    // The header version contains "nightly"
    await waitFor(() =>
      expect(screen.getAllByText(/nightly/).length).toBeGreaterThan(0),
    );
    fireEvent.click(
      screen.getByRole('button', { name: /release notes on GitHub/ }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/releases/tag/nightly',
    });
    vi.unstubAllEnvs();
  });

  it('GitHub icon button opens the repo', async () => {
    await renderAbout();
    fireEvent.click(
      screen.getByRole('button', { name: 'View Thuki on GitHub' }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki',
    });
  });

  it('X icon button opens @quiet_node', async () => {
    await renderAbout();
    fireEvent.click(
      screen.getByRole('button', { name: /Reach out to Logan on X/ }),
    );
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://x.com/quiet_node',
    });
  });

  it('Feedback icon button opens GitHub Issues', async () => {
    await renderAbout();
    fireEvent.click(screen.getByRole('button', { name: /Open an issue/ }));
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/issues',
    });
  });

  it('Globe icon button opens thuki.app', async () => {
    await renderAbout();
    fireEvent.click(screen.getByRole('button', { name: /Visit thuki.app/ }));
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://www.thuki.app/',
    });
  });

  it('Reveal Thuki app data invokes reveal_config_in_finder', async () => {
    await renderAbout();
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
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() => screen.getByText(/Refresh config\.toml/));
    fireEvent.click(
      screen.getByRole('button', { name: /Refresh config\.toml/ }),
    );
    expect(onReload).toHaveBeenCalled();
  });

  it('Reset all opens the confirm dialog and a Cancel keeps the file untouched', async () => {
    await renderAbout();
    fireEvent.click(screen.getByRole('button', { name: /Reset all/ }));
    expect(
      screen.getByText(/Reset all settings to defaults/),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    // The dialog animates out, then unmounts once the exit finishes.
    await waitFor(() =>
      expect(screen.queryByText(/Reset all settings to defaults\?/)).toBeNull(),
    );
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
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
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
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

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
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
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
    await act(async () => {
      await Promise.resolve();
    });

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

describe('BehaviorTab', () => {
  const TOGGLE_NAME = /Auto-replace selected text after \/rewrite or \/refine/;
  const AUTO_SEARCH_NAME = /Auto search the web when needed without \/search/;

  it('renders the Web search section with Auto search on by default', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Web search')).toBeInTheDocument();
    expect(screen.getByText('Auto search')).toBeInTheDocument();
    expect(
      screen.getByRole('switch', { name: AUTO_SEARCH_NAME }),
    ).toHaveAttribute('aria-checked', 'true');
  });

  it('reflects a disabled auto_search value on the toggle', () => {
    render(
      <BehaviorTab
        config={{
          ...CONFIG,
          behavior: {
            auto_replace: false,
            auto_close: false,
            auto_search: false,
            search_notice_acknowledged: false,
            auto_save_conversations: true,
            history_retention_days: -1,
            auto_save_notice_acknowledged: false,
            dismissed_memory_fit_models: [],
          },
        }}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    expect(
      screen.getByRole('switch', { name: AUTO_SEARCH_NAME }),
    ).toHaveAttribute('aria-checked', 'false');
  });

  it('highlights Auto search with wiggle then clears via timeout callback', () => {
    vi.useFakeTimers();
    const onDone = vi.fn();
    const { unmount, rerender } = render(
      <BehaviorTab
        config={CONFIG}
        resyncToken={0}
        onSaved={() => {}}
        highlightAutoSearchNonce={1}
        onHighlightAutoSearchDone={onDone}
      />,
    );
    expect(screen.getByTestId('auto-search-row')).toHaveAttribute(
      'data-highlight',
      'true',
    );
    expect(screen.getByTestId('auto-search-wiggle')).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(7200);
    });
    expect(onDone).toHaveBeenCalledTimes(1);
    // Nonce bump restarts highlight without needing false first.
    rerender(
      <BehaviorTab
        config={CONFIG}
        resyncToken={0}
        onSaved={() => {}}
        highlightAutoSearchNonce={2}
        onHighlightAutoSearchDone={onDone}
      />,
    );
    expect(screen.getByTestId('auto-search-wiggle')).toBeInTheDocument();
    // Optional callback absent: timeout still safe.
    unmount();
    render(
      <BehaviorTab
        config={CONFIG}
        resyncToken={0}
        onSaved={() => {}}
        highlightAutoSearchNonce={1}
      />,
    );
    act(() => {
      vi.advanceTimersByTime(7200);
    });
    vi.useRealTimers();
  });

  it('renders the Text Replacement section with the Auto replace toggle', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Text Replacement')).toBeInTheDocument();
    // Label is the short one-line form; full copy lives in the "?" tooltip.
    expect(screen.getByText('Auto replace')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: TOGGLE_NAME })).toHaveAttribute(
      'aria-checked',
      'false',
    );
  });

  it('reflects an enabled auto_replace value on the toggle', () => {
    render(
      <BehaviorTab
        config={{
          ...CONFIG,
          behavior: {
            auto_replace: true,
            auto_close: false,
            auto_search: true,
            search_notice_acknowledged: false,
            auto_save_conversations: true,
            history_retention_days: -1,
            auto_save_notice_acknowledged: false,
            dismissed_memory_fit_models: [],
          },
        }}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    expect(screen.getByRole('switch', { name: TOGGLE_NAME })).toHaveAttribute(
      'aria-checked',
      'true',
    );
  });

  const CLOSE_NAME = /Close Thuki after replacing selected text/;

  it('renders the Auto close toggle in the Text Replacement section', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Auto close')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: CLOSE_NAME })).toHaveAttribute(
      'aria-checked',
      'false',
    );
  });

  it('reflects an enabled auto_close value on the toggle', () => {
    render(
      <BehaviorTab
        config={{
          ...CONFIG,
          behavior: {
            auto_replace: false,
            auto_close: true,
            auto_search: true,
            search_notice_acknowledged: false,
            auto_save_conversations: true,
            history_retention_days: -1,
            auto_save_notice_acknowledged: false,
            dismissed_memory_fit_models: [],
          },
        }}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    expect(screen.getByRole('switch', { name: CLOSE_NAME })).toHaveAttribute(
      'aria-checked',
      'true',
    );
  });

  it('opens the Auto search help tooltip downward so it is not clipped at the top edge', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const help = screen.getByRole('button', { name: 'About Auto search' });
    fireEvent.mouseEnter(help.parentElement!);
    // placement="bottom" uses translateX(-50%) (below the trigger).
    expect(
      document.body.querySelector('[style*="translateX(-50%)"]'),
    ).not.toBeNull();
    expect(screen.getByText(/live facts/i)).toBeInTheDocument();
  });

  it('opens Text Replacement help tooltips upward so they are not clipped at the bottom edge', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const help = screen.getByRole('button', { name: 'About Auto replace' });
    fireEvent.mouseEnter(help.parentElement!);
    // placement="top" positions the tooltip box with a `translate(..., -100%)`
    // transform (it sits above the trigger).
    expect(
      document.body.querySelector('[style*="translate(-50%, -100%)"]'),
    ).not.toBeNull();
  });

  it('shows a scope help tooltip on the Text Replacement section heading', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.mouseEnter(
      screen.getByRole('button', { name: 'About Text Replacement' })
        .parentElement!,
    );
    expect(
      screen.getByText(/Applies only to \/rewrite and \/refine/),
    ).toBeInTheDocument();
  });

  it('does not show a section-level help control on Web search', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(
      screen.queryByRole('button', { name: 'About Web search' }),
    ).not.toBeInTheDocument();
  });

  it('keeps the Diagnostics block collapsed until opened, then reveals trace + folder actions', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(
      screen.queryByRole('switch', { name: 'Enable trace recording' }),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));

    expect(
      screen.getByRole('switch', { name: 'Enable trace recording' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Open traces folder' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Free traces…' }),
    ).toBeInTheDocument();
  });

  it('opens the traces folder via the open_traces_in_finder command', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    fireEvent.click(screen.getByRole('button', { name: 'Open traces folder' }));
    expect(invokeMock).toHaveBeenCalledWith('open_traces_in_finder');
  });

  it('deletes traces only after confirming the destructive dialog', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    fireEvent.click(screen.getByRole('button', { name: 'Free traces…' }));
    // The modal arms the action but nothing is deleted yet.
    expect(invokeMock).not.toHaveBeenCalledWith('free_traces');
    // Exact name match hits the modal's confirm, not the "Free traces…" trigger.
    fireEvent.click(screen.getByRole('button', { name: 'Free traces' }));
    expect(invokeMock).toHaveBeenCalledWith('free_traces');
  });

  it('cancels the free-traces confirmation without deleting', async () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    fireEvent.click(screen.getByRole('button', { name: 'Free traces…' }));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(invokeMock).not.toHaveBeenCalledWith('free_traces');
    // The dialog animates out, then unmounts once the exit finishes.
    await waitFor(() =>
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument(),
    );
  });

  it('shows the on-disk footprint subtext below the side-by-side action bar', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'traces_stats')
        return Promise.resolve({ count: 12, bytes: 4404019 });
      return Promise.resolve(undefined);
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));

    const subtext = await screen.findByText('12 traces · 4.2 MB on disk');
    const openBtn = screen.getByRole('button', { name: 'Open traces folder' });
    const freeBtn = screen.getByRole('button', { name: 'Free traces…' });
    // Both actions share one side-by-side bar...
    expect(openBtn.parentElement).toBe(freeBtn.parentElement);
    // ...and the footprint renders directly below that bar.
    expect(openBtn.parentElement?.nextElementSibling).toBe(subtext);
  });

  it('hides the footprint subtext when the stats command fails', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'traces_stats') return Promise.reject(new Error('nope'));
      return Promise.resolve(undefined);
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Open traces folder' }),
      ).toBeInTheDocument(),
    );
    expect(screen.queryByText(/on disk/)).not.toBeInTheDocument();
    expect(
      screen.queryByText('No traces recorded yet'),
    ).not.toBeInTheDocument();
  });

  it('refreshes the footprint to the empty state after freeing traces', async () => {
    let freed = false;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'traces_stats')
        return Promise.resolve(
          freed ? { count: 0, bytes: 0 } : { count: 3, bytes: 900 },
        );
      if (cmd === 'free_traces') {
        freed = true;
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    await screen.findByText('3 traces · 900 B on disk');

    fireEvent.click(screen.getByRole('button', { name: 'Free traces…' }));
    fireEvent.click(screen.getByRole('button', { name: 'Free traces' }));

    expect(
      await screen.findByText('No traces recorded yet'),
    ).toBeInTheDocument();
  });

  it('still refreshes the footprint when freeing fails', async () => {
    let attempted = false;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'traces_stats')
        return Promise.resolve(
          attempted ? { count: 0, bytes: 0 } : { count: 3, bytes: 900 },
        );
      if (cmd === 'free_traces') {
        attempted = true;
        return Promise.reject(new Error('locked'));
      }
      return Promise.resolve(undefined);
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    await screen.findByText('3 traces · 900 B on disk');

    fireEvent.click(screen.getByRole('button', { name: 'Free traces…' }));
    fireEvent.click(screen.getByRole('button', { name: 'Free traces' }));

    // Delete rejected, but the reload still resyncs to the true (empty) state.
    expect(
      await screen.findByText('No traces recorded yet'),
    ).toBeInTheDocument();
  });

  /** Stubs `window.matchMedia` so `prefers-reduced-motion` resolves to `reduce`. */
  function stubReducedMotion(reduce: boolean) {
    vi.stubGlobal(
      'matchMedia',
      vi.fn().mockReturnValue({
        matches: reduce,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    );
  }

  it('greys the Free traces button when there are no traces on disk', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'traces_stats')
        return Promise.resolve({ count: 0, bytes: 0 });
      return Promise.resolve(undefined);
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    await screen.findByText('No traces recorded yet');
    expect(screen.getByRole('button', { name: 'Free traces…' })).toBeDisabled();
  });

  it('keeps the Free traces button active when traces exist', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'traces_stats')
        return Promise.resolve({ count: 5, bytes: 1000 });
      return Promise.resolve(undefined);
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    await screen.findByText('5 traces · 1000 B on disk');
    expect(screen.getByRole('button', { name: 'Free traces…' })).toBeEnabled();
  });

  it('leaves the Free traces button active while the count is unknown', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'traces_stats') return Promise.reject(new Error('nope'));
      return Promise.resolve(undefined);
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    // The stats probe failed, so the count stays unknown and the button must
    // not be trapped in the disabled state.
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Open traces folder' }),
      ).toBeInTheDocument(),
    );
    expect(screen.getByRole('button', { name: 'Free traces…' })).toBeEnabled();
  });

  it('draws the success tick after freeing, then settles into the disabled state', async () => {
    vi.useFakeTimers();
    stubReducedMotion(false);
    let freed = false;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'traces_stats')
        return Promise.resolve(
          freed ? { count: 0, bytes: 0 } : { count: 3, bytes: 900 },
        );
      if (cmd === 'free_traces') {
        freed = true;
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    // Flush the initial (non-empty) stats load.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'Free traces…' })).toBeEnabled();

    fireEvent.click(screen.getByRole('button', { name: 'Free traces…' }));
    fireEvent.click(screen.getByRole('button', { name: 'Free traces' }));
    // Flush free_traces + the post-delete stats refetch + the success kick-off.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    // Mid-hold: the button shows the drawn green tick and is non-interactive.
    const tick = screen.getByRole('button', { name: 'Traces freed' });
    expect(tick).toBeDisabled();
    expect(tick).toHaveAttribute('data-freed', 'true');
    expect(tick.querySelector('svg')).not.toBeNull();

    // Hold elapses: settle into the plain disabled/grey empty state.
    act(() => {
      vi.advanceTimersByTime(FREE_SUCCESS_HOLD_MS);
    });
    const settled = screen.getByRole('button', { name: 'Free traces…' });
    expect(settled).toBeDisabled();
    expect(settled).not.toHaveAttribute('data-freed');
  });

  it('skips the success tick under reduced motion and settles straight to grey', async () => {
    stubReducedMotion(true);
    let freed = false;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'traces_stats')
        return Promise.resolve(
          freed ? { count: 0, bytes: 0 } : { count: 3, bytes: 900 },
        );
      if (cmd === 'free_traces') {
        freed = true;
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    await screen.findByText('3 traces · 900 B on disk');

    fireEvent.click(screen.getByRole('button', { name: 'Free traces…' }));
    fireEvent.click(screen.getByRole('button', { name: 'Free traces' }));

    expect(
      await screen.findByText('No traces recorded yet'),
    ).toBeInTheDocument();
    // No tick was ever drawn; the button went directly to disabled/grey.
    expect(screen.queryByRole('button', { name: 'Traces freed' })).toBeNull();
    const btn = screen.getByRole('button', { name: 'Free traces…' });
    expect(btn).toBeDisabled();
    expect(btn).not.toHaveAttribute('data-freed');
  });

  const RETENTION_NAME = 'Days to keep recorded traces';

  function retentionConfig(days: number): RawAppConfig {
    return {
      ...CONFIG,
      debug: { trace_enabled: false, trace_retention_days: days },
    };
  }

  it('renders the Retention row with a number input, days unit, and help', () => {
    render(
      <BehaviorTab
        config={retentionConfig(7)}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));

    // History + Diagnostics both use the short "Retention" label.
    expect(screen.getAllByText('Retention').length).toBeGreaterThanOrEqual(2);
    const input = screen.getByRole('spinbutton', { name: RETENTION_NAME });
    expect(input).toHaveValue(7);
    // History retention also shows a "days" unit above Diagnostics.
    expect(screen.getAllByText('days').length).toBeGreaterThanOrEqual(1);

    // Compact explanation lives only inside the Diagnostics "?" affordance.
    // Both History and Diagnostics expose "About Retention"; pick the traces one.
    const aboutRetention = screen
      .getAllByRole('button', { name: 'About Retention' })
      .find((btn) => btn.closest('#dev-diagnostics') != null);
    expect(aboutRetention).toBeTruthy();
    fireEvent.mouseEnter(aboutRetention!.parentElement!);
    expect(screen.getByText(/kept on disk/)).toBeInTheDocument();
  });

  it('writes an edited retention value through set_config_field', async () => {
    render(
      <BehaviorTab
        config={retentionConfig(7)}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));

    const input = screen.getByRole('spinbutton', { name: RETENTION_NAME });
    fireEvent.change(input, { target: { value: '30' } });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_config_field', {
        section: 'debug',
        key: 'trace_retention_days',
        value: 30,
      }),
    );
  });

  it('accepts -1 (keep forever) as a valid retention value', async () => {
    render(
      <BehaviorTab
        config={retentionConfig(7)}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));

    const input = screen.getByRole('spinbutton', { name: RETENTION_NAME });
    fireEvent.change(input, { target: { value: '-1' } });
    expect(input).toHaveValue(-1);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_config_field', {
        section: 'debug',
        key: 'trace_retention_days',
        value: -1,
      }),
    );
  });

  it('clamps an out-of-range retention entry to the input maximum', () => {
    render(
      <BehaviorTab
        config={retentionConfig(7)}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));

    const input = screen.getByRole('spinbutton', { name: RETENTION_NAME });
    fireEvent.change(input, { target: { value: '99999' } });
    expect(input).toHaveValue(3650);
  });

  it('reverts a cleared retention field to the last committed value on blur', () => {
    render(
      <BehaviorTab
        config={retentionConfig(14)}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));

    const input = screen.getByRole('spinbutton', { name: RETENTION_NAME });
    fireEvent.change(input, { target: { value: '' } });
    expect(input).toHaveValue(null);
    fireEvent.blur(input);
    expect(input).toHaveValue(14);
  });

  it('keeps a valid retention value untouched on blur', () => {
    render(
      <BehaviorTab
        config={retentionConfig(7)}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));

    const input = screen.getByRole('spinbutton', { name: RETENTION_NAME });
    fireEvent.change(input, { target: { value: '21' } });
    fireEvent.blur(input);
    expect(input).toHaveValue(21);
  });

  it('re-seeds the retention field on resync when it is not focused', () => {
    const { rerender } = render(
      <BehaviorTab
        config={retentionConfig(7)}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    expect(
      screen.getByRole('spinbutton', { name: RETENTION_NAME }),
    ).toHaveValue(7);

    rerender(
      <BehaviorTab
        config={retentionConfig(90)}
        resyncToken={1}
        onSaved={() => {}}
      />,
    );
    expect(
      screen.getByRole('spinbutton', { name: RETENTION_NAME }),
    ).toHaveValue(90);
  });

  it('preserves an in-progress retention edit across a resync while focused', () => {
    const { rerender } = render(
      <BehaviorTab
        config={retentionConfig(7)}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));

    const input = screen.getByRole('spinbutton', { name: RETENTION_NAME });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '45' } });

    // A background reload arrives mid-edit; the focused field must not snap
    // back to the reloaded config value.
    rerender(
      <BehaviorTab
        config={retentionConfig(90)}
        resyncToken={1}
        onSaved={() => {}}
      />,
    );
    expect(input).toHaveValue(45);
  });

  it('renders History with Auto save on by default', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('History')).toBeInTheDocument();
    expect(screen.getByText('Auto save')).toBeInTheDocument();
    expect(screen.getByText('Retention')).toBeInTheDocument();
    expect(screen.getByText('Chats')).toBeInTheDocument();
    expect(
      screen.getByRole('switch', {
        name: 'Auto-save completed chats to history',
      }),
    ).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByTestId('history-retention-input')).toHaveValue(-1);
  });

  it('highlights Auto-save with wiggle then clears via timeout callback', () => {
    vi.useFakeTimers();
    const onDone = vi.fn();
    render(
      <BehaviorTab
        config={CONFIG}
        resyncToken={0}
        onSaved={() => {}}
        highlightAutoSaveNonce={1}
        onHighlightAutoSaveDone={onDone}
      />,
    );
    expect(screen.getByTestId('auto-save-conversations-row')).toHaveAttribute(
      'data-highlight',
      'true',
    );
    expect(screen.getByTestId('auto-save-wiggle')).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(7200);
    });
    expect(onDone).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it('changing history retention to forever writes without confirm', async () => {
    const onSaved = vi.fn();
    const cfg = {
      ...CONFIG,
      behavior: { ...CONFIG.behavior, history_retention_days: 30 },
    };
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'set_config_field') {
        const a = args as { value: number };
        return {
          ...cfg,
          behavior: { ...cfg.behavior, history_retention_days: a.value },
        };
      }
      return undefined;
    });
    render(<BehaviorTab config={cfg} resyncToken={0} onSaved={onSaved} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '-1' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'set_config_field',
        expect.objectContaining({
          section: 'behavior',
          key: 'history_retention_days',
          value: -1,
        }),
      ),
    );
    expect(invokeMock).not.toHaveBeenCalledWith('prune_conversation_history');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('finite history retention cancel reverts draft and does not write', async () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '7' } });
    fireEvent.blur(input);
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() =>
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument(),
    );
    expect(input).toHaveValue(-1);
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_config_field',
      expect.objectContaining({ key: 'history_retention_days' }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith('prune_conversation_history');
  });

  it('finite history retention confirm writes config and prunes', async () => {
    const onSaved = vi.fn();
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'set_config_field') {
        const a = args as { value: number };
        return {
          ...CONFIG,
          behavior: {
            ...CONFIG.behavior,
            history_retention_days: a.value,
          },
        };
      }
      if (cmd === 'history_retention_prune_count') return 2;
      if (cmd === 'prune_conversation_history') return 2;
      if (cmd === 'history_stats') return { count: 1, bytes: 10 };
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={onSaved} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '14' } });
    fireEvent.blur(input);
    fireEvent.click(await screen.findByRole('button', { name: 'Acknowledge' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'set_config_field',
        expect.objectContaining({
          key: 'history_retention_days',
          value: 14,
        }),
      ),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('prune_conversation_history'),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('history_stats'),
    );
    expect(onSaved).toHaveBeenCalled();
  });

  it('shortening retention with zero prune count commits without dialog', async () => {
    const onSaved = vi.fn();
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'history_retention_prune_count') return 0;
      if (cmd === 'set_config_field') {
        const a = args as { value: number };
        return {
          ...CONFIG,
          behavior: {
            ...CONFIG.behavior,
            history_retention_days: a.value,
          },
        };
      }
      if (cmd === 'prune_conversation_history') return 0;
      if (cmd === 'history_stats') return { count: 0, bytes: 0 };
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={onSaved} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '7' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('history_retention_prune_count', {
        days: 7,
      }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'set_config_field',
        expect.objectContaining({
          key: 'history_retention_days',
          value: 7,
        }),
      ),
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('prune_conversation_history'),
    );
    expect(onSaved).toHaveBeenCalled();
  });

  it('shortening retention with prune count opens confirm dialog', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_retention_prune_count') return 2;
      if (cmd === 'history_stats') return { count: 2, bytes: 50 };
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '7' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('history_retention_prune_count', {
        days: 7,
      }),
    );
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_config_field',
      expect.objectContaining({ key: 'history_retention_days' }),
    );
  });

  it('shortening retention probe failure fails closed to confirm dialog', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_retention_prune_count') {
        throw new Error('db locked');
      }
      if (cmd === 'history_stats') return { count: 1, bytes: 10 };
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '14' } });
    fireEvent.blur(input);
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_config_field',
      expect.objectContaining({ key: 'history_retention_days' }),
    );
  });

  it('history retention confirm copy uses singular day for 1', async () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '1' } });
    fireEvent.blur(input);
    expect(
      await screen.findByText(/older than 1 day will be permanently deleted/),
    ).toBeInTheDocument();
  });

  it('shows saved-chat footprint subtext under Free chats', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats') return { count: 12, bytes: 4404019 };
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(
      await screen.findByText('12 chats · 4.2 MB on disk'),
    ).toBeInTheDocument();
  });

  it('hides history subtext when history_stats fails', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats') throw new Error('db locked');
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await waitFor(() =>
      expect(screen.getByTestId('clear-all-history')).toBeEnabled(),
    );
    expect(screen.queryByText(/on disk/)).not.toBeInTheDocument();
    expect(screen.queryByText('No saved chats yet')).not.toBeInTheDocument();
  });

  it('greys Free chats when there are no saved chats', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats') return { count: 0, bytes: 0 };
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(await screen.findByText('No saved chats yet')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Free chats…' })).toBeDisabled();
  });

  it('keeps Free chats active when chats exist', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats') return { count: 5, bytes: 1000 };
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(
      await screen.findByText('5 chats · 1000 B on disk'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Free chats…' })).toBeEnabled();
  });

  it('leaves Free chats active while history count is unknown', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats') throw new Error('nope');
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Free chats…' })).toBeEnabled(),
    );
  });

  it('free chats invoke failure is swallowed without history-cleared', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats') return { count: 2, bytes: 50 };
      if (cmd === 'clear_all_conversations') throw new Error('db locked');
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await screen.findByText('2 chats · 50 B on disk');
    fireEvent.click(screen.getByTestId('clear-all-history'));
    fireEvent.click(screen.getByRole('button', { name: 'Free chats' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('clear_all_conversations'),
    );
    expect(emit).not.toHaveBeenCalledWith(HISTORY_CLEARED_EVENT);
  });

  it('free chats only runs after confirm', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats') return { count: 3, bytes: 90 };
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await screen.findByText('3 chats · 90 B on disk');
    fireEvent.click(screen.getByTestId('clear-all-history'));
    expect(invokeMock).not.toHaveBeenCalledWith('clear_all_conversations');
    fireEvent.click(screen.getByRole('button', { name: 'Free chats' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('clear_all_conversations'),
    );
  });

  it('free chats emits history-cleared and refreshes stats after success', async () => {
    const onCleared = vi.fn();
    await listen(HISTORY_CLEARED_EVENT, onCleared);
    let freed = false;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats')
        return freed ? { count: 0, bytes: 0 } : { count: 3, bytes: 900 };
      if (cmd === 'clear_all_conversations') {
        freed = true;
        return undefined;
      }
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await screen.findByText('3 chats · 900 B on disk');
    fireEvent.click(screen.getByTestId('clear-all-history'));
    fireEvent.click(screen.getByRole('button', { name: 'Free chats' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('clear_all_conversations'),
    );
    await waitFor(() =>
      expect(emit).toHaveBeenCalledWith(HISTORY_CLEARED_EVENT),
    );
    expect(onCleared).toHaveBeenCalled();
    expect(await screen.findByText('No saved chats yet')).toBeInTheDocument();
  });

  it('free chats cancel does not delete', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats') return { count: 2, bytes: 40 };
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await screen.findByText('2 chats · 40 B on disk');
    fireEvent.click(screen.getByTestId('clear-all-history'));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() =>
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument(),
    );
    expect(invokeMock).not.toHaveBeenCalledWith('clear_all_conversations');
  });

  it('draws Free chats success tick then settles disabled', async () => {
    vi.useFakeTimers();
    stubReducedMotion(false);
    let freed = false;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats')
        return freed ? { count: 0, bytes: 0 } : { count: 3, bytes: 900 };
      if (cmd === 'clear_all_conversations') {
        freed = true;
        return undefined;
      }
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'Free chats…' })).toBeEnabled();

    fireEvent.click(screen.getByRole('button', { name: 'Free chats…' }));
    fireEvent.click(screen.getByRole('button', { name: 'Free chats' }));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    const tick = screen.getByRole('button', { name: 'Chats freed' });
    expect(tick).toBeDisabled();
    expect(tick).toHaveAttribute('data-freed', 'true');
    expect(tick.querySelector('svg')).not.toBeNull();

    act(() => {
      vi.advanceTimersByTime(FREE_SUCCESS_HOLD_MS);
    });
    const settled = screen.getByRole('button', { name: 'Free chats…' });
    expect(settled).toBeDisabled();
    expect(settled).not.toHaveAttribute('data-freed');
  });

  it('skips Free chats success tick under reduced motion', async () => {
    stubReducedMotion(true);
    let freed = false;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats')
        return freed ? { count: 0, bytes: 0 } : { count: 3, bytes: 900 };
      if (cmd === 'clear_all_conversations') {
        freed = true;
        return undefined;
      }
      return undefined;
    });
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await screen.findByText('3 chats · 900 B on disk');

    fireEvent.click(screen.getByRole('button', { name: 'Free chats…' }));
    fireEvent.click(screen.getByRole('button', { name: 'Free chats' }));

    expect(await screen.findByText('No saved chats yet')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Chats freed' })).toBeNull();
    const btn = screen.getByRole('button', { name: 'Free chats…' });
    expect(btn).toBeDisabled();
    expect(btn).not.toHaveAttribute('data-freed');
  });

  it('lengthening history retention applies without confirm dialog', async () => {
    const onSaved = vi.fn();
    const cfg = {
      ...CONFIG,
      behavior: { ...CONFIG.behavior, history_retention_days: 7 },
    };
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'set_config_field') {
        const a = args as { value: number };
        return {
          ...cfg,
          behavior: { ...cfg.behavior, history_retention_days: a.value },
        };
      }
      if (cmd === 'prune_conversation_history') return 0;
      return undefined;
    });
    render(<BehaviorTab config={cfg} resyncToken={0} onSaved={onSaved} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '30' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'set_config_field',
        expect.objectContaining({
          key: 'history_retention_days',
          value: 30,
        }),
      ),
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('prune_conversation_history'),
    );
  });

  it('history retention keeps unparseable raw text then reverts on blur', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '-' } });
    expect(input).toHaveValue(null);
    fireEvent.blur(input);
    expect(input).toHaveValue(-1);
  });

  it('history retention Enter on unparseable value reverts without apply', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId(
      'history-retention-input',
    ) as HTMLInputElement;
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '-' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(input).toHaveValue(-1);
    expect(screen.queryByTestId('history-retention-error')).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'history_retention_prune_count',
      expect.anything(),
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_config_field',
      expect.objectContaining({ key: 'history_retention_days' }),
    );
  });

  it('shortening finite retention with prune count still opens confirm', async () => {
    const cfg = {
      ...CONFIG,
      behavior: { ...CONFIG.behavior, history_retention_days: 30 },
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_retention_prune_count') return 3;
      if (cmd === 'history_stats') return { count: 3, bytes: 90 };
      return undefined;
    });
    render(<BehaviorTab config={cfg} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '7' } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('history_retention_prune_count', {
        days: 7,
      }),
    );
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_config_field',
      expect.objectContaining({ key: 'history_retention_days' }),
    );
  });

  it('history retention blur of same committed value is a no-op write', async () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '-1' } });
    fireEvent.blur(input);
    await act(async () => {});
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_config_field',
      expect.objectContaining({ key: 'history_retention_days' }),
    );
  });

  it('history retention rejects 0 with inline error and does not write', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '0' } });
    fireEvent.blur(input);
    // Keep invalid "0" visible so the user can fix it; never silent-revert.
    expect(input).toHaveValue(0);
    expect(screen.getByTestId('history-retention-error')).toHaveTextContent(
      HISTORY_RETENTION_ZERO_ERROR,
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_config_field',
      expect.objectContaining({ key: 'history_retention_days' }),
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('history retention clears zero-error when user types a valid number', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '0' } });
    fireEvent.blur(input);
    expect(screen.getByTestId('history-retention-error')).toBeInTheDocument();
    fireEvent.change(input, { target: { value: '7' } });
    expect(
      screen.queryByTestId('history-retention-error'),
    ).not.toBeInTheDocument();
  });

  it('history retention write failure reverts draft to committed', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_config_field') throw new Error('write failed');
      return undefined;
    });
    render(
      <BehaviorTab
        config={{
          ...CONFIG,
          behavior: { ...CONFIG.behavior, history_retention_days: 30 },
        }}
        resyncToken={0}
        onSaved={() => {}}
      />,
    );
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '-1' } });
    fireEvent.blur(input);
    await waitFor(() => expect(input).toHaveValue(30));
  });

  it('re-seeds history retention on resync when not focused', () => {
    const { rerender } = render(
      <BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />,
    );
    rerender(
      <BehaviorTab
        config={{
          ...CONFIG,
          behavior: { ...CONFIG.behavior, history_retention_days: 90 },
        }}
        resyncToken={1}
        onSaved={() => {}}
      />,
    );
    expect(screen.getByTestId('history-retention-input')).toHaveValue(90);
  });

  it('preserves focused history retention draft across resync', () => {
    const { rerender } = render(
      <BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />,
    );
    const input = screen.getByTestId('history-retention-input');
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '12' } });
    rerender(
      <BehaviorTab
        config={{
          ...CONFIG,
          behavior: { ...CONFIG.behavior, history_retention_days: 90 },
        }}
        resyncToken={1}
        onSaved={() => {}}
      />,
    );
    expect(input).toHaveValue(12);
  });

  it('history retention ignores non-Enter keys', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId(
      'history-retention-input',
    ) as HTMLInputElement;
    fireEvent.focus(input);
    const blurSpy = vi.spyOn(input, 'blur');
    fireEvent.keyDown(input, { key: 'a' });
    expect(blurSpy).not.toHaveBeenCalled();
    blurSpy.mockRestore();
  });

  it('history retention Enter applies via currentTarget without double commit', async () => {
    const onSaved = vi.fn();
    const cfg = {
      ...CONFIG,
      behavior: { ...CONFIG.behavior, history_retention_days: 30 },
    };
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'set_config_field') {
        const a = args as { value: number };
        return {
          ...cfg,
          behavior: { ...cfg.behavior, history_retention_days: a.value },
        };
      }
      return undefined;
    });
    render(<BehaviorTab config={cfg} resyncToken={0} onSaved={onSaved} />);
    const input = screen.getByTestId(
      'history-retention-input',
    ) as HTMLInputElement;
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '-1' } });
    const blurSpy = vi.spyOn(input, 'blur');
    fireEvent.keyDown(input, { key: 'Enter' });
    // Enter applies immediately and still blurs for focus cleanup.
    expect(blurSpy).toHaveBeenCalled();
    // Synthetic blur must not re-apply (skipHistoryRetentionBlurRef).
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'set_config_field',
        expect.objectContaining({
          key: 'history_retention_days',
          value: -1,
        }),
      ),
    );
    expect(
      invokeMock.mock.calls.filter(
        (c) =>
          c[0] === 'set_config_field' &&
          (c[1] as { key?: string })?.key === 'history_retention_days',
      ),
    ).toHaveLength(1);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    blurSpy.mockRestore();
  });

  it('Enter on shortening retention opens confirm without auto-Acknowledge', async () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId(
      'history-retention-input',
    ) as HTMLInputElement;
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: '7' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    // queueMicrotask defers dialog open past the Enter keystroke.
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Acknowledge' }),
    ).toBeInTheDocument();
    // Destructive focus lands on Cancel, not Acknowledge.
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();
    expect(
      screen.getByRole('button', { name: 'Acknowledge' }),
    ).not.toHaveFocus();
    // No write/prune until the user clicks Acknowledge.
    expect(invokeMock).not.toHaveBeenCalledWith(
      'set_config_field',
      expect.objectContaining({ key: 'history_retention_days' }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith('prune_conversation_history');
  });

  it('shortening retention opens confirm labeled Acknowledge', async () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByTestId('history-retention-input');
    fireEvent.change(input, { target: { value: '7' } });
    fireEvent.blur(input);
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Acknowledge' }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Delete old chats' }),
    ).toBeNull();
  });

  it('free chats still shows success tick when history-cleared emit fails', async () => {
    vi.useFakeTimers();
    stubReducedMotion(false);
    let freed = false;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'history_stats')
        return freed ? { count: 0, bytes: 0 } : { count: 3, bytes: 900 };
      if (cmd === 'clear_all_conversations') {
        freed = true;
        return undefined;
      }
      return undefined;
    });
    const emitMock = emit as unknown as ReturnType<typeof vi.fn>;
    emitMock.mockRejectedValueOnce(new Error('emit failed'));

    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Free chats…' }));
    fireEvent.click(screen.getByRole('button', { name: 'Free chats' }));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    // Wipe succeeded: green tick must still draw even if emit rejects.
    const tick = screen.getByRole('button', { name: 'Chats freed' });
    expect(tick).toBeDisabled();
    expect(tick).toHaveAttribute('data-freed', 'true');
    expect(tick.querySelector('svg')).not.toBeNull();
  });

  it('renders Diagnostics Auto record label', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /Diagnostics/ }));
    expect(screen.getByText('Auto record')).toBeInTheDocument();
  });
});
