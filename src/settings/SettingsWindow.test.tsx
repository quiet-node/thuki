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
import { SettingsWindow } from './SettingsWindow';
import type { CorruptMarker, RawAppConfig } from './types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const SAMPLE: RawAppConfig = {
  inference: {
    ollama_url: 'http://127.0.0.1:11434',
    keep_warm_inactivity_minutes: 0,
    num_ctx: 16384,
  },
  prompt: { system: '' },
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

  it('renders the four tab labels after config loads', async () => {
    render(<SettingsWindow />);
    await waitFor(() =>
      expect(screen.getByRole('tab', { name: /AI/ })).toBeInTheDocument(),
    );
    expect(screen.getByRole('tab', { name: /Web/ })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Display/ })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /About/ })).toBeInTheDocument();
  });

  it('starts on the AI tab', async () => {
    render(<SettingsWindow />);
    await waitFor(() =>
      expect(screen.getByRole('tab', { name: /AI/ })).toHaveAttribute(
        'aria-selected',
        'true',
      ),
    );
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
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));
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
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));

    const modelTab = screen.getByRole('tab', { name: /AI/ });
    fireEvent.keyDown(modelTab, { key: 'ArrowRight' });
    expect(screen.getByRole('tab', { name: /Web/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });

  it('ArrowLeft wraps to the last tab when starting on the first', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));

    const modelTab = screen.getByRole('tab', { name: /AI/ });
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
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));

    const modelTab = screen.getByRole('tab', { name: /AI/ });
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
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));

    __mockWindow.setFocus.mockClear();
    fireEvent.keyDown(document, { key: ',', metaKey: true });
    expect(__mockWindow.setFocus).toHaveBeenCalled();
  });

  it('Other keystrokes do not trigger setFocus', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));

    __mockWindow.setFocus.mockClear();
    fireEvent.keyDown(document, { key: ',' }); // no Meta
    fireEvent.keyDown(document, { key: 'a', metaKey: true });
    expect(__mockWindow.setFocus).not.toHaveBeenCalled();
  });

  it('Cmd+W on the document hides the settings window', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));

    __mockWindow.hide.mockClear();
    fireEvent.keyDown(document, { key: 'w', metaKey: true });
    expect(__mockWindow.hide).toHaveBeenCalled();
  });

  it('the close button hides the window instead of quitting', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));
    __mockWindow.hide.mockClear();
    fireEvent.click(screen.getByRole('button', { name: /Close/ }));
    expect(__mockWindow.hide).toHaveBeenCalled();
  });

  it('mousedown on the chrome triggers startDragging when not on an interactive element', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));
    __mockWindow.startDragging.mockClear();
    // Click on the body container itself (not on a button/input).
    const root = screen
      .getByRole('tab', { name: /AI/ })
      .closest('[role="tablist"]')!.parentElement!;
    fireEvent.mouseDown(root, { target: root });
    // The root is a div; not in INTERACTIVE_TAGS, so dragging fires.
    expect(__mockWindow.startDragging).toHaveBeenCalled();
  });

  it('mousedown that originates from an interactive element does NOT trigger drag', async () => {
    render(<SettingsWindow />);
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));
    __mockWindow.startDragging.mockClear();
    fireEvent.mouseDown(screen.getByRole('tab', { name: /AI/ }));
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
    await waitFor(() => screen.getByRole('tab', { name: /AI/ }));
    __mockWindow.startDragging.mockClear();
    const root = screen
      .getByRole('tab', { name: /AI/ })
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
    expect(screen.getByRole('status')).toHaveTextContent('Saved');

    // Second save before pill auto-hides — clearTimeout(savedTimerRef.current) fires.
    fireEvent.click(incBtns()[0]);
    await act(async () => {
      vi.advanceTimersByTime(400);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole('status')).toHaveTextContent('Saved');
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

    expect(screen.getByRole('status')).toHaveTextContent('Saved');

    // After SAVED_PILL_DURATION_MS the pill toggles back to invisible. We
    // don't assert on that visibility here because the underlying class
    // change is verified in components.test (SavedPill).
    await act(async () => {
      vi.advanceTimersByTime(2000);
      await Promise.resolve();
    });
  });
});
