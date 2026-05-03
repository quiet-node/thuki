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
import {
  clearEventHandlers,
  emitTauriEvent,
} from '../../testUtils/mocks/tauri';

import { ModelTab } from './ModelTab';
import { DisplayTab } from './DisplayTab';
import { SearchTab } from './SearchTab';
import { AboutTab } from './AboutTab';
import type { RawAppConfig } from '../types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const CONFIG: RawAppConfig = {
  inference: {
    ollama_url: 'http://127.0.0.1:11434',
    keep_warm_inactivity_minutes: 0,
    num_ctx: 16384,
  },
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
  debug: {
    search_trace_enabled: false,
  },
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'get_loaded_model') return Promise.resolve(null);
    return Promise.resolve(CONFIG);
  });
});

afterEach(() => {
  vi.useRealTimers();
  clearEventHandlers();
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

  it('renders the Keep Warm section with Release after input and Unload now button', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Keep Warm')).toBeInTheDocument();
    expect(screen.getByText('Keep active model in VRAM')).toBeInTheDocument();
    expect(screen.getByText('Release after')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Unload now' }),
    ).toBeInTheDocument();
  });

  it('Unload now button invokes evict_model', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(undefined);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Unload now' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('evict_model'));
  });

  it('Unload now button is disabled while ejecting and stays disabled after model unloads', async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(undefined);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    // Flush the get_loaded_model effect so the button becomes enabled.
    await act(async () => {
      await Promise.resolve();
    });
    const btn = screen.getByRole('button', { name: 'Unload now' });
    expect(btn).not.toBeDisabled();
    fireEvent.click(btn);
    expect(btn).toBeDisabled(); // disabled from ejecting state
    // Flush microtasks so evict_model resolves, then backend emits model-evicted.
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      emitTauriEvent('warmup:model-evicted', null);
    });
    act(() => {
      vi.advanceTimersByTime(2500); // ejecting clears
    });
    // Button stays disabled because loadedModel is now null.
    expect(btn).toBeDisabled();
  });

  it('Unload now button resets immediately when evict_model rejects', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      if (cmd === 'evict_model')
        return Promise.reject(new Error('connection refused'));
      return Promise.resolve(undefined);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    const btn = screen.getByRole('button', { name: 'Unload now' });
    expect(btn).not.toBeDisabled();
    fireEvent.click(btn);
    expect(btn).toBeDisabled();
    await act(async () => {
      await Promise.resolve();
    });
    // Ejecting cleared; loadedModel still set (eject failed), button re-enabled.
    expect(btn).not.toBeDisabled();
  });

  it('Unload now button is disabled when no model is loaded in VRAM', async () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
  });

  it('Unload now button is enabled when a model is loaded in VRAM', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(CONFIG);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      screen.getByRole('button', { name: 'Unload now' }),
    ).not.toBeDisabled();
  });

  it('shows VRAM subtitle with model name and dot when a model is loaded', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(CONFIG);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText('llama3.2:3b')).toBeInTheDocument();
    expect(screen.getByTestId('vram-status-dot')).toBeInTheDocument();
  });

  it('hides VRAM subtitle when no model is loaded', async () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
  });

  it('handles get_loaded_model failure gracefully and leaves button disabled', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model')
        return Promise.reject(new Error('network error'));
      return Promise.resolve(CONFIG);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
  });

  it('clears VRAM subtitle and keeps button disabled after successful eject', async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(undefined);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText('llama3.2:3b')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Unload now' }));
    // Flush microtasks so evict_model resolves, then backend emits model-evicted.
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      emitTauriEvent('warmup:model-evicted', null);
    });
    expect(screen.queryByText('llama3.2:3b')).not.toBeInTheDocument();
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
    // Button disabled: ejecting still true (timer not yet fired).
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
    act(() => {
      vi.advanceTimersByTime(2500);
    });
    // After timer: ejecting clears but loadedModel=null keeps button disabled.
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
  });

  it('changing the inactivity minutes input updates its value', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.change(input, { target: { value: '60' } });
    expect((input as HTMLInputElement).value).toBe('60');
  });

  it('non-numeric inactivity input is ignored', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.change(input, { target: { value: '' } });
    expect((input as HTMLInputElement).value).toBe('0');
  });

  it('clamps below-range inactivity input to -1', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.change(input, { target: { value: '-99' } });
    expect((input as HTMLInputElement).value).toBe('-1');
  });

  it('clamps above-range inactivity input to 1440', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    fireEvent.change(input, { target: { value: '9999' } });
    expect((input as HTMLInputElement).value).toBe('1440');
  });

  it('updates VRAM subtitle when warmup:model-loaded event fires', async () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
    act(() => {
      emitTauriEvent('warmup:model-loaded', 'phi3:mini');
    });
    expect(screen.getByText('phi3:mini')).toBeInTheDocument();
    expect(screen.getByTestId('vram-status-dot')).toBeInTheDocument();
  });

  it('clears VRAM subtitle when warmup:model-evicted event fires', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(CONFIG);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText('llama3.2:3b')).toBeInTheDocument();
    act(() => {
      emitTauriEvent('warmup:model-evicted', null);
    });
    expect(screen.queryByText('llama3.2:3b')).not.toBeInTheDocument();
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();
  });

  it('re-queries get_loaded_model when visibilitychange fires and panel is visible', async () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    // Initially no model loaded.
    expect(screen.queryByTestId('vram-status-dot')).not.toBeInTheDocument();

    // Switch mock: now a model is loaded in VRAM.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.resolve('llama3.2:3b');
      return Promise.resolve(CONFIG);
    });

    // Simulate settings panel becoming visible (document.hidden is false in happy-dom).
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await Promise.resolve();
    });

    expect(screen.getByTestId('vram-status-dot')).toBeInTheDocument();
    expect(screen.getByText('llama3.2:3b')).toBeInTheDocument();
  });

  it('handles get_loaded_model failure gracefully on visibilitychange', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_loaded_model') return Promise.reject(new Error('fail'));
      return Promise.resolve(CONFIG);
    });
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });
    // Fires visibilitychange with a rejecting get_loaded_model — covers the .catch path.
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'Unload now' })).toBeDisabled();
  });

  it('skips get_loaded_model when visibilitychange fires while document is hidden', async () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    await act(async () => {
      await Promise.resolve();
    });

    invokeMock.mockClear();

    Object.defineProperty(document, 'hidden', {
      configurable: true,
      get: () => true,
    });

    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await Promise.resolve();
    });

    Object.defineProperty(document, 'hidden', {
      configurable: true,
      get: () => false,
    });

    expect(invokeMock).not.toHaveBeenCalledWith('get_loaded_model');
  });

  it('resyncs inactivity minutes when resyncToken changes', () => {
    const { rerender } = render(
      <ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />,
    );
    const input = screen.getByRole('spinbutton', {
      name: 'Release after N minutes',
    });
    expect((input as HTMLInputElement).value).toBe('0');

    const updatedConfig: RawAppConfig = {
      ...CONFIG,
      inference: { ...CONFIG.inference, keep_warm_inactivity_minutes: 60 },
    };
    rerender(
      <ModelTab config={updatedConfig} resyncToken={1} onSaved={() => {}} />,
    );
    expect((input as HTMLInputElement).value).toBe('60');
  });

  it('renders Context Window section with label, slider, chip, tick marks, and VRAM note', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Context Window')).toBeInTheDocument();
    expect(screen.getByText('Context window')).toBeInTheDocument();
    expect(
      screen.getByRole('slider', { name: 'Context window tokens' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('spinbutton', { name: 'Context window tokens' }),
    ).toBeInTheDocument();
    // Tick marks
    expect(screen.getByText('8K')).toBeInTheDocument();
    expect(screen.getByText('16K')).toBeInTheDocument();
    expect(screen.getByText('1M')).toBeInTheDocument();
    // VRAM note
    expect(
      screen.getByText(
        /doubling the context roughly doubles its memory footprint/,
      ),
    ).toBeInTheDocument();
    // Embedded button opens the tuning doc on GitHub via open_url so the
    // link works inside the Tauri webview (target="_blank" is a no-op here).
    const tuneButton = screen.getByRole('button', {
      name: /how to tune Context Window/i,
    });
    fireEvent.click(tuneButton);
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/blob/main/docs/tuning-context-window.md#the-5-minute-benchmark-recipe',
    });
  });

  it('typing a valid value in the chip and blurring commits it', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: '32768' } });
    fireEvent.blur(chip);
    expect(chip.value).toBe('32768');
  });

  it('typing an invalid value in the chip and blurring reverts to committed value', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: 'abc' } });
    fireEvent.blur(chip);
    expect(chip.value).toBe('16384');
  });

  it('typing a value below CTX_MIN and blurring reverts to committed value', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: '512' } });
    fireEvent.blur(chip);
    expect(chip.value).toBe('16384');
  });

  it('typing a value above CTX_MAX and blurring clamps to CTX_MAX', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: '99999999' } });
    fireEvent.blur(chip);
    expect(chip.value).toBe('1048576');
  });

  it('Enter key in chip commits by blurring', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: '131072' } });
    fireEvent.keyDown(chip, { key: 'Enter' });
    expect(chip.value).toBe('131072');
  });

  it('non-Enter keyDown in chip does not commit', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    fireEvent.change(chip, { target: { value: '32768' } });
    fireEvent.keyDown(chip, { key: 'Tab' });
    // No blur triggered, so the chip still shows the in-progress text.
    expect(chip.value).toBe('32768');
  });

  it('slider onChange updates chip text via posToCtx', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // pos=556 → 2048 * 512^(556/1000) ≈ 64K (65536) with CTX_MAX=1M
    fireEvent.change(slider, { target: { value: '556' } });
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    expect(chip.value).toBe('65536');
  });

  it('slider onMouseUp commits the current slider position', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // pos=444 → 2048 * 512^(444/1000) ≈ 32K (32768) with CTX_MAX=1M
    fireEvent.change(slider, { target: { value: '444' } });
    fireEvent.mouseUp(slider);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    expect(chip.value).toBe('32768');
  });

  it('slider onTouchEnd commits the current slider position', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // pos=667 → 2048 * 512^(667/1000) ≈ 128K (131072) with CTX_MAX=1M
    fireEvent.change(slider, { target: { value: '667' } });
    fireEvent.touchEnd(slider);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    expect(chip.value).toBe('131072');
  });

  it('slider onKeyUp commits when not in a drag sequence', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // No preceding onChange, so ctxDraggingRef is false → onKeyUp commits.
    fireEvent.keyUp(slider);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // No position change yet; committed value stays 16384.
    expect(chip.value).toBe('16384');
  });

  it('slider onKeyUp does not commit when a drag is in progress', () => {
    render(<ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const slider = screen.getByRole('slider', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // onChange sets ctxDraggingRef to true; wrap in act so React flushes the
    // setCtxPos/setCtxChip state updates before the keyUp fires.
    act(() => {
      fireEvent.change(slider, { target: { value: '556' } });
    });
    // onKeyUp while dragging: skips commitCtx, chip still shows intermediate.
    fireEvent.keyUp(slider);
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    // pos=556 → 64K (65536); numCtx unchanged, chip shows the intermediate value.
    expect(chip.value).toBe('65536');
  });

  it('resyncs context window chip and slider when resyncToken changes', () => {
    const { rerender } = render(
      <ModelTab config={CONFIG} resyncToken={0} onSaved={() => {}} />,
    );
    const chip = screen.getByRole('spinbutton', {
      name: 'Context window tokens',
    }) as HTMLInputElement;
    expect(chip.value).toBe('16384');

    const updatedConfig: RawAppConfig = {
      ...CONFIG,
      inference: { ...CONFIG.inference, num_ctx: 65536 },
    };
    rerender(
      <ModelTab config={updatedConfig} resyncToken={1} onSaved={() => {}} />,
    );
    expect(chip.value).toBe('65536');
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
  it('renders Services, Pipeline, Timeouts, and Diagnostics sections', () => {
    render(<SearchTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    expect(screen.getByText('Services')).toBeInTheDocument();
    expect(screen.getByText('Pipeline')).toBeInTheDocument();
    expect(screen.getByText('Timeouts')).toBeInTheDocument();
    expect(screen.getByText('Diagnostics')).toBeInTheDocument();
    expect(screen.getByText('SearXNG URL')).toBeInTheDocument();
    expect(screen.getByText('Router timeout')).toBeInTheDocument();
    expect(screen.getByText('Search trace')).toBeInTheDocument();
  });

  it('renders the search trace toggle in the off state by default', () => {
    render(<SearchTab config={CONFIG} resyncToken={0} onSaved={() => {}} />);
    const toggle = screen.getByRole('switch', { name: 'Enable search trace' });
    expect(toggle).toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-checked', 'false');
  });

  it('renders the search trace toggle in the on state when config is true', () => {
    const configOn: RawAppConfig = {
      ...CONFIG,
      debug: { search_trace_enabled: true },
    };
    render(<SearchTab config={configOn} resyncToken={0} onSaved={() => {}} />);
    const toggle = screen.getByRole('switch', { name: 'Enable search trace' });
    expect(toggle).toHaveAttribute('aria-checked', 'true');
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
