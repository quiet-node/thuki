import {
  fireEvent,
  render,
  screen,
  act,
  waitFor,
} from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { invoke } from '@tauri-apps/api/core';

import { __mockWindow } from '../testUtils/mocks/tauri-window';
import { emitTauriEvent } from '../testUtils/mocks/tauri';
import { SettingsWindow } from './SettingsWindow';
import type { CorruptMarker, RawAppConfig } from './types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const SAMPLE: RawAppConfig = {
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
  prompt: { system: '' },
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

function defaultInvoke(cmd: string): unknown {
  switch (cmd) {
    case 'get_config':
      return SAMPLE;
    case 'get_corrupt_marker':
      return null;
    case 'check_accessibility_permission':
      return true;
    case 'check_screen_recording_permission':
      return true;
    case 'get_model_picker_state':
      return { active: null, all: [], displayNames: {}, ollamaReachable: true };
    case 'list_installed_models':
      return [];
    case 'get_engine_status':
      return { state: 'stopped', model_path: '', port: null, error: null };
    case 'get_loaded_model':
      return null;
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

afterEach(() => {
  vi.useRealTimers();
});

describe('SettingsWindow', () => {
  it('renders nothing while the initial get_config is in flight', () => {
    invokeMock.mockImplementation(() => new Promise(() => {}));
    const { container } = render(<SettingsWindow />);
    expect(container.firstChild).toBeNull();
  });

  it('renders the five tab labels after config loads', async () => {
    render(<SettingsWindow />);
    await waitFor(() =>
      expect(screen.getByRole('tab', { name: /Models/ })).toBeInTheDocument(),
    );
    expect(screen.getByRole('tab', { name: /Behavior/ })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Web/ })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Display/ })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /About/ })).toBeInTheDocument();
  });

  it('jumps to the Models Discover view on the show-discover event', async () => {
    const { unmount } = render(<SettingsWindow />);
    await waitFor(() =>
      expect(screen.getByRole('tab', { name: /Models/ })).toBeInTheDocument(),
    );
    // Models opens on Providers by default: no built-in gate yet.
    expect(
      screen.queryByRole('button', { name: 'Switch to built-in' }),
    ).toBeNull();

    await act(async () => {
      emitTauriEvent('thuki://settings-show-discover', undefined);
      await Promise.resolve();
    });

    // Discover is gated for the ollama SAMPLE config, so the switch prompt is
    // the proof the deep-link landed on Discover rather than Providers.
    expect(
      screen.getByRole('button', { name: 'Switch to built-in' }),
    ).toBeInTheDocument();

    unmount();
    await act(async () => {
      await Promise.resolve();
    });
  });

  it('switching to the Behavior tab shows the Text Replacement section', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Behavior/ }));

    fireEvent.click(screen.getByRole('tab', { name: /Behavior/ }));
    expect(screen.getByRole('tab', { name: /Behavior/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.getByText('Text Replacement')).toBeInTheDocument();
    expect(
      screen.getByRole('switch', {
        name: /Auto-replace selected text after \/rewrite or \/refine/,
      }),
    ).toBeInTheDocument();
  });

  it('starts on the Models tab', async () => {
    render(<SettingsWindow />);
    await waitFor(() =>
      expect(screen.getByRole('tab', { name: /Models/ })).toHaveAttribute(
        'aria-selected',
        'true',
      ),
    );
  });

  // Regression: the Settings window is its own webview root. The Discover panes
  // read the app-root download context, so the Settings tree must provide a
  // DownloadProvider or opening Discover throws and blanks the window.
  it('opens Discover without crashing the Settings window', async () => {
    // Built-in active so Discover renders ungated; this test guards the
    // DownloadProvider wiring, not the non-built-in gate (covered in ModelTab).
    const builtinActive: RawAppConfig = {
      ...SAMPLE,
      inference: { ...SAMPLE.inference, active_provider: 'builtin' },
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_config') return builtinActive;
      if (cmd === 'get_staff_picks') return [];
      return defaultInvoke(cmd);
    });
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    await act(async () => {
      fireEvent.click(screen.getByRole('tab', { name: 'Discover' }));
      await Promise.resolve();
    });
    expect(
      await screen.findByRole('tab', { name: 'Staff picks' }),
    ).toBeInTheDocument();
  });

  it('switching tabs swaps the active tab body', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Display/ }));

    fireEvent.click(screen.getByRole('tab', { name: /Display/ }));
    expect(screen.getByRole('tab', { name: /Display/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });

  it('marks the body as scrollable only when natural content exceeds the cap', async () => {
    // happy-dom's `requestAnimationFrame` runs callbacks via setTimeout
    // which would loop here as the auto-resize animation reschedules
    // itself; the assertion only needs the synchronous state flip, so
    // stub rAF to a no-op for this test.
    const rafSpy = vi
      .spyOn(globalThis, 'requestAnimationFrame')
      .mockImplementation(() => 0);
    const { container } = render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    const body = container.querySelector('[role="tabpanel"]')!;
    expect(body.className).not.toMatch(/bodyScrollable/);

    const wrapper = body.firstElementChild as HTMLElement;
    Object.defineProperty(wrapper, 'scrollHeight', {
      configurable: true,
      value: 1500,
    });
    fireEvent.click(screen.getByRole('tab', { name: /Web/ }));
    await waitFor(() =>
      expect(container.querySelector('[role="tabpanel"]')!.className).toMatch(
        /bodyScrollable/,
      ),
    );
    rafSpy.mockRestore();
  });

  it('ArrowRight rotates focus to the next tab', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));

    const modelTab = screen.getByRole('tab', { name: /Models/ });
    fireEvent.keyDown(modelTab, { key: 'ArrowRight' });
    expect(screen.getByRole('tab', { name: /Behavior/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });

  it('ArrowLeft wraps to the last tab when starting on the first', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));

    const modelTab = screen.getByRole('tab', { name: /Models/ });
    await act(async () => {
      fireEvent.keyDown(modelTab, { key: 'ArrowLeft' });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole('tab', { name: /About/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });

  it('non-arrow keys are ignored by the tab key handler', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));

    const modelTab = screen.getByRole('tab', { name: /Models/ });
    fireEvent.keyDown(modelTab, { key: 'Enter' });
    expect(modelTab).toHaveAttribute('aria-selected', 'true');
  });

  it('renders the corrupt-recovery banner when get_corrupt_marker returns one', async () => {
    const marker: CorruptMarker = {
      path: '/Users/x/Library/Application Support/com.quietnode.thuki/config.toml.corrupt-99',
      ts: 99,
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_corrupt_marker') return marker;
      return defaultInvoke(cmd);
    });

    render(<SettingsWindow />);
    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent(/syntax error/),
    );
  });

  it('Reveal opens the corrupt file via open_url', async () => {
    const marker: CorruptMarker = {
      path: '/path/to/config.toml.corrupt-99',
      ts: 99,
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_corrupt_marker') return marker;
      return defaultInvoke(cmd);
    });

    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('alert'));
    fireEvent.click(screen.getByRole('button', { name: /Reveal/ }));
    expect(invokeMock).toHaveBeenCalledWith(
      'open_url',
      expect.objectContaining({ url: expect.stringContaining('file://') }),
    );
  });

  it('Dismiss hides the corrupt banner', async () => {
    const marker: CorruptMarker = { path: '/p/x', ts: 1 };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_corrupt_marker') return marker;
      return defaultInvoke(cmd);
    });

    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('alert'));
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('Cmd+, on the document re-focuses the settings window', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));

    __mockWindow.setFocus.mockClear();
    fireEvent.keyDown(document, { key: ',', metaKey: true });
    expect(__mockWindow.setFocus).toHaveBeenCalled();
  });

  it('Other keystrokes do not trigger setFocus', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));

    __mockWindow.setFocus.mockClear();
    fireEvent.keyDown(document, { key: ',' }); // no Meta
    fireEvent.keyDown(document, { key: 'a', metaKey: true });
    expect(__mockWindow.setFocus).not.toHaveBeenCalled();
  });

  it('Cmd+W on the document hides the settings window', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));

    __mockWindow.hide.mockClear();
    fireEvent.keyDown(document, { key: 'w', metaKey: true });
    expect(__mockWindow.hide).toHaveBeenCalled();
  });

  it('the close button hides the window instead of quitting', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    __mockWindow.hide.mockClear();
    fireEvent.click(screen.getByRole('button', { name: /Close/ }));
    expect(__mockWindow.hide).toHaveBeenCalled();
  });

  it('mousedown on the chrome triggers startDragging when not on an interactive element', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    __mockWindow.startDragging.mockClear();
    // Click on the body container itself (not on a button/input).
    const root = screen
      .getByRole('tab', { name: /Models/ })
      .closest('[role="tablist"]')!.parentElement!;
    fireEvent.mouseDown(root, { target: root });
    // The root is a div; not in INTERACTIVE_TAGS, so dragging fires.
    expect(__mockWindow.startDragging).toHaveBeenCalled();
  });

  it('mousedown that originates from an interactive element does NOT trigger drag', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    __mockWindow.startDragging.mockClear();
    fireEvent.mouseDown(screen.getByRole('tab', { name: /Models/ }));
    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });

  it('mousedown on a text-bearing element does NOT trigger drag (so users can highlight + copy)', async () => {
    const marker: CorruptMarker = { path: '/tmp/config.toml.corrupt-9', ts: 9 };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_corrupt_marker') return marker;
      return defaultInvoke(cmd);
    });
    render(<SettingsWindow />);
    // Banner renders <code>config.toml</code> directly inside the
    // banner text — a text-bearing leaf. Mousedown on it must NOT drag.
    const banner = await screen.findByRole('alert');
    const codeEl = banner.querySelector('code')!;
    __mockWindow.startDragging.mockClear();
    fireEvent.mouseDown(codeEl, { target: codeEl, button: 0 });
    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });

  it('mousedown with a non-primary button is ignored (no drag, lets context menus through)', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    __mockWindow.startDragging.mockClear();
    const root = screen
      .getByRole('tab', { name: /Models/ })
      .closest('[role="tablist"]')!.parentElement!;
    fireEvent.mouseDown(root, { target: root, button: 2 });
    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });

  it('basename helper handles paths without a slash by rendering them verbatim', async () => {
    const marker: CorruptMarker = { path: 'config.toml.corrupt-7', ts: 7 };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_corrupt_marker') return marker;
      return defaultInvoke(cmd);
    });
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('alert'));
    // The bare filename appears inside the banner copy.
    expect(screen.getByRole('alert').textContent).toContain(
      'config.toml.corrupt-7',
    );
  });

  it('successive saves restart the savedPill timer (covers clearTimeout branch)', async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_config_field') return SAMPLE;
      return defaultInvoke(cmd);
    });

    render(<SettingsWindow />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    fireEvent.click(screen.getByRole('tab', { name: /Display/ }));
    const incBtns = () => screen.getAllByRole('button', { name: 'Increase' });

    // First save.
    fireEvent.click(incBtns()[0]);
    await act(async () => {
      vi.advanceTimersByTime(400);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByText('✓ Saved')).toHaveTextContent('Saved');

    // Second save before pill auto-hides — clearTimeout(savedTimerRef.current) fires.
    fireEvent.click(incBtns()[0]);
    await act(async () => {
      vi.advanceTimersByTime(400);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByText('✓ Saved')).toHaveTextContent('Saved');
  });

  it('unmount with the savedPill timer still pending clears it cleanly', async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_config_field') return SAMPLE;
      return defaultInvoke(cmd);
    });

    const { unmount } = render(<SettingsWindow />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    fireEvent.click(screen.getByRole('tab', { name: /Display/ }));
    fireEvent.click(screen.getAllByRole('button', { name: 'Increase' })[0]);
    await act(async () => {
      vi.advanceTimersByTime(400);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    // Tear down WITH the savedPill timer still pending — exercises the
    // unmount cleanup branch that clears the savedTimerRef.
    unmount();
  });

  it('shows the Saved pill briefly after a successful field save', async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_config_field') return SAMPLE;
      return defaultInvoke(cmd);
    });

    render(<SettingsWindow />);
    await act(async () => {
      // Microtasks for get_config + corrupt marker.
      await Promise.resolve();
      await Promise.resolve();
    });

    // Switch to Display tab where stepper buttons are easy to click.
    fireEvent.click(screen.getByRole('tab', { name: /Display/ }));
    fireEvent.click(screen.getAllByRole('button', { name: 'Increase' })[0]);
    await act(async () => {
      vi.advanceTimersByTime(400);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.getByText('✓ Saved')).toHaveTextContent('Saved');

    // After SAVED_PILL_DURATION_MS the pill toggles back to invisible. We
    // don't assert on that visibility here because the underlying class
    // change is verified in components.test (SavedPill).
    await act(async () => {
      vi.advanceTimersByTime(2000);
      await Promise.resolve();
    });
  });

  it('renders UpdateBanner when an update is available and not snoozed', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_updater_state') {
        return {
          last_check_at_unix: 100,
          update: { version: '0.8.0', notes_url: null },
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        };
      }
      return defaultInvoke(cmd);
    });
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    await waitFor(() =>
      expect(screen.getByText(/0\.8\.0 is ready/)).toBeInTheDocument(),
    );
  });

  it("opens the update window when What's New clicked on UpdateBanner", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_updater_state') {
        return {
          last_check_at_unix: 100,
          update: { version: '0.8.0', notes_url: null },
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        };
      }
      return defaultInvoke(cmd);
    });
    render(<SettingsWindow />);
    await waitFor(() => screen.getByText(/0\.8\.0 is ready/));
    fireEvent.click(screen.getByRole('button', { name: /what's new/i }));
    expect(invokeMock).toHaveBeenCalledWith('open_update_window');
  });

  it('calls snooze_update_settings when Later button clicked on UpdateBanner', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_updater_state') {
        return {
          last_check_at_unix: 100,
          update: { version: '0.8.0', notes_url: null },
          settings_snoozed_until: null,
          chat_snoozed_until: null,
        };
      }
      return defaultInvoke(cmd);
    });
    render(<SettingsWindow />);
    await waitFor(() => screen.getByText(/0\.8\.0 is ready/));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /^later$/i }));
      await Promise.resolve();
    });
    expect(invokeMock).toHaveBeenCalledWith('snooze_update_settings', {
      hours: 24,
    });
  });

  it('hides UpdateBanner when settings_snoozed_until is in the future', async () => {
    const futureUnix = Math.floor(Date.now() / 1000) + 3600;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_updater_state') {
        return {
          last_check_at_unix: 100,
          update: { version: '0.8.0', notes_url: null },
          settings_snoozed_until: futureUnix,
          chat_snoozed_until: null,
        };
      }
      return defaultInvoke(cmd);
    });
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    // Allow time for updater state to load
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.queryByText(/0\.8\.0 is ready/)).not.toBeInTheDocument();
  });
});

describe('SettingsWindow left sidebar (Phase 3)', () => {
  it('renders the section nav as a vertical sidebar', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    // Scope to the sidebar: the Models pane also renders a (horizontal)
    // segmented tablist for Library/Discover/Providers.
    expect(
      screen.getByRole('tablist', { name: 'Settings sections' }),
    ).toHaveAttribute('aria-orientation', 'vertical');
  });

  it('renders Models as the first section label', async () => {
    render(<SettingsWindow />);
    await waitFor(() =>
      expect(screen.getByRole('tab', { name: /Models/ })).toBeInTheDocument(),
    );
  });

  it('ArrowDown rotates focus to the next sidebar section', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    fireEvent.keyDown(screen.getByRole('tab', { name: /Models/ }), {
      key: 'ArrowDown',
    });
    expect(screen.getByRole('tab', { name: /Behavior/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });

  it('ArrowUp wraps to the last sidebar section from the first', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /Models/ }));
    await act(async () => {
      fireEvent.keyDown(screen.getByRole('tab', { name: /Models/ }), {
        key: 'ArrowUp',
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole('tab', { name: /About/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });
});
