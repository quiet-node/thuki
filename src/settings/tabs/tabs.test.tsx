/**
 * Smoke + interaction tests for the five Settings tabs.
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
import { clearEventHandlers } from '../../testUtils/mocks/tauri';

import { ModelTab } from './ModelTab';
import { DownloadProvider } from '../../contexts/DownloadContext';
import { DisplayTab } from './DisplayTab';
import { SearchTab } from './SearchTab';
import { AboutTab } from './AboutTab';
import { BehaviorTab } from './BehaviorTab';
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
        label: 'Built-in (Thuki)',
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
  debug: {
    trace_enabled: false,
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
    return Promise.resolve(CONFIG);
  });
});

afterEach(() => {
  vi.useRealTimers();
  clearEventHandlers();
});

async function renderModelTab() {
  const view = render(
    <ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />,
    { wrapper: DownloadProvider },
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
      { wrapper: DownloadProvider },
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

describe('SearchTab', () => {
  it('renders Services, Pipeline, and Timeouts sections', () => {
    render(<SearchTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Services')).toBeInTheDocument();
    expect(screen.getByText('Pipeline')).toBeInTheDocument();
    expect(screen.getByText('Timeouts')).toBeInTheDocument();
    expect(screen.getByText('SearXNG URL')).toBeInTheDocument();
    expect(screen.getByText('Per-URL timeout')).toBeInTheDocument();
    expect(screen.getByText('Batch timeout')).toBeInTheDocument();
    expect(screen.getByText('Router timeout')).toBeInTheDocument();
  });

  it('does not render any Diagnostics affordance', () => {
    render(<SearchTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.queryByText(/Diagnostics/i)).not.toBeInTheDocument();
    expect(screen.queryByText('Trace recording')).not.toBeInTheDocument();
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

  it('renders the Text Replacement section with the Auto-replace toggle', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Text Replacement')).toBeInTheDocument();
    // Label is the short one-line form; full copy lives in the "?" tooltip.
    expect(screen.getByText('Auto-replace')).toBeInTheDocument();
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
          behavior: { auto_replace: true, auto_close: false },
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

  it('renders the Auto-close toggle in the Text Replacement section', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Auto-close')).toBeInTheDocument();
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
          behavior: { auto_replace: false, auto_close: true },
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

  it('opens the help tooltip upward so it is not clipped at the short window edge', () => {
    render(<BehaviorTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const help = screen.getByRole('button', { name: 'About Auto-replace' });
    fireEvent.mouseEnter(help.parentElement!);
    // placement="top" positions the tooltip box with a `translate(..., -100%)`
    // transform (it sits above the trigger). The default "bottom" placement
    // uses `translateX(-50%)`, which here would overflow the bottom edge.
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
});
